use crate::broker::runtime as storage;
use crate::broker::runtime::ObservedTransition;
use crate::broker::runtime::PersistenceMode;
use crate::broker::runtime::RunRecord;
use crate::db::SharedDeadline;
use crate::db::with_shared_database_deadline;
use crate::runtime::*;
use rusqlite::Connection;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum WriterBoundary {
    Receive,
    Persist,
}

/// Equal to the connection's busy timeout, so one worker step waits at most as long as one
/// contended statement may.
const WRITER_STEP_BOUND: Duration = Duration::from_secs(5);
const WRITER_JOIN_GRACE: Duration = Duration::from_millis(50);

#[cfg(test)]
pub(crate) trait WriterHooks: Send + Sync {
    fn before(&self, boundary: WriterBoundary, bound: &StepBound) -> Result<()>;
}

/// One worker step's limits: real time for the step itself, so a virtual-time jump never
/// expires it, and the shutdown deadline on the worker's clock.
pub(crate) struct StepBound {
    until: Instant,
    shutdown: SharedDeadline,
    clock: Arc<dyn crate::time::Clock>,
}
impl StepBound {
    fn new(shutdown: &SharedDeadline, clock: &Arc<dyn crate::time::Clock>) -> Self {
        Self {
            until: Instant::now() + WRITER_STEP_BOUND,
            shutdown: shutdown.clone(),
            clock: clock.clone(),
        }
    }
    pub(crate) fn check(&self) -> Result<()> {
        let shutdown = *self.shutdown.lock().expect("writer shutdown deadline");
        if Instant::now() >= self.until || shutdown.is_some_and(|end| self.clock.now() >= end) {
            return Err(CafleetError::DeadlineExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WriterStatus {
    pub(crate) pending_count: u64,
    pub(crate) gap_count: u64,
    pub(crate) diagnostic: Option<String>,
}
#[derive(Debug)]
pub(crate) struct RetainedTransition {
    pub(crate) transition: ObservedTransition,
    pub(crate) cause: CafleetError,
}
#[derive(Default)]
struct Mailbox {
    pending: u64,
    gaps: u64,
    diagnostic: Option<String>,
    last_sequence: i64,
    stop: bool,
    deadline: SharedDeadline,
}
impl Mailbox {
    fn failure(&mut self, cause: &CafleetError) {
        self.gaps = self.gaps.checked_add(1).expect("writer gap count overflow");
        self.diagnostic = Some(cause.message().into_owned());
    }
    fn deadline(&self) -> Option<Instant> {
        *self.deadline.lock().expect("writer shutdown deadline")
    }
    fn success(&mut self) {
        self.diagnostic = None;
    }
}
pub(crate) struct EventWriter {
    sender: mpsc::SyncSender<Box<ObservedTransition>>,
    mailbox: Arc<Mutex<Mailbox>>,
    thread: Option<std::thread::JoinHandle<()>>,
    clock: Arc<dyn crate::time::Clock>,
}
impl EventWriter {
    pub(crate) fn start(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        clock: Arc<dyn crate::time::Clock>,
    ) -> Result<Self> {
        Self::start_worker(
            database_url,
            run,
            lock,
            clock,
            #[cfg(test)]
            crate::runtime::writer::test_support::default_writer_hooks(),
        )
    }

    #[cfg(test)]
    pub(crate) fn start_with_hooks(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        clock: Arc<dyn crate::time::Clock>,
        hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        Self::start_worker(database_url, run, lock, clock, hooks)
    }

    fn start_worker(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        clock: Arc<dyn crate::time::Clock>,
        #[cfg(test)] hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        let mut conn = crate::db::connect(&database_url)?;
        with_database_deadline(&mut conn, clock.now() + Duration::from_secs(1), |conn| {
            lock.validate(conn, &run.owner())
        })?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let shutdown = mailbox.lock().expect("writer mailbox").deadline.clone();
        let worker = Worker {
            conn,
            run,
            lock,
            mailbox: mailbox.clone(),
            receiver,
            queue: VecDeque::new(),
            shutdown,
            clock: clock.clone(),
            #[cfg(test)]
            hooks,
        };
        let thread = std::thread::Builder::new()
            .name("cafleet-events".into())
            .spawn(move || worker.serve())
            .map_err(io_error)?;
        Ok(Self {
            sender,
            mailbox,
            thread: Some(thread),
            clock,
        })
    }
    fn send(&self, transition: Box<ObservedTransition>) -> Result<()> {
        self.sender
            .try_send(transition)
            .map_err(|_| error("runtime observation channel is full or unavailable"))
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
            storage::validate_evidence(&transition.evidence_json)?;
            self.send(Box::new(transition.clone()))
        })();
        match result {
            Ok(()) => {
                state.pending = state
                    .pending
                    .checked_add(1)
                    .expect("writer pending overflow");
                assert!(state.pending <= 128, "bounded accepted writer capacity");
                state.last_sequence = transition.event_seq;
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
    #[cfg(test)]
    pub(crate) fn joined(&self) -> bool {
        self.thread.is_none()
    }
    pub(crate) fn status(&self) -> WriterStatus {
        let state = self.mailbox.lock().expect("writer mailbox");
        WriterStatus {
            pending_count: state.pending,
            gap_count: state.gaps,
            diagnostic: state.diagnostic.clone(),
        }
    }
    pub(crate) fn diagnostic(&self) -> Option<String> {
        self.status().diagnostic
    }
    pub(crate) fn set_deadline(&mut self, until: Instant) {
        let state = self.mailbox.lock().expect("writer mailbox");
        let mut deadline = state.deadline.lock().expect("writer shutdown deadline");
        *deadline = Some(deadline.map_or(until, |old| old.min(until)));
    }
    pub(crate) fn begin_shutdown(&mut self, until: Instant) {
        self.set_deadline(until);
        self.mailbox.lock().expect("writer mailbox").stop = true;
    }
    /// Joins the worker once it finishes or the shutdown deadline plus a short grace passes; an
    /// in-flight step aborts at the shutdown deadline, so the join returns.
    pub(crate) fn finish(&mut self, until: Instant) -> (u64, u64) {
        self.begin_shutdown(until);
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline()
            .expect("shutdown deadline");
        if let Some(thread) = self.thread.take() {
            let join_by = until + WRITER_JOIN_GRACE;
            while !thread.is_finished() && self.clock.now() < join_by {
                nap_until(self.clock.as_ref(), join_by);
            }
            thread.join().expect("writer processing is panic-free");
        }
        let state = self.mailbox.lock().expect("writer mailbox");
        let unsaved = if state.diagnostic.is_some() {
            state.gaps
        } else {
            0
        };
        (state.pending, unsaved)
    }
}
impl Drop for EventWriter {
    fn drop(&mut self) {
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline()
            .unwrap_or_else(|| self.clock.now() + Duration::from_secs(4));
        self.finish(until);
    }
}

struct Worker {
    conn: Connection,
    run: RunRecord,
    lock: Arc<RunLock>,
    mailbox: Arc<Mutex<Mailbox>>,
    receiver: mpsc::Receiver<Box<ObservedTransition>>,
    queue: VecDeque<Box<ObservedTransition>>,
    shutdown: SharedDeadline,
    clock: Arc<dyn crate::time::Clock>,
    #[cfg(test)]
    hooks: Arc<dyn WriterHooks>,
}
impl Worker {
    fn serve(mut self) {
        let mut retry = Duration::from_millis(100);
        let mut next = self.clock.now();
        loop {
            {
                let state = self.mailbox.lock().expect("writer mailbox");
                if state.deadline().is_some_and(|end| self.clock.now() >= end) {
                    break;
                }
                if state.stop && state.pending == 0 {
                    break;
                }
            }
            if self.clock.now() < next {
                nap_until(self.clock.as_ref(), next);
                continue;
            }
            match self.step() {
                Ok(()) => {
                    retry = Duration::from_millis(100);
                    next = self.clock.now();
                }
                Err(cause) => {
                    self.mailbox.lock().expect("writer mailbox").failure(&cause);
                    next = self.clock.now() + retry;
                    retry = (retry * 2).min(Duration::from_secs(2));
                }
            }
            if self.mailbox.lock().expect("writer mailbox").pending == 0 {
                self.clock.sleep(Duration::from_millis(2));
            }
        }
    }
    fn step(&mut self) -> Result<()> {
        let bound = StepBound::new(&self.shutdown, &self.clock);
        if !self.receive(&bound)? {
            return Ok(());
        }
        self.persist_front(&bound)
    }
    /// Returns `false` when a test hook ends this pass before any persistence.
    fn receive(&mut self, bound: &StepBound) -> Result<bool> {
        while self.queue.len() < 64 {
            #[cfg(test)]
            match self.hooks.before(WriterBoundary::Receive, bound) {
                Ok(()) => {}
                Err(CafleetError::DeadlineExceeded) => return Ok(false),
                Err(cause) => return Err(cause),
            }

            bound.check()?;
            match self.receiver.try_recv() {
                Ok(transition) => self.queue.push_back(transition),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Ok(true)
    }
    fn persist_front(&mut self, bound: &StepBound) -> Result<()> {
        let Some(transition) = self.queue.front() else {
            return Ok(());
        };
        #[cfg(test)]
        self.hooks.before(WriterBoundary::Persist, bound)?;

        with_shared_database_deadline(&mut self.conn, bound.until, &self.shutdown, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            storage::persist_transition(conn, &self.run.owner(), transition, PersistenceMode::Live)
                .map(|_| ())
        })?;
        self.queue.pop_front();
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.pending = state.pending.checked_sub(1).expect("accepted writer count");
        state.success();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::writer::test_support::*;
    use crate::runtime::writer::*;
    use crate::time::Clock;
    use crate::time::test_support::TestClock;
    use std::time::Duration;

    fn clock() -> Arc<TestClock> {
        crate::runtime::test_support::virtual_clock()
    }

    fn until(clock: &TestClock) -> Instant {
        clock.now() + Duration::from_secs(2)
    }

    mod worker {
        use crate::runtime::writer::tests::*;
        #[test]
        fn writer_worker_receive_stall_pins_64_channel_slots_and_returns_the_failed_original() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            for sequence in 1..=64 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            let original = transition(&fixture, 65);
            let rejected = writer
                .try_submit(original.clone())
                .expect_err("channel pressure returns the original");
            assert_eq!(rejected.transition, original);
            assert!(
                matches!(rejected.reason, RetainReason::ChannelFull),
                "{:?}",
                rejected.reason
            );
            let status = writer.status();
            assert_eq!(status.pending_count, 64);
            assert!(status.diagnostic.is_some());
            assert!(status.gap_count > 0);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            yield_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), 64);
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }

        #[test]
        fn writer_worker_database_failure_retains_the_same_transition_for_retry() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.fail_once(WriterBoundary::Persist);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let _release = Release(hooks.clone());
            let original = transition(&fixture, 1);
            assert!(writer.try_submit(original.clone()).is_ok());
            yield_until(|| writer.status().diagnostic.is_some());
            assert_eq!(committed(&fixture), 0, "the retry waits for its backoff");
            assert_eq!(writer.status().pending_count, 1);
            clock.advance(Duration::from_millis(100));
            yield_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let persisted = storage::get_event(&fixture.fixture.conn, 1).unwrap().event;
            assert_eq!(
                persisted.kind, original.kind,
                "a transient failure retries the original"
            );
            assert_eq!(persisted.payload_json["evidence"], original.evidence_json);
            assert_eq!(committed(&fixture), 1);
            assert_eq!(writer.status().diagnostic, None);
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }

        #[test]
        fn writer_commits_each_submitted_transition_directly_with_its_event_sequence() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let mut writer = start_on(&fixture, Arc::new(Hooks::default()), &clock);
            for sequence in 1..=3 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            yield_until(|| committed(&fixture) == 3 && writer.status().pending_count == 0);
            let sequences: Vec<i64> = fixture
                .fixture
                .conn
                .prepare(
                    "SELECT event_seq FROM member_runtime_events WHERE run_id=?1 ORDER BY event_id",
                )
                .unwrap()
                .query_map([fixture.fixture.run.run_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(sequences, [1, 2, 3]);
            assert!(writer.status().diagnostic.is_none());
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }

        #[test]
        fn writer_fences_when_the_rejection_event_also_fails_permanently() {
            let clock = clock();
            let mut fixture = WriterFixture::new();
            crate::db::test_support::FaultPlan::Abort {
                trigger: "injected_event_failure",
                on: "BEFORE INSERT ON member_runtime_events",
                message: "injected event failure",
            }
            .arm(&mut fixture.fixture.conn);
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            assert!(writer.try_submit(transition(&fixture, 2)).is_ok());
            drop(release);
            yield_until(|| {
                writer
                    .status()
                    .diagnostic
                    .is_some_and(|diagnostic| diagnostic.starts_with("event writer fenced: "))
            });
            let fenced = writer.status();
            assert_eq!(
                fenced.pending_count, 0,
                "the queue is discarded: {fenced:?}"
            );
            assert!(
                fenced
                    .diagnostic
                    .as_deref()
                    .is_some_and(|diagnostic| diagnostic.contains("injected event failure")),
                "{fenced:?}"
            );
            assert_eq!(committed(&fixture), 0);
            let refused = writer
                .try_submit(transition(&fixture, 3))
                .expect_err("a fenced writer accepts no transition");
            assert!(
                matches!(refused.reason, RetainReason::Fenced),
                "{:?}",
                refused.reason
            );
            assert_eq!(refused.cause.message(), "event writer is fenced");
            let after = writer.status();
            assert_eq!(
                after.diagnostic, fenced.diagnostic,
                "a refusal after the fence keeps the fence diagnostic"
            );
            assert_eq!(after.gap_count, fenced.gap_count);
            assert_eq!(
                writer.finish(until(&clock)),
                (0, 2),
                "the discarded transitions are counted unsaved"
            );
            assert_eq!(committed(&fixture), 0);
        }

        #[test]
        fn writer_worker_pending_capacity_counts_channel_and_worker_queue_without_growth() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
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
            assert!(writer.status().diagnostic.is_some());
            drop(release);
            yield_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }
    }

    mod step_bound {
        use crate::runtime::writer::tests::*;

        fn lock_holder(fixture: &WriterFixture) -> rusqlite::Connection {
            let holder = crate::db::connect(&format!(
                "sqlite:///{}",
                fixture.fixture.conn.path().unwrap()
            ))
            .unwrap();
            holder.execute_batch("BEGIN IMMEDIATE").unwrap();
            holder
        }

        #[test]
        fn writer_virtual_clock_jump_while_the_worker_idles_never_expires_a_step() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            clock.advance(Duration::from_secs(10));
            drop(release);
            let calls = hooks.state.lock().unwrap().calls[index(WriterBoundary::Receive)];
            hooks.wait(WriterBoundary::Receive, calls + 2);
            let idle = writer.status();
            assert_eq!(idle.diagnostic, None, "{idle:?}");
            assert_eq!(idle.gap_count, 0, "{idle:?}");
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            yield_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(writer.status().diagnostic, None);
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }

        #[test]
        fn writer_persist_waits_beyond_100_ms_under_a_held_lock_without_failing() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks.clone());
            let holder = lock_holder(&fixture);
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            std::thread::sleep(Duration::from_millis(300));
            let held = writer.status();
            assert_eq!(held.diagnostic, None, "{held:?}");
            assert_eq!(held.gap_count, 0, "{held:?}");
            assert_eq!(held.pending_count, 1, "{held:?}");
            holder.execute_batch("ROLLBACK").unwrap();
            wait_until_within(Duration::from_secs(5), || {
                committed(&fixture) == 1 && writer.status().pending_count == 0
            });
            let persisted = writer.status();
            assert_eq!(persisted.diagnostic, None, "{persisted:?}");
            assert_eq!(persisted.gap_count, 0, "{persisted:?}");
            assert_eq!(writer.finish(deadline()), (0, 0));
        }

        enum InFlight {
            PausedPersistHook,
            HeldDatabaseLock,
        }

        struct FinishCase {
            name: &'static str,
            in_flight: InFlight,
        }

        const FINISH_CASES: &[FinishCase] = &[
            FinishCase {
                name: "paused persist hook",
                in_flight: InFlight::PausedPersistHook,
            },
            FinishCase {
                name: "held database lock",
                in_flight: InFlight::HeldDatabaseLock,
            },
        ];

        #[test]
        fn writer_finish_joins_an_in_flight_step_at_the_shutdown_deadline() {
            const SHUTDOWN_AFTER: Duration = Duration::from_millis(200);
            for case in FINISH_CASES {
                let name = case.name;
                let fixture = WriterFixture::new();
                let hooks = Arc::new(Hooks::default());
                let holder = match case.in_flight {
                    InFlight::PausedPersistHook => {
                        hooks.pause(WriterBoundary::Persist);
                        None
                    }
                    InFlight::HeldDatabaseLock => Some(lock_holder(&fixture)),
                };
                let release = Release(hooks.clone());
                let mut writer = start(&fixture, hooks.clone());
                assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
                hooks.wait(WriterBoundary::Persist, 1);
                std::thread::sleep(Duration::from_millis(50));
                let started = Instant::now();
                let (pending, _) = writer.finish(started + SHUTDOWN_AFTER);
                let elapsed = started.elapsed();
                assert!(
                    writer.joined(),
                    "{name}: finish joins the worker once its step aborts"
                );
                assert!(
                    elapsed < Duration::from_secs(1),
                    "{name}: the in-flight step ends at the shutdown deadline, not the 5 s step bound: {elapsed:?}"
                );
                assert_eq!(pending, 1, "{name}: the unsaved transition stays counted");
                drop(release);
                if let Some(holder) = holder {
                    holder.execute_batch("ROLLBACK").unwrap();
                }
                std::thread::sleep(Duration::from_millis(50));
                assert_eq!(
                    committed(&fixture),
                    0,
                    "{name}: nothing commits after finish returns"
                );
            }
        }
    }

