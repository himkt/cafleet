use super::*;
use crate::broker::runtime::{
    self as storage, InteractionEvidenceUpdate, ObservedTransition, PersistenceMode, RunRecord,
    RuntimeEventKind,
};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub(crate) enum WriterBoundary {
    Receive,
    Append,
    Sync,
    Persist,
    Compact,
}
pub(crate) trait WriterHooks: Send + Sync {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> Result<()>;
}
pub(crate) struct InertWriterHooks;
impl WriterHooks for InertWriterHooks {
    fn before(&self, _: WriterBoundary, _: Instant) -> Result<()> {
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub(crate) struct WriterStatus {
    pub pending_count: u64,
    pub pending_evidence: bool,
    pub gap_count: u64,
    pub degraded: bool,
    pub diagnostic: Option<String>,
}
#[derive(Debug)]
pub(crate) struct RetainedTransition {
    pub transition: ObservedTransition,
    pub cause: CafleetError,
}
#[derive(Debug)]
pub(crate) struct RetainedEvidenceUpdate {
    pub update: InteractionEvidenceUpdate,
    pub cause: CafleetError,
}
#[derive(Clone)]
struct Refresh {
    update: InteractionEvidenceUpdate,
    barrier: i64,
    revision: u64,
}
struct Mailbox {
    pending: u64,
    gaps: u64,
    diagnostic: Option<String>,
    last_sequence: i64,
    active: Option<i64>,
    refresh: Option<Refresh>,
    revision: u64,
    refill: bool,
    grant: Option<std::ops::RangeInclusive<i64>>,
    stop: bool,
    deadline: Option<Instant>,
}
impl Mailbox {
    fn failure(&mut self, cause: &CafleetError) {
        self.gaps = self.gaps.checked_add(1).expect("writer gap count overflow");
        self.diagnostic = Some(cause.message().into_owned());
    }
    fn work_deadline(&self) -> Instant {
        let local = Instant::now() + Duration::from_millis(100);
        self.deadline.map_or(local, |until| until.min(local))
    }
    fn success(&mut self) {
        self.diagnostic = None;
    }
}
pub(crate) struct EventWriter {
    sender: Option<mpsc::SyncSender<ObservedTransition>>,
    mailbox: Arc<Mutex<Mailbox>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
struct Pending {
    transition: ObservedTransition,
    synced: bool,
}
impl EventWriter {
    pub(crate) fn start(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
    ) -> Result<Self> {
        Self::start_with_hooks(database_url, run, lock, outbox, Arc::new(InertWriterHooks))
    }
    pub(crate) fn start_with_hooks(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
        hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        let mut conn = crate::db::connect(&database_url)?;
        let paths = validate_run_paths(&conn, &run)?;
        ensure(
            outbox == paths.outbox,
            "writer outbox path differs from recorded path",
        )?;
        let mut spool = Outbox::open_owned(
            &conn,
            &run,
            lock.clone(),
            Instant::now() + Duration::from_secs(1),
        )?;
        ensure(
            std::fs::metadata(&outbox).map_err(io_error)?.len() == 0,
            "new runner outbox must start empty",
        )?;
        spool.set_hooks(hooks.clone());
        let (sender, receiver) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: 0,
            gaps: 0,
            diagnostic: None,
            last_sequence: 0,
            active: None,
            refresh: None,
            revision: 0,
            refill: false,
            grant: None,
            stop: false,
            deadline: None,
        }));
        let observed = mailbox.clone();
        let thread = std::thread::Builder::new()
            .name("cafleet-events".into())
            .spawn(move || {
                let mut queue = VecDeque::<Pending>::new();
                let mut committed = 0;
                let mut compact = false;
                let mut retry = Duration::from_millis(100);
                let mut next = Instant::now();
                loop {
                    {
                        let state = observed.lock().expect("writer mailbox");
                        if state.deadline.is_some_and(|end| Instant::now() >= end) {
                            break;
                        }
                        if state.stop && state.pending == 0 && state.refresh.is_none() && !compact {
                            break;
                        }
                    }
                    if Instant::now() < next {
                        std::thread::sleep(
                            Duration::from_millis(2)
                                .min(next.saturating_duration_since(Instant::now())),
                        );
                        continue;
                    }
                    let result = (|| -> Result<()> {
                        while queue.len() < 64 {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            match hooks.before(WriterBoundary::Receive, until) {
                                Ok(()) => {}
                                Err(CafleetError::DeadlineExceeded) => return Ok(()),
                                Err(cause) => return Err(cause),
                            }
                            check_deadline(Instant::now(), until)?;
                            match receiver.try_recv() {
                                Ok(transition) => queue.push_back(Pending {
                                    transition,
                                    synced: false,
                                }),
                                Err(
                                    mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected,
                                ) => break,
                            }
                        }
                        for pending in &mut queue {
                            if !pending.synced {
                                let until =
                                    observed.lock().expect("writer mailbox").work_deadline();
                                spool.append_synced(&pending.transition, until)?;
                                pending.synced = true;
                            }
                        }
                        let refresh = {
                            let state = observed.lock().expect("writer mailbox");
                            state
                                .refresh
                                .as_ref()
                                .filter(|value| {
                                    value.barrier <= committed
                                        && queue.front().is_none_or(|event| {
                                            event.transition.event_seq > value.barrier
                                        })
                                })
                                .cloned()
                        };
                        if let Some(refresh) = refresh {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            hooks.before(WriterBoundary::Receive, until)?;
                            hooks.before(WriterBoundary::Persist, until)?;
                            storage::update_interaction_evidence(
                                &mut conn,
                                &run.owner(),
                                &refresh.update,
                                refresh.barrier,
                                until,
                            )?;
                            let mut state = observed.lock().expect("writer mailbox");
                            if state
                                .refresh
                                .as_ref()
                                .is_some_and(|current| current.revision == refresh.revision)
                            {
                                state.refresh = None;
                            }
                            state.success();
                        } else if let Some(pending) = queue.front() {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            hooks.before(WriterBoundary::Persist, until)?;
                            with_database_deadline(&mut conn, until, |conn| {
                                lock.validate(conn, &run.owner())?;
                                storage::persist_transition(
                                    conn,
                                    &run.owner(),
                                    &pending.transition,
                                    PersistenceMode::Live,
                                )
                                .map(|_| ())
                            })?;
                            committed = pending.transition.event_seq;
                            compact = true;
                            queue.pop_front();
                            {
                                let mut state = observed.lock().expect("writer mailbox");
                                state.pending =
                                    state.pending.checked_sub(1).expect("accepted writer count");
                                state.success();
                            }
                        }
                        if compact && queue.is_empty() {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            spool.compact_committed(&mut conn, &run.owner(), until)?;
                            compact = false;
                            observed.lock().expect("writer mailbox").success();
                        }
                        let refill = {
                            let state = observed.lock().expect("writer mailbox");
                            state.refill && state.grant.is_none()
                        };
                        if refill {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            let grant = with_database_deadline(&mut conn, until, |conn| {
                                lock.validate(conn, &run.owner())?;
                                storage::reserve_event_sequences(conn, &run.owner())
                            })?;
                            let mut state = observed.lock().expect("writer mailbox");
                            state.grant = Some(grant);
                            state.refill = false;
                            state.success();
                        }
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            retry = Duration::from_millis(100);
                            next = Instant::now();
                        }
                        Err(cause) => {
                            observed.lock().expect("writer mailbox").failure(&cause);
                            next = Instant::now() + retry;
                            retry = (retry * 2).min(Duration::from_secs(2));
                        }
                    }
                    if observed.lock().expect("writer mailbox").pending == 0 {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            })
            .map_err(io_error)?;
        Ok(Self {
            sender: Some(sender),
            mailbox,
            thread: Some(thread),
        })
    }
    pub(crate) fn try_submit(
        &self,
        transition: ObservedTransition,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            ensure(!state.stop, "event writer is stopped")?;
            ensure(
                transition.event_seq > state.last_sequence,
                "accepted event sequence regressed",
            )?;
            crate::broker::runtime::evidence::validate(&transition.evidence_json)?;
            let sender = self
                .sender
                .as_ref()
                .ok_or_else(|| error("event writer is stopped"))?;
            match sender.try_send(transition.clone()) {
                Ok(()) => Ok(()),
                Err(_) => Err(error("runtime observation channel is full or unavailable")),
            }
        })();
        match result {
            Ok(()) => {
                state.pending = state
                    .pending
                    .checked_add(1)
                    .expect("writer pending overflow");
                assert!(state.pending <= 128, "bounded accepted writer capacity");
                state.last_sequence = transition.event_seq;
                match transition.kind {
                    RuntimeEventKind::InteractionOpened => state.active = transition.episode_seq,
                    RuntimeEventKind::InteractionClosed | RuntimeEventKind::RunnerExited => {
                        state.active = None
                    }
                    _ => {}
                }
                Ok(())
            }
            Err(cause) => {
                state.failure(&cause);
                Err(Box::new(RetainedTransition { transition, cause }))
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn submit(&self, transition: ObservedTransition) -> Result<()> {
        self.try_submit(transition)
            .map_err(|retained| retained.cause)
    }
    pub(crate) fn try_refresh(
        &self,
        update: InteractionEvidenceUpdate,
        until: Instant,
    ) -> std::result::Result<(), Box<RetainedEvidenceUpdate>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            check_deadline(Instant::now(), until)?;
            ensure(
                !state.stop && state.active == Some(update.episode_seq) && state.last_sequence > 0,
                "refresh requires current accepted open episode",
            )?;
            crate::broker::runtime::evidence::validate(&update.evidence_json)?;
            ensure(
                update.output_seq
                    == update.evidence_json["screen_seq"]
                        .as_u64()
                        .ok_or_else(|| error("refresh sequence required"))?,
                "refresh sequence mismatch",
            )?;
            if let Some(old) = &state.refresh {
                ensure(
                    old.update.run_id == update.run_id
                        && old.update.episode_seq == update.episode_seq,
                    "refresh episode collision",
                )?;
                ensure(
                    update.output_seq > old.update.output_seq || (update == old.update),
                    "refresh revision regressed or conflicts",
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .expect("refresh revision overflow");
                state.refresh = Some(Refresh {
                    update,
                    barrier: state.last_sequence,
                    revision: state.revision,
                });
                Ok(())
            }
            Err(cause) => {
                state.failure(&cause);
                Err(Box::new(RetainedEvidenceUpdate { update, cause }))
            }
        }
    }
    pub(crate) fn request_sequence_range(&self) -> Result<()> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        ensure(!state.stop, "writer stopped before sequence refill")?;
        if state.grant.is_none() {
            state.refill = true;
        }
        Ok(())
    }
    pub(crate) fn take_sequence_range(&self) -> Option<std::ops::RangeInclusive<i64>> {
        self.mailbox.lock().expect("writer mailbox").grant.take()
    }
    pub(crate) fn status(&self) -> WriterStatus {
        let state = self.mailbox.lock().expect("writer mailbox");
        WriterStatus {
            pending_count: state.pending,
            pending_evidence: state.refresh.is_some(),
            gap_count: state.gaps,
            degraded: state.diagnostic.is_some(),
            diagnostic: state.diagnostic.clone(),
        }
    }
    pub(crate) fn diagnostic(&self) -> Option<String> {
        self.status().diagnostic
    }
    pub(crate) fn set_deadline(&mut self, until: Instant) {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.deadline = Some(state.deadline.map_or(until, |old| old.min(until)));
    }
    pub(crate) fn begin_shutdown(&mut self, until: Instant) {
        self.set_deadline(until);
        self.mailbox.lock().expect("writer mailbox").stop = true;
        self.sender.take();
    }
    pub(crate) fn finish(&mut self, until: Instant) -> (u64, u64) {
        self.begin_shutdown(until);
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline
            .expect("shutdown deadline");
        if let Some(thread) = self.thread.as_ref() {
            while !thread.is_finished() && Instant::now() < until {
                std::thread::sleep(
                    Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
                );
            }
        }
        if self
            .thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            self.thread
                .take()
                .expect("writer thread")
                .join()
                .expect("writer processing is panic-free");
        }
        let mut state = self.mailbox.lock().expect("writer mailbox");
        if self.thread.is_some() && state.diagnostic.is_none() {
            state.failure(&CafleetError::DeadlineExceeded);
        }
        let unsaved = if state.diagnostic.is_some() {
            state.gaps
        } else {
            0
        };
        (
            state.pending,
            unsaved
                .checked_add(u64::from(state.refresh.is_some()))
                .expect("unsaved writer observation count overflow"),
        )
    }
}
impl Drop for EventWriter {
    fn drop(&mut self) {
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(4));
        self.finish(until);
    }
}

#[cfg(test)]
mod contract_tests;
