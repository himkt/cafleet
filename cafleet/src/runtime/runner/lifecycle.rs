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

#[cfg(test)]
mod tests {
    use super::*;
    mod shutdown_tests {
        use std::collections::BTreeSet;
        use std::result::Result;
        use std::time::{Duration, Instant};

        use super::*;
        use crate::error::CafleetError;

        #[test]
        fn runner_recovery_clock_clips_each_original_boundary_and_never_renews_time() {
            let start = Instant::now();
            for offset in [0, 1, 2, 3, 4, 9] {
                let until = start + Duration::from_secs(offset);
                let clock = ShutdownClock::new(start).clipped(until);
                assert_eq!(
                    clock.child_deadline(),
                    (start + Duration::from_secs(2)).min(until)
                );
                assert_eq!(
                    clock.display_deadline(),
                    (start + Duration::from_secs(3)).min(until)
                );
                assert_eq!(
                    clock.storage_deadline(),
                    (start + Duration::from_secs(4)).min(until)
                );
                assert_eq!(clock.remaining_storage_at(until), Duration::ZERO);
            }
            let past = start.checked_sub(Duration::from_secs(1)).unwrap();
            let clock = ShutdownClock::new(start).clipped(past);
            assert_eq!(clock.child_deadline(), past);
            assert_eq!(clock.display_deadline(), past);
            assert_eq!(clock.storage_deadline(), past);
            assert_eq!(clock.remaining_storage_at(start), Duration::ZERO);
        }

        #[test]
        fn runner_recovery_clock_repeated_clipping_only_tightens_the_same_original_schedule() {
            let start = Instant::now();
            let first = start + Duration::from_millis(2500);
            let second = start + Duration::from_millis(1500);
            let clock = ShutdownClock::new(start)
                .clipped(first)
                .clipped(start + Duration::from_secs(20));
            assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
            assert_eq!(clock.display_deadline(), first);
            assert_eq!(clock.storage_deadline(), first);
            let clock = clock.clipped(second).clipped(first);
            assert_eq!(clock.child_deadline(), second);
            assert_eq!(clock.display_deadline(), second);
            assert_eq!(clock.storage_deadline(), second);
        }