    mod rejection {
        use crate::broker::runtime::RuntimeEventKind;
        use crate::broker::runtime::RuntimeEventRecord;
        use crate::runtime::writer::tests::*;

        const REJECT_COVERAGE_UNKNOWN: crate::db::test_support::FaultPlan =
            crate::db::test_support::FaultPlan::Abort {
                trigger: "reject_coverage_unknown",
                on: "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='coverage_unknown'",
                message: "injected kind rejection",
            };

        fn event_at(fixture: &WriterFixture, sequence: i64) -> RuntimeEventRecord {
            let id: i64 = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2",
                    rusqlite::params![fixture.fixture.run.run_id, sequence],
                    |row| row.get(0),
                )
                .unwrap();
            storage::get_event(&fixture.fixture.conn, id).unwrap().event
        }

        fn assert_rejection(event: &RuntimeEventRecord, rejected: &ObservedTransition) {
            assert_eq!(event.kind, RuntimeEventKind::RunnerDegraded);
            assert_eq!(event.event_seq, rejected.event_seq);
            let cause = &event.payload_json["cause"];
            assert_eq!(
                cause
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["kind", "rejected_kind", "rejected_observed_at", "error"]
            );
            assert_eq!(cause["kind"], "event_rejected");
            assert_eq!(cause["rejected_kind"], rejected.kind.as_str());
            assert_eq!(cause["rejected_observed_at"], rejected.observed_at.as_str());
            assert!(
                cause["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("injected kind rejection")),
                "{cause}"
            );
            assert_eq!(
                event.payload_json["director_action"],
                serde_json::Value::Null
            );
            assert_eq!(
                event.payload_json["evidence"]["screen_seq"],
                rejected.evidence_json["screen_seq"]
            );
        }

        #[test]
        fn writer_replaces_a_permanent_failure_with_its_rejection_and_continues_without_a_gap() {
            let clock = clock();
            let mut fixture = WriterFixture::new();
            REJECT_COVERAGE_UNKNOWN.arm(&mut fixture.fixture.conn);
            let hooks = Arc::new(Hooks::default());
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let rejected = transition(&fixture, 1);
            assert!(writer.try_submit(rejected.clone()).is_ok());
            yield_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let replaced = writer.status();
            assert_eq!(replaced.diagnostic, None, "{replaced:?}");
            assert_eq!(replaced.gap_count, 0, "{replaced:?}");
            assert_rejection(&event_at(&fixture, 1), &rejected);
            REJECT_COVERAGE_UNKNOWN.clear(&mut fixture.fixture.conn);
            let next = transition(&fixture, 2);
            assert!(writer.try_submit(next.clone()).is_ok());
            yield_until(|| committed(&fixture) == 2 && writer.status().pending_count == 0);
            assert_eq!(event_at(&fixture, 2).kind, next.kind);
            let continued = writer.status();
            assert_eq!(continued.diagnostic, None, "{continued:?}");
            assert_eq!(continued.gap_count, 0, "{continued:?}");
            assert_eq!(writer.finish(until(&clock)), (0, 0));
        }

        #[test]
        fn writer_keeps_a_transiently_failing_replacement_at_the_front_until_the_backoff_retry() {
            let clock = clock();
            let mut fixture = WriterFixture::new();
            REJECT_COVERAGE_UNKNOWN.arm(&mut fixture.fixture.conn);
            let hooks = Arc::new(Hooks::default());
            hooks.busy_on_call(WriterBoundary::Persist, 2);
            let mut writer = start_on(&fixture, hooks.clone(), &clock);
            let rejected = transition(&fixture, 1);
            assert!(writer.try_submit(rejected.clone()).is_ok());
            yield_until(|| writer.status().diagnostic.is_some());
            let held = writer.status();
            assert_eq!(held.pending_count, 1, "{held:?}");
            assert!(
                held.diagnostic
                    .as_deref()
                    .is_some_and(|diagnostic| diagnostic.contains("fixture writer boundary busy")),
                "the transient cause takes the ordinary path: {held:?}"
            );
            assert_eq!(committed(&fixture), 0, "the retry waits for its backoff");
            clock.advance(Duration::from_millis(100));
            yield_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_rejection(&event_at(&fixture, 1), &rejected);
            assert_eq!(writer.status().diagnostic, None);
            assert_eq!(writer.finish(until(&clock)).0, 0);
        }

        #[test]
        fn writer_fences_on_a_sequence_slot_failure() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let mut writer = start_on(&fixture, Arc::new(Hooks::default()), &clock);
            assert!(writer.try_submit(transition(&fixture, 2)).is_ok());
            yield_until(|| {
                writer
                    .status()
                    .diagnostic
                    .is_some_and(|diagnostic| diagnostic.starts_with("event writer fenced: "))
            });
            let fenced = writer.status();
            assert!(
                fenced
                    .diagnostic
                    .as_deref()
                    .is_some_and(|diagnostic| diagnostic.contains("event sequence")),
                "{fenced:?}"
            );
            assert_eq!(fenced.pending_count, 0, "{fenced:?}");
            assert_eq!(committed(&fixture), 0);
            assert_eq!(writer.finish(until(&clock)), (0, 1));
        }

