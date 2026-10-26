use super::*;
use crate::broker::runtime::CleanupState;
use crate::input::{Deadline, with_database_budget};
use rusqlite::Connection;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ChildExit {
    Code(i32),
    Signal(i32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShutdownClock {
    start: Instant,
    limit: Option<Instant>,
}

impl ShutdownClock {
    pub(crate) fn new(start: Instant) -> Self {
        Self { start, limit: None }
    }
    pub(crate) fn clipped(self, deadline: Instant) -> Self {
        Self {
            start: self.start,
            limit: Some(self.limit.map_or(deadline, |prior| prior.min(deadline))),
        }
    }
    fn boundary(&self, seconds: u64) -> Instant {
        let original = self.start + Duration::from_secs(seconds);
        self.limit.map_or(original, |limit| original.min(limit))
    }
    pub(crate) fn child_deadline(&self) -> Instant {
        self.boundary(2)
    }
    pub(crate) fn display_deadline(&self) -> Instant {
        self.boundary(3)
    }
    pub(crate) fn storage_deadline(&self) -> Instant {
        self.boundary(4)
    }
    pub(crate) fn remaining_storage_at(&self, now: Instant) -> Duration {
        self.storage_deadline().saturating_duration_since(now)
    }
}

pub(crate) fn runner_exit_status(native: Option<ChildExit>, complete: bool) -> Result<i32> {
    let status = match native {
        Some(ChildExit::Code(code)) => {
            ensure(code >= 0, "negative native exit code")?;
            code
        }
        Some(ChildExit::Signal(signal)) => {
            ensure(signal > 0, "positive native signal required")?;
            128i32
                .checked_add(signal)
                .ok_or_else(|| error("native exit signal overflow"))?
        }
        None => {
            ensure(!complete, "complete cleanup requires actual native status")?;
            1
        }
    };
    Ok(if !complete && status == 0 { 1 } else { status })
}

struct CancellationScopes {
    identity: Option<usize>,
    deadlines: Vec<Instant>,
}

thread_local! {
    static CANCELLATION_SCOPES: std::cell::RefCell<CancellationScopes> = const {
        std::cell::RefCell::new(CancellationScopes { identity: None, deadlines: Vec::new() })
    };
}

fn database_cancelled() -> bool {
    CANCELLATION_SCOPES.with(|scopes| {
        let scopes = scopes.borrow();
        let until = scopes
            .deadlines
            .iter()
            .min()
            .expect("a progress handler has an owning scope");
        Instant::now() >= *until
    })
}

struct CancellationScope<'a> {
    conn: &'a mut Connection,
    identity: usize,
    until: Instant,
    stop: Option<std::sync::mpsc::Sender<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    active: bool,
}

impl<'a> CancellationScope<'a> {
    fn new(conn: &'a mut Connection) -> Result<Self> {
        let until = Deadline::inherited_or_new().instant();
        let identity = std::ptr::from_ref(conn).addr();
        CANCELLATION_SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            if let Some(owner) = scopes.identity {
                assert_eq!(
                    owner, identity,
                    "nested cancellation scopes use the same connection"
                );
            }
            scopes.identity = Some(identity);
            scopes.deadlines.push(until);
        });
        let mut scope = Self {
            conn,
            identity,
            until,
            stop: None,
            watchdog: None,
            active: true,
        };
        scope
            .conn
            .progress_handler(1000, Some(database_cancelled))
            .map_err(crate::broker::members::db_err)?;
        let interrupt = scope.conn.get_interrupt_handle();
        let (stop, receive) = std::sync::mpsc::channel();
        scope.stop = Some(stop);
        scope.watchdog = Some(
            std::thread::Builder::new()
                .name("cafleet-sql-deadline".into())
                .spawn(move || {
                    if matches!(
                        receive.recv_timeout(until.saturating_duration_since(Instant::now())),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        interrupt.interrupt();
                    }
                })
                .map_err(io_error)?,
        );
        Ok(scope)
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let watchdog = self
            .watchdog
            .take()
            .map(|worker| worker.join())
            .transpose()
            .map_err(|_| error("SQLite deadline watchdog panicked"));
        let nested = CANCELLATION_SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            assert_eq!(scopes.identity, Some(self.identity));
            assert_eq!(scopes.deadlines.pop(), Some(self.until));
            let nested = !scopes.deadlines.is_empty();
            if !nested {
                scopes.identity = None;
            }
            nested
        });
        self.active = false;
        let restored = if nested {
            self.conn.progress_handler(1000, Some(database_cancelled))
        } else {
            self.conn.progress_handler(0, None::<fn() -> bool>)
        }
        .map_err(crate::broker::members::db_err);
        match (watchdog, restored) {
            (Ok(_), Ok(())) => Ok(()),
            (Err(primary), Err(cleanup)) => Err(primary.with_cleanup(cleanup.message())),
            (Err(cause), _) | (_, Err(cause)) => Err(cause),
        }
    }
}

