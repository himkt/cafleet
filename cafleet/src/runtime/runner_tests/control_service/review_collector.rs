use super::observation_services::{capture, health, private_write, wait_until};
use super::*;

#[test]
fn runner_collector_f3_common_service_arms_on_actual_resize_and_preserves_trigger_geometry_and_sequence()
 {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        capture(fixture, spec);
        private_write(
            &fixture._dir.path().join("collector-review-entry"),
            b"resize",
        );
    });
    let started = fixture.start_running();
    let before = health(&mut fixture)["output_seq"].as_u64().unwrap();
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
    let pair_path = fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/actual-resize-pair.json");
    wait_until(|| pair_path.exists());
    let pair: Value = serde_json::from_slice(&std::fs::read(pair_path).unwrap()).unwrap();
    assert_eq!(pair["status"], "complete");
    let samples = pair["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0]["trigger"], "resize");
    assert_eq!(samples[1]["trigger"], "timer");
    for sample in samples {
        assert_eq!(sample["rows"], 25);
        assert_eq!(sample["cols"], 90);
        assert_eq!(sample["screen_seq"], before + 1);
    }
    assert!(
        samples[1]["monotonic_start_ns"]
            .as_u64()
            .unwrap()
            .checked_sub(samples[0]["monotonic_end_ns"].as_u64().unwrap())
            .unwrap()
            >= 100_000_000
    );
    assert_eq!(pair["source"]["clock"]["kind"], "real");
    assert_eq!(fixture.health().child, Some(started.child));
    assert!(fixture.child.try_wait().unwrap().is_none());
    fixture.assert_manual_io();
    fixture.finish();
}
