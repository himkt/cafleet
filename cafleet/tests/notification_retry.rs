mod common;
#[path = "common/receive.rs"]
mod receive;

use std::time::{Duration, Instant};

use common::{code, stderr, stdout};
use receive::{
    Fixture, Worker, assert_keys, assert_no_capture, assert_no_input, assert_no_text_input,
    assert_result, json,
};

const ESCAPE_REFUSED: &str = "escape refused by test";

fn diagnostic(worker: &Worker, message: i64, cause: &str) -> String {
    format!(
        "message {message} was not notified to member {}: {cause}; the next send to member {} or the next message wait retries it.",
        worker.id, worker.id
    )
}

fn assert_diagnostic(value: &serde_json::Value, worker: &Worker, message: i64) {
    assert_keys(value, &["member_id", "message_id", "error"]);
    assert_eq!(value["member_id"], worker.id);
    assert_eq!(value["message_id"], message);
    let error = value["error"].as_str().unwrap();
    let prefix = format!(
        "message {message} was not notified to member {}: ",
        worker.id
    );
    let suffix = format!(
        "; the next send to member {} or the next message wait retries it.",
        worker.id
    );
    let cause = error
        .strip_prefix(&prefix)
        .and_then(|text| text.strip_suffix(&suffix))
        .expect("notification diagnostics retain the exact retry template");
    assert!(!cause.is_empty());
}

fn lock_busy(worker: &Worker, message: i64) -> String {
    diagnostic(worker, message, "pane input remained busy")
}

fn literals_for(fixture: &Fixture, worker: &Worker) -> Vec<String> {
    fixture
        .cli
        .literal_calls()
        .into_iter()
        .filter(|line| line.starts_with(&format!("send-keys -t {} -l ", worker.pane)))
        .collect()
}

#[test]
fn retries_rotate_members_once_per_second_and_attempt_each_pending_message_once() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let first = fixture.worker("first");
    let second = fixture.worker("second");
    fixture.refuse_escape(&first);
    fixture.refuse_escape(&second);
    let first_id = fixture.assignment(&first, "first pending message");
    let second_id = fixture.assignment(&second, "second member pending message");
    let later_id = fixture.assignment(&first, "later message for the first member");
    let before = fixture.records();
    let mut waiting = fixture.spawn(fixture.director, 5);
    let mut observed = Vec::new();
    for count in 1..=3 {
        waiting.until("next round-robin message attempt", || {
            fixture
                .events()
                .iter()
                .filter(|event| event.stage == "failed")
                .count()
                >= count
        });
        observed.push(Instant::now());
    }
    let result = json(&waiting.output());
    assert_result(&result, fixture.director, "timeout");
    let escapes: Vec<_> = fixture
        .events()
        .into_iter()
        .filter(|event| event.stage == "escape")
        .collect();
    assert_eq!(escapes.len(), 3);
    assert_eq!(
        escapes
            .iter()
            .map(|event| event.pane.as_str())
            .collect::<Vec<_>>(),
        [&first.pane, &second.pane, &first.pane]
    );
    assert!(
        observed
            .windows(2)
            .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(750))
    );
    assert_eq!(fixture.records(), before);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 3);
    for (error, worker, message) in [
        (&errors[0], &first, first_id),
        (&errors[1], &second, second_id),
        (&errors[2], &first, later_id),
    ] {
        assert_diagnostic(error, worker, message);
        assert!(error["error"].as_str().unwrap().contains(ESCAPE_REFUSED));
    }
    assert_no_text_input(&fixture);
    assert_no_capture(&fixture);
}