impl Drop for CancellationScope<'_> {
    fn drop(&mut self) {
        if let Err(cause) = self.restore() {
            if std::thread::panicking() {
                eprintln!("SQLite cancellation cleanup failed during unwind: {cause}");
            } else {
                panic!("SQLite cancellation cleanup failed: {cause}");
            }
        }
    }
}

pub(crate) fn with_database_deadline<T>(
    conn: &mut Connection,
    until: Instant,
    operation: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    let deadline = Deadline::until(until);
    with_database_budget(conn, deadline, |conn| {
        let mut scope = CancellationScope::new(conn)?;
        let result = operation(scope.conn);
        let restored = scope.restore();
        match (result, restored) {
            (Ok(value), Ok(())) => {
                deadline.remaining()?;
                Ok(value)
            }
            (Err(primary), Err(cleanup)) => Err(primary.with_cleanup(cleanup.message())),
            (Err(cause), _) | (_, Err(cause)) => Err(cause),
        }
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ShutdownOutcome {
    pub native_exit: Option<ChildExit>,
    pub cleanup_state: CleanupState,
    pub diagnostics: Vec<String>,
    pub dropped_display_bytes: u64,
    pub pending_event_frames: u64,
    pub unsaved_observation_gap: u64,
}

pub(crate) trait ShutdownHost {
    fn now(&self) -> Instant;
    fn gate_programmatic_input(&mut self);
    fn term_verified_descendants(&mut self) -> Result<()>;
    fn drain_and_reap_until(&mut self, deadline: Instant) -> Result<Option<ChildExit>>;
    fn kill_verified_survivors(&mut self) -> Result<()>;
    fn close_pty(&mut self) -> Result<()>;
    fn final_native_exit(&mut self) -> Result<Option<ChildExit>> {
        Ok(None)
    }
    fn native_was_spawned(&self) -> bool {
        true
    }
    fn flush_display_until(&mut self, deadline: Instant) -> Result<u64>;
    fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64)>;
    fn restore_terminal(&mut self) -> Result<()>;
}

pub(crate) fn shutdown(
    host: &mut impl ShutdownHost,
    clock: ShutdownClock,
    native: Option<ChildExit>,
) -> ShutdownOutcome {
    let mut outcome = ShutdownOutcome {
        native_exit: native,
        cleanup_state: CleanupState::Complete,
        diagnostics: Vec::new(),
        dropped_display_bytes: 0,
        pending_event_frames: 0,
        unsaved_observation_gap: 0,
    };
    fn collect<T>(result: Result<T>, outcome: &mut ShutdownOutcome) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(cause) => {
                outcome.diagnostics.push(cause.message().into_owned());
                None
            }
        }
    }
    host.gate_programmatic_input();
    collect(host.term_verified_descendants(), &mut outcome);
    if let Some(exit) = collect(
        host.drain_and_reap_until(clock.child_deadline()),
        &mut outcome,
    )
    .flatten()
    {
        if let Some(recorded) = outcome.native_exit {
            if recorded != exit {
                outcome
                    .diagnostics
                    .push("conflicting native exit status".into());
            }
        } else {
            outcome.native_exit = Some(exit);
        }
    }
    collect(host.kill_verified_survivors(), &mut outcome);
    collect(host.close_pty(), &mut outcome);
    if let Some(exit) = collect(host.final_native_exit(), &mut outcome).flatten() {
        if let Some(recorded) = outcome.native_exit {
            if recorded != exit {
                outcome
                    .diagnostics
                    .push("conflicting final native exit status".into());
            }
        } else {
            outcome.native_exit = Some(exit);
        }
    }
    if let Some(dropped) = collect(
        host.flush_display_until(clock.display_deadline()),
        &mut outcome,
    ) {
        outcome.dropped_display_bytes = dropped;
    }
    if let Some((pending, gap)) = collect(
        host.finish_events_until(clock.storage_deadline()),
        &mut outcome,
    ) {
        outcome.pending_event_frames = pending;
        outcome.unsaved_observation_gap = gap;
    }
    collect(host.restore_terminal(), &mut outcome);
    if host.now() > clock.storage_deadline() {
        outcome
            .diagnostics
            .push("shutdown exceeded its original storage deadline".into());
    }
    if outcome.dropped_display_bytes > 0 {
        outcome.diagnostics.push(format!(
            "display truncated: {} undelivered bytes",
            outcome.dropped_display_bytes
        ));
    }
    if outcome.pending_event_frames > 0 || outcome.unsaved_observation_gap > 0 {
        outcome.diagnostics.push(format!(
            "pending events={} unsaved observation gap={}",
            outcome.pending_event_frames, outcome.unsaved_observation_gap
        ));
    }
    if outcome.native_exit.is_none() && host.native_was_spawned() {
        outcome
            .diagnostics
            .push("native exit remains unconfirmed".into());
    }
    if !outcome.diagnostics.is_empty() {
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    outcome
}
