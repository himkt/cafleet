mod common;
#[path = "common/receive.rs"]
mod receive;

use std::os::unix::process::ExitStatusExt;
use std::time::Duration;

use common::{Cli, code, stderr, stdout};
use receive::{
    Fixture, assert_keys, assert_no_capture, assert_no_input, assert_no_text_input, assert_result,
    json,
};

#[test]
fn wait_help_declares_the_default_timeout() {
    let _test_resource = common::process_test_resource_guard();
    let cli = Cli::new();
    let output = cli.run(&["message", "wait", "--help"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let help = stdout(&output);
    assert!(help.contains("--timeout"), "{help}");
    assert!(
        regex::Regex::new(r"default:\s*300")
            .unwrap()
            .is_match(&help),
        "{help}"
    );
    assert!(help.contains("--json"), "{help}");
    assert!(cli.shim_calls().is_empty());
}

#[test]
fn wait_rejects_timeout_values_outside_the_integer_range_before_io() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    for timeout in ["0", "-1", "1801", "1.5", "abc", "18446744073709551616"] {
        let timeout_arg = format!("--timeout={timeout}");
        let output = fixture.cli.run(&[
            "message",
            "wait",
            &fixture.director.to_string(),
            &timeout_arg,
            "--json",
        ]);
        assert_eq!(code(&output), 2, "{timeout}: {}", stderr(&output));
        assert_eq!(stdout(&output), "");
        let error = stderr(&output);
        assert!(error.contains("invalid value"), "{timeout}: {error}");
        assert!(error.contains("--timeout"), "{timeout}: {error}");
    }
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn existing_inbox_returns_full_ordered_poll_records_for_default_and_range_endpoints() {
    let _test_resource = common::process_test_resource_guard();
    for timeout in [None, Some("1"), Some("1800")] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let long = "界".repeat(250);
        let first = fixture.reply(&worker, &long);
        let second = fixture.reply(&worker, "second");
        fixture.cli.sqlite().execute(
            "UPDATE messages SET created_at='2099-01-01T00:00:00+00:00',status_timestamp='2026-01-01T00:00:00+00:00' WHERE message_id=?1", [first]).unwrap();
        fixture.cli.sqlite().execute(
            "UPDATE messages SET created_at='2020-01-01T00:00:00+00:00',status_timestamp='2026-02-01T00:00:00+00:00' WHERE message_id=?1", [second]).unwrap();
        let polled =
            json(
                &fixture
                    .cli
                    .run(&["message", "poll", &fixture.director.to_string(), "--json"]),
            );
        let subject = fixture.director.to_string();
        let mut args = vec!["message", "wait", subject.as_str(), "--json"];
        if let Some(timeout) = timeout {
            args.extend(["--timeout", timeout]);
        }
        let run = fixture.spawn_args(&args, 4);
        let start = run.started;
        let result = json(&run.output());
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_result(&result, fixture.director, "messages");
        assert_eq!(result["messages"], polled);
        assert_eq!(result["messages"][0]["message_id"], second);
        assert_eq!(result["messages"][1]["message_id"], first);
        assert_eq!(result["messages"][1]["text"], long);
        assert_keys(
            &result["messages"][0],
            &[
                "message_id",
                "from_member_id",
                "to_member_id",
                "created_at",
                "status_state",
                "status_timestamp",
                "text",
            ],
        );
        assert_eq!(result["notification_errors"], serde_json::json!([]));
        assert_eq!(fixture.stamp(first), None);
        assert_eq!(fixture.stamp(second), None);
        assert!(fixture.cli.shim_calls().is_empty());
    }
}

#[test]
fn equal_status_timestamps_keep_descending_message_id_order() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let first = fixture.reply(&worker, "first");
    let second = fixture.reply(&worker, "second");
    fixture
        .cli
        .sqlite()
        .execute_batch("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00'")
        .unwrap();
    let result = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(result["messages"][0]["message_id"], second);
    assert_eq!(result["messages"][1]["message_id"], first);
}

#[test]
fn timeout_json_is_one_ordered_object_and_starts_no_synthetic_input() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let before = fixture.records();
    let run = fixture.spawn(fixture.director, 1);
    let start = run.started;
    let output = run.output();
    let result = json(&output);
    assert_result(&result, fixture.director, "timeout");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert!(start.elapsed() >= Duration::from_millis(850));
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(stderr(&output), "");
    assert_eq!(fixture.records(), before);
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn wait_text_preserves_timeout_and_full_message_lifecycle_output() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let subject = fixture.director.to_string();
    let timeout = fixture
        .spawn_args(&["message", "wait", &subject, "--timeout", "1"], 4)
        .output();
    assert_eq!(code(&timeout), 0, "{}", stderr(&timeout));
    assert_eq!(
        stdout(&timeout),
        format!("Timed out waiting for member {}.\n", fixture.director)
    );
    let worker = fixture.worker("worker");
    let id = fixture.reply(&worker, "visible reply");
    let show = fixture.cli.run(&["message", "show", &id.to_string()]);
    assert_eq!(code(&show), 0);
    let output = fixture
        .spawn_args(&["message", "wait", &subject, "--timeout", "1"], 4)
        .output();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "Messages available for member {}.\n{}",
            fixture.director,
            stdout(&show)
        )
    );
    assert_eq!(stderr(&output), "");
    assert_no_input(&fixture);
}

