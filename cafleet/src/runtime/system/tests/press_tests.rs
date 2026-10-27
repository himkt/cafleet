use super::*;

#[test]
fn step5_press_absolute_runner_reserves_cleanup_and_reaps_the_actual_direct_child() {
    let fixture = ProcessFixture::new();
    let argv = vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf '%s' \"$$\" > \"$1/pid\"\nexec sleep 4".into(),
        "cafleet-absolute-press-test".into(),
        fixture.dir.path().to_str().unwrap().into(),
    ];
    let not_started = SystemRunner.run_until(&argv, Instant::now() + Duration::from_millis(200));
    let not_started = match not_started {
        Ok(_) => panic!("insufficient cleanup reserve starts no process"),
        Err(cause) => cause,
    };
    assert!(matches!(not_started.source, RunError::NotStarted(_)));
    assert!(not_started.cleanup_diagnostics.is_empty());
    assert!(!fixture.dir.path().join("pid").exists());

    let started = Instant::now();
    let until = started + Duration::from_secs(2);
    let result = SystemRunner.run_until(&argv, until);
    let returned = Instant::now();
    let observed = result
        .as_ref()
        .cloned()
        .map_err(|cause| cause.source.clone());
    fixture.assert_direct_child_reaped(&observed);
    let cause = match result {
        Ok(_) => panic!("the direct child outlives the collection budget"),
        Err(cause) => cause,
    };
    assert!(
        matches!(cause.source, RunError::Timeout),
        "{:?}",
        cause.source
    );
    assert!(
        cause.cleanup_diagnostics.is_empty(),
        "{:?}",
        cause.cleanup_diagnostics
    );
    assert!(
        returned >= until - Duration::from_millis(350),
        "elapsed={:?}",
        returned - started
    );
    assert!(
        returned <= until - Duration::from_millis(150),
        "elapsed={:?}",
        returned - started
    );
}
