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

fn assert_diagnostic(value: &serde_json::Value, worker: &Worker, message: i64, delivery: &str) {
    assert_keys(value, &["member_id", "message_id", "delivery", "error"]);
    assert_eq!(value["member_id"], worker.id);
    assert_eq!(value["message_id"], message);
    assert_eq!(value["delivery"], delivery);
    assert!(!value["error"].as_str().unwrap().is_empty());
}

fn lock_busy(worker: &Worker, message: i64) -> String {
    format!(
        "member {}'s pane input remained busy; preview {message} was not sent.",
        worker.id
    )
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
fn retries_rotate_members_once_per_second_in_memory() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let first = fixture.worker("first");
    let second = fixture.worker("second");
    for worker in [&first, &second] {
        fixture.refuse_escape(worker);
        fixture.assignment(worker, "refused assignment");
    }
    let before = fixture.records();
    let mut waiting = fixture.spawn(fixture.director, 6);
    let mut observed = Vec::new();
    for count in 1..=3 {
        waiting.until("next round-robin retry", || {
            fixture
                .events()
                .iter()
                .filter(|event| event.stage == "escape")
                .count()
                >= count
        });
        observed.push(Instant::now());
    }
    let escapes: Vec<_> = fixture
        .events()
        .into_iter()
        .filter(|event| event.stage == "escape")
        .collect();
    assert_eq!(escapes.len(), 3);
    assert_ne!(escapes[0].pane, escapes[1].pane);
    assert_eq!(escapes[0].pane, escapes[2].pane);
    assert!(
        escapes
            .iter()
            .all(|event| { event.pane == first.pane || event.pane == second.pane })
    );
    assert!(
        observed
            .windows(2)
            .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(750))
    );
    assert_eq!(fixture.records(), before, "not-sent claims are released");
    fixture.reply(&first, "stop receiving with a real reply");
    let result = json(&waiting.output());
    assert_result(&result, fixture.director, "messages");
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    for error in errors {
        assert_eq!(error["delivery"], "not_sent");
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
fn retries_include_the_director_and_exclude_the_subject_self_acked_notified_and_pending_placements()
 {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let subject = fixture.worker("observer");
    let worker = fixture.worker("worker");
    let pending = fixture.worker("pending");
    let director = fixture.director_pane();
    let to_director = fixture.reply(&worker, "Director receives in its pane");
    let to_subject = fixture.assignment(&subject, "the subject's own preview");
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
    let result = json(&fixture.spawn(subject.id, 1).output());
    assert_result(&result, subject.id, "timeout");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert!(fixture.stamp(to_director).is_some());
    assert_eq!(fixture.stamp(to_subject), None);
    assert!(fixture.stamp(self_id).is_some());
    let literals = literals_for(&fixture, &director);
    assert_eq!(literals.len(), 1, "{:?}", fixture.cli.shim_calls());
    assert!(
        literals[0].contains(&format!("[cafleet msg {to_director} from {} ", worker.id)),
        "{literals:?}"
    );
    assert_eq!(fixture.count("escape", &director), 1);
    for untouched in [&subject, &worker, &pending] {
        assert!(
            fixture
                .events()
                .iter()
                .all(|event| event.pane != untouched.pane),
            "{}",
            untouched.pane
        );
    }
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
    waiting.until("local delivery", || fixture.count("enter_done", &local) >= 1);
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
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
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
fn escape_failures_are_not_sent_diagnostics_with_the_actual_cause() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "escape unavailable");
    fixture.fail(&worker, "escape", "distinct escape bridge failure");
    let result = json(&fixture.spawn(fixture.director, 2).output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
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
fn a_confirmed_no_input_failure_keeps_its_classification_when_claim_cleanup_fails() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let previous = fixture.assignment(&worker, "no input arrived");
    fixture.cli.sqlite().execute_batch(
        "CREATE TRIGGER reject_claim_release BEFORE UPDATE OF preview_started_at ON messages
         WHEN NEW.preview_started_at IS NULL BEGIN SELECT RAISE(ABORT, 'claim cleanup unavailable'); END;"
    ).unwrap();
    fixture.fail(&worker, "literal", "can't find pane: missing");
    let result = json(&fixture.spawn(fixture.director, 1).output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, previous, "not_sent");
    assert!(
        errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("claim cleanup unavailable")
    );
    assert_eq!(fixture.count("enter", &worker), 0);
    std::fs::remove_file(
        fixture
            .control
            .join(format!("fail-literal-{}", worker.pane)),
    )
    .unwrap();
    fixture
        .cli
        .sqlite()
        .execute_batch("DROP TRIGGER reject_claim_release")
        .unwrap();
    let next = fixture.assignment(&worker, "a later independent message");
    let result = json(&fixture.spawn(fixture.director, 2).output());
    assert!(fixture.stamp(next).is_some());
    assert_eq!(fixture.stamp(previous), None);
    assert_eq!(fixture.count("enter", &worker), 1);
    assert_diagnostic(
        &result["notification_errors"][0],
        &worker,
        previous,
        "uncertain",
    );
}

#[test]
fn a_claimed_preview_preserves_its_diagnostic_while_later_notifications_progress() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let previous = fixture.assignment(&worker, "previous uncertain notification");
    fixture
        .cli
        .sqlite()
        .execute(
            "UPDATE messages SET preview_started_at=created_at WHERE message_id=?1",
            [previous],
        )
        .unwrap();
    let next = fixture.assignment(&worker, "notify this message");
    let result = json(&fixture.spawn(fixture.director, 2).output());
    assert!(fixture.stamp(next).is_some());
    assert_eq!(fixture.stamp(previous), None);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, previous, "uncertain");
    assert_eq!(fixture.count("literal", &worker), 1);
    assert!(
        fixture
            .cli
            .shim_calls()
            .iter()
            .any(|line| line.contains(&format!("[cafleet msg {next} from ")))
    );
    fixture.ack(previous);
    let settled = json(&fixture.spawn(fixture.director, 1).output());
    assert_eq!(settled["notification_errors"], serde_json::json!([]));
}