#[test]
fn incoming_messages_are_available_without_a_multiplexer() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let id = fixture.reply(&worker, "broker-only reply");
    let output = fixture.cli.run_outside_tmux(&[
        "message",
        "wait",
        &fixture.director.to_string(),
        "--timeout",
        "1",
        "--json",
    ]);
    let result = json(&output);
    assert_eq!(result["messages"][0]["message_id"], id);
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn repeated_reads_keep_unacknowledged_messages_and_do_not_ack_implicitly() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.reply(&worker, "question requiring a later action");
    let before = fixture.records();
    let first = json(&fixture.spawn(fixture.director, 1).output());
    let second = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(first["messages"], second["messages"]);
    assert_eq!(second["messages"][0]["status_state"], "input_required");
    assert_eq!(fixture.records(), before);
}

#[test]
fn arrival_during_a_finite_receive_returns_the_new_unacknowledged_reply() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    fixture.assignment(&worker, "ongoing assignment");
    let mut waiting = fixture.spawn(fixture.director, 4);
    waiting.until("first retry attempt", || {
        fixture.count("failed", &worker) >= 1
    });
    let id = fixture.reply(&worker, "new reply");
    let output = waiting.output();
    let result = json(&output);
    assert_result(&result, fixture.director, "messages");
    assert_eq!(result["messages"][0]["message_id"], id);
    assert_eq!(result["messages"][0]["status_state"], "input_required");
    assert_no_text_input(&fixture);
    assert_no_capture(&fixture);
}

#[test]
fn ack_on_read_allows_a_deferred_question_and_a_later_reply() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    let question = fixture.reply(&worker, "May I deploy after review?");
    let first = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(first["messages"][0]["message_id"], question);
    fixture.ack(question);
    fixture.assignment(&worker, "continue the already authorized work");
    let mut next = fixture.spawn(fixture.director, 4);
    next.until("a new receive rather than the consumed question", || {
        fixture.count("failed", &worker) >= 1
    });
    let reply = fixture.reply(&worker, "review evidence is ready");
    let result = json(&next.output());
    assert_eq!(result["messages"].as_array().unwrap().len(), 1);
    assert_eq!(result["messages"][0]["message_id"], reply);
    let shown = json(
        &fixture
            .cli
            .run(&["message", "show", &question.to_string(), "--json"]),
    );
    assert_eq!(shown["message"]["status_state"], "completed");
    assert_no_text_input(&fixture);
}

#[test]
fn unknown_and_deregistered_subjects_fail_before_retrying() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("deregistered");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
            rusqlite::params![
                cafleet::time::format_utc(cafleet::time::now_utc()),
                worker.id
            ],
        )
        .unwrap();
    for subject in [999, worker.id] {
        let output = fixture.spawn(subject, 1).output();
        assert_eq!(code(&output), 1);
        assert_eq!(stdout(&output), "");
        assert!(
            stderr(&output).contains(&format!("Member {subject} not found")),
            "{}",
            stderr(&output)
        );
    }
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn a_subject_or_fleet_invalidated_during_receive_exits_with_its_cause() {
    let _test_resource = common::process_test_resource_guard();
    for invalidation in ["subject", "fleet"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        fixture.refuse_escape(&worker);
        fixture.assignment(&worker, "pending assignment");
        let mut waiting = fixture.spawn(fixture.director, 5);
        waiting.until("receive has started", || {
            fixture.count("failed", &worker) >= 1
        });
        if invalidation == "subject" {
            fixture
                .cli
                .sqlite()
                .execute(
                    "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                    rusqlite::params![
                        cafleet::time::format_utc(cafleet::time::now_utc()),
                        fixture.director
                    ],
                )
                .unwrap();
        } else {
            fixture
                .cli
                .sqlite()
                .execute(
                    "UPDATE fleets SET deleted_at='2026-10-23T00:00:00+00:00' WHERE fleet_id=?1",
                    [fixture.fleet],
                )
                .unwrap();
        }
        let output = waiting.output();
        assert_eq!(code(&output), 1, "{}", stdout(&output));
        assert_eq!(stdout(&output), "");
        let error = stderr(&output);
        assert!(
            error.contains(&fixture.director.to_string())
                || error.contains(&fixture.fleet.to_string()),
            "{error}"
        );
        assert!(!error.contains("panicked"), "{error}");
    }
}

#[test]
fn a_deleted_fleet_is_rejected_before_waiting() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE fleets SET deleted_at='2026-10-23T00:00:00+00:00' WHERE fleet_id=?1",
            [fixture.fleet],
        )
        .unwrap();
    let output = fixture.spawn(worker.id, 1).output();
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("deleted"), "{}", stderr(&output));
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn receive_preserves_a_storage_read_failure_during_the_loop() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    fixture.assignment(&worker, "pending assignment");
    let mut waiting = fixture.spawn(fixture.director, 5);
    waiting.until("receive has started", || {
        fixture.count("failed", &worker) >= 1
    });
    fixture
        .cli
        .sqlite()
        .execute_batch("DROP TABLE messages")
        .unwrap();
    let output = waiting.output();
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("no such table: messages"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn receive_reports_the_actual_corrupt_database_error() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    std::fs::write(fixture.cli.db_path(), b"this is not a SQLite database").unwrap();
    let output = fixture.spawn(fixture.director, 1).output();
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("file is not a database"),
        "{}",
        stderr(&output)
    );
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn ctrl_c_terminates_the_owned_finite_receive_without_registry_state() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    fixture.assignment(&worker, "work retained after cancellation");
    let before = fixture.records();
    let mut waiting = fixture.spawn(fixture.director, 10);
    waiting.until("receive reached a released not-sent attempt", || {
        fixture.count("failed", &worker) >= 1
    });
    waiting.signal(nix::sys::signal::Signal::SIGINT);
    let output = waiting.output();
    assert!(!output.status.success());
    assert!(
        output.status.signal() == Some(2) || output.status.code() == Some(130),
        "{:?}",
        output.status
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(fixture.records(), before);
    assert_no_text_input(&fixture);
}
