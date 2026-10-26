use super::observation_services::{count, health, private_write, synthetic, wait_until};
use super::*;
use crate::error::CafleetError;
use std::sync::Arc;

const CANARY: &str = "private-observation-fault-canary-194";

struct Hooks {
    site: ObservationSite,
    operation: ObservationOperation,
    directory: std::path::PathBuf,
}
impl ObservationHooks for Hooks {
    fn before(
        &self,
        site: ObservationSite,
        operation: ObservationOperation,
    ) -> Result<(), CafleetError> {
        if site != self.site || operation != self.operation {
            return Ok(());
        }
        let arm = self.directory.join("observation-fault-armed");
        if !arm.try_exists().map_err(io_error)? {
            return Ok(());
        }
        std::fs::remove_file(arm).map_err(io_error)?;
        private_write(
            &self.directory.join("observation-fault-fired"),
            format!("{site:?}/{operation:?}").as_bytes(),
        );
        Err(error(CANARY))
    }
}
pub(super) fn hooks(
    value: &Value,
    directory: &Path,
) -> Result<Option<Arc<dyn ObservationHooks>>, CafleetError> {
    let Some(selected) = value.get("observation_fault") else {
        return Ok(None);
    };
    let site = match selected["site"].as_str() {
        Some("child_output") => ObservationSite::ChildOutput,
        Some("quiet_timer") => ObservationSite::QuietTimer,
        Some("resize") => ObservationSite::Resize,
        Some("shutdown_drain") => ObservationSite::ShutdownDrain,
        _ => return Err(error("owned observation fault site required")),
    };
    let operation = match selected["operation"].as_str() {
        Some("parser") => ObservationOperation::Parser,
        Some("detector") => ObservationOperation::Detector,
        _ => return Err(error("owned observation fault operation required")),
    };
    Ok(Some(Arc::new(Hooks {
        site,
        operation,
        directory: directory.to_path_buf(),
    })))
}
fn configure(fixture: &mut Fixture, spec: &mut SpawnSpec, site: &str, operation: &str) {
    synthetic(fixture, spec);
    let path = fixture._dir.path().join("observation-services.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["observation_fault"] = json!({"site":site,"operation":operation});
    std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
}
fn arm(fixture: &ServiceFixture) {
    private_write(
        &fixture.fixture._dir.path().join("observation-fault-armed"),
        b"owned one-shot fault",
    );
}
fn assert_fired(fixture: &ServiceFixture, expected: &str) {
    let path = fixture.fixture._dir.path().join("observation-fault-fired");
    wait_until(|| path.exists());
    assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
    assert!(
        !fixture
            .fixture
            ._dir
            .path()
            .join("observation-fault-armed")
            .exists()
    );
}
fn assert_private_cause_stays_local(fixture: &ServiceFixture) {
    let mut statement = fixture.fixture.conn.prepare(
        "SELECT payload_json FROM member_runtime_events UNION ALL SELECT evidence_json FROM member_interactions UNION ALL SELECT text FROM messages UNION ALL SELECT cleanup_diagnostics_json FROM member_runs WHERE cleanup_diagnostics_json IS NOT NULL",
    ).unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap();
    for row in rows {
        assert!(!row.unwrap().contains(CANARY));
    }
}
fn assert_durable_gap(fixture: &ServiceFixture) {
    wait_until(|| count(fixture, "runner_degraded") >= 2);
    let payloads: Vec<String> = fixture.fixture.conn.prepare(
        "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='runner_degraded' ORDER BY event_seq",
    ).unwrap().query_map([fixture.fixture.run.run_id], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let causes: Vec<Value> = payloads
        .iter()
        .map(|text| serde_json::from_str::<Value>(text).unwrap()["cause"].clone())
        .collect();
    assert!(
        causes
            .iter()
            .any(|cause| cause["kind"] == "observer_error"
                && cause["gap_count"].as_u64().unwrap() > 0)
    );
    assert!(
        causes
            .iter()
            .any(|cause| cause["kind"] == "storage_recovered")
    );
}
fn assert_live_after_fault(fixture: &mut ServiceFixture, expected_diagnostic: &str) {
    assert!(fixture.child.try_wait().unwrap().is_none());
    let status = health(fixture);
    assert_eq!(status["observation"]["eligibility"]["state"], "hold");
    assert!(status["observation"]["gap_count"].as_u64().unwrap() > 0);
    if let Some(diagnostic) = status["observation"]["diagnostic"].as_str() {
        assert_eq!(diagnostic, expected_diagnostic);
    }
    fixture.assert_manual_io();
    assert_durable_gap(fixture);
}
fn install_trap(spec: &SpawnSpec, command: &str) {
    let script = Path::new(&spec.argv[0]);
    let body = std::fs::read_to_string(script).unwrap();
    let (prefix, suffix) = body
        .split_once("while IFS=")
        .expect("owned synthetic input loop");
    std::fs::write(
        script,
        format!("{prefix}trap '{command}' TERM\nwhile IFS={suffix}"),
    )
    .unwrap();
}
fn shutdown_reply(fixture: &mut ServiceFixture) -> ShutdownOutcome {
    let mut wire = fixture.connect("observation fault shutdown");
    let reply = wire
        .exchange(&fixture.request("shutdown", fixture.prepared.as_ref().unwrap()))
        .unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    serde_json::from_value(reply["result"].clone()).unwrap()
}

#[test]
fn runner_review_r3_quiet_timer_detector_failure_keeps_real_service_health_and_manual_io() {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, "quiet_timer", "detector")
    });
    fixture.start_running();
    arm(&fixture);
    fixture.master.write_all(b"form-quiet-failure\r").unwrap();
    fixture.read_until("Operation: write form-quiet-failure.txt");
    assert_fired(&fixture, "QuietTimer/Detector");
    assert_live_after_fault(&mut fixture, "detector_unavailable");
    fixture.finish();
    assert_private_cause_stays_local(&fixture);
}

