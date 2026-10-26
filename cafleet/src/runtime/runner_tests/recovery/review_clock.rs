use super::*;
use crate::input::PaneLock;

struct ClockHost {
    inner: Host,
    before_entry: Instant,
    after_entry: Option<Instant>,
    delay: Duration,
    observed: Option<ShutdownClock>,
}
impl ClockHost {
    fn new(until: Instant, delay: Duration) -> Self {
        Self {
            inner: Host::new(until),
            before_entry: Instant::now(),
            after_entry: None,
            delay,
            observed: None,
        }
    }
}
impl RecoveryHost for ClockHost {
    fn current_process(&mut self) -> TestResult<ProcessIdentity> {
        self.inner.current_process()
    }
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.inner.process_evidence(process, deadline)
    }
    fn startup_process_evidence(
        &mut self,
        run: &RunRecord,
        placement: Option<&Placement>,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.inner
            .startup_process_evidence(run, placement, deadline)
    }
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> TestResult<RecoveryHealth> {
        self.inner.health(run, deadline)
    }
    fn checkpoint(
        &mut self,
        boundary: RecoveryBoundary,
        run: &RunRecord,
        deadline: Instant,
    ) -> TestResult<()> {
        if matches!(boundary, RecoveryBoundary::AfterSnapshot) {
            assert!(self.after_entry.replace(Instant::now()).is_none());
            self.inner.checkpoint(boundary, run, deadline)?;
            let resume = Instant::now() + self.delay;
            assert!(resume < deadline);
            std::thread::sleep(resume.saturating_duration_since(Instant::now()));
            return Ok(());
        }
        self.inner.checkpoint(boundary, run, deadline)
    }
    fn cleanup_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownClock,
    ) -> TestResult<ShutdownOutcome> {
        assert!(self.observed.replace(clock).is_none());
        let after = self
            .after_entry
            .expect("actual coordinator snapshot brackets its entry");
        let until = self.inner.deadline;
        let observed = [
            clock.child_deadline(),
            clock.display_deadline(),
            clock.storage_deadline(),
        ];
        let bounds = [2, 3, 4].map(|seconds| {
            (
                (self.before_entry + Duration::from_secs(seconds)).min(until),
                (after + Duration::from_secs(seconds)).min(until),
            )
        });
        eprintln!(
            "recovery cleanup clock: observed={observed:?}, entry_bounds={bounds:?}, original_cap={until:?}"
        );
        assert!(
            observed
                .iter()
                .zip(bounds)
                .all(|(actual, (lower, upper))| *actual >= lower && *actual <= upper),
            "cleanup carries the one clock captured at common coordinator entry"
        );
        self.inner.cleanup_owned(run, clock)
    }
}

fn clock_case(borrowed: bool, cap: Duration, delay: Duration) {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let expected = case.fixture.run.clone();
    let held = borrowed.then(|| {
        PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap()
    });
    case.release();
    let until = Instant::now() + cap;
    let mut host = ClockHost::new(until, delay);
    let report = if let Some(held) = held.as_ref() {
        with_database_deadline(&mut case.fixture.conn, until, |conn| {
            std::thread::sleep(Duration::from_millis(100));
            host.before_entry = Instant::now();
            recover_member_with_held_pane_lock(
                conn,
                &case.settings,
                expected.member_id,
                &expected,
                &case.placement,
                None,
                until,
                held,
                &mut host,
            )
            .map(|completion| completion.report)
        })
        .unwrap()
    } else {
        host.before_entry = Instant::now();
        recover_member_with_host(
            &mut case.fixture.conn,
            &case.settings,
            expected.member_id,
            None,
            until,
            &mut host,
        )
        .unwrap()
    };
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert!(host.observed.is_some());
    assert_eq!(counts(&report), (1, 3, 0));
}

#[test]
fn runner_recovery_review_r2_standalone_cleanup_uses_original_entry_clock_after_wait_and_replay() {
    clock_case(false, Duration::from_secs(7), Duration::from_millis(150));
}

#[test]
fn runner_recovery_review_r2_borrowed_cleanup_keeps_entry_clock_and_original_outer_cap() {
    clock_case(
        true,
        Duration::from_millis(3300),
        Duration::from_millis(150),
    );
}

#[test]
fn runner_recovery_review_r2_short_outer_cap_clips_every_actual_cleanup_boundary() {
    for borrowed in [false, true] {
        clock_case(
            borrowed,
            Duration::from_millis(500),
            Duration::from_millis(80),
        );
    }
}
