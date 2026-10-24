mod common;
#[path = "common/receive.rs"]
mod receive;

use std::time::{Duration, Instant};

use common::{code, stderr, stdout};
use receive::{
    APPROVAL, DRAFT, Fixture, IDLE, UNKNOWN, WORKING, Worker, assert_keys, assert_no_input,
    assert_result, json,
};

fn assert_diagnostic(value: &serde_json::Value, worker: &Worker, message: i64, delivery: &str) {
    assert_keys(value, &["member_id", "message_id", "delivery", "error"]);
    assert_eq!(value["member_id"], worker.id);
    assert_eq!(value["message_id"], message);
    assert_eq!(value["delivery"], delivery);
    assert!(!value["error"].as_str().unwrap().is_empty());
}

#[test]
fn retries_rotate_members_once_per_second_in_memory() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let first = fixture.worker("first");
    let second = fixture.worker("second");
    for worker in [&first, &second] {
        fixture.view(worker, WORKING);
        fixture.assignment(worker, "held assignment");
    }
    let before = fixture.records();
    let mut waiting = fixture.spawn(fixture.director, 6);
    let mut observed = Vec::new();
    for count in 1..=3 {
        waiting.until("next round-robin retry", || {
            fixture
                .events()
                .iter()
                .filter(|event| event.stage == "capture")
                .count()
                >= count
        });
        observed.push(Instant::now());
    }
    let captures: Vec<_> = fixture
        .events()
        .into_iter()
        .filter(|event| event.stage == "capture")
        .collect();
    assert_eq!(captures.len(), 3);
    assert_ne!(captures[0].pane, captures[1].pane);
    assert_eq!(captures[0].pane, captures[2].pane);
    assert!(
        captures
            .iter()
            .all(|event| { event.pane == first.pane || event.pane == second.pane })
    );
    assert!(
        observed
            .windows(2)
            .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(750))
    );
    assert_eq!(fixture.records(), before);
    fixture.reply(&first, "stop receiving with a real reply");
    let result = json(&waiting.output());
    assert_result(&result, fixture.director, "messages");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_no_input(&fixture);
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
        let gate = fixture.gate(&first, "capture");
        let mut waiting = fixture.spawn(fixture.director, 5);
        waiting.until("locked first capture", || {
            fixture.count("capture", &first) == 1
        });
        fixture.assignment(&second, "other member");
        let reply = fixture.reply(&second, "incoming result");
        gate.release();
        let result = json(&waiting.output());
        assert_eq!(result["messages"][0]["message_id"], reply);
        assert!(fixture.stamp(old).is_some());
        assert_eq!(fixture.stamp(later), None);
        assert_eq!(fixture.count("capture", &first), 1);
        assert_eq!(fixture.count("capture", &second), 0);
        let literals: Vec<_> = fixture
            .cli
            .shim_calls()
            .into_iter()
            .filter(|line| line.contains(" -l "))
            .collect();
        assert_eq!(literals.len(), 1);
        assert!(
            literals[0].contains(&format!("[cafleet msg {old} from ")),
            "{literals:?}"
        );
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
fn retries_exclude_director_self_acked_notified_and_pending_placements() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let subject = fixture.worker("observer");
    let worker = fixture.worker("worker");
    let pending = fixture.worker("pending");
    fixture.reply(&worker, "Director receives through broker");
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
    let before = fixture.records();
    let result = json(&fixture.spawn(subject.id, 1).output());
    assert_result(&result, subject.id, "timeout");
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.records(), before);
    assert!(fixture.stamp(self_id).is_some());
    assert!(fixture.cli.shim_calls().is_empty());
}

#[test]
fn retry_candidates_stay_inside_the_subjects_fleet() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let local = fixture.worker("local");
    fixture.view(&local, WORKING);
    fixture.assignment(&local, "local work");
    let (other_fleet, other_director) = fixture.cli.seed_fleet("other");
    let foreign_id = fixture.cli.seed_member(other_fleet, "foreign");
    let foreign_pane = format!("%{}", foreign_id + 4);
    fixture.cli.set_pane_capture(&foreign_pane, IDLE);
    let foreign_message = fixture.message(other_director, foreign_id, "foreign work");
    let mut waiting = fixture.spawn(fixture.director, 4);
    waiting.until("local retry", || fixture.count("capture", &local) >= 1);
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
fn a_busy_pane_is_retried_after_ownership_is_released() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "pending behind input");
    let lock = fixture.lock(&worker);
    let mut waiting = fixture.spawn(fixture.director, 5);
    std::thread::sleep(Duration::from_millis(1200));
    waiting.assert_running();
    assert!(fixture.events().is_empty());
    assert_eq!(fixture.stamp(message), None);
    drop(lock);
    waiting.until("retry after lock release", || {
        fixture.count("enter_done", &worker) >= 1
    });
    fixture.reply(&worker, "ready");
    let result = json(&waiting.output());
    assert!(fixture.stamp(message).is_some());
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.count("literal", &worker), 1);
}

