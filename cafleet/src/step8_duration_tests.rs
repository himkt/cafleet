// Stage inside runtime::system::tests as a child module using include! with
// concat!(env!("CARGO_MANIFEST_DIR"), "/src/step8_duration_tests.rs").
// Reuse the existing process-fixture mutex so concurrent process tests cannot
// exhaust the deadline before the PID fixture gets scheduled.
use super::{ProcessFixture, SystemRunner};
use crate::multiplexer::{RunError, spawn::TimedCommandRunner};
use std::time::Duration;
fn run(fixture: &ProcessFixture, script: &str, timeout: Duration) -> Result<String, RunError> {
    SystemRunner.run_for(
        &[
            "/bin/sh".into(),
            "-c".into(),
            format!("printf '%s' \"$$\" > \"$1/pid\"\n{script}"),
            "cafleet-duration-test".into(),
            fixture.dir.path().to_str().unwrap().into(),
        ],
        timeout,
    )
}
#[test]
fn duration_entry_drains_large_dual_streams_and_keeps_exit_classification() {
    let fixture = ProcessFixture::new();
    let result = run(
        &fixture,
        "exec /usr/bin/python3 -c 'import os; os.write(1,b\"x\"*1048576); os.write(2,b\"y\"*1048576)'",
        Duration::from_millis(10_125),
    );
    fixture.assert_direct_child_reaped(&result);
    assert_eq!(result.unwrap(), "x".repeat(1_048_576));
    let result = run(
        &fixture,
        "exec /usr/bin/python3 -c 'import os; os.write(1,b\"x\"*1048576); os.write(2,b\"y\"*1048576); raise SystemExit(7)'",
        Duration::from_millis(10_125),
    );
    fixture.assert_direct_child_reaped(&result);
    assert_eq!(
        result,
        Err(RunError::Failed {
            stderr: "y".repeat(1_048_576)
        })
    );
}
#[test]
fn duration_timeout_uses_fractional_budget_and_reaps_direct_child() {
    let fixture = ProcessFixture::new();
    let result = run(&fixture, "exec /bin/sleep 5", Duration::from_millis(1_125));
    assert_eq!(result, Err(RunError::Timeout));
    fixture.assert_direct_child_reaped(&result);
}
