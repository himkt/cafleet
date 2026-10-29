use super::support::*;
use super::*;
use std::time::Duration;

#[test]
fn runner_batch2_capture_seed_prepared_and_finalized_boundaries_enable_exactly_one_request() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
    assert!(receiver.finalized_identity().is_none());
    assert!(receiver.request(fixture.request("early")).is_err());
    fixture.prepare(&mut port);
    assert!(receiver.finalized_identity().is_none());
    assert!(receiver.request(fixture.request("prepared")).is_err());
    let producer =
        read_native_capture_producer(&fixture.producer_path, &NativeCaptureLimits::default())
            .unwrap();
    assert!(
        port.bind_prepared(
            &fixture.fixture.run,
            &fixture.placement,
            &fixture.spec,
            producer,
            &fixture.clock
        )
        .is_err()
    );
    fixture.finalize(&mut port);
    let identity = receiver.finalized_identity().unwrap();
    assert_eq!(identity.run_id, fixture.fixture.run.run_id);
    assert_eq!(identity.owner_epoch, fixture.fixture.run.owner_epoch);
    let child = ProcessIdentity {
        pid: fixture.fixture.run.child_pid.unwrap(),
        start_identity: fixture.fixture.run.child_start_identity.clone().unwrap(),
    };
    assert!(
        port.finalize_source(&fixture.fixture.run, &fixture.placement, &child)
            .is_err()
    );
    receiver.request(fixture.request("accepted")).unwrap();
    assert!(receiver.request(fixture.request("second")).is_err());
    let screen = TerminalState::new(fixture.spec.size).unwrap().snapshot();
    port.observe(&screen, CaptureTrigger::Timer, &fixture.clock)
        .unwrap();
    fixture.clock.set(100);
    port.observe(&screen, CaptureTrigger::Timer, &fixture.clock)
        .unwrap();
    let lease = receiver.try_take().unwrap().unwrap();
    assert!(receiver.request(fixture.request("leased")).is_err());
    assert!(receiver.try_take().unwrap().is_none());
    drop(lease);
    assert!(receiver.request(fixture.request("consumed")).is_err());
}

#[test]
fn runner_batch2_capture_source_rejects_running_seed_and_mismatched_child_or_placement() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let (mut port, receiver) = fixture.channel(NativeCaptureLimits::default(), &[]);
    fixture.prepare(&mut port);
    let identity = ProcessIdentity::current().unwrap();
    let mut run = fixture.fixture.run.clone();
    run.status = RunStatus::Running;
    run.child_pid = Some(identity.pid);
    run.child_start_identity = Some(identity.start_identity.clone());
    run.started_at = Some("2026-10-26T17:00:00Z".into());
    let mut wrong = identity.clone();
    wrong.start_identity.push_str("-wrong");
    assert!(
        port.finalize_source(&run, &fixture.placement, &wrong)
            .is_err()
    );
    assert!(receiver.finalized_identity().is_none());
    let mut placement = fixture.placement.clone();
    placement.mux_pane_id = Some("%999".into());
    assert!(port.finalize_source(&run, &placement, &identity).is_err());
    assert!(receiver.finalized_identity().is_none());
    let policy = NativeCapturePolicy::from_launch(&fixture.spec, &[], &catalog()).unwrap();
    assert!(native_capture_channel(&run, policy, NativeCaptureLimits::default()).is_err());
    fixture.finalize(&mut port);
    assert!(receiver.finalized_identity().is_some());
}

