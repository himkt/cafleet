use super::*;

mod observation_failures;

struct Timeline {
    origin: Instant,
    millis: AtomicU64,
}
impl Timeline {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            millis: AtomicU64::new(0),
        }
    }
    fn set(&self, millis: u64) {
        self.millis.store(millis, Ordering::Release);
    }
    fn timestamp(millis: u64) -> String {
        let origin = chrono::DateTime::parse_from_rfc3339(NOW)
            .unwrap()
            .with_timezone(&chrono::Utc);
        crate::time::format_utc(
            origin + chrono::Duration::milliseconds(i64::try_from(millis).unwrap()),
        )
    }
}
impl ObservationClock for Timeline {
    fn sample(&self) -> ObservationMoment {
        let millis = self.millis.load(Ordering::Acquire);
        ObservationMoment {
            monotonic: self.origin + Duration::from_millis(millis),
            observed_at: Self::timestamp(millis),
        }
    }
    fn kind(&self) -> ObservationClockKind {
        ObservationClockKind::Deterministic
    }
}

struct LiveCase {
    writer: EventWriter,
    controller: ObservationController,
    hooks: Arc<Hooks>,
    child: ChildGuard,
    terminal: TerminalState,
    clock: Timeline,
    fixture: WriterFixture,
}
impl LiveCase {
    fn new() -> Self {
        Self::with_observation_hooks(None)
    }
    fn with_observation_hooks(observation_hooks: Option<Arc<dyn ObservationHooks>>) -> Self {
        let mut fixture = WriterFixture::new();
        configure(&mut fixture);
        let hooks = Arc::new(Hooks::default());
        let writer = start(&fixture, hooks.clone());
        let mut controller = ObservationController::new(
            &fixture.fixture.run,
            &catalog(),
            storage::evidence::EvidencePolicy::metadata(),
            1..=256,
        )
        .unwrap();
        if let Some(hooks) = observation_hooks {
            controller.set_observation_hooks(hooks).unwrap();
        }
        let child = ChildGuard::spawn(std::path::Path::new("/bin/cat"), &[]);
        running(&mut fixture, &child.identity);
        let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let clock = Timeline::new();
        controller
            .started(&fixture.fixture.run, &terminal.snapshot(), &writer, &clock)
            .unwrap();
        terminal
            .feed_observed(
                &form("write terminal-order.txt"),
                &mut controller.observer(&writer, &clock),
            )
            .unwrap();
        clock.set(100);
        controller
            .poll(&terminal.snapshot(), &writer, &clock)
            .unwrap();
        wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
        Self {
            writer,
            controller,
            hooks,
            child,
            terminal,
            clock,
            fixture,
        }
    }
    fn fail_refresh(&mut self) -> Release {
        self.hooks.pause(WriterBoundary::Persist);
        let release = Release(self.hooks.clone());
        self.clock.set(200);
        self.terminal
            .feed_observed(
                b"\x1b",
                &mut self.controller.observer(&self.writer, &self.clock),
            )
            .unwrap();
        self.hooks.wait(WriterBoundary::Persist, 2);
        wait_until(|| self.writer.status().degraded);
        assert!(self.writer.status().pending_evidence);
        assert_eq!(committed(&self.fixture), 1);
        release
    }
    fn exit(&mut self, millis: u64) -> u64 {
        assert!(self.child.identity.matches().unwrap());
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(self.child.identity.pid).unwrap()),
            nix::sys::signal::Signal::SIGTERM,
        )
        .unwrap();
        let until = deadline();
        loop {
            if let Some(exit) = self.child.child.try_wait().unwrap() {
                assert_eq!(exit.signal(), Some(15));
                break;
            }
            assert!(Instant::now() < until, "owned child exit deadline");
            std::thread::sleep(Duration::from_millis(2));
        }
        self.clock.set(millis);
        let sequence = self.terminal.snapshot().screen_seq;
        self.controller
            .child_exited(
                ChildExit::Signal(15),
                &self.terminal.snapshot(),
                &self.writer,
                &self.clock,
            )
            .unwrap();
        sequence
    }
    fn drain(&mut self, until: Instant) -> (u64, u64) {
        self.writer.set_deadline(until);
        while Instant::now() < until {
            self.controller
                .poll(&self.terminal.snapshot(), &self.writer, &self.clock)
                .unwrap();
            if !self.controller.has_pending() && self.writer.status().pending_count == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.writer.finish(until)
    }
    fn events(&self) -> Vec<storage::RuntimeEventView> {
        let ids: Vec<i64> = self
            .fixture
            .fixture
            .conn
            .prepare(
                "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
            )
            .unwrap()
            .query_map([self.fixture.fixture.run.run_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        ids.into_iter()
            .map(|id| storage::get_event(&self.fixture.fixture.conn, id).unwrap())
            .collect()
    }
}
impl Drop for LiveCase {
    fn drop(&mut self) {
        let unfinished = self
            .writer
            .finish(Instant::now() + Duration::from_millis(250));
        if unfinished != (0, 0) {
            eprintln!("owned review fixture retained writer work: {unfinished:?}");
        }
    }
}

#[test]
fn runner_review_r1_degraded_exit_records_gap_before_original_closure_and_terminal_fence() {
    let mut case = LiveCase::new();
    let release = case.fail_refresh();
    let exit_sequence = case.exit(300);
    assert!(case.controller.has_pending());
    assert!(case.controller.unsaved_gap_count() > 0);
    assert!(matches!(
        case.controller.status().eligibility,
        InputEligibility::Hold(_)
    ));
    case.clock.set(700);
    drop(release);
    let unfinished = case.drain(deadline());
    let events = case.events();
    let kinds: Vec<_> = events.iter().map(|view| view.event.kind).collect();
    assert_eq!(
        kinds,
        [
            RuntimeEventKind::InteractionOpened,
            RuntimeEventKind::RunnerDegraded,
            RuntimeEventKind::RunnerDegraded,
            RuntimeEventKind::InteractionClosed,
            RuntimeEventKind::RunnerExited,
        ]
    );
    assert_eq!(unfinished, (0, 0));
    assert!(!case.controller.has_pending());
    assert_eq!(case.controller.unsaved_gap_count(), 0);
    assert_eq!(
        events[1].event.payload_json["cause"]["kind"],
        "storage_pressure"
    );
    assert_eq!(events[1].event.observed_at, Timeline::timestamp(300));
    assert_eq!(
        events[2].event.payload_json["cause"]["kind"],
        "storage_recovered"
    );
    assert_eq!(events[2].event.observed_at, Timeline::timestamp(700));
    for view in &events[3..] {
        assert_eq!(view.event.observed_at, Timeline::timestamp(300));
        assert_eq!(
            view.event.payload_json["evidence"]["screen_seq"],
            exit_sequence
        );
    }
    assert_eq!(
        events[3].event.payload_json["close_reason"],
        "process_exited"
    );
    assert_eq!(events[4].event.payload_json["cause"]["exit_signal"], 15);
    let run =
        storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Exited);
    assert_eq!(run.exit_signal, Some(15));
    for _ in 0..3 {
        case.controller
            .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
            .unwrap();
    }
    assert_eq!(case.events().len(), 5);
    assert!(!case.controller.has_pending());
}

#[test]
fn runner_review_r1_expired_storage_budget_reports_unsaved_gap_and_pending_exit_work() {
    let mut case = LiveCase::new();
    let _release = case.fail_refresh();
    let until = Instant::now() + Duration::from_millis(120);
    case.writer.set_deadline(until);
    case.exit(300);
    let begin = Instant::now();
    let unfinished = case.drain(until);
    assert!(begin.elapsed() < Duration::from_millis(500));
    assert!(case.controller.has_pending() || unfinished.0 > 0);
    assert!(case.controller.unsaved_gap_count() > 0 || unfinished.1 > 0);
    assert_eq!(committed(&case.fixture), 1);
    let run =
        storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert!(case.child.child.try_wait().unwrap().is_some());
    assert_eq!(
        case.events()[0].event.kind,
        RuntimeEventKind::InteractionOpened
    );
}

#[test]
fn runner_review_r1_failure_after_terminal_enqueue_never_appends_a_live_post_terminal_gap() {
    let mut case = LiveCase::new();
    case.hooks.pause(WriterBoundary::Persist);
    let release = Release(case.hooks.clone());
    case.exit(300);
    case.hooks.wait(WriterBoundary::Persist, 2);
    wait_until(|| case.writer.status().degraded);
    case.clock.set(500);
    case.controller
        .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
        .unwrap();
    case.clock.set(700);
    drop(release);
    let unfinished = case.drain(deadline());
    let events = case.events();
    assert_eq!(unfinished, (0, 0));
    assert!(!case.controller.has_pending());
    assert_eq!(case.controller.unsaved_gap_count(), 0);
    assert_eq!(case.controller.status().gap_count, 0);
    let run =
        storage::get_run(&case.fixture.fixture.conn, case.fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.observation_gap_count, 0);
    assert_eq!(
        events
            .iter()
            .map(|view| view.event.kind)
            .collect::<Vec<_>>(),
        [
            RuntimeEventKind::InteractionOpened,
            RuntimeEventKind::InteractionClosed,
            RuntimeEventKind::RunnerExited,
        ]
    );
    assert_eq!(
        events.last().unwrap().event.kind,
        RuntimeEventKind::RunnerExited
    );
    assert_eq!(
        events
            .iter()
            .filter(|view| view.event.kind == RuntimeEventKind::RunnerExited)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|view| view.event.kind == RuntimeEventKind::InteractionClosed)
            .count(),
        1
    );
    assert_eq!(
        events.last().unwrap().event.observed_at,
        Timeline::timestamp(300)
    );
    for _ in 0..3 {
        case.controller
            .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
            .unwrap();
    }
    assert_eq!(case.events().len(), events.len());
    assert!(!case.controller.has_pending());
}
