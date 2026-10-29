use super::*;
use crate::broker::runtime::{
    self as storage, EffectivePolicyObservation, InteractionEvidenceUpdate, ObservedTransition,
    PersistenceMode, RunRecord, RuntimeEventKind,
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
    #[cfg(test)]
    pub policy_diagnostic: Option<String>,
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
    policies: u64,
    #[cfg(test)]
    policy_diagnostic: Option<String>,
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
    // Compaction validates every retained frame in one attempt and each retry repeats that
    // whole pass, so it gets a budget wider than one work slice; otherwise an outbox whose
    // validation outgrows the slice could never compact.
    fn compaction_deadline(&self) -> Instant {
        let local = Instant::now() + Duration::from_secs(1);
        self.deadline.map_or(local, |until| until.min(local))
    }
    fn success(&mut self) {
        self.diagnostic = None;
    }
    fn policy_dropped(&mut self, cause: &CafleetError) {
        eprintln!("member runtime: effective policy update dropped: {cause}");
        #[cfg(test)]
        {
            self.policy_diagnostic = Some(cause.message().into_owned());
        }
    }
}
pub(crate) struct EventWriter {
    sender: Option<mpsc::SyncSender<Work>>,
    mailbox: Arc<Mutex<Mailbox>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
enum Work {
    Event {
        transition: Box<ObservedTransition>,
        effective_policy: Option<EffectivePolicyObservation>,
    },
    Policy(EffectivePolicyObservation),
}
struct Pending {
    work: Work,
    synced: bool,
}
impl Pending {
    fn event_seq(&self) -> Option<i64> {
        match &self.work {
            Work::Event { transition, .. } => Some(transition.event_seq),
            Work::Policy(_) => None,
        }
    }
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
            policies: 0,
            #[cfg(test)]
            policy_diagnostic: None,
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
                        if state.stop
                            && state.pending == 0
                            && state.policies == 0
                            && state.refresh.is_none()
                            && !compact
                        {
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
                                Ok(work) => queue.push_back(Pending {
                                    synced: matches!(work, Work::Policy(_)),
                                    work,
                                }),
                                Err(
                                    mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected,
                                ) => break,
                            }
                        }
                        for pending in &mut queue {
                            if !pending.synced {
                                let Work::Event {
                                    transition,
                                    effective_policy,
                                } = &pending.work
                                else {
                                    unreachable!("effective policy updates are never spooled")
                                };
                                let until =
                                    observed.lock().expect("writer mailbox").work_deadline();
                                spool.append_synced_with_policy(
                                    transition,
                                    effective_policy.as_ref(),
                                    until,
                                )?;
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
                                        && queue.front().is_none_or(|pending| {
                                            pending
                                                .event_seq()
                                                .is_none_or(|sequence| sequence > value.barrier)
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
                            match &pending.work {
                                Work::Event {
                                    transition,
                                    effective_policy,
                                } => {
                                    hooks.before(WriterBoundary::Persist, until)?;
                                    with_database_deadline(&mut conn, until, |conn| {
                                        lock.validate(conn, &run.owner())?;
                                        match effective_policy {
                                            Some(observation) => {
                                                storage::persist_transition_with_effective_policy(
                                                    conn,
                                                    &run.owner(),
                                                    transition,
                                                    observation,
                                                    PersistenceMode::Live,
                                                )
                                            }
                                            None => storage::persist_transition(
                                                conn,
                                                &run.owner(),
                                                transition,
                                                PersistenceMode::Live,
                                            ),
                                        }
                                        .map(|_| ())
                                    })?;
                                    committed = transition.event_seq;
                                    compact = true;
                                    queue.pop_front();
                                    let mut state = observed.lock().expect("writer mailbox");
                                    state.pending = state
                                        .pending
                                        .checked_sub(1)
                                        .expect("accepted writer count");
                                    state.success();
                                }
                                Work::Policy(observation) => {
                                    let outcome = hooks
                                        .before(WriterBoundary::Persist, until)
                                        .and_then(|()| lock.validate(&conn, &run.owner()))
                                        .and_then(|()| {
                                            storage::record_effective_policy(
                                                &mut conn,
                                                &run.owner(),
                                                observation,
                                                until,
                                            )
                                        });
                                    queue.pop_front();
                                    let mut state = observed.lock().expect("writer mailbox");
                                    state.policies = state
                                        .policies
                                        .checked_sub(1)
                                        .expect("accepted policy update count");
                                    if let Err(cause) = outcome {
                                        state.policy_dropped(&cause);
                                    }
                                }
                            }
                        }
                        if compact && queue.is_empty() {
                            let until = observed
                                .lock()
                                .expect("writer mailbox")
                                .compaction_deadline();
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
    #[cfg(test)]
    pub(crate) fn try_submit(
        &self,
        transition: ObservedTransition,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        self.try_submit_with_policy(transition, None)
    }
    pub(crate) fn try_submit_with_policy(
        &self,
        transition: ObservedTransition,
        effective_policy: Option<EffectivePolicyObservation>,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            ensure(!state.stop, "event writer is stopped")?;
            ensure(
                transition.event_seq > state.last_sequence,
                "accepted event sequence regressed",
            )?;
            ensure(
                effective_policy.is_none()
                    || transition.kind == RuntimeEventKind::AutomaticReviewUnavailable,
                "only an unavailable review event carries an effective policy",
            )?;
            crate::broker::runtime::evidence::validate(&transition.evidence_json)?;
            let sender = self
                .sender
                .as_ref()
                .ok_or_else(|| error("event writer is stopped"))?;
            match sender.try_send(Work::Event {
                transition: Box::new(transition.clone()),
                effective_policy,
            }) {
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
    pub(crate) fn try_record_policy(&self, observation: EffectivePolicyObservation) {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let sent = if state.stop {
            Err(error("event writer is stopped"))
        } else {
            self.sender
                .as_ref()
                .ok_or_else(|| error("event writer is stopped"))
                .and_then(|sender| {
                    sender
                        .try_send(Work::Policy(observation))
                        .map_err(|_| error("runtime observation channel is full or unavailable"))
                })
        };
        match sent {
            Ok(()) => {
                state.policies = state
                    .policies
                    .checked_add(1)
                    .expect("policy update count overflow");
            }
            Err(cause) => state.policy_dropped(&cause),
        }
    }
    #[cfg(test)]
    pub(crate) fn submit(&self, transition: ObservedTransition) -> Result<()> {
        self.try_submit(transition)
            .map_err(|retained| retained.cause)
    }
    #[cfg(test)]
    pub(crate) fn joined(&self) -> bool {
        self.thread.is_none()
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
            #[cfg(test)]
            policy_diagnostic: state.policy_diagnostic.clone(),
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
pub(super) mod contract_tests {
    use super::*;
    use crate::broker::runtime::*;
    use crate::runtime::runner_tests::Fixture;
    mod refresh {
        use super::worker::{Hooks, Release, committed, start, wait_until};
        use super::*;

        fn opening(fixture: &WriterFixture) -> ObservedTransition {
            ObservedTransition {
                episode_seq: Some(1),
                kind: RuntimeEventKind::InteractionOpened,
                classification: Some(InteractionClass::PermissionRequest),
                cause_json: None,
                ..transition(fixture, 1)
            }
        }
        fn update(fixture: &WriterFixture, sequence: u64) -> InteractionEvidenceUpdate {
            InteractionEvidenceUpdate {
                run_id: fixture.fixture.run.run_id,
                episode_seq: 1,
                output_seq: sequence,
                evidence_json: transition(fixture, sequence.try_into().unwrap()).evidence_json,
            }
        }
        fn latest(fixture: &WriterFixture) -> Value {
            let text: String = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT evidence_json FROM member_interactions WHERE run_id=?1 AND episode_seq=1",
                    [fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&text).unwrap()
        }
        fn committed_open(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
            let writer = start(fixture, hooks);
            assert!(writer.try_submit(opening(fixture)).is_ok());
            wait_until(|| committed(fixture) == 1 && writer.status().pending_count == 0);
            writer
        }

        #[test]
        fn runner_batch2_refresh_waits_for_its_opening_barrier_without_an_outbox_frame() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            assert!(writer.try_submit(opening(&fixture)).is_ok());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(committed(&fixture), 0);
            assert!(writer.status().pending_evidence);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![opening(&fixture)]
            );
            drop(release);
            wait_until(|| committed(&fixture) == 1 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 2);
            assert_eq!(writer.finish(deadline()).0, 0);
            assert_eq!(committed(&fixture), 1);
        }

        #[test]
        fn runner_batch2_refresh_newer_revision_survives_an_inflight_successful_attempt() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
            assert!(writer.status().pending_evidence);
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 3);
            assert_eq!(committed(&fixture), 1);
            writer.finish(deadline());
        }

        #[test]
        fn runner_batch2_refresh_newer_revision_survives_an_inflight_failed_attempt() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
            hooks.fail_once(WriterBoundary::Persist);
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 3);
            assert_eq!(committed(&fixture), 1);
            writer.finish(deadline());
        }

        #[test]
        fn runner_batch2_refresh_rejects_regression_conflict_and_foreign_episode_without_replacing_newer_work()
         {
            for field in ["regression", "conflict", "episode"] {
                let fixture = WriterFixture::new();
                let hooks = Arc::new(Hooks::default());
                let mut writer = committed_open(&fixture, hooks.clone());
                hooks.pause(WriterBoundary::Persist);
                let release = Release(hooks.clone());
                assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
                hooks.wait(WriterBoundary::Persist, 2);
                let mut invalid = update(&fixture, if field == "regression" { 2 } else { 3 });
                if field == "episode" {
                    invalid.episode_seq = 2;
                }
                if field == "conflict" {
                    let mut raw = invalid.evidence_json.clone();
                    raw["sensitive"] = serde_json::json!(false);
                    raw["truncated"] = serde_json::json!(true);
                    invalid.evidence_json = storage::evidence::EvidencePolicy::metadata()
                        .apply(&raw)
                        .unwrap();
                }
                let returned = writer
                    .try_refresh(invalid, deadline())
                    .expect_err("invalid refresh retains caller ownership");
                assert_eq!(
                    returned.update.output_seq,
                    if field == "regression" { 2 } else { 3 }
                );
                assert!(writer.status().degraded);
                drop(release);
                wait_until(|| !writer.status().pending_evidence);
                assert_eq!(latest(&fixture)["screen_seq"], 3);
                writer.finish(deadline());
            }
        }

        #[test]
        fn runner_batch2_refresh_closure_follows_earlier_evidence_and_rejects_late_old_episode_updates()
         {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            let close = ObservedTransition {
                event_seq: 2,
                kind: RuntimeEventKind::InteractionClosed,
                close_reason: Some(InteractionCloseReason::NotVisible),
                evidence_json: transition(&fixture, 3).evidence_json,
                ..opening(&fixture)
            };
            assert!(writer.try_submit(close.clone()).is_ok());
            assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_err());
            drop(release);
            wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture), update(&fixture, 2).evidence_json);
            writer.finish(deadline());
        }