#[test]
fn runner_batch2_capture_invalid_request_leaves_the_finalized_channel_unused() {
    for field in [
        "blank",
        "path",
        "control",
        "long",
        "unicode",
        "run",
        "epoch",
        "interval_low",
        "interval_high",
        "past",
        "deadline_high",
    ] {
        let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let (_port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
        let mut request = fixture.request("valid");
        match field {
            "blank" => request.capture_id = " ".into(),
            "path" => request.capture_id = "../private".into(),
            "control" => request.capture_id = "a\u{7f}".into(),
            "long" => request.capture_id = "a".repeat(129),
            "unicode" => request.capture_id = "世界".into(),
            "run" => request.run_id += 1,
            "epoch" => request.owner_epoch += 1,
            "interval_low" => request.minimum_interval = Duration::from_millis(99),
            "interval_high" => request.minimum_interval = Duration::from_millis(5001),
            "past" => {
                request.deadline = fixture.clock.sample().monotonic - Duration::from_millis(1)
            }
            _ => request.deadline = fixture.clock.sample().monotonic + Duration::from_secs(31),
        }
        assert!(receiver.request(request).is_err(), "{field}");
        receiver.request(fixture.request("valid")).unwrap();
    }
}

#[test]
fn runner_batch2_capture_slow_consumer_holds_one_immutable_pair_while_observation_keeps_progressing()
 {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let lease = fixture.pair(&mut port, &receiver, b"first");
    let original = value(&lease);
    let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
    let began = std::time::Instant::now();
    for _ in 0..100 {
        terminal.feed(b"more native output\r\n").unwrap();
        port.observe(
            &terminal.snapshot(),
            CaptureTrigger::ChildOutput,
            &fixture.clock,
        )
        .unwrap();
    }
    assert!(began.elapsed() < Duration::from_millis(500));
    assert_eq!(value(&lease), original);
    assert!(receiver.try_take().unwrap().is_none());
    assert!(receiver.request(fixture.request("replacement")).is_err());
}

#[test]
fn runner_batch2_capture_pair_uses_atomic_snapshots_and_second_selection_after_first_end() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
    terminal
        .feed(b"\x1b]2;first title\x07\x1b[2;4Hfirst frame")
        .unwrap();
    let first = terminal.snapshot();
    receiver.request(fixture.request("atomic")).unwrap();
    port.observe(&first, CaptureTrigger::ChildOutput, &fixture.clock)
        .unwrap();
    assert!(receiver.try_take().unwrap().is_none());
    assert!(port.next_deadline().is_some());
    terminal
        .resize(TerminalSize { rows: 25, cols: 90 })
        .unwrap();
    terminal
        .feed(b"\x1b[?1049h\x1b]2;second title\x07\x1b[3;7Hsecond frame")
        .unwrap();
    let second = terminal.snapshot();
    fixture.clock.set(99);
    port.observe(&second, CaptureTrigger::Resize, &fixture.clock)
        .unwrap();
    assert!(receiver.try_take().unwrap().is_none());
    fixture.clock.set(100);
    port.observe(&second, CaptureTrigger::Timer, &fixture.clock)
        .unwrap();
    let lease = receiver.try_take().unwrap().unwrap();
    let result = value(&lease);
    assert_keys(
        &result,
        &[
            "schema_version",
            "capture_id",
            "source",
            "policy",
            "limits",
            "status",
            "samples",
            "diagnostic",
        ],
    );
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["status"], "complete");
    let samples = result["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2);
    for (index, screen) in [&first, &second].into_iter().enumerate() {
        let sample = &samples[index];
        assert_keys(
            sample,
            &[
                "sample_index",
                "trigger",
                "monotonic_start_ns",
                "monotonic_end_ns",
                "observed_at",
                "screen_seq",
                "rows",
                "cols",
                "cursor_row",
                "cursor_col",
                "alternate_screen",
                "title",
                "projection",
                "unavailable_regions",
                "grid",
            ],
        );
        assert_eq!(sample["screen_seq"], screen.screen_seq);
        assert_eq!(sample["rows"], screen.rows);
        assert_eq!(sample["cols"], screen.cols);
        assert_eq!(sample["cursor_row"], screen.cursor_row);
        assert_eq!(sample["cursor_col"], screen.cursor_col);
        assert_eq!(sample["alternate_screen"], screen.alternate_screen);
        assert_eq!(sample["title"], screen.title);
        assert!(
            sample["monotonic_end_ns"].as_u64().unwrap()
                >= sample["monotonic_start_ns"].as_u64().unwrap()
        );
    }
    assert!(
        samples[1]["monotonic_start_ns"].as_u64().unwrap()
            - samples[0]["monotonic_end_ns"].as_u64().unwrap()
            >= 100_000_000
    );
    assert!(
        serde_json::to_string(&samples[0])
            .unwrap()
            .contains("first")
    );
    assert!(
        !serde_json::to_string(&samples[0])
            .unwrap()
            .contains("second")
    );
    assert!(port.next_deadline().is_none());
}

