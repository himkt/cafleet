use crate::broker::runtime as storage;
use crate::broker::runtime::ObservedTransition;
use crate::broker::runtime::PersistenceMode;
use crate::broker::runtime::RunRecord;
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

pub(in crate::runtime) const WORK_SLICE: Duration = Duration::from_millis(100);

#[cfg(test)]
pub(crate) trait WriterHooks: Send + Sync {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> Result<()>;
    fn work_slice(&self) -> Duration {
        WORK_SLICE
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
    deadline: Option<Instant>,
}
impl Mailbox {
    fn failure(&mut self, cause: &CafleetError) {
        self.gaps = self.gaps.checked_add(1).expect("writer gap count overflow");
        self.diagnostic = Some(cause.message().into_owned());
    }
    fn work_deadline(&self, now: Instant, slice: Duration) -> Instant {
        let local = now + slice;
        self.deadline.map_or(local, |until| until.min(local))
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
        #[cfg(not(test))]
        let work_slice = WORK_SLICE;
        #[cfg(test)]
        let work_slice = hooks.work_slice();
        let worker = Worker {
            conn,
            run,
            lock,
            mailbox: mailbox.clone(),
            receiver,
            queue: VecDeque::new(),
            work_slice,
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
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.deadline = Some(state.deadline.map_or(until, |old| old.min(until)));
    }
    pub(crate) fn begin_shutdown(&mut self, until: Instant) {
        self.set_deadline(until);
        self.mailbox.lock().expect("writer mailbox").stop = true;
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
            while !thread.is_finished() && self.clock.now() < until {
                nap_until(self.clock.as_ref(), until);
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
        (state.pending, unsaved)
    }
}
impl Drop for EventWriter {
    fn drop(&mut self) {
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline
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
    work_slice: Duration,
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
                if state.deadline.is_some_and(|end| self.clock.now() >= end) {
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
        if !self.receive()? {
            return Ok(());
        }
        self.persist_front()
    }
    fn work_deadline(&self) -> Instant {
        self.mailbox
            .lock()
            .expect("writer mailbox")
            .work_deadline(self.clock.now(), self.work_slice)
    }
    /// Returns `false` when a test hook ends this pass before any persistence.
    fn receive(&mut self) -> Result<bool> {
        while self.queue.len() < 64 {
            let until = self.work_deadline();
            #[cfg(test)]
            match self.hooks.before(WriterBoundary::Receive, until) {
                Ok(()) => {}
                Err(CafleetError::DeadlineExceeded) => return Ok(false),
                Err(cause) => return Err(cause),
            }

            check_deadline(self.clock.now(), until)?;
            match self.receiver.try_recv() {
                Ok(transition) => self.queue.push_back(transition),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Ok(true)
    }
    fn persist_front(&mut self) -> Result<()> {
        let Some(transition) = self.queue.front() else {
            return Ok(());
        };
        let until = self.work_deadline();
        #[cfg(test)]
        self.hooks.before(WriterBoundary::Persist, until)?;

        with_database_deadline(&mut self.conn, until, |conn| {
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
    use crate::runtime::writer::test_support::WriterFixture;
    use crate::runtime::writer::test_support::deadline;
    use crate::runtime::writer::test_support::transition;
    use crate::runtime::writer::*;
    use std::time::Duration;

    mod supplied_connection_compaction {}

    mod terminal_tail {}

    use crate::runtime::writer::test_support::*;

    mod refresh {}

    mod worker {
        use crate::runtime::writer::tests::*;
        #[test]
        fn writer_worker_receive_stall_pins_64_channel_slots_and_returns_the_failed_original() {
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
            assert!(status.diagnostic.is_some());
            assert!(status.gap_count > 0);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), 64);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_worker_database_failure_retains_the_same_transition_for_retry() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.fail_once(WriterBoundary::Persist);
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

        #[test]
        fn writer_commits_each_submitted_transition_directly_with_its_event_sequence() {
            let fixture = WriterFixture::new();
            let mut writer = start(&fixture, Arc::new(Hooks::default()));
            for sequence in 1..=3 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            wait_until(|| committed(&fixture) == 3 && writer.status().pending_count == 0);
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
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_retry_exhaustion_reports_a_degraded_status_and_keeps_the_transition() {
            let fixture = WriterFixture::new();
            fixture
                .fixture
                .conn
                .execute_batch(
                    "CREATE TRIGGER injected_event_failure BEFORE INSERT ON member_runtime_events BEGIN SELECT RAISE(ABORT,'injected event failure'); END",
                )
                .unwrap();
            let writer = start(&fixture, Arc::new(Hooks::default()));
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| writer.status().diagnostic.is_some());
            let status = writer.status();
            assert_eq!(status.pending_count, 1, "{status:?}");
            assert_eq!(committed(&fixture), 0);
        }

        #[test]
        fn writer_worker_pending_capacity_counts_channel_and_worker_queue_without_growth() {
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
            assert!(writer.status().diagnostic.is_some());
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
            assert_eq!(writer.finish(deadline()).0, 0);
        }
    }

    fn assert_start_rejected(fixture: &WriterFixture) {
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

    #[test]
    fn writer_stale_owner_is_rejected_before_worker_start() {
        let mut fixture = WriterFixture::new();
        fixture.fixture.run.owner_epoch += 1;
        assert_start_rejected(&fixture);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::error::Result;
    use crate::runtime::writer::{WriterBoundary, WriterHooks};
    use std::sync::Arc;
    use std::time::Instant;
    struct DefaultWriterHooks;
    impl WriterHooks for DefaultWriterHooks {
        fn before(&self, _: WriterBoundary, _: Instant) -> Result<()> {
            Ok(())
        }
    }
    pub(in crate::runtime) fn default_writer_hooks() -> Arc<dyn WriterHooks> {
        Arc::new(DefaultWriterHooks)
    }

    use crate::broker::runtime::*;
    use crate::runtime::test_support::Fixture;
    use crate::runtime::writer::*;
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
            drop(state);
            self.changed.notify_all();
        }
        pub(crate) fn fail_once(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().failures[index(boundary)] += 1;
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
    pub(crate) struct Release(pub(crate) Arc<Hooks>);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release_for_cleanup();
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