        struct Host {
            now: Instant,
            calls: Vec<(&'static str, Instant)>,
            deadlines: Vec<(&'static str, Instant)>,
            fail: BTreeSet<&'static str>,
            native: Option<ChildExit>,
            dropped: u64,
            pending: u64,
            gap: u64,
        }

        impl Host {
            fn new(now: Instant) -> Self {
                Self {
                    now,
                    calls: Vec::new(),
                    deadlines: Vec::new(),
                    fail: BTreeSet::new(),
                    native: Some(ChildExit::Code(0)),
                    dropped: 0,
                    pending: 0,
                    gap: 0,
                }
            }
            fn call(&mut self, name: &'static str) -> Result<(), CafleetError> {
                self.calls.push((name, self.now));
                if self.fail.contains(name) {
                    Err(CafleetError::App(format!("fixture {name} failure")))
                } else {
                    Ok(())
                }
            }
        }

        impl ShutdownHost for Host {
            fn now(&self) -> Instant {
                self.now
            }
            fn gate_programmatic_input(&mut self) {
                self.calls.push(("gate", self.now));
            }
            fn term_verified_descendants(&mut self) -> Result<(), CafleetError> {
                self.call("term")
            }
            fn drain_and_reap_until(
                &mut self,
                deadline: Instant,
            ) -> Result<Option<ChildExit>, CafleetError> {
                self.deadlines.push(("drain", deadline));
                self.now = self.now.max(deadline);
                self.call("drain")?;
                Ok(self.native)
            }
            fn kill_verified_survivors(&mut self) -> Result<(), CafleetError> {
                self.call("kill")
            }
            fn close_pty(&mut self) -> Result<(), CafleetError> {
                self.call("close")
            }
            fn flush_display_until(&mut self, deadline: Instant) -> Result<u64, CafleetError> {
                self.deadlines.push(("display", deadline));
                self.now = self.now.max(deadline);
                self.call("display")?;
                Ok(self.dropped)
            }
            fn finish_events_until(
                &mut self,
                deadline: Instant,
            ) -> Result<(u64, u64), CafleetError> {
                self.deadlines.push(("events", deadline));
                self.now = self.now.max(deadline);
                self.call("events")?;
                Ok((self.pending, self.gap))
            }
            fn restore_terminal(&mut self) -> Result<(), CafleetError> {
                self.call("restore")
            }
        }

        #[test]
        fn runner_shutdown_uses_one_clock_for_descendants_display_events_and_restoration() {
            let start = Instant::now();
            let mut host = Host::new(start);
            host.native = None;
            let outcome = shutdown(
                &mut host,
                ShutdownClock::new(start),
                Some(ChildExit::Code(7)),
            );
            assert_eq!(
                host.calls.iter().map(|(call, _)| *call).collect::<Vec<_>>(),
                [
                    "gate", "term", "drain", "kill", "close", "display", "events", "restore"
                ]
            );
            assert_eq!(
                host.deadlines,
                [
                    ("drain", start + Duration::from_secs(2)),
                    ("display", start + Duration::from_secs(3)),
                    ("events", start + Duration::from_secs(4))
                ]
            );
            for (name, time) in &host.calls {
                if ["kill", "close"].contains(name) {
                    assert_eq!(*time, start + Duration::from_secs(2));
                }
            }
            assert_eq!(outcome.native_exit, Some(ChildExit::Code(7)));
            assert_eq!(outcome.cleanup_state, CleanupState::Complete);
        }

        #[test]
        fn runner_shutdown_collects_each_failure_and_still_restores_terminal_without_an_extra_budget()
         {
            let start = Instant::now();
            let mut host = Host::new(start);
            host.native = None;
            host.fail = [
                "term", "drain", "kill", "close", "display", "events", "restore",
            ]
            .into();
            let outcome = shutdown(
                &mut host,
                ShutdownClock::new(start),
                Some(ChildExit::Code(9)),
            );
            assert_eq!(host.calls.last().unwrap().0, "restore");
            assert_eq!(host.now, start + Duration::from_secs(4));
            assert_eq!(outcome.native_exit, Some(ChildExit::Code(9)));
            assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
            for failure in &host.fail {
                assert!(
                    outcome
                        .diagnostics
                        .iter()
                        .any(|line| line.contains(failure)),
                    "{failure}"
                );
            }
        }

        #[test]
        fn runner_shutdown_retains_display_truncation_pending_frames_and_unsaved_gaps_separately() {
            let start = Instant::now();
            let mut host = Host::new(start);
            host.dropped = 17;
            host.pending = 2;
            host.gap = 3;
            let outcome = shutdown(&mut host, ShutdownClock::new(start), None);
            assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
            assert_eq!(outcome.dropped_display_bytes, 17);
            assert_eq!(outcome.pending_event_frames, 2);
            assert_eq!(outcome.unsaved_observation_gap, 3);
            assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
            assert!(!outcome.diagnostics.is_empty());
        }

        #[test]
        fn runner_shutdown_never_rebases_a_clock_that_was_started_before_the_current_phase() {
            let start = Instant::now();
            let mut host = Host::new(start + Duration::from_secs(3));
            host.native = None;
            let outcome = shutdown(
                &mut host,
                ShutdownClock::new(start),
                Some(ChildExit::Signal(15)),
            );
            assert_eq!(host.deadlines[0].1, start + Duration::from_secs(2));
            assert_eq!(host.deadlines[2].1, start + Duration::from_secs(4));
            assert_eq!(host.now, start + Duration::from_secs(4));
            assert_eq!(outcome.native_exit, Some(ChildExit::Signal(15)));
        }
    }
    mod lifecycle_tests {
        use std::result::Result;
        use std::time::{Duration, Instant};

        use super::*;
        use crate::error::CafleetError;
        use crate::runtime::runner::test_support::Fixture;

