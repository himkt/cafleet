use super::*;
use std::sync::atomic::AtomicBool;

struct Fault {
    site: ObservationSite,
    armed: AtomicBool,
    calls: AtomicU64,
}
impl Fault {
    fn new(site: ObservationSite) -> Self {
        Self {
            site,
            armed: AtomicBool::new(false),
            calls: AtomicU64::new(0),
        }
    }
    fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }
}
impl ObservationHooks for Fault {
    fn before(&self, site: ObservationSite, operation: ObservationOperation) -> Result<()> {
        if site == self.site && operation == ObservationOperation::Detector {
            self.calls.fetch_add(1, Ordering::AcqRel);
            if self.armed.swap(false, Ordering::AcqRel) {
                return Err(error("private detector failure review-canary-194"));
            }
        }
        Ok(())
    }
}

#[test]
fn runner_review_r3_observation_hooks_install_once_before_the_matching_native_start() {
    let mut fixture = WriterFixture::new();
    configure(&mut fixture);
    let mut controller = ObservationController::new(
        &fixture.fixture.run,
        &catalog(),
        storage::evidence::EvidencePolicy::metadata(),
        1..=256,
    )
    .unwrap();
    controller
        .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
        .unwrap();
    assert!(
        controller
            .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
            .is_err()
    );
    let mut case = LiveCase::new();
    assert!(
        case.controller
            .set_observation_hooks(Arc::new(Fault::new(ObservationSite::QuietTimer)))
            .is_err()
    );
}

#[test]
fn runner_review_r3_quiet_detector_failure_returns_typed_held_status_and_preserves_the_episode() {
    let fault = Arc::new(Fault::new(ObservationSite::QuietTimer));
    let mut case = LiveCase::with_observation_hooks(Some(fault.clone()));
    case.clock.set(200);
    case.terminal
        .feed(&form("write candidate-after-open.txt"))
        .unwrap();
    case.controller
        .observer(&case.writer, &case.clock)
        .observe(&case.terminal.snapshot())
        .unwrap();
    let sequence = case.terminal.snapshot().screen_seq;
    let calls = fault.calls.load(Ordering::Acquire);
    fault.arm();
    case.clock.set(300);
    let status = case
        .controller
        .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
        .expect("recoverable quiet detector failure preserves the live controller");
    assert_eq!(fault.calls.load(Ordering::Acquire), calls + 1);
    assert!(!fault.armed.load(Ordering::Acquire));
    assert_eq!(status.diagnostic.as_deref(), Some("detector_unavailable"));
    assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
    assert!(status.gap_count > 0);
    assert_eq!(case.terminal.snapshot().screen_seq, sequence);
    let active = case.events()[0].interaction.clone().unwrap();
    assert_eq!(active.episode_seq, 1);
    assert!(active.closed_at.is_none());
    assert_eq!(
        case.fixture.fixture.conn.query_row(
            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
            [case.fixture.fixture.run.run_id],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        1
    );
    assert!(case.child.identity.matches().unwrap());
}

#[test]
fn runner_review_r3_ordered_adapter_preserves_detector_attribution_and_bounded_metadata() {
    let fault = Arc::new(Fault::new(ObservationSite::ChildOutput));
    let mut case = LiveCase::with_observation_hooks(Some(fault.clone()));
    fault.arm();
    case.clock.set(200);
    let calls = fault.calls.load(Ordering::Acquire);
    let _attempt = case
        .controller
        .observer(&case.writer, &case.clock)
        .observe(&case.terminal.snapshot());
    assert_eq!(fault.calls.load(Ordering::Acquire), calls + 1);
    assert!(!fault.armed.load(Ordering::Acquire));
    let status = case.controller.status();
    assert_eq!(status.diagnostic.as_deref(), Some("detector_unavailable"));
    assert!(matches!(status.eligibility, InputEligibility::Hold(_)));
    assert!(status.gap_count > 0);
    assert_eq!(
        case.fixture.fixture.conn.query_row(
            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
            [case.fixture.fixture.run.run_id],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        1
    );
    assert!(
        case.events()[0]
            .interaction
            .as_ref()
            .unwrap()
            .closed_at
            .is_none()
    );
    assert!(case.child.identity.matches().unwrap());
}

#[test]
fn runner_review_r3_clock_and_output_sequence_invariants_propagate_outside_recoverable_failures() {
    for invariant in ["clock", "sequence"] {
        let fault = Arc::new(Fault::new(ObservationSite::ChildOutput));
        let mut case = LiveCase::with_observation_hooks(Some(fault));
        let before = case.controller.status().gap_count;
        let result = if invariant == "clock" {
            case.clock.set(99);
            case.controller
                .poll(&case.terminal.snapshot(), &case.writer, &case.clock)
                .map(|_| ())
        } else {
            case.clock.set(200);
            let mut stale = case.terminal.snapshot();
            stale.screen_seq = 0;
            case.controller
                .observer(&case.writer, &case.clock)
                .observe(&stale)
        };
        let cause =
            result.expect_err("clock/sequence corruption remains an explicit invariant error");
        assert!(cause.message().contains("regress"), "{invariant}: {cause}");
        assert_eq!(case.controller.status().gap_count, before);
        assert_eq!(case.controller.status().diagnostic, None);
        assert!(
            case.events()[0]
                .interaction
                .as_ref()
                .unwrap()
                .closed_at
                .is_none()
        );
    }
}