#[test]
fn runner_batch2_capture_metadata_projects_no_grid_title_secret_or_private_source_arguments() {
    let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let lease = fixture.pair(
        &mut port,
        &receiver,
        b"\x1b]2;PRIVATE-TITLE\x07PRIVATE-CONTENT",
    );
    let result = value(&lease);
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("PRIVATE-"));
    assert!(!encoded.contains("private assignment canary"));
    assert!(!encoded.contains("fingerprint"));
    assert!(!encoded.contains("raw_bytes"));
    for sample in result["samples"].as_array().unwrap() {
        assert!(sample["grid"].is_null());
        assert!(sample["title"].is_null());
        assert!(!sample["unavailable_regions"].as_array().unwrap().is_empty());
    }
    let source = &result["source"];
    assert_eq!(source["run_id"], fixture.fixture.run.run_id);
    assert_eq!(source["owner_epoch"], fixture.fixture.run.owner_epoch);
    assert_eq!(
        source["child"]["pid"],
        fixture.fixture.run.child_pid.unwrap()
    );
    assert_eq!(
        source["permission_argv"],
        fixture.fixture.run.permission_argv_json
    );
    assert_eq!(source["resolved_mode"], "approve-for-me");
    assert_eq!(source["producer"]["kind"], "synthetic");
    assert!(
        serde_json::to_string(&source["clock"])
            .unwrap()
            .contains("deterministic")
    );
}

#[test]
fn runner_batch2_capture_patterns_mask_whole_logical_fields_across_softwraps_and_cells() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let mut bytes = vec![b'x'; 75];
    bytes.extend_from_slice(b"PRIVATE-SECRET");
    let lease = fixture.pair(&mut port, &receiver, &bytes);
    let result = value(&lease);
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("PRIVATE-SECRET"));
    assert!(!encoded.contains("[REDACTED]"));
    assert!(!encoded.contains("PRIVATE-[A-Z]+"));
    let mut cells = Vec::new();
    objects(&result["samples"][0]["grid"], "has_contents", &mut cells);
    assert_eq!(cells.len(), 24 * 80);
    assert!(
        cells
            .iter()
            .take(89)
            .all(|cell| cell["availability"] == "masked")
    );
    assert!(
        cells
            .iter()
            .filter(|cell| cell["availability"] == "masked")
            .count()
            >= 89
    );
    for cell in cells.iter().filter(|cell| cell["availability"] == "masked") {
        for field in [
            "contents",
            "has_contents",
            "wide",
            "wide_continuation",
            "fg",
            "bg",
            "bold",
            "dim",
            "italic",
            "underline",
            "inverse",
        ] {
            assert!(
                cell.get(field)
                    .expect("required masked cell field")
                    .is_null(),
                "masked {field}"
            );
        }
    }
}

#[test]
fn runner_batch2_capture_missing_in_range_parser_cell_fails_at_the_projection_boundary() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    receiver.request(fixture.request("missing-cell")).unwrap();
    let mut screen = TerminalState::new(fixture.spec.size).unwrap().snapshot();
    screen.rows += 1;
    assert!(
        port.observe(&screen, CaptureTrigger::ChildOutput, &fixture.clock)
            .is_err()
    );
}

#[test]
fn runner_batch2_capture_pattern_configuration_is_bounded_before_observer_callbacks() {
    for patterns in [
        vec!["absent".to_owned(); 65],
        vec!["x".repeat(4097)],
        vec!["x".repeat(2048); 33],
    ] {
        let mut policy = redacted();
        policy["config"]["patterns"] = json!(patterns);
        let fixture = CaptureFixture::new(policy);
        assert!(NativeCapturePolicy::from_launch(&fixture.spec, &[], &catalog()).is_err());
    }
}

#[test]
fn runner_batch2_capture_coordinate_mask_includes_wide_partner_and_preserves_empty_cells() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[mask(0, 2, 1, 3)]);
    let lease = fixture.pair(&mut port, &receiver, "A界Z".as_bytes());
    let result = value(&lease);
    let mut cells = Vec::new();
    objects(&result["samples"][0]["grid"], "has_contents", &mut cells);
    assert_eq!(cells.len(), 1920);
    assert_eq!(
        cells
            .iter()
            .filter(|cell| cell["availability"] == "masked")
            .count(),
        2
    );
    assert!(cells.iter().any(|cell| cell["contents"] == "A"));
    assert!(cells.iter().any(|cell| cell["contents"] == "Z"));
    assert!(
        cells
            .iter()
            .any(|cell| cell["contents"] == "" && cell["has_contents"] == false)
    );
    assert!(!serde_json::to_string(&result).unwrap().contains('界'));
}