#[test]
fn a_working_pane_becomes_deliverable_on_a_fresh_idle_capture() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.view(&worker, WORKING);
    let message = fixture.assignment(&worker, "pending work");
    let mut waiting = fixture.spawn(fixture.director, 5);
    waiting.until("held working pane", || {
        fixture.count("capture_done", &worker) >= 1
    });
    assert_eq!(fixture.count("literal", &worker), 0);
    fixture.view(&worker, IDLE);
    waiting.until("idle retry", || fixture.count("enter_done", &worker) >= 1);
    fixture.reply(&worker, "accepted");
    let result = json(&waiting.output());
    assert!(fixture.stamp(message).is_some());
    assert!(fixture.count("capture", &worker) >= 2);
    assert_eq!(result["notification_errors"], serde_json::json!([]));
}

#[test]
fn concurrent_waiters_serialize_one_preview_and_revalidate_its_stamp() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "one preview");
    let gate = fixture.gate(&worker, "capture");
    let mut first = fixture.spawn(fixture.director, 5);
    first.until("first waiter owns capture", || {
        fixture.count("capture", &worker) == 1
    });
    let mut second = fixture.spawn(fixture.director, 5);
    std::thread::sleep(Duration::from_millis(200));
    second.assert_running();
    assert_eq!(fixture.count("capture", &worker), 1);
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
    assert_eq!(fixture.count("literal", &worker), 1);
    assert_eq!(fixture.count("enter", &worker), 1);
}

#[test]
fn ack_during_capture_is_revalidated_before_submission() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "consumed during capture");
    let gate = fixture.gate(&worker, "capture");
    let mut waiting = fixture.spawn(fixture.director, 4);
    waiting.until("capture before ACK", || {
        fixture.count("capture", &worker) == 1
    });
    fixture.ack(message);
    fixture.reply(&worker, "ACK arrived first");
    gate.release();
    let result = json(&waiting.output());
    assert_eq!(fixture.stamp(message), None);
    assert_eq!(result["notification_errors"], serde_json::json!([]));
    assert_no_input(&fixture);
}

#[test]
fn activity_and_placement_are_revalidated_after_capture_before_typing() {
    let _test_resource = common::process_test_resource_guard();
    for mutation in ["deregistered", "new-pane"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        let peer = fixture.worker("peer");
        let message = fixture.assignment(&worker, "target changes");
        let gate = fixture.gate(&worker, "capture");
        let mut waiting = fixture.spawn(fixture.director, 4);
        waiting.until("capture before placement mutation", || {
            fixture.count("capture", &worker) == 1
        });
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
            fixture.cli.set_pane_capture("%99", IDLE);
        }
        fixture.reply(&peer, "new inbox result");
        gate.release();
        let result = json(&waiting.output());
        assert_result(&result, fixture.director, "messages");
        assert_eq!(fixture.stamp(message), None);
        assert_no_input(&fixture);
    }
}

#[test]
fn draft_and_unknown_diagnostics_are_exact_and_deduplicated_per_member() {
    let _test_resource = common::process_test_resource_guard();
    for (state, view) in [("draft", DRAFT), ("unknown", UNKNOWN)] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        fixture.view(&worker, view);
        let message = fixture.assignment(&worker, "retained work");
        fixture
            .cli
            .sqlite()
            .execute(
                "UPDATE messages SET created_at='1900-01-01T00:00:00+00:00' WHERE message_id=?1",
                [message],
            )
            .unwrap();
        let before = fixture.records();
        let result = json(&fixture.spawn(fixture.director, 2).output());
        assert_result(&result, fixture.director, "timeout");
        let errors = result["notification_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_diagnostic(&errors[0], &worker, message, "not_sent");
        assert_eq!(
            errors[0]["error"],
            format!(
                "member {} is {state}; preview {message} was not sent.",
                worker.id
            )
        );
        assert_eq!(fixture.records(), before);
        assert_no_input(&fixture);
    }
}