#[test]
fn an_inbox_reply_is_checked_before_another_retry_attempt() {
    let _test_resource = common::process_test_resource_guard();
    for selection in ["created-at", "id-tie"] {
        let fixture = Fixture::new();
        let first = fixture.worker("first");
        let second = fixture.worker("second");
        let earlier_id = fixture.assignment(&first, "earlier identifier");
        let later_id = fixture.assignment(&first, "later identifier");
        let (old, later) = if selection == "created-at" {
            fixture.cli.sqlite().execute(
                "UPDATE messages SET created_at=CASE message_id WHEN ?1 THEN '2099-01-01T00:00:00+00:00' ELSE '1900-01-01T00:00:00+00:00' END",
                [earlier_id],
            ).unwrap();
            (later_id, earlier_id)
        } else {
            fixture
                .cli
                .sqlite()
                .execute_batch("UPDATE messages SET created_at='2026-01-01T00:00:00+00:00'")
                .unwrap();
            (earlier_id, later_id)
        };
        let gate = fixture.gate(&first, "escape");
        let mut waiting = fixture.spawn(fixture.director, 5);
        waiting.until("locked first Escape", || {
            fixture.count("escape", &first) == 1
        });
        fixture.assignment(&second, "other member");
        let reply = fixture.reply(&second, "incoming result");
        gate.release();
        let result = json(&waiting.output());
        assert_eq!(result["messages"][0]["message_id"], reply);
        assert!(fixture.stamp(old).is_some());
        assert!(fixture.stamp(later).is_some(), "one attempt drains both");
        assert_eq!(fixture.count("escape", &first), 1);
        assert_eq!(fixture.count("escape", &second), 0);
        let literals = literals_for(&fixture, &first);
        assert_eq!(literals.len(), 2, "{literals:?}");
        assert!(
            literals[0].contains(&format!("[cafleet msg {old} from ")),
            "{literals:?}"
        );
        assert!(
            literals[1].contains(&format!("[cafleet msg {later} from ")),
            "{literals:?}"
        );
        assert!(literals_for(&fixture, &second).is_empty());
    }
}

#[test]
fn immediate_incoming_messages_take_priority_over_pending_outgoing_notifications() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let assignment = fixture.assignment(&worker, "pending outgoing");
    let reply = fixture.reply(&worker, "already incoming");
    let result = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(result["messages"][0]["message_id"], reply);
    assert_eq!(fixture.stamp(assignment), None);
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn an_ordinary_subject_retries_only_other_ordinary_members_with_eligible_previews() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let subject = fixture.worker("observer");
    let worker = fixture.worker("worker");
    let eligible = fixture.worker("eligible");
    let pending = fixture.worker("pending");
    let director = fixture.director_pane();
    let to_director = fixture.reply(&worker, "Director receives through its inbox");
    let assignment = fixture.assignment(&eligible, "ordinary member receives in its pane");
    let self_id = fixture.message(worker.id, worker.id, "self note");
    let acked = fixture.assignment(&worker, "consumed");
    fixture.ack(acked);
    let notified = fixture.assignment(&worker, "already notified");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
            [notified],
        )
        .unwrap();
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
            [pending.id],
        )
        .unwrap();
    fixture.assignment(&pending, "pending pane");
    let before_director: (Option<String>, String) = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT notified_at, status_state FROM messages WHERE message_id=?1",
            [to_director],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let result = json(&fixture.spawn(subject.id, 1).output());
    assert_result(&result, subject.id, "timeout");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.stamp(to_director), None);
    assert!(fixture.stamp(assignment).is_some());
    assert!(fixture.stamp(self_id).is_some());
    let literals = literals_for(&fixture, &eligible);
    assert_eq!(literals.len(), 1, "{:?}", fixture.cli.shim_calls());
    assert!(
        literals[0].contains(&format!(
            "[cafleet msg {assignment} from {} ",
            fixture.director
        )),
        "{literals:?}"
    );
    assert_eq!(fixture.count("escape", &eligible), 1);
    for untouched in [&director, &subject, &worker, &pending] {
        assert!(
            fixture
                .events()
                .iter()
                .all(|event| event.pane != untouched.pane),
            "{}",
            untouched.pane
        );
    }
    let after_director: (Option<String>, String) = fixture
        .cli
        .sqlite()
        .query_row(
            "SELECT notified_at, status_state FROM messages WHERE message_id=?1",
            [to_director],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after_director, before_director);
    let inbox =
        json(
            &fixture
                .cli
                .run(&["message", "poll", &fixture.director.to_string(), "--json"]),
        );
    assert_eq!(inbox[0]["message_id"], to_director);
    assert_no_capture(&fixture);
}