        #[test]
        fn runner_shutdown_clock_anchors_all_work_to_one_original_instant() {
            let start = Instant::now();
            let clock = ShutdownClock::new(start);
            assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
            assert_eq!(clock.display_deadline(), start + Duration::from_secs(3));
            assert_eq!(clock.storage_deadline(), start + Duration::from_secs(4));
            for elapsed in [0, 1, 2, 3, 4, 9] {
                let now = start + Duration::from_secs(elapsed);
                assert_eq!(
                    clock.remaining_storage_at(now),
                    Duration::from_secs(4).saturating_sub(Duration::from_secs(elapsed))
                );
                assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
            }
        }

        #[test]
        fn runner_exit_status_keeps_native_code_signal_and_cleanup_outcome_distinct() {
            for (native, complete, expected) in [
                (Some(ChildExit::Code(0)), true, 0),
                (Some(ChildExit::Code(7)), true, 7),
                (Some(ChildExit::Signal(15)), true, 143),
                (Some(ChildExit::Code(0)), false, 1),
                (Some(ChildExit::Code(7)), false, 7),
                (Some(ChildExit::Signal(15)), false, 143),
                (None, false, 1),
            ] {
                assert_eq!(runner_exit_status(native, complete).unwrap(), expected);
            }
            assert!(runner_exit_status(None, true).is_err());
        }

        #[test]
        fn runner_database_deadline_bounds_a_locked_write_and_restores_the_busy_timeout() {
            let mut fixture = Fixture::new();
            fixture
                .conn
                .execute_batch("CREATE TABLE deadline_fixture(value INTEGER)")
                .unwrap();
            fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            let other =
                crate::db::connect(&format!("sqlite:///{}", fixture.conn.path().unwrap())).unwrap();
            other.execute_batch("BEGIN IMMEDIATE").unwrap();
            let start = Instant::now();
            let result = with_database_deadline(
                &mut fixture.conn,
                start + Duration::from_millis(80),
                |conn| {
                    conn.execute("INSERT INTO deadline_fixture VALUES(1)", [])
                        .map(|_| ())
                        .map_err(|error| CafleetError::App(error.to_string()))
                },
            );
            assert!(result.is_err());
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(
                fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                137
            );
            other.execute_batch("ROLLBACK").unwrap();
            fixture
                .conn
                .execute("INSERT INTO deadline_fixture VALUES(2)", [])
                .unwrap();
        }

        #[test]
        fn runner_database_deadline_cancels_a_long_statement_and_removes_its_progress_handler() {
            let mut fixture = Fixture::new();
            let start = Instant::now();
            let result = with_database_deadline(
                &mut fixture.conn,
                start + Duration::from_millis(80),
                |conn| {
                    conn.query_row("WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",[],|row|row.get::<_,i64>(0)).map_err(|error|CafleetError::App(error.to_string()))
                },
            );
            assert!(result.is_err());
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(fixture.conn.query_row("WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",[],|row|row.get::<_,i64>(0)).unwrap(),50005000);
            assert_eq!(
                fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                5000
            );
        }

        #[test]
        fn runner_database_deadline_preserves_an_already_expired_shared_budget() {
            let mut fixture = Fixture::new();
            let mut invoked = false;
            let result = with_database_deadline(
                &mut fixture.conn,
                Instant::now() - Duration::from_secs(1),
                |_| {
                    invoked = true;
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert!(!invoked);
        }

        #[derive(Debug, PartialEq)]
        struct DeadlinePanic(u32);

        fn progress_checked_sum(conn: &Connection) -> Result<i64, CafleetError> {
            conn.query_row(
            "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",
            [],
            |row| row.get(0),
        )
        .map_err(|cause| CafleetError::App(cause.to_string()))
        }

        #[test]
        fn runner_database_deadline_review_unwind_preserves_the_panic_and_restores_connection_reuse()
         {
            let mut fixture = Fixture::new();
            fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            let until = Instant::now() + Duration::from_millis(40);
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
                    std::panic::panic_any(DeadlinePanic(194));
                })
            }));
            let payload = match unwound {
                Err(payload) => payload,
                Ok(_) => panic!("the original operation panic must propagate"),
            };
            assert_eq!(
                payload.downcast_ref::<DeadlinePanic>(),
                Some(&DeadlinePanic(194))
            );
            std::thread::sleep(
                until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
            assert_eq!(
                fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                137
            );
        }