        #[test]
        fn runner_batch2_refresh_newer_same_episode_revision_advances_its_semantic_barrier() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            assert!(writer.try_submit(opening(&fixture)).is_ok());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            let obscured = ObservedTransition {
                event_seq: 2,
                kind: RuntimeEventKind::InteractionObscured,
                evidence_json: transition(&fixture, 3).evidence_json,
                ..opening(&fixture)
            };
            assert!(writer.try_submit(obscured).is_ok());
            assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_ok());
            drop(release);
            wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 4);
            writer.finish(deadline());
        }

        #[test]
        fn runner_batch2_refresh_equal_identical_requests_coalesce_and_expiry_has_no_enqueue_effect()
         {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            let begin = Instant::now();
            assert!(
                writer
                    .try_refresh(update(&fixture, 3), begin - Duration::from_millis(1))
                    .is_err()
            );
            assert!(begin.elapsed() < Duration::from_millis(250));
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 2);
            writer.finish(deadline());
        }
    }
    pub(in crate::runtime::runner) mod support {
        use super::*;
        use crate::coding_agent::ProfileCatalog;
        use crate::runtime::runner_tests::owned_child::OwnedChild;
        use std::process::{Command, Stdio};

        pub(in crate::runtime::runner) const VERSION: &str = "synthetic-0.0.1";
        pub(in crate::runtime::runner) const PROFILE: &str = "synthetic-profile-1";
        pub(in crate::runtime::runner) const NOW: &str = "2026-10-26T16:40:00.000000+00:00";
        pub(in crate::runtime::runner) fn definitions() -> Value {
            serde_json::from_str(include_str!(
                "../runner_tests/fixtures/synthetic-profile.json"
            ))
            .unwrap()
        }
        pub(in crate::runtime::runner) fn catalog() -> ProfileCatalog {
            ProfileCatalog::from_definitions(&definitions()).unwrap()
        }
        pub(in crate::runtime::runner) fn configure(fixture: &mut WriterFixture) {
            fixture.fixture.conn.execute("UPDATE member_runs SET native_version=?1,profile_version=?2,profile_match='exact',coverage='tested',runner_pid=owner_pid,runner_start_identity=owner_start_identity WHERE run_id=?3",rusqlite::params![VERSION,PROFILE,fixture.fixture.run.run_id]).unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }
        pub(in crate::runtime::runner) fn configure_redacted(fixture: &mut WriterFixture) {
            configure(fixture);
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }
        pub(in crate::runtime::runner) struct Clock {
            origin: Instant,
            millis: AtomicU64,
        }
        impl Clock {
            pub(in crate::runtime::runner) fn new() -> Self {
                Self {
                    origin: Instant::now(),
                    millis: AtomicU64::new(0),
                }
            }
            pub(in crate::runtime::runner) fn set(&self, millis: u64) {
                self.millis.store(millis, Ordering::Release);
            }
        }
        impl ObservationClock for Clock {
            fn sample(&self) -> ObservationMoment {
                ObservationMoment {
                    monotonic: self.origin
                        + Duration::from_millis(self.millis.load(Ordering::Acquire)),
                    observed_at: NOW.into(),
                }
            }
            fn kind(&self) -> ObservationClockKind {
                ObservationClockKind::Deterministic
            }
        }
        pub(in crate::runtime::runner) struct ChildGuard {
            pub child: OwnedChild,
            pub identity: ProcessIdentity,
        }
        impl ChildGuard {
            pub(in crate::runtime::runner) fn spawn(
                executable: &std::path::Path,
                arguments: &[String],
            ) -> Self {
                let child = OwnedChild::new(
                    Command::new(executable)
                        .args(arguments)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap(),
                );
                let identity = ProcessIdentity::read(i64::from(child.id()))
                    .unwrap()
                    .expect("owned child generation");
                Self { child, identity }
            }
        }
        pub(in crate::runtime::runner) fn running(
            fixture: &mut WriterFixture,
            child: &ProcessIdentity,
        ) {
            fixture.fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![child.pid,child.start_identity,NOW,fixture.fixture.run.run_id]).unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }
        pub(in crate::runtime::runner) fn form(operation: &str) -> Vec<u8> {
            format!("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ").into_bytes()
        }
        pub(in crate::runtime::runner) fn detailed_form(operation: &str, detail: &str) -> Vec<u8> {
            format!("\x1b[2J\x1b[18;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nDetail: {detail}\r\nSelection: ").into_bytes()
        }
        // Rewrites only the non-fingerprint detail row of `detailed_form` and returns the cursor to the end of `Selection: `.
        pub(in crate::runtime::runner) fn detail_rewrite(detail: &str) -> Vec<u8> {
            format!("\x1b[23;1H\x1b[2KDetail: {detail}\x1b[24;12H").into_bytes()
        }
        pub(in crate::runtime::runner) fn redacted_policy() -> storage::evidence::EvidencePolicy {
            storage::evidence::EvidencePolicy::redacted(
                &serde_json::json!({"schema_version":1,"patterns":["synthetic-redaction-canary"],"sensitive_rules":[]}),
                &["synthetic-permission"],
            )
            .unwrap()
        }
    }
    pub(in crate::runtime::runner) mod worker {
        use super::*;
        use std::sync::Condvar;

        fn index(boundary: WriterBoundary) -> usize {
            match boundary {
                WriterBoundary::Receive => 0,
                WriterBoundary::Append => 1,
                WriterBoundary::Sync => 2,
                WriterBoundary::Persist => 3,
                WriterBoundary::Compact => 4,
            }
        }
        #[derive(Debug, Default)]
        struct HookState {
            paused: [bool; 5],
            failures: [u64; 5],
            calls: [u64; 5],
            first: [Option<u64>; 5],
            ordinal: u64,
        }
        #[derive(Default)]
        pub(in crate::runtime::runner) struct Hooks {
            state: Mutex<HookState>,
            changed: Condvar,
        }
        impl Hooks {
            pub(in crate::runtime::runner) fn pause(&self, boundary: WriterBoundary) {
                self.state.lock().unwrap().paused[index(boundary)] = true;
            }
            fn release_for_cleanup(&self) {
                let mut state = match self.state.lock() {
                    Ok(state) => state,
                    Err(poisoned) => {
                        eprintln!("fixture hook cleanup recovered poisoned state");
                        poisoned.into_inner()
                    }
                };
                state.paused = [false; 5];
                drop(state);
                self.changed.notify_all();
            }
            pub(in crate::runtime::runner) fn fail_once(&self, boundary: WriterBoundary) {
                self.state.lock().unwrap().failures[index(boundary)] += 1;
            }
            pub(in crate::runtime::runner) fn wait(&self, boundary: WriterBoundary, count: u64) {
                let slot = index(boundary);
                let until = deadline();
                let mut state = self.state.lock().unwrap();
                while state.calls[slot] < count {
                    let remaining = until.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        let calls = state.calls;
                        drop(state);
                        panic!("worker boundary {slot} was not reached; calls={calls:?}");
                    }
                    state = self.changed.wait_timeout(state, remaining).unwrap().0;
                }
            }
            pub(in crate::runtime::runner) fn calls(&self, boundary: WriterBoundary) -> u64 {
                self.state.lock().unwrap().calls[index(boundary)]
            }
        }
        impl WriterHooks for Hooks {
            fn before(&self, boundary: WriterBoundary, until: Instant) -> Result<()> {
                let slot = index(boundary);
                let mut state = self.state.lock().unwrap();
                state.ordinal = state.ordinal.checked_add(1).unwrap();
                state.calls[slot] = state.calls[slot].checked_add(1).unwrap();
                let ordinal = state.ordinal;
                state.first[slot].get_or_insert(ordinal);
                self.changed.notify_all();
                while state.paused[slot] {
                    let remaining = until.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(CafleetError::DeadlineExceeded);
                    }
                    state = self.changed.wait_timeout(state, remaining).unwrap().0;
                }
                if state.failures[slot] > 0 {
                    state.failures[slot] -= 1;
                    return Err(error("fixture writer boundary failure"));
                }
                Ok(())
            }
        }
        pub(in crate::runtime::runner) struct Release(pub Arc<Hooks>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.release_for_cleanup();
            }
        }

        pub(in crate::runtime::runner) fn start(
            fixture: &WriterFixture,
            hooks: Arc<Hooks>,
        ) -> EventWriter {
            EventWriter::start_with_hooks(
                format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                fixture.fixture.run.clone(),
                fixture.lock.clone(),
                fixture.artifacts.paths.outbox.clone(),
                hooks,
            )
            .unwrap()
        }
        pub(in crate::runtime::runner) fn wait_until(condition: impl FnMut() -> bool) {
            wait_until_within(Duration::from_secs(2), condition);
        }
        pub(in crate::runtime::runner) fn wait_until_within(
            budget: Duration,
            mut condition: impl FnMut() -> bool,
        ) {
            let until = Instant::now() + budget;
            while !condition() {
                assert!(
                    Instant::now() < until,
                    "bounded worker condition did not complete"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        pub(in crate::runtime::runner) fn committed(fixture: &WriterFixture) -> i64 {
            fixture
                .fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[test]
        fn runner_batch2_hook_timeout_releases_the_mutex_before_unwinding_through_cleanup() {
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let observed = hooks.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _release = Release(observed.clone());
                observed.wait(WriterBoundary::Persist, 1);
            }));
            assert!(result.is_err());
            assert!(!hooks.state.is_poisoned());
            assert_eq!(hooks.state.lock().unwrap().paused, [false; 5]);
        }

        #[test]
        fn runner_batch2_hook_cleanup_releases_known_state_after_a_worker_poison_without_panicking()
        {
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let observed = hooks.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _state = observed.state.lock().unwrap();
                panic!("owned fixture worker panic");
            }));
            assert!(result.is_err());
            let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(Release(hooks.clone()))
            }));
            assert!(cleanup.is_ok());
            let state = hooks
                .state
                .lock()
                .expect_err("ordinary access preserves poisoned-state invariant")
                .into_inner();
            assert_eq!(state.paused, [false; 5]);
        }

        #[test]
        fn runner_batch2_worker_receive_stall_pins_64_channel_slots_and_returns_the_failed_original()
         {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            for sequence in 1..=64 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            let original = transition(&fixture, 65);
            let begin = Instant::now();
            let rejected = writer
                .try_submit(original.clone())
                .expect_err("channel pressure returns the original");
            assert!(begin.elapsed() < Duration::from_millis(250));
            assert_eq!(rejected.transition, original);
            let status = writer.status();
            assert_eq!(status.pending_count, 64);
            assert!(status.degraded);
            assert!(status.gap_count > 0);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), 64);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_batch2_worker_sync_precedes_sqlite_and_pending_delivery_is_not_an_inbox_commit() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(committed(&fixture), 0);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            let first = hooks.state.lock().unwrap().first;
            assert!(first[1].unwrap() < first[2].unwrap());
            assert!(first[2].unwrap() < first[3].unwrap());
            assert_eq!(writer.status().pending_count, 1);
            drop(release);
            wait_until(|| committed(&fixture) == 1);
            assert_eq!(writer.finish(deadline()), (0, 0));
        }

        #[test]
        fn runner_batch2_worker_ambiguous_sync_failure_rescans_and_reuses_the_complete_original() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.fail_once(WriterBoundary::Sync);
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            assert_eq!(hooks.calls(WriterBoundary::Append), 1);
            assert!(hooks.calls(WriterBoundary::Sync) >= 2);
            drop(release);
            wait_until(|| committed(&fixture) == 1);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_batch2_worker_append_and_database_failures_retain_the_same_transition_for_retry()
        {
            for boundary in [WriterBoundary::Append, WriterBoundary::Persist] {
                let fixture = WriterFixture::new();
                let hooks = Arc::new(Hooks::default());
                hooks.fail_once(boundary);
                let mut writer = start(&fixture, hooks.clone());
                let _release = Release(hooks);
                let original = transition(&fixture, 1);
                assert!(writer.try_submit(original.clone()).is_ok());
                wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
                assert_eq!(
                    storage::get_event(&fixture.fixture.conn, 1)
                        .unwrap()
                        .event
                        .payload_json["evidence"],
                    original.evidence_json
                );
                assert_eq!(committed(&fixture), 1);
                assert_eq!(writer.finish(deadline()).0, 0);
            }
        }

        #[test]
        fn runner_batch2_worker_pending_capacity_counts_channel_and_worker_queue_without_growth() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let mut accepted = 1;
            for sequence in 2..=256 {
                match writer.try_submit(transition(&fixture, sequence)) {
                    Ok(()) => accepted += 1,
                    Err(retained) => {
                        assert_eq!(retained.transition.event_seq, sequence);
                        break;
                    }
                }
            }
            assert!((64..=128).contains(&accepted));
            assert_eq!(writer.status().pending_count, accepted);
            assert!(writer.status().degraded);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_batch2_worker_refill_is_nonblocking_coalesced_and_published_only_after_commit() {
            let fixture = WriterFixture::new();
            let blocking = crate::db::connect(&format!(
                "sqlite:///{}",
                fixture.fixture.conn.path().unwrap()
            ))
            .unwrap();
            blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let begin = Instant::now();
            for _ in 0..10 {
                writer.request_sequence_range().unwrap();
            }
            assert!(begin.elapsed() < Duration::from_millis(250));
            assert!(writer.take_sequence_range().is_none());
            assert_eq!(
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                    .unwrap()
                    .event_seq_reserved_through,
                256
            );
            blocking.execute_batch("ROLLBACK").unwrap();
            let mut range = None;
            wait_until(|| {
                range = writer.take_sequence_range();
                range.is_some()
            });
            assert_eq!(range.unwrap(), 257..=512);
            assert!(writer.take_sequence_range().is_none());
            assert_eq!(
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                    .unwrap()
                    .event_seq_reserved_through,
                512
            );
            writer.finish(deadline());
        }

        #[test]
        fn runner_batch2_worker_shutdown_retains_synced_uncommitted_frames_and_the_original_budget()
        {
            let fixture = WriterFixture::new();
            let blocking = crate::db::connect(&format!(
                "sqlite:///{}",
                fixture.fixture.conn.path().unwrap()
            ))
            .unwrap();
            blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let begin = Instant::now();
            writer.set_deadline(begin + Duration::from_millis(80));
            writer.begin_shutdown(begin + Duration::from_secs(3));
            let pending = writer.finish(begin + Duration::from_secs(3));
            assert!(begin.elapsed() < Duration::from_millis(750));
            assert_eq!(pending.0, 1);
            assert_eq!(committed(&fixture), 0);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            blocking.execute_batch("ROLLBACK").unwrap();
        }

        struct SlowCompaction {
            calls: AtomicU64,
        }
        impl WriterHooks for SlowCompaction {
            fn before(&self, boundary: WriterBoundary, _: Instant) -> Result<()> {
                if matches!(boundary, WriterBoundary::Compact) {
                    self.calls.fetch_add(1, Ordering::AcqRel);
                    std::thread::sleep(Duration::from_millis(250));
                }
                Ok(())
            }
        }

        #[test]
        fn runner_batch2_worker_final_compaction_slower_than_one_work_slice_commits_before_the_shutdown_deadline()
         {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(SlowCompaction {
                calls: AtomicU64::new(0),
            });
            let mut writer = EventWriter::start_with_hooks(
                format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                fixture.fixture.run.clone(),
                fixture.lock.clone(),
                fixture.artifacts.paths.outbox.clone(),
                hooks.clone(),
            )
            .unwrap();
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let begin = Instant::now();
            assert_eq!(writer.finish(begin + Duration::from_secs(3)), (0, 0));
            assert!(
                writer.joined(),
                "the worker stops once its final compaction commits"
            );
            assert!(begin.elapsed() < Duration::from_secs(3));
            assert!(
                std::fs::read(&fixture.artifacts.paths.outbox)
                    .unwrap()
                    .is_empty(),
                "the committed outbox is compacted"
            );
            assert!(hooks.calls.load(Ordering::Acquire) >= 1);
        }
    }

    pub(in crate::runtime::runner) mod effective_policy {
        use super::worker::{Hooks, Release, committed, start, wait_until};
        use super::*;

        const UNAVAILABLE_RULE: &str = "codex-auto-review-unavailable";
        const AVAILABLE_RULE: &str = "codex-auto-review-indicator";

        pub(in crate::runtime::runner) fn unavailable(
            fixture: &WriterFixture,
            sequence: i64,
        ) -> ObservedTransition {
            let evidence = storage::evidence::EvidencePolicy::metadata()
                .apply(&serde_json::json!({"backend":fixture.fixture.run.backend,"native_version":fixture.fixture.run.native_version,"profile_version":fixture.fixture.run.profile_version,"rule_id":UNAVAILABLE_RULE,"screen_seq":sequence,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}))
                .unwrap();
            ObservedTransition {
                kind: RuntimeEventKind::AutomaticReviewUnavailable,
                evidence_json: evidence,
                cause_json: None,
                ..transition(fixture, sequence)
            }
        }

        pub(in crate::runtime::runner) fn observation(
            automatic_review: AutomaticReview,
        ) -> EffectivePolicyObservation {
            EffectivePolicyObservation {
                automatic_review,
                observed_mode: None,
                rule_id: if automatic_review == AutomaticReview::Unavailable {
                    UNAVAILABLE_RULE
                } else {
                    AVAILABLE_RULE
                }
                .into(),
                observed_at: "2026-10-26T16:40:00.000000+00:00".into(),
            }
        }

        pub(in crate::runtime::runner) fn record(
            observation: &EffectivePolicyObservation,
            event_id: Option<i64>,
        ) -> Value {
            serde_json::json!({
                "schema_version":1,
                "automatic_review":observation.automatic_review.as_str(),
                "observed_mode":observation.observed_mode,
                "rule_id":observation.rule_id,
                "event_id":event_id,
                "observed_at":observation.observed_at,
            })
        }

        pub(in crate::runtime::runner) fn frame_bodies(mut bytes: &[u8]) -> Vec<Value> {
            let mut bodies = Vec::new();
            while !bytes.is_empty() {
                let length =
                    usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
                let body = &bytes[36..36 + length];
                assert_eq!(&bytes[4..36], &Sha256::digest(body)[..]);
                bodies.push(serde_json::from_slice(body).unwrap());
                bytes = &bytes[36 + length..];
            }
            bodies
        }

        fn set_status(fixture: &mut WriterFixture, status: &str) {
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                    rusqlite::params![status, fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }

        fn stored(fixture: &WriterFixture) -> Option<Value> {
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                .unwrap()
                .effective_policy_json
        }

        fn spool(fixture: &WriterFixture) -> Vec<u8> {
            std::fs::read(&fixture.artifacts.paths.outbox).unwrap()
        }

        #[test]
        fn runner_policy_unavailable_frame_spools_its_observation_and_commits_it_with_the_event() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = unavailable(&fixture, 1);
            let lost = observation(AutomaticReview::Unavailable);
            assert!(
                writer
                    .try_submit_with_policy(value.clone(), Some(lost.clone()))
                    .is_ok()
            );
            hooks.wait(WriterBoundary::Persist, 1);
            let mut expected = serde_json::to_value(&value).unwrap();
            expected["effective_policy"] = serde_json::to_value(&lost).unwrap();
            assert_eq!(frame_bodies(&spool(&fixture)), vec![expected]);
            assert_eq!(stored(&fixture), None);
            drop(release);
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let event = storage::get_event(&fixture.fixture.conn, 1).unwrap().event;
            assert_eq!(event.kind, RuntimeEventKind::AutomaticReviewUnavailable);
            assert_eq!(stored(&fixture), Some(record(&lost, Some(event.event_id))));
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_other_frames_keep_their_bytes_and_reject_an_attached_observation() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let plain = transition(&fixture, 1);
            assert!(
                writer
                    .try_submit_with_policy(
                        plain.clone(),
                        Some(observation(AutomaticReview::Unavailable))
                    )
                    .is_err()
            );
            assert!(writer.try_submit_with_policy(plain.clone(), None).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(spool(&fixture), frame(&plain));
            drop(release);
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(stored(&fixture), None);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_commits_behind_accepted_transitions_without_spooling() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let earlier = transition(&fixture, 1);
            assert!(writer.try_submit(earlier.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let shown = observation(AutomaticReview::Available);
            writer.try_record_policy(shown.clone());
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(stored(&fixture), None, "queued behind the paused event");
            assert_eq!(
                spool(&fixture),
                frame(&earlier),
                "the update is not spooled"
            );
            drop(release);
            wait_until(|| stored(&fixture).is_some());
            assert_eq!(committed(&fixture), 1);
            assert_eq!(stored(&fixture), Some(record(&shown, None)));
            let status = writer.status();
            assert_eq!(status.policy_diagnostic, None);
            assert!(!status.degraded);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_on_a_non_live_run_is_dropped_once_and_the_worker_continues()
         {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "stopping");
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            writer.try_record_policy(observation(AutomaticReview::Available));
            wait_until(|| writer.status().policy_diagnostic.is_some());
            let persists = hooks.calls(WriterBoundary::Persist);
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                hooks.calls(WriterBoundary::Persist),
                persists,
                "a dropped update is not retried"
            );
            assert_eq!(stored(&fixture), None);
            let status = writer.status();
            assert!(!status.degraded, "{status:?}");
            assert_eq!(status.gap_count, 0);
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_from_a_superseded_owner_is_dropped_with_a_diagnostic() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            let lost = observation(AutomaticReview::Unavailable);
            assert!(
                writer
                    .try_submit_with_policy(unavailable(&fixture, 1), Some(lost.clone()))
                    .is_ok()
            );
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let current = stored(&fixture).expect("event-carried record");
            assert_eq!(current, record(&lost, Some(1)));
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            writer.try_record_policy(observation(AutomaticReview::Available));
            wait_until(|| writer.status().policy_diagnostic.is_some());
            assert_eq!(
                stored(&fixture),
                Some(current),
                "a superseded runner never overwrites the record"
            );
            writer.finish(deadline());
        }
    }

    pub(in crate::runtime::runner) struct WriterFixture {
        pub(in crate::runtime::runner) fixture: Fixture,
        pub(in crate::runtime::runner) artifacts: RunArtifacts,
        pub(in crate::runtime::runner) lock: Arc<RunLock>,
    }
    impl WriterFixture {
        pub(in crate::runtime::runner) fn new() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let lock = Arc::new(
                RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap(),
            );
            lock.transfer_to_runner(
                &mut fixture.conn,
                &fixture.run.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner()).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            Self {
                fixture,
                artifacts,
                lock,
            }
        }
        fn start(&self, path: PathBuf) -> Result<EventWriter> {
            EventWriter::start(
                format!("sqlite:///{}", self.fixture.conn.path().unwrap()),
                self.fixture.run.clone(),
                self.lock.clone(),
                path,
            )
        }
    }

    pub(in crate::runtime::runner) fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    pub(in crate::runtime::runner) fn transition(
        fixture: &WriterFixture,
        sequence: i64,
    ) -> ObservedTransition {
        let evidence=storage::evidence::EvidencePolicy::metadata().apply(&serde_json::json!({"backend":fixture.fixture.run.backend,"native_version":fixture.fixture.run.native_version,"profile_version":fixture.fixture.run.profile_version,"rule_id":"runner-lifecycle","screen_seq":sequence,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
        ObservedTransition {
            run_id: fixture.fixture.run.run_id,
            event_seq: sequence,
            episode_seq: None,
            kind: RuntimeEventKind::CoverageUnknown,
            classification: None,
            observed_at: "2026-10-26T16:40:00.000000+00:00".into(),
            evidence_json: evidence,
            close_reason: None,
            cause_json: Some(serde_json::json!({"kind":"coverage_unknown"})),
        }
    }

    pub(in crate::runtime::runner) fn frame(transition: &ObservedTransition) -> Vec<u8> {
        let body = serde_json::to_vec(transition).unwrap();
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(&Sha256::digest(&body));
        bytes.extend(body);
        bytes
    }

    pub(in crate::runtime::runner) fn independent_frames(
        mut bytes: &[u8],
    ) -> Vec<ObservedTransition> {
        let mut values = Vec::new();
        while !bytes.is_empty() {
            assert!(bytes.len() >= 36);
            let length =
                usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
            assert!(length > 0 && length <= 16 * 1024 * 1024);
            assert!(bytes.len() >= 36 + length);
            let body = &bytes[36..36 + length];
            assert_eq!(&bytes[4..36], &Sha256::digest(body)[..]);
            values.push(serde_json::from_slice(body).unwrap());
            bytes = &bytes[36 + length..];
        }
        values
    }

    fn assert_start_rejected(fixture: &WriterFixture, path: PathBuf) {
        let result = fixture.start(path);
        let rejected = result.is_err();
        if let Ok(mut writer) = result {
            writer.finish(Instant::now() + Duration::from_millis(100));
        }
        assert!(
            rejected,
            "writer startup validates the recorded owned outbox before accepting work"
        );
    }

    #[test]
    fn runner_batch2_writer_starts_with_the_exact_private_recorded_outbox() {
        let fixture = WriterFixture::new();
        let mut writer = fixture
            .start(fixture.artifacts.paths.outbox.clone())
            .unwrap();
        assert_eq!(
            writer.finish(Instant::now() + Duration::from_secs(1)),
            (0, 0)
        );
    }

    #[test]
    fn runner_batch2_writer_foreign_outbox_path_fails_before_worker_start() {
        let fixture = WriterFixture::new();
        let foreign = fixture.fixture._dir.path().join("foreign-outbox");
        std::fs::write(&foreign, []).unwrap();
        assert_start_rejected(&fixture, foreign.clone());
        assert_eq!(std::fs::read(foreign).unwrap(), b"");
    }

    #[test]
    fn runner_batch2_writer_missing_or_symlinked_outbox_fails_before_worker_start() {
        for symlink in [false, true] {
            let fixture = WriterFixture::new();
            let path = &fixture.artifacts.paths.outbox;
            std::fs::remove_file(path).unwrap();
            let target = fixture.fixture._dir.path().join("untouched-target");
            std::fs::write(&target, b"original target").unwrap();
            if symlink {
                std::os::unix::fs::symlink(&target, path).unwrap();
            }
            assert_start_rejected(&fixture, path.clone());
            assert_eq!(std::fs::read(target).unwrap(), b"original target");
        }
    }

    #[test]
    fn runner_batch2_writer_stale_owner_is_rejected_before_worker_start() {
        let mut fixture = WriterFixture::new();
        fixture.fixture.run.owner_epoch += 1;
        assert_start_rejected(&fixture, fixture.artifacts.paths.outbox.clone());
    }

    #[test]
    fn runner_batch2_writer_spool_sync_is_pending_until_the_actual_sqlite_commit() {
        let fixture = WriterFixture::new();
        let blocking = crate::db::connect(&format!(
            "sqlite:///{}",
            fixture.fixture.conn.path().unwrap()
        ))
        .unwrap();
        blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
        let value = transition(&fixture, 1);
        let mut writer = fixture
            .start(fixture.artifacts.paths.outbox.clone())
            .unwrap();
        writer.submit(value.clone()).unwrap();
        let until = deadline();
        let bytes = loop {
            let bytes = std::fs::read(&fixture.artifacts.paths.outbox).unwrap();
            if bytes.len() == frame(&value).len() {
                break bytes;
            }
            assert!(
                Instant::now() < until,
                "complete pending frame was not written: {:?}",
                writer.diagnostic()
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(independent_frames(&bytes), vec![value]);
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        blocking.execute_batch("ROLLBACK").unwrap();
        let until = deadline();
        loop {
            let count = fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            if count == 1 {
                break;
            }
            assert!(
                Instant::now() < until,
                "event did not commit: {:?}",
                writer.diagnostic()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(writer.finish(deadline()), (0, 0));
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