#[test]
fn retry_candidates_stay_inside_the_subjects_fleet() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let local = fixture.worker("local");
    fixture.assignment(&local, "local work");
    let (other_fleet, other_director) = fixture.cli.seed_fleet("other");
    let foreign_id = fixture.cli.seed_member(other_fleet, "foreign");
    let foreign_pane = format!("%{}", foreign_id + 4);
    let foreign_message = fixture.message(other_director, foreign_id, "foreign work");
    let mut waiting = fixture.spawn(fixture.director, 4);
    waiting.until("local delivery", || {
        fixture.count("enter_done", &local) >= 1
    });
    fixture.reply(&local, "local reply");
    json(&waiting.output());
    assert_eq!(fixture.stamp(foreign_message), None);
    assert!(
        fixture
            .events()
            .iter()
            .all(|event| event.pane != foreign_pane)
    );
}

#[test]
fn a_lock_released_within_the_wait_lets_the_same_attempt_deliver() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "pending behind input");
    let lock = fixture.lock(&worker);
    let mut waiting = fixture.spawn(fixture.director, 5);
    std::thread::sleep(Duration::from_millis(900));
    waiting.assert_running();
    assert!(fixture.events().is_empty());
    assert_eq!(fixture.stamp(message), None);
    drop(lock);
    waiting.until("delivery after lock release", || {
        fixture.count("enter_done", &worker) >= 1
    });
    fixture.reply(&worker, "ready");
    let result = json(&waiting.output());
    assert!(fixture.stamp(message).is_some());
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.count("escape", &worker), 1);
    assert_eq!(fixture.count("literal", &worker), 1);
}

#[test]
fn a_lock_busy_for_the_whole_window_reports_one_lock_busy_entry() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "pending behind a stuck writer");
    let _lock = fixture.lock(&worker);
    let waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "deadline plus one attempt plus finalization grace: {:?}",
        start.elapsed()
    );
    assert_result(&result, fixture.director, "timeout");
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message);
    assert_eq!(errors[0]["error"], lock_busy(&worker, message));
    assert_eq!(fixture.stamp(message), None);
    assert_no_input(&fixture);
}

#[test]
fn concurrent_waiters_serialize_one_preview_and_revalidate_its_stamp() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "one preview");
    let gate = fixture.gate(&worker, "escape");
    let mut first = fixture.spawn(fixture.director, 5);
    first.until("first waiter owns the pane", || {
        fixture.count("escape", &worker) == 1
    });
    let mut second = fixture.spawn(fixture.director, 5);
    std::thread::sleep(Duration::from_millis(200));
    second.assert_running();
    assert_eq!(fixture.count("escape", &worker), 1);
    assert_eq!(fixture.count("literal", &worker), 0);
    gate.release();
    first.until("one submission", || {
        fixture.count("enter_done", &worker) == 1
    });
    let reply = fixture.reply(&worker, "shared incoming reply");
    for result in [json(&first.output()), json(&second.output())] {
        assert_eq!(result["messages"][0]["message_id"], reply);
        assert_eq!(result["notification_errors"], serde_json::json!([]));
    }
    assert!(fixture.stamp(message).is_some());
    assert_eq!(fixture.count("escape", &worker), 1);
    assert_eq!(fixture.count("literal", &worker), 1);
    assert_eq!(fixture.count("enter", &worker), 1);
}

#[test]
fn an_ack_during_the_lock_wait_is_revalidated_before_input() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "consumed during the lock wait");
    let lock = fixture.lock(&worker);
    let mut waiting = fixture.spawn(fixture.director, 4);
    std::thread::sleep(Duration::from_millis(300));
    waiting.assert_running();
    fixture.ack(message);
    fixture.reply(&worker, "ACK arrived first");
    drop(lock);
    let result = json(&waiting.output());
    assert_eq!(fixture.stamp(message), None);
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_no_input(&fixture);
}

#[test]
fn activity_and_placement_are_revalidated_under_the_lock_before_typing() {
    let _test_resource = common::process_test_resource_guard();
    for mutation in ["deregistered", "new-pane"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let peer = fixture.worker("peer");
        let message = fixture.assignment(&worker, "target changes");
        let lock = fixture.lock(&worker);
        let mut waiting = fixture.spawn(fixture.director, 4);
        std::thread::sleep(Duration::from_millis(300));
        waiting.assert_running();
        if mutation == "deregistered" {
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
        } else {
            fixture
                .cli
                .sqlite()
                .execute(
                    "UPDATE member_placements SET mux_pane_id='%99' WHERE member_id=?1",
                    [worker.id],
                )
                .unwrap();
        }
        fixture.reply(&peer, "new inbox result");
        drop(lock);
        let result = json(&waiting.output());
        assert_result(&result, fixture.director, "messages");
        assert_eq!(fixture.stamp(message), None);
        assert_no_input(&fixture);
    }
}