#[test]
fn working_approval_and_lock_contention_are_ordinary_holds() {
    let _test_resource = common::process_test_resource_guard();
    for state in ["working", "approval", "busy"] {
        let fixture = Fixture::new();
        let worker = fixture.worker("worker");
        fixture.assignment(&worker, "held work");
        if state == "working" {
            fixture.view(&worker, WORKING);
        }
        if state == "approval" {
            fixture.view(&worker, APPROVAL);
        }
        let _lock = if state == "busy" {
            Some(fixture.lock(&worker))
        } else {
            None
        };
        let result = json(&fixture.spawn(fixture.director, 2).output());
        assert_eq!(result["notification_errors"], serde_json::json!([]));
        assert_no_input(&fixture);
    }
}

#[test]
fn capture_failures_are_not_sent_diagnostics_with_the_actual_cause() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "capture unavailable");
    fixture.fail(&worker, "capture", "distinct capture bridge failure");
    let result = json(&fixture.spawn(fixture.director, 2).output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
    assert!(
        errors[0]["error"]
            .as_str()
            .unwrap()
            .contains("distinct capture bridge failure")
    );
    assert_eq!(fixture.stamp(message), None);
    assert_no_input(&fixture);
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
        let captures = fixture.count("capture", &worker);
        let later = json(&fixture.spawn(fixture.director, 1).output());
        let errors = later["notification_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_diagnostic(&errors[0], &worker, message, "uncertain");
        assert_eq!(fixture.count("capture", &worker), captures);
        assert_eq!(
            fixture.count("literal", &worker),
            fixture.count("capture", &worker)
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
    fixture.fail(&worker, "capture", "first bridge failure");
    let mut waiting = fixture.spawn(fixture.director, 6);
    waiting.until("first transport error", || {
        fixture
            .events()
            .iter()
            .any(|event| event.stage == "failed" && event.detail == "first bridge failure")
    });
    fixture.fail(&worker, "capture", "second bridge failure");
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
    fixture.fail(&first, "capture", "first pane cause");
    fixture.fail(&second, "capture", "second pane cause");
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
        fixture.view(&worker, DRAFT);
        let message = fixture.assignment(&worker, "pending diagnostic");
        let mut waiting = fixture.spawn(fixture.director, 6);
        waiting.until("an accumulated readiness diagnostic", || {
            fixture.count("capture", &worker) >= 2
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
        assert_no_input(&fixture);
    }
}

fn assert_late_ack_clears_the_owned_attempt_diagnostic(stage: &str) {
    assert!(matches!(stage, "capture" | "enter"));
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let peer = fixture.worker("peer");
    let message = fixture.assignment(&worker, "preview whose earlier attempt failed");
    fixture.fail(
        &worker,
        "capture",
        "initial capture failure before late ACK",
    );
    let mut waiting = fixture.spawn(fixture.director, 3);
    let started = waiting.started;
    waiting.until("first notification error", || {
        fixture.events().iter().any(|event| {
            event.stage == "failed" && event.detail == "initial capture failure before late ACK"
        })
    });
    let receive_expiry_upper_bound = Instant::now() + Duration::from_secs(3);
    let gate = fixture.gate(&worker, stage);
    fixture.view(&worker, IDLE);
    std::fs::remove_file(
        fixture
            .control
            .join(format!("fail-capture-{}", worker.pane)),
    )
    .unwrap();
    waiting.until("later owned retry started capture", || {
        fixture.count("capture", &worker) == 2
    });
    let capture_observed_after = started.elapsed();
    assert!(
        capture_observed_after < Duration::from_secs(3),
        "retry capture observed at {capture_observed_after:?}"
    );
    waiting.until("later owned retry reached its gate", || {
        fixture.count(stage, &worker) == if stage == "capture" { 2 } else { 1 }
    });
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
    assert_eq!(fixture.count("capture", &worker), 2);
    let submissions = usize::from(stage == "enter");
    assert_eq!(fixture.count("literal", &worker), submissions);
    assert_eq!(fixture.count("enter", &worker), submissions);
    assert_eq!(fixture.count("enter_done", &worker), submissions);
    assert_eq!(fixture.stamp(message), None);
    assert_eq!(result["notification_errors"], serde_json::json!([]));

    let next = json(&fixture.spawn(fixture.director, 1).output());
    assert_result(&next, fixture.director, "messages");
    assert_eq!(next["messages"].as_array().unwrap().len(), 1);
    assert_eq!(next["messages"][0]["message_id"], reply);
    assert_eq!(next["notification_errors"], serde_json::json!([]));
    assert_eq!(fixture.count("capture", &worker), 2);
}

#[test]
fn a_late_prewrite_revalidation_clears_an_acked_preview_diagnostic_without_a_final_receive_check() {
    let _test_resource = common::process_test_resource_guard();
    assert_late_ack_clears_the_owned_attempt_diagnostic("capture");
}

#[test]
fn a_late_post_enter_stamp_observation_clears_an_acked_preview_diagnostic_without_a_final_receive_check()
 {
    let _test_resource = common::process_test_resource_guard();
    assert_late_ack_clears_the_owned_attempt_diagnostic("enter");
}

#[test]
fn a_different_pending_preview_replaces_the_member_diagnostic_identity() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.view(&worker, UNKNOWN);
    let old = fixture.assignment(&worker, "old pending");
    let mut waiting = fixture.spawn(fixture.director, 8);
    waiting.until("first diagnostic", || {
        fixture.count("capture", &worker) >= 1
    });
    fixture.ack(old);
    let next = fixture.assignment(&worker, "next pending");
    let baseline = fixture.count("capture", &worker);
    waiting.until("new pending preview diagnostic", || {
        fixture.count("capture", &worker) >= baseline + 2
    });
    fixture.reply(&worker, "return");
    let result = json(&waiting.output());
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, next, "not_sent");
    assert_eq!(
        errors[0]["error"],
        format!(
            "member {} is unknown; preview {next} was not sent.",
            worker.id
        )
    );
}

#[test]
fn successful_notification_by_another_invocation_clears_the_waiters_diagnostic() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let blocker = fixture.worker("blocker");
    fixture.view(&worker, DRAFT);
    fixture.view(&blocker, WORKING);
    let old = fixture.assignment(&worker, "old pending");
    let gate = fixture.gate(&blocker, "capture");
    let mut waiting = fixture.spawn(fixture.director, 8);
    waiting.until("readiness diagnostic before external success", || {
        fixture.count("capture", &worker) >= 2
    });
    fixture.assignment(&blocker, "working peer");
    waiting.until("waiter has moved to another pane", || {
        fixture.count("capture", &blocker) == 1
    });
    fixture.view(&worker, IDLE);
    let other = fixture.spawn_args(
        &[
            "message",
            "send",
            "--from-member-id",
            &fixture.director.to_string(),
            "--to-member-id",
            &worker.id.to_string(),
            "new committed message",
            "--json",
        ],
        7,
    );
    let other_pid = other.pid();
    let sent = json(&other.output());
    assert_eq!(sent["notification_sent"], false);
    assert!(fixture.stamp(old).is_some());
    assert!(fixture.events().iter().any(|event| event.pid == other_pid
        && event.stage == "literal"
        && event.pane == worker.pane));
    fixture.ack(sent["message"]["message_id"].as_i64().unwrap());
    fixture.reply(&blocker, "return after external success");
    gate.release();
    let result = json(&waiting.output());
    assert_eq!(result["notification_errors"], serde_json::json!([]));
}