        #[test]
        fn writer_refuses_invalid_evidence_without_degrading() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let mut writer = start_on(&fixture, Arc::new(Hooks::default()), &clock);
            let mut forged = transition(&fixture, 1);
            forged.evidence_json = serde_json::json!({"forged": true});
            let refused = writer
                .try_submit(forged.clone())
                .expect_err("invalid evidence is refused before hand-off");
            assert!(
                matches!(refused.reason, RetainReason::EvidenceInvalid),
                "{:?}",
                refused.reason
            );
            assert_eq!(refused.transition, forged);
            let status = writer.status();
            assert_eq!(status.diagnostic, None, "{status:?}");
            assert_eq!(status.gap_count, 0, "{status:?}");
            assert_eq!(status.pending_count, 0, "{status:?}");
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            yield_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(writer.finish(until(&clock)), (0, 0));
        }

        #[test]
        fn writer_refuses_submissions_after_stop_and_sequence_regression_as_pressure() {
            let clock = clock();
            let fixture = WriterFixture::new();
            let mut writer = start_on(&fixture, Arc::new(Hooks::default()), &clock);
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            let regressed = writer
                .try_submit(transition(&fixture, 1))
                .expect_err("a regressed sequence is refused");
            assert!(
                matches!(regressed.reason, RetainReason::SequenceRegressed),
                "{:?}",
                regressed.reason
            );
            writer.begin_shutdown(until(&clock));
            let stopped = writer
                .try_submit(transition(&fixture, 3))
                .expect_err("a stopped writer refuses new work");
            assert!(
                matches!(stopped.reason, RetainReason::Stopped),
                "{:?}",
                stopped.reason
            );
            writer.finish(until(&clock));
        }
    }

    #[test]
    fn writer_stale_owner_is_rejected_before_worker_start() {
        let mut fixture = WriterFixture::new();
        fixture.fixture.run.owner_epoch += 1;
        let result = fixture.start();
        let rejected = result.is_err();
        if let Ok(mut writer) = result {
            writer.finish(Instant::now() + Duration::from_millis(100));
        }
        assert!(
            rejected,
            "writer startup validates the run owner before accepting work"
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::error::Result;
    use crate::runtime::writer::{StepBound, WriterBoundary, WriterHooks};
    use std::sync::Arc;
    use std::time::Instant;
    struct DefaultWriterHooks;
    impl WriterHooks for DefaultWriterHooks {
        fn before(&self, _: WriterBoundary, _: &StepBound) -> Result<()> {
            Ok(())
        }
    }
    pub(in crate::runtime) fn default_writer_hooks() -> Arc<dyn WriterHooks> {
        Arc::new(DefaultWriterHooks)
    }

    use crate::broker::runtime::*;
    use crate::runtime::test_support::Fixture;
    use crate::runtime::writer::*;
    use crate::time::Clock;
    use crate::time::ClockKind;
    use crate::time::MonotonicSample;
    use crate::time::test_support::TestClock;
    use std::sync::Condvar;
    pub(crate) fn index(boundary: WriterBoundary) -> usize {
        match boundary {
            WriterBoundary::Receive => 0,
            WriterBoundary::Persist => 1,
        }
    }
    #[derive(Debug, Default)]
    pub(crate) struct HookState {
        pub(crate) paused: [bool; 2],
        pub(crate) failures: [u64; 2],
        pub(crate) calls: [u64; 2],
        pub(crate) first: [Option<u64>; 2],
        pub(crate) ordinal: u64,
        pub(crate) busy_call: [Option<u64>; 2],
        pub(crate) busy: [bool; 2],
    }
    #[derive(Default)]
    pub(crate) struct Hooks {
        pub(crate) state: Mutex<HookState>,
        pub(crate) changed: Condvar,
    }
    impl Hooks {
        pub(crate) fn pause(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().paused[index(boundary)] = true;
        }
        pub(crate) fn release_for_cleanup(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.paused = [false; 2];
            state.busy = [false; 2];
            drop(state);
            self.changed.notify_all();
        }
        pub(crate) fn fail_once(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().failures[index(boundary)] += 1;
        }
        /// Fails only the `call`-th entry to `boundary`, counting from 1, as a transient busy cause.
        pub(crate) fn busy_on_call(&self, boundary: WriterBoundary, call: u64) {
            self.state.lock().unwrap().busy_call[index(boundary)] = Some(call);
        }
        /// Fails every entry to `boundary` as a transient busy cause until released.
        pub(crate) fn busy(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().busy[index(boundary)] = true;
        }
        pub(crate) fn wait(&self, boundary: WriterBoundary, count: u64) {
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
    }
    const PAUSE_SLICE: Duration = Duration::from_millis(10);
    impl WriterHooks for Hooks {
        fn before(&self, boundary: WriterBoundary, bound: &StepBound) -> Result<()> {
            let slot = index(boundary);
            let mut state = self.state.lock().unwrap();
            state.ordinal = state.ordinal.checked_add(1).unwrap();
            state.calls[slot] = state.calls[slot].checked_add(1).unwrap();
            let ordinal = state.ordinal;
            state.first[slot].get_or_insert(ordinal);
            self.changed.notify_all();
            while state.paused[slot] {
                if bound.check().is_err() {
                    return Err(CafleetError::DeadlineExceeded);
                }
                state = self.changed.wait_timeout(state, PAUSE_SLICE).unwrap().0;
            }
            if state.failures[slot] > 0 {
                state.failures[slot] -= 1;
                return Err(CafleetError::DatabaseBusy(
                    "fixture writer boundary busy".into(),
                ));
            }
            if state.busy[slot] || state.busy_call[slot] == Some(state.calls[slot]) {
                return Err(CafleetError::DatabaseBusy(
                    "fixture writer boundary busy".into(),
                ));
            }
            Ok(())
        }
    }
    pub(crate) struct Release(pub(crate) Arc<Hooks>);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release_for_cleanup();
        }
    }

    /// A [`TestClock`] as a background worker sees it: the same virtual time, but its sleeps
    /// yield instead of advancing, so only the test moves time. A sleep on a panicking thread
    /// advances the clock, so an unwinding writer still reaches its shutdown deadline.
    pub(crate) struct YieldingClock(pub(crate) Arc<TestClock>);

    impl Clock for YieldingClock {
        fn now(&self) -> Instant {
            self.0.now()
        }
        fn wall_now(&self) -> chrono::DateTime<chrono::Utc> {
            self.0.wall_now()
        }
        fn monotonic_sample(&self) -> Result<MonotonicSample> {
            self.0.monotonic_sample()
        }
        fn sleep(&self, duration: Duration) {
            if std::thread::panicking() {
                self.0.advance(duration);
            } else {
                std::thread::yield_now();
            }
        }
        fn shutdown_unit(&self) -> Duration {
            self.0.shutdown_unit()
        }
        fn kind(&self) -> ClockKind {
            ClockKind::Deterministic
        }
    }

    /// Starts a writer whose worker reads `clock`'s virtual time.
    pub(crate) fn start_on(
        fixture: &WriterFixture,
        hooks: Arc<Hooks>,
        clock: &Arc<TestClock>,
    ) -> EventWriter {
        EventWriter::start_with_hooks(
            format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
            fixture.fixture.run.clone(),
            fixture.lock.clone(),
            Arc::new(YieldingClock(clock.clone())),
            hooks,
        )
        .unwrap()
    }

    /// Yields until the worker thread makes `condition` hold, within a real-time bound.
    pub(crate) fn yield_until(condition: impl FnMut() -> bool) {
        yield_until_within(Duration::from_secs(10), condition);
    }

    /// Yields until the worker thread makes `condition` hold, within `budget` of real time.
    pub(crate) fn yield_until_within(budget: Duration, mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + budget;
        while !condition() {
            assert!(
                Instant::now() < until,
                "bounded worker condition did not complete"
            );
            std::thread::yield_now();
        }
    }

    pub(crate) fn start(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
        EventWriter::start_with_hooks(
            format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
            fixture.fixture.run.clone(),
            fixture.lock.clone(),
            std::sync::Arc::new(crate::time::SystemClock),
            hooks,
        )
        .unwrap()
    }
    pub(crate) fn wait_until(condition: impl FnMut() -> bool) {
        wait_until_within(Duration::from_secs(2), condition);
    }
    pub(crate) fn wait_until_within(budget: Duration, mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + budget;
        while !condition() {
            assert!(
                Instant::now() < until,
                "bounded worker condition did not complete"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub(crate) fn committed(fixture: &WriterFixture) -> i64 {
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

    pub(crate) struct WriterFixture {
        pub(crate) fixture: Fixture,
        pub(crate) artifacts: RunArtifacts,
        pub(crate) lock: Arc<RunLock>,
    }
    impl WriterFixture {
        pub(crate) fn new() -> Self {
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
            Self {
                fixture,
                artifacts,
                lock,
            }
        }
        pub(crate) fn start(&self) -> Result<EventWriter> {
            EventWriter::start(
                format!("sqlite:///{}", self.fixture.conn.path().unwrap()),
                self.fixture.run.clone(),
                self.lock.clone(),
                std::sync::Arc::new(crate::time::SystemClock),
            )
        }
    }

    pub(crate) fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    pub(crate) fn transition(fixture: &WriterFixture, sequence: i64) -> ObservedTransition {
        run_transition(&fixture.fixture.run, sequence)
    }

    /// The coverage-unknown transition `sequence` of `run`, usable off the fixture's thread.
    pub(crate) fn run_transition(run: &storage::RunRecord, sequence: i64) -> ObservedTransition {
        let evidence = crate::broker::runtime::run_evidence(
            run,
            "runner-lifecycle",
            u64::try_from(sequence).unwrap(),
        )
        .unwrap();
        ObservedTransition {
            run_id: run.run_id,
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
}