#[test]
fn escape_failures_report_the_not_notified_template_and_actual_cause() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "escape unavailable");
    fixture.fail(&worker, "escape", "distinct escape bridge failure");
    let result = json(&fixture.spawn(fixture.director, 2).output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message);
    assert!(
        errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("distinct escape bridge failure")
    );
    assert_eq!(fixture.stamp(message), None);
    assert_no_text_input(&fixture);
}

#[test]
fn a_failed_message_stays_pending_while_a_later_message_for_the_same_member_progresses() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let previous = fixture.assignment(&worker, "first message fails");
    let next = fixture.assignment(&worker, "later message progresses");
    fixture.refuse_escape(&worker);
    let mut waiting = fixture.spawn(fixture.director, 5);
    waiting.until("first message failure", || {
        fixture.count("failed", &worker) == 1
    });
    std::fs::remove_file(fixture.control.join(format!("fail-escape-{}", worker.pane))).unwrap();
    waiting.until("later message completes", || fixture.stamp(next).is_some());
    fixture.reply(&worker, "return");
    let result = json(&waiting.output());
    assert_eq!(fixture.stamp(previous), None);
    assert_eq!(fixture.count("escape", &worker), 2);
    assert_eq!(fixture.count("literal", &worker), 1);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, previous);
    assert!(literals_for(&fixture, &worker)[0].contains(&format!("[cafleet msg {next} from ")));
}

#[test]
fn transport_failures_leave_pending_messages_retryable_by_a_later_wait() {
    let _test_resource = common::process_test_resource_guard();
    for stage in ["escape", "literal", "enter"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let message = fixture.assignment(&worker, "retry the same message ID");
        fixture.fail(&worker, stage, &format!("distinct {stage} failure"));
        for invocation in 1..=2 {
            let result = json(&fixture.spawn(fixture.director, 2).output());
            let errors = result["notification_errors"].as_array().unwrap();
            assert_eq!(errors.len(), 1);
            assert_diagnostic(&errors[0], &worker, message);
            assert!(
                errors[0]["error"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("distinct {stage} failure"))
            );
            assert_eq!(fixture.stamp(message), None);
            assert_eq!(fixture.count("escape", &worker), invocation);
            assert_eq!(
                fixture.count("literal", &worker),
                if stage == "escape" { 0 } else { invocation }
            );
            assert_eq!(
                fixture.count("enter", &worker),
                if stage == "enter" { invocation } else { 0 }
            );
        }
        std::fs::remove_file(
            fixture
                .control
                .join(format!("fail-{stage}-{}", worker.pane)),
        )
        .unwrap();
        let recovered = json(&fixture.spawn(fixture.director, 2).output());
        assert_eq!(recovered["notification_errors"], serde_json::json!([]));
        assert!(fixture.stamp(message).is_some());
        assert_eq!(fixture.count("escape", &worker), 3);
    }
}

#[test]
fn distinct_message_failures_for_one_member_retain_separate_diagnostics() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let first = fixture.assignment(&worker, "first message");
    let second = fixture.assignment(&worker, "second message");
    fixture.fail(&worker, "escape", "first bridge failure");
    let mut waiting = fixture.spawn(fixture.director, 6);
    waiting.until("first transport error", || {
        fixture.count("failed", &worker) == 1
    });
    fixture.fail(&worker, "escape", "second bridge failure");
    waiting.until("second message transport error", || {
        fixture.count("failed", &worker) == 2
    });
    fixture.reply(&worker, "return");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    for (error, message, cause) in [
        (&errors[0], first, "first bridge failure"),
        (&errors[1], second, "second bridge failure"),
    ] {
        assert_diagnostic(error, &worker, message);
        assert!(error["error"].as_str().unwrap().contains(cause));
    }
}