#[test]
fn runner_review_r3_resize_detector_failure_keeps_actual_resized_service_and_native_child() {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, "resize", "detector")
    });
    let started = fixture.start_running();
    fixture.master.write_all(b"form-resize-failure\r").unwrap();
    fixture.read_until("Operation: write form-resize-failure.txt");
    wait_until(|| count(&fixture, "interaction_opened") == 1);
    let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
    arm(&fixture);
    rustix::termios::tcsetwinsize(
        &fixture.master,
        rustix::termios::Winsize {
            ws_row: 25,
            ws_col: 90,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
    assert!(fixture.process.as_ref().unwrap().matches().unwrap());
    kill(
        Pid::from_raw(i32::try_from(fixture.child.id()).unwrap()),
        Signal::SIGWINCH,
    )
    .unwrap();
    assert_fired(&fixture, "Resize/Detector");
    let status = fixture.health();
    assert_eq!(status.child, Some(started.child));
    assert!(health(&mut fixture)["output_seq"].as_u64().unwrap() > sequence);
    assert_live_after_fault(&mut fixture, "detector_unavailable");
    fixture.finish();
    assert_private_cause_stays_local(&fixture);
}

#[test]
fn runner_review_r3_child_output_parser_and_detector_failures_keep_transport_and_typed_metadata() {
    for (operation, fired, diagnostic) in [
        ("parser", "ChildOutput/Parser", "parser_unavailable"),
        ("detector", "ChildOutput/Detector", "detector_unavailable"),
    ] {
        let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
            configure(fixture, spec, "child_output", operation)
        });
        fixture.start_running();
        arm(&fixture);
        fixture.master.write_all(b"form-output-failure\r").unwrap();
        fixture.read_until("Operation: write form-output-failure.txt");
        assert_fired(&fixture, fired);
        assert_live_after_fault(&mut fixture, diagnostic);
        fixture.finish();
        assert_private_cause_stays_local(&fixture);
    }
}

#[test]
fn runner_review_r3_shutdown_detector_failure_records_gap_before_truthful_native_exit() {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, "shutdown_drain", "detector");
        install_trap(
            spec,
            "printf \"\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write shutdown-failure.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \"; sleep 0.2; exit 0",
        );
    });
    fixture.start_running();
    arm(&fixture);
    let begin = Instant::now();
    let outcome = shutdown_reply(&mut fixture);
    assert!(begin.elapsed() < Duration::from_secs(3));
    assert_fired(&fixture, "ShutdownDrain/Detector");
    assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
    assert_eq!(
        outcome.cleanup_state,
        CleanupState::Complete,
        "{:?}",
        outcome.diagnostics
    );
    assert_eq!(outcome.pending_event_frames, 0);
    assert_eq!(outcome.unsaved_observation_gap, 0);
    assert_durable_gap(&fixture);
    assert_eq!(fixture.wait().code(), Some(0));
    assert_eq!(count(&fixture, "runner_exited"), 1);
    assert_private_cause_stays_local(&fixture);
}

#[test]
fn runner_review_r3_shutdown_parser_loss_after_terminal_fence_remains_unsaved_without_live_append()
{
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, "shutdown_drain", "parser");
        spec.environment.push((
            "CAFLEET_REVIEW_TERMINAL_TAIL_RELEASE".into(),
            fixture
                ._dir
                .path()
                .join("release-terminal-tail")
                .to_str()
                .unwrap()
                .into(),
        ));
        install_trap(
            spec,
            "(trap \"\" HUP INT TERM; index=0; while [ ! -e \"$CAFLEET_REVIEW_TERMINAL_TAIL_RELEASE\" ]; do index=$((index + 1)); [ \"$index\" -lt 150 ] || exit 7; sleep 0.01; done; printf \"review-after-terminal-tail\\n\") & exit 0",
        );
    });
    fixture.start_running();
    arm(&fixture);
    let mut wire = fixture.connect("post-terminal observation loss");
    let request = fixture.request("shutdown", fixture.prepared.as_ref().unwrap());
    let begin = Instant::now();
    let reply = std::thread::scope(|scope| {
        let response = scope.spawn(move || wire.exchange(&request));
        wait_until(|| count(&fixture, "runner_exited") == 1);
        private_write(
            &fixture.fixture._dir.path().join("release-terminal-tail"),
            b"owned descendant tail release",
        );
        response.join().unwrap().unwrap()
    });
    assert!(begin.elapsed() < Duration::from_secs(5));
    assert!(reply.get("error").is_none(), "{reply}");
    let outcome: ShutdownOutcome = serde_json::from_value(reply["result"].clone()).unwrap();
    assert_fired(&fixture, "ShutdownDrain/Parser");
    assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
    assert_eq!(outcome.pending_event_frames, 0);
    assert!(outcome.unsaved_observation_gap > 0);
    assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
    let _exit = fixture.wait();
    let kinds: Vec<String> = fixture
        .fixture
        .conn
        .prepare("SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq")
        .unwrap()
        .query_map([fixture.fixture.run.run_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(kinds, ["runner_exited"]);
    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Exited);
    assert_eq!(run.exit_code, Some(0));
    assert_eq!(run.cleanup_state, CleanupState::Incomplete);
    assert_private_cause_stays_local(&fixture);
}