#[test]
fn runner_batch2_capture_layout_mismatch_masks_whole_content_instead_of_clipping_coordinates() {
    let mut fixture = CaptureFixture::new(redacted());
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[mask(0, 0, 1, 1)]);
    let mut terminal = TerminalState::new(TerminalSize { rows: 25, cols: 80 }).unwrap();
    terminal.feed(b"PRIVATE-LAYOUT").unwrap();
    receiver.request(fixture.request("layout")).unwrap();
    port.observe(&terminal.snapshot(), CaptureTrigger::Resize, &fixture.clock)
        .unwrap();
    fixture.clock.set(100);
    port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
        .unwrap();
    let result = value(&receiver.try_take().unwrap().unwrap());
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("PRIVATE-LAYOUT")
    );
    assert!(
        !result["samples"][0]["unavailable_regions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn runner_batch2_capture_invalid_mask_geometry_fails_before_collection() {
    let fixture = CaptureFixture::new(redacted());
    for invalid in [
        mask(0, 0, 0, 1),
        mask(0, 0, 1, 0),
        mask(0, 0, 25, 1),
        mask(0, 0, 1, 81),
    ] {
        assert!(NativeCapturePolicy::from_launch(&fixture.spec, &[invalid], &catalog()).is_err());
    }
}

#[test]
fn runner_batch2_capture_sensitive_rule_narrows_redacted_launch_to_metadata() {
    let mut policy = redacted();
    policy["config"]["sensitive_rules"] = json!(["synthetic-permission"]);
    let mut fixture = CaptureFixture::new(policy);
    let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
    let lease=fixture.pair(&mut port,&receiver,b"\x1b[19;1HSynthetic permission request\r\nOperation: PRIVATE-SECRET\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ");
    let result = value(&lease);
    for sample in result["samples"].as_array().unwrap() {
        assert!(sample["grid"].is_null());
    }
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("PRIVATE-SECRET")
    );
}

#[test]
fn runner_batch2_capture_lowered_budgets_report_capacity_without_a_padded_complete_pair() {
    for limits in [
        NativeCaptureLimits {
            max_cells: 10,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_source_bytes: 4,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_title_bytes: 4,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_scratch_bytes: 1,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_projected_heap_bytes: 1,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_encoded_sample_bytes: 1,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_encoded_result_bytes: 1,
            ..Default::default()
        },
    ] {
        let mut fixture = CaptureFixture::new(redacted());
        let (mut port, receiver) = fixture.ready(limits, &[]);
        receiver.request(fixture.request("capacity")).unwrap();
        let mut terminal = TerminalState::new(fixture.spec.size).unwrap();
        terminal
            .feed(b"\x1b]2;title too large\x07content too large")
            .unwrap();
        port.observe(
            &terminal.snapshot(),
            CaptureTrigger::ChildOutput,
            &fixture.clock,
        )
        .unwrap();
        fixture.clock.set(100);
        port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
            .unwrap();
        let result = value(
            &receiver
                .try_take()
                .unwrap()
                .expect("explicit bounded capacity result"),
        );
        assert_eq!(result["status"], "capacity");
        assert!(result["samples"].as_array().unwrap().len() < 2);
    }
}

#[test]
fn runner_batch2_capture_limits_reject_zero_and_expansion_but_allow_lowered_budgets() {
    assert!(NativeCaptureLimits::default().validate().is_ok());
    assert!(
        NativeCaptureLimits {
            max_cells: 1,
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
    for limits in [
        NativeCaptureLimits {
            max_cells: 0,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_cells: 32769,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_source_bytes: 1048577,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_patterns: 65,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_pattern_bytes: 4097,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_total_pattern_bytes: 65537,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_compiled_regex_bytes: 262145,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_pattern_input_bytes: 67108865,
            ..Default::default()
        },
        NativeCaptureLimits {
            max_interval: Duration::from_millis(99),
            ..Default::default()
        },
        NativeCaptureLimits {
            max_request_duration: Duration::from_millis(100),
            ..Default::default()
        },
    ] {
        assert!(limits.validate().is_err());
    }
}

#[test]
fn runner_batch2_capture_deadline_owner_exit_and_cancellation_preserve_actual_incomplete_status() {
    for status in [
        NativeCaptureStatus::Deadline,
        NativeCaptureStatus::OwnerExit,
        NativeCaptureStatus::Cancelled,
    ] {
        let mut fixture = CaptureFixture::new(redacted());
        let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
        receiver.request(fixture.request("incomplete")).unwrap();
        let terminal = TerminalState::new(fixture.spec.size).unwrap();
        port.observe(
            &terminal.snapshot(),
            CaptureTrigger::ChildOutput,
            &fixture.clock,
        )
        .unwrap();
        if status == NativeCaptureStatus::Deadline {
            fixture.clock.set(1001);
            port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &fixture.clock)
                .unwrap();
        } else {
            port.cancel(status).unwrap();
        }
        let result = value(&receiver.try_take().unwrap().unwrap());
        assert_eq!(result["status"], serde_json::to_value(status).unwrap());
        assert_eq!(result["samples"].as_array().unwrap().len(), 1);
        assert!(receiver.request(fixture.request("replacement")).is_err());
    }
}