#[test]
fn independent_members_keep_one_diagnostic_per_failed_message() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let first = fixture.worker("first");
    let second = fixture.worker("second");
    let first_id = fixture.assignment(&first, "first work");
    let second_id = fixture.assignment(&second, "second work");
    fixture.fail(&first, "escape", "first pane cause");
    fixture.fail(&second, "escape", "second pane cause");
    let mut waiting = fixture.spawn(fixture.director, 6);
    waiting.until("both member errors", || {
        fixture
            .events()
            .iter()
            .filter(|event| event.stage == "failed")
            .count()
            >= 2
    });
    fixture.reply(&first, "stop");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    for (worker, id, cause) in [
        (&first, first_id, "first pane cause"),
        (&second, second_id, "second pane cause"),
    ] {
        let error = errors
            .iter()
            .find(|error| error["member_id"] == worker.id)
            .unwrap();
        assert_diagnostic(error, worker, id);
        assert!(error["error"].as_str().unwrap().contains(cause));
    }
}

#[test]
fn diagnostics_are_reconciled_after_ack_departure_or_external_notification() {
    let _test_resource = common::process_test_resource_guard();
    for cleanup in ["ack", "leave", "notified"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let peer = fixture.worker("peer");
        fixture.refuse_escape(&worker);
        let message = fixture.assignment(&worker, "pending diagnostic");
        let mut waiting = fixture.spawn(fixture.director, 6);
        waiting.until("an accumulated delivery diagnostic", || {
            fixture.count("failed", &worker) >= 1
        });
        if cleanup == "ack" {
            fixture.ack(message);
        } else if cleanup == "notified" {
            fixture
                .cli
                .sqlite()
                .execute(
                    "UPDATE messages SET notified_at=created_at WHERE message_id=?1",
                    [message],
                )
                .unwrap();
        } else {
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
        }
        fixture.reply(&peer, "return now");
        let result = json(&waiting.output());
        assert_eq!(result["notification_errors"], serde_json::json!([]));
        assert_no_text_input(&fixture);
    }
}

#[test]
fn an_ack_after_enter_counts_as_notified_without_a_final_receive_check() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let peer = fixture.worker("peer");
    let message = fixture.assignment(&worker, "ACK while Enter is in flight");
    let gate = fixture.gate(&worker, "enter");
    let timeout = Duration::from_secs(2);
    let mut waiting = fixture.spawn(fixture.director, timeout.as_secs());
    waiting.until("owned attempt reaches Enter", || {
        fixture.count("enter", &worker) == 1
    });
    let expiry_upper_bound = Instant::now() + timeout;
    waiting.until("receive expires during its owned attempt", || {
        Instant::now() >= expiry_upper_bound + Duration::from_millis(100)
    });
    fixture.ack(message);
    let reply = fixture.reply(&peer, "next receive sees this");
    gate.release();
    let result = json(&waiting.output());
    assert_result(&result, fixture.director, "timeout");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.stamp(message), None);
    assert_eq!(fixture.count("escape", &worker), 1);
    assert_eq!(fixture.count("enter_done", &worker), 1);
    let next = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(next["messages"][0]["message_id"], reply);
}

#[test]
fn an_acked_message_diagnostic_is_reconciled_before_a_later_message_failure() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    let old = fixture.assignment(&worker, "old pending");
    let mut waiting = fixture.spawn(fixture.director, 8);
    waiting.until("first diagnostic", || fixture.count("failed", &worker) >= 1);
    fixture.ack(old);
    let next = fixture.assignment(&worker, "next pending");
    let baseline = fixture.count("failed", &worker);
    waiting.until("new pending preview diagnostic", || {
        fixture.count("failed", &worker) > baseline
    });
    fixture.reply(&worker, "return");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, next);
    assert!(
        errors[0]["error"]
            .as_str()
            .unwrap()
            .contains(ESCAPE_REFUSED)
    );
}

#[test]
fn text_receives_report_delivery_diagnostics_on_stderr() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.refuse_escape(&worker);
    fixture.assignment(&worker, "refused pane");
    let output = fixture
        .spawn_args(
            &[
                "message",
                "wait",
                &fixture.director.to_string(),
                "--timeout",
                "2",
            ],
            9,
        )
        .output();
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("Timed out waiting for member {}.\n", fixture.director)
    );
    assert!(
        stderr(&output).contains(ESCAPE_REFUSED),
        "{}",
        stderr(&output)
    );
}