#[test]
fn first_write_and_enter_failures_suspend_retries_across_wait_invocations() {
    let _test_resource = common::process_test_resource_guard();
    for stage in ["literal", "enter"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let message = fixture.assignment(&worker, "uncertain preview");
        fixture.fail(&worker, stage, &format!("distinct {stage} failure"));
        let result = json(&fixture.spawn(fixture.director, 2).output());
        let errors = result["notification_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_diagnostic(&errors[0], &worker, message, "uncertain");
        assert!(
            errors[0]["error"]
                .as_str()
                .unwrap()
                .contains(&format!("distinct {stage} failure"))
        );
        assert_eq!(fixture.stamp(message), None);
        let escapes = fixture.count("escape", &worker);
        let later = json(&fixture.spawn(fixture.director, 1).output());
        let errors = later["notification_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_diagnostic(&errors[0], &worker, message, "uncertain");
        assert_eq!(fixture.count("escape", &worker), escapes);
        assert_eq!(
            fixture.count("literal", &worker),
            fixture.count("escape", &worker)
        );
        assert_eq!(
            fixture.count("enter", &worker),
            if stage == "enter" {
                fixture.count("literal", &worker)
            } else {
                0
            }
        );
    }
}

#[test]
fn a_later_error_replaces_an_earlier_error_for_the_same_member() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "retry errors");
    fixture.fail(&worker, "escape", "first bridge failure");
    let mut waiting = fixture.spawn(fixture.director, 6);
    waiting.until("first transport error", || {
        fixture
            .events()
            .iter()
            .any(|event| event.stage == "failed" && event.detail == "first bridge failure")
    });
    fixture.fail(&worker, "escape", "second bridge failure");
    waiting.until("replacement transport error", || {
        fixture
            .events()
            .iter()
            .any(|event| event.stage == "failed" && event.detail == "second bridge failure")
    });
    fixture.reply(&worker, "reply");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
    assert!(
        errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("second bridge failure")
    );
    assert!(
        !errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("first bridge failure")
    );
}

#[test]
fn independent_members_keep_one_latest_diagnostic_each() {
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
            >= 3
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
        assert_diagnostic(error, worker, id, "not_sent");
        assert!(error["error"].as_str().unwrap().contains(cause));
    }
}

#[test]
fn diagnostics_are_removed_after_ack_or_member_leave_before_return() {
    let _test_resource = common::process_test_resource_guard();
    for cleanup in ["ack", "leave"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let peer = fixture.worker("peer");
        fixture.refuse_escape(&worker);
        let message = fixture.assignment(&worker, "pending diagnostic");
        let mut waiting = fixture.spawn(fixture.director, 6);
        waiting.until("an accumulated delivery diagnostic", || {
            fixture.count("failed", &worker) >= 2
        });
        if cleanup == "ack" {
            fixture.ack(message);
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
fn a_late_post_enter_stamp_observation_clears_an_acked_preview_diagnostic_without_a_final_receive_check()
 {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let peer = fixture.worker("peer");
    let message = fixture.assignment(&worker, "preview whose earlier attempt failed");
    fixture.fail(&worker, "escape", "initial escape failure before late ACK");
    let mut waiting = fixture.spawn(fixture.director, 3);
    let started = waiting.started;
    waiting.until("first notification error", || {
        fixture.events().iter().any(|event| {
            event.stage == "failed" && event.detail == "initial escape failure before late ACK"
        })
    });
    let receive_expiry_upper_bound = Instant::now() + Duration::from_secs(3);
    let gate = fixture.gate(&worker, "enter");
    std::fs::remove_file(fixture.control.join(format!("fail-escape-{}", worker.pane))).unwrap();
    waiting.until("later owned retry reached its Enter gate", || {
        fixture.count("enter", &worker) == 1
    });
    assert!(started.elapsed() < Duration::from_secs(3));
    waiting.until(
        "receive expired while the owned retry remains in flight",
        || Instant::now() >= receive_expiry_upper_bound + Duration::from_millis(100),
    );
    fixture.ack(message);
    let reply = fixture.reply(&peer, "observe this only on the next receive invocation");
    gate.release();

    let result = json(&waiting.output());
    assert_result(&result, fixture.director, "timeout");
    assert_eq!(result["messages"], serde_json::json!([]));
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(fixture.count("escape", &worker), 2);
    assert_eq!(fixture.count("literal", &worker), 1);
    assert_eq!(fixture.count("enter_done", &worker), 1);
    assert_eq!(fixture.stamp(message), None);
    assert_eq!(result["notification_errors"], serde_json::json!([]));

    let next = json(&fixture.spawn(fixture.director, 1).output());
    assert_result(&next, fixture.director, "messages");
    assert_eq!(next["messages"].as_array().unwrap().len(), 1);
    assert_eq!(next["messages"][0]["message_id"], reply);
    assert_eq!(next["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.count("escape", &worker), 2);
}

#[test]
fn a_different_pending_preview_replaces_the_member_diagnostic_identity() {
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
        fixture.count("failed", &worker) >= baseline + 2
    });
    fixture.reply(&worker, "return");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, next, "not_sent");
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
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
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
    assert_diagnostic(&errors[0], &worker, message, "uncertain");
}

#[test]
fn a_notification_stamp_wait_is_bounded_and_remains_uncertain() {
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
    assert_diagnostic(&errors[0], &worker, message, "uncertain");
}