#[test]
fn text_receives_report_readiness_diagnostics_on_stderr() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.view(&worker, UNKNOWN);
    let message = fixture.assignment(&worker, "unknown pane");
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
        stderr(&output).contains(&format!(
            "member {} is unknown; preview {message} was not sent.",
            worker.id
        )),
        "{}",
        stderr(&output)
    );
}

#[test]
fn send_reports_the_new_message_even_when_it_notifies_an_older_preview() {
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
    assert_eq!(result["notification_sent"], false);
    assert!(fixture.stamp(old).is_some());
    assert_eq!(fixture.stamp(new), None);
    let calls = fixture.cli.shim_calls();
    let literals: Vec<_> = calls.iter().filter(|line| line.contains(" -l ")).collect();
    assert_eq!(literals.len(), 1);
    assert!(literals[0].contains(&format!("[cafleet msg {old} from ")));
}

#[test]
fn committed_send_stdout_survives_notification_failure_with_its_message_id() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    fixture.fail(&worker, "capture", "notification transport is unavailable");
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
fn slow_capture_is_bounded_by_one_inflight_attempt_after_receive_deadline() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "slow capture");
    fixture.delay(&worker, "capture", "8");
    let waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
    assert_result(&result, fixture.director, "timeout");
    assert_eq!(fixture.count("capture", &worker), 1);
    assert_eq!(fixture.count("literal", &worker), 0);
    let errors = result["notification_errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_diagnostic(&errors[0], &worker, message, "not_sent");
    assert_eq!(fixture.stamp(message), None);
}

#[test]
fn capture_and_write_share_the_attempt_budget_and_stop_before_enter() {
    let _test_resource = common::process_test_resource_guard();
    let fixture = Fixture::new();
    let worker = fixture.worker("worker");
    let message = fixture.assignment(&worker, "partial write timeout");
    fixture.delay(&worker, "capture", "2");
    fixture.delay(&worker, "literal", "3");
    let waiting = fixture.spawn(fixture.director, 2);
    let start = waiting.started;
    let result = json(&waiting.output());
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(fixture.count("capture", &worker), 1);
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