#[test]
fn send_drains_an_older_preview_before_the_new_message() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let old = fixture.assignment(&worker, "oldest preview");
    let full = "x".repeat(300);
    let output = fixture
        .spawn_args(
            &[
                "message",
                "send",
                "--from-member-id",
                &fixture.director.to_string(),
                "--to-member-id",
                &worker.id.to_string(),
                &full,
                "--json",
            ],
            7,
        )
        .output();
    let result = json(&output);
    let new = result["message"]["message_id"].as_i64().unwrap();
    assert_ne!(new, old);
    assert_eq!(result["message"]["text"], full);
    assert_eq!(result["notification_sent"], true);
    assert!(fixture.stamp(old).is_some());
    assert!(fixture.stamp(new).is_some());
    assert_eq!(fixture.count("escape", &worker), 1, "one Escape per drain");
    let literals = literals_for(&fixture, &worker);
    assert_eq!(literals.len(), 2);
    assert!(literals[0].contains(&format!("[cafleet msg {old} from ")));
    assert!(literals[1].contains(&format!("[cafleet msg {new} from ")));
    assert!(literals[1].contains(&full), "the full body is inline");
    assert_eq!(stderr(&output), "");
}

#[test]
fn committed_send_stdout_survives_notification_failure_with_its_message_id() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.fail(&worker, "escape", "notification transport is unavailable");
    let output = fixture
        .spawn_args(
            &[
                "message",
                "send",
                "--from-member-id",
                &fixture.director.to_string(),
                "--to-member-id",
                &worker.id.to_string(),
                "persist this once",
                "--json",
            ],
            7,
        )
        .output();
    let result = json(&output);
    let id = result["message"]["message_id"].as_i64().unwrap();
    assert_eq!(result["message"]["text"], "persist this once");
    assert_eq!(result["notification_sent"], false);
    assert_eq!(fixture.stamp(id), None);
    assert!(
        stderr(&output).contains(&format!("Message {id} was persisted.")),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("notification transport is unavailable"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fixture
            .cli
            .sqlite()
            .query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn precommit_storage_failure_returns_no_committed_message_or_notification() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture
        .cli
        .sqlite()
        .execute_batch(
            "CREATE TRIGGER reject_send BEFORE INSERT ON messages
        BEGIN SELECT RAISE(ABORT, 'injected precommit failure'); END;",
        )
        .unwrap();
    let output = fixture
        .spawn_args(
            &[
                "message",
                "send",
                "--from-member-id",
                &fixture.director.to_string(),
                "--to-member-id",
                &worker.id.to_string(),
                "rejected",
                "--json",
            ],
            7,
        )
        .output();
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("injected precommit failure"),
        "{}",
        stderr(&output)
    );
    assert!(fixture.cli.shim_calls().is_empty());
    assert_eq!(
        fixture
            .cli
            .sqlite()
            .query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn a_hung_escape_is_bounded_by_its_timeout_and_types_nothing() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "hung escape");
    fixture.delay(&worker, "escape", "8");
    let waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
    assert_result(&result, fixture.director, "timeout");
    assert!(fixture.count("escape", &worker) >= 1);
    assert_eq!(fixture.count("literal", &worker), 0);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message);
    assert_eq!(fixture.stamp(message), None);
}

#[test]
fn escape_settle_and_write_share_the_attempt_budget_and_stop_before_enter() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "partial write timeout");
    fixture.delay(&worker, "escape", "0.4");
    fixture.delay(&worker, "literal", "6");
    let waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(fixture.count("escape", &worker), 1);
    assert_eq!(fixture.count("literal", &worker), 1);
    assert_eq!(fixture.count("enter", &worker), 0);
    assert_eq!(fixture.stamp(message), None);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message);
}

#[test]
fn a_notification_stamp_wait_is_bounded_and_retryable() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "blocked stamp");
    let gate = fixture.gate(&worker, "enter");
    let mut waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    waiting.until("submission before blocked stamp", || {
        fixture.count("enter", &worker) == 1
    });
    let writer = fixture.cli.sqlite();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    gate.release();
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
    let stamp: Option<String> = writer
        .query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [message],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stamp, None);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message);
    drop(writer);
    let retried = json(&fixture.spawn(fixture.director, 2).output());
    assert_eq!(retried["notification_errors"], serde_json::json!([]));
    assert!(fixture.stamp(message).is_some());
    assert_eq!(fixture.count("escape", &worker), 2);
}