        #[test]
        fn runner_database_deadline_review_operation_error_stops_its_watchdog_and_restores_reuse() {
            let mut fixture = Fixture::new();
            fixture
                .conn
                .busy_timeout(Duration::from_millis(173))
                .unwrap();
            let until = Instant::now() + Duration::from_millis(100);
            let started = Instant::now();
            let error =
                with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
                    Err(CafleetError::Usage(
                        "fixture deadline operation error".into(),
                    ))
                })
                .unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert_eq!(error.message(), "fixture deadline operation error");
            assert!(started.elapsed() < Duration::from_millis(750));
            std::thread::sleep(
                until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
            assert_eq!(
                fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                173
            );
        }

        #[test]
        fn runner_database_deadline_review_inner_unwind_restores_the_live_outer_cancellation_scope()
        {
            let mut fixture = Fixture::new();
            let started = Instant::now();
            let outer_until = started + Duration::from_millis(400);
            let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
                let inner_until = Instant::now() + Duration::from_millis(40);
                let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    with_database_deadline(conn, inner_until, |_| -> Result<(), CafleetError> {
                        std::panic::panic_any(DeadlinePanic(195));
                    })
                }));
                let payload = match unwound {
                    Err(payload) => payload,
                    Ok(_) => panic!("the inner operation panic must propagate"),
                };
                assert_eq!(
                    payload.downcast_ref::<DeadlinePanic>(),
                    Some(&DeadlinePanic(195))
                );
                std::thread::sleep(
                    inner_until.saturating_duration_since(Instant::now())
                        + Duration::from_millis(20),
                );
                assert_eq!(progress_checked_sum(conn)?, 50005000);
                conn.query_row(
                "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|cause| CafleetError::App(cause.to_string()))
            });
            assert!(result.is_err());
            assert!(
                started.elapsed() >= Duration::from_millis(350),
                "{result:?}"
            );
            assert!(started.elapsed() < Duration::from_millis(1250));
            assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
        }

        #[test]
        fn runner_database_deadline_review_nested_later_deadline_keeps_the_earliest_outer_budget() {
            let mut fixture = Fixture::new();
            let started = Instant::now();
            let outer_until = started + Duration::from_millis(100);
            let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
                with_database_deadline(conn, started + Duration::from_secs(1), |conn| {
                    conn.query_row(
                    "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|cause| CafleetError::App(cause.to_string()))
                })
            });
            assert!(result.is_err());
            assert!(started.elapsed() >= Duration::from_millis(70));
            assert!(started.elapsed() < Duration::from_millis(750));
            assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
        }

        fn assert_retired_inner_scope_preserves_outer(operation_fails: bool) {
            let mut fixture = Fixture::new();
            let started = Instant::now();
            let outer_until = started + Duration::from_millis(400);
            let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
                let inner_until = Instant::now() + Duration::from_millis(40);
                let inner = with_database_deadline(conn, inner_until, |_| {
                    if operation_fails {
                        Err(CafleetError::Usage("fixture inner operation error".into()))
                    } else {
                        Ok(7)
                    }
                });
                if operation_fails {
                    let error = inner.unwrap_err();
                    assert_eq!(error.exit_code(), 2);
                    assert_eq!(error.message(), "fixture inner operation error");
                } else {
                    assert_eq!(inner.unwrap(), 7);
                }
                std::thread::sleep(
                    inner_until.saturating_duration_since(Instant::now())
                        + Duration::from_millis(20),
                );
                assert_eq!(progress_checked_sum(conn)?, 50005000);
                conn.query_row(
                "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|cause| CafleetError::App(cause.to_string()))
            });
            assert!(result.is_err());
            assert!(
                started.elapsed() >= Duration::from_millis(350),
                "{result:?}"
            );
            assert!(started.elapsed() < Duration::from_millis(1250));
            assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
        }

        #[test]
        fn runner_database_deadline_review_inner_success_restores_the_live_outer_scope() {
            assert_retired_inner_scope_preserves_outer(false);
        }

        #[test]
        fn runner_database_deadline_review_inner_error_restores_the_live_outer_scope() {
            assert_retired_inner_scope_preserves_outer(true);
        }
    }
}
