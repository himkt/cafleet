use crate::broker::records::*;
use serde_json::{Value, json};

pub(crate) fn member_press(value: &crate::runtime::KeyAcknowledgment) -> Value {
    json!({"member_id":value.member_id,"pane_id":value.pane_id,"keys":value.keys,"status":"acknowledged"})
}

pub(crate) fn runtime_recovery(report: &crate::runtime::runner::RecoveryReport) -> Value {
    json!({"member_id":report.member_id,"run_id":report.run_id,"outcome":report.outcome.as_str(),
        "replayed_count":report.replayed_count,"inbox_count":report.inbox_count,
        "archived_count":report.archived_count,"status":report.status.as_str(),
        "cleanup_diagnostics":report.cleanup_diagnostics})
}

pub(crate) fn placement(row: &Placement) -> Value {
    json!({"backend":row.backend,"mux_session":row.mux_session,
        "mux_window_id":row.mux_window_id,"mux_pane_id":row.mux_pane_id,
        "coding_agent":row.coding_agent,"created_at":row.created_at})
}

pub(crate) fn member(row: &MemberRecord) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"description":row.description,
        "status":row.status.as_str(),"registered_at":row.registered_at,
        "kind":row.kind.as_str(),
        "placement":row.placement.as_ref().map(placement),
        "runtime":row.runtime.as_ref().map(runtime_snapshot)})
}

pub(crate) fn runtime_snapshot(row: &crate::broker::runtime::RuntimeSnapshot) -> Value {
    let run = &row.run;
    json!({"run_id":run.run_id,"resolved_mode":run.resolved_mode,"status":run.status.as_str(),
        "reachability":run.reachability.as_str(),"owner_epoch":run.owner_epoch,"last_health_at":run.last_health_at,
        "coverage":run.coverage.as_str(),
        "current_interaction_id":row.current_interaction_id,
        "unresolved_input_ids":row.unresolved_input_ids,"owner_kind":run.owner_kind.as_str(),"owner_pid":run.owner_pid,
        "owner_start_identity":run.owner_start_identity,"runner_pid":run.runner_pid,"runner_start_identity":run.runner_start_identity,
        "child_pid":run.child_pid,"child_start_identity":run.child_start_identity,
        "effective_policy":run.effective_policy_json})
}

fn runtime_interaction(row: &crate::broker::runtime::InteractionRecord) -> Value {
    json!({"interaction_id":row.interaction_id,"run_id":row.run_id,"episode_seq":row.episode_seq,
        "classification":row.classification.as_str(),"evidence_digest":row.evidence_digest,"opened_at":row.opened_at,
        "evidence_json":row.evidence_json,"closed_at":row.closed_at,"close_reason":row.close_reason.map(|reason| reason.as_str())})
}

pub(crate) fn runtime_event(view: &crate::broker::runtime::RuntimeEventView) -> Value {
    let event = &view.event;
    json!({"event":event.payload_json,"notification_state":event.notification_state.as_str(),
        "message_id":event.message_id,"archive_reason":event.archive_reason.map(|reason| reason.as_str()),
        "run":runtime_snapshot(&view.run),"interaction":view.interaction.as_ref().map(runtime_interaction)})
}

pub(crate) fn input_operation(row: &crate::broker::runtime::InputOperation) -> Value {
    json!({"input":{"input_id":row.input_id,"run_id":row.run_id,"operation_key":row.operation_key,
        "kind":row.kind.as_str(),"payload_digest":row.payload_digest,"state":row.state.as_str(),
        "created_at":row.created_at,"updated_at":row.updated_at,"message_id":row.message_id,
        "disposition":row.disposition.map(|disposition| disposition.as_str()),"evidence_ref":row.evidence_ref,"permission_response":row.permission_response.as_ref().map(permission_response),
        "delivery":row.delivery().as_ref().map(crate::broker::runtime::DeliveryProjection::to_json),"deferred_at":row.deferred_at,"defer_reason":row.defer_reason,
        "payload_digest_version":row.payload_digest_version}})
}

pub(crate) fn member_activity(row: &MemberActivity) -> Value {
    json!({"member_id":row.member.member_id,"name":row.member.name,
        "kind":row.member.kind.as_str(),"placement":row.member.placement.as_ref().map(placement),
        "last_sent":row.last_sent,"last_recv":row.last_recv,"last_ack":row.last_ack,"idle":row.idle})
}

pub(crate) fn message(row: &MessageRecord) -> Value {
    json!({"message_id":row.message_id,"from_member_id":row.from_member_id,
        "to_member_id":row.to_member_id,"created_at":row.created_at,"status_state":row.status.as_str(),
        "status_timestamp":row.status_timestamp,"text":row.text})
}

pub(crate) fn listen_ready(member_id: i64, fleet_id: i64) -> Value {
    json!({"event":"ready", "member_id":member_id, "fleet_id":fleet_id})
}

pub(crate) fn listen_message(row: &MessageRecord) -> Value {
    json!({"event":"message", "message":message(row)})
}

pub(crate) fn listen_closed(member_id: i64, reason: &str) -> Value {
    json!({"event":"closed", "member_id":member_id, "reason":reason})
}

pub(crate) fn message_envelope(row: &MessageRecord) -> Value {
    json!({"message":message(row)})
}

pub(crate) fn send_outcome(
    row: &MessageRecord,
    notification_sent: bool,
    input_id: Option<i64>,
    delivery: &crate::broker::runtime::DeliveryProjection,
) -> Value {
    json!({"message":message(row),"notification_sent":notification_sent,
        "input_id":input_id,"delivery":delivery.to_json()})
}

pub(crate) fn member_prompt(
    member_id: i64,
    pane_id: &str,
    text: &str,
    input_id: Option<i64>,
    delivery: &crate::broker::runtime::DeliveryProjection,
) -> Value {
    json!({"member_id":member_id,"pane_id":pane_id,"text":text,"input_id":input_id,
        "delivery":delivery.to_json()})
}

pub(crate) fn member_exec(
    member_id: i64,
    pane_id: &str,
    coding_agent: &str,
    command: &str,
    input_id: Option<i64>,
    delivery: &crate::broker::runtime::DeliveryProjection,
) -> Value {
    json!({"member_id":member_id,"pane_id":pane_id,"coding_agent":coding_agent,"command":command,
        "dispatched":delivery.status == crate::broker::runtime::DeliveryStatus::Submitted,
        "input_id":input_id,"delivery":delivery.to_json()})
}

pub(crate) fn member_capture(capture: &crate::capture::MemberCapture) -> Value {
    let snapshot = &capture.snapshot;
    json!({"member_id":capture.member_id,"pane_id":capture.pane_id,"lines":capture.lines,
        "content":snapshot.content,"captured_at":snapshot.captured_at,"content_sha256":snapshot.content_sha256})
}

pub(crate) fn fleet(row: &FleetRecord) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "deleted_at":row.deleted_at, "director_member_id":row.director_member_id})
}

pub(crate) fn fleet_summary(row: &FleetSummary) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "director_member_id":row.director_member_id, "member_count":row.member_count})
}

pub(crate) fn created_fleet(row: &CreatedFleet) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "director":member(&row.director)})
}

pub(crate) fn deleted_fleet(row: &DeletedFleet) -> Value {
    let members = row
        .members
        .iter()
        .map(|member| json!({"member_id":member.member_id, "pane_status":member.pane_status}))
        .collect::<Vec<_>>();
    json!({"fleet_id":row.fleet_id, "deregistered_count":row.deregistered_count, "members":members})
}

fn permission_response(row: &crate::broker::runtime::PermissionResponseRecord) -> Value {
    let metadata = &row.metadata;
    json!({"input_id":row.input_id,"event_id":metadata.event_id,"interaction_id":metadata.interaction_id,
        "expected_owner_epoch":metadata.expected_owner_epoch,"request_token":metadata.request_token,
        "decision":metadata.decision.as_str(),"director_member_id":metadata.director_member_id,
        "director_mode":metadata.director_mode.as_str(),"decision_source":metadata.decision_source.as_str(),
        "mode_evidence_ref":metadata.mode_evidence_ref,"decision_evidence_ref":metadata.decision_evidence_ref,
        "answer_profile_version":metadata.answer_profile_version,"answer_rule_id":metadata.answer_rule_id,"decided_at":row.decided_at})
}

pub(crate) fn permission_response_result(
    row: &crate::runtime::runner::PermissionResponseResult,
) -> Value {
    json!({"response":{"status":row.submission.status.as_str(),"input_id":row.submission.input_id,"member_id":row.member_id,
        "run_id":row.run_id,"owner_epoch":row.owner_epoch,"interaction_id":row.interaction_id,"event_id":row.event_id,
        "decision":row.decision.as_str(),"decision_source":row.decision_source.as_str(),"stage":row.submission.stage.as_str(),
        "output_seq":row.submission.output_seq,"reason":row.submission.reason}})
}

pub(crate) fn permission_inspection(row: &crate::runtime::runner::PermissionInspection) -> Value {
    json!({"request":{"event_id":row.event_id,"message_id":row.message_id,"member_id":row.member_id,"fleet_id":row.fleet_id,
        "director_member_id":row.director_member_id,"run_id":row.run_id,"owner_epoch":row.owner_epoch,
        "interaction_id":row.interaction_id,"episode_seq":row.episode_seq,"classification":row.classification,
        "visibility":row.visibility,"backend":row.backend,"native_version":row.native_version,"profile_version":row.profile_version,
        "answer_profile_version":row.answer_profile_version,"answer_rule_id":row.answer_rule_id,"answer_capability":row.answer_capability,
        "capability_reason":row.capability_reason,"supported_decisions":row.supported_decisions,"request_token":row.request_token,
        "output_seq":row.output_seq,"evidence":row.evidence,"missing_evidence":row.missing_evidence}})
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::broker::runtime::RunStatus;
    use crate::runtime::runner::{RecoveryOutcome, RecoveryReport};
    #[test]
    fn runner_recovery_presentation_pins_ordered_json_outcomes_and_exact_cleanup_text() {
        for (outcome, wire, status) in [
            (RecoveryOutcome::Recovered, "recovered", RunStatus::Lost),
            (
                RecoveryOutcome::Recovering,
                "recovering",
                RunStatus::Recovering,
            ),
            (
                RecoveryOutcome::LiveOrUnreachable,
                "live_or_unreachable",
                RunStatus::Running,
            ),
            (
                RecoveryOutcome::OperatorReconciliationRequired,
                "operator_reconciliation_required",
                RunStatus::Starting,
            ),
        ] {
            let diagnostics = vec![
                json!("bounded diagnostic"),
                json!({"source":"operator_via_director","evidence_ref":"audit:termination"}),
            ];
            let report = RecoveryReport {
                member_id: 7,
                run_id: 11,
                outcome,
                replayed_count: 2,
                inbox_count: 4,
                archived_count: 0,
                status,
                cleanup_diagnostics: diagnostics.clone(),
            };
            let value = crate::presentation::runtime_recovery(&report);
            assert_eq!(
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                vec![
                    "member_id",
                    "run_id",
                    "outcome",
                    "replayed_count",
                    "inbox_count",
                    "archived_count",
                    "status",
                    "cleanup_diagnostics"
                ]
            );
            assert_eq!(
                value,
                json!({"member_id":7,"run_id":11,"outcome":wire,"replayed_count":2,"inbox_count":4,"archived_count":0,"status":status.as_str(),"cleanup_diagnostics":diagnostics})
            );
            assert_eq!(
                crate::output::format_runtime_recovery(&report),
                format!(
                    "Recovery: member=7 run=11 outcome={wire} status={} replayed=2 inbox=4 archived=0\nCleanup: bounded diagnostic\nCleanup: {}",
                    status.as_str(),
                    diagnostics[1]
                )
            );
        }
    }

    mod delivery_outcomes {
        use crate::broker::records::MessageRecord;
        use crate::broker::runtime::test_support::assert_key_order;
        use crate::broker::runtime::{DeliveryMethod, DeliveryProjection, DeliveryStatus};
        use serde_json::{Value, json};

        fn projection(
            status: DeliveryStatus,
            method: Option<DeliveryMethod>,
            reason: Option<&str>,
        ) -> DeliveryProjection {
            DeliveryProjection {
                status,
                method,
                reason: reason.map(str::to_owned),
            }
        }

        fn cases() -> [(bool, Option<i64>, DeliveryProjection, Value); 6] {
            [
                (
                    true,
                    Some(5),
                    projection(
                        DeliveryStatus::Submitted,
                        Some(DeliveryMethod::Profile),
                        None,
                    ),
                    json!({"status":"submitted","method":"profile","reason":null}),
                ),
                (
                    false,
                    Some(6),
                    projection(DeliveryStatus::Queued, None, Some("interaction_open")),
                    json!({"status":"queued","method":null,"reason":"interaction_open"}),
                ),
                (
                    false,
                    None,
                    projection(DeliveryStatus::Held, None, Some("observer_unavailable")),
                    json!({"status":"held","method":null,"reason":"observer_unavailable"}),
                ),
                (
                    false,
                    Some(7),
                    projection(
                        DeliveryStatus::Uncertain,
                        Some(DeliveryMethod::Fallback),
                        Some("submission_plan_unavailable"),
                    ),
                    json!({"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}),
                ),
                (
                    false,
                    None,
                    projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
                    json!({"status":"not_applicable","method":null,"reason":"director_stdout"}),
                ),
                (
                    false,
                    None,
                    projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
                    json!({"status":"not_applicable","method":null,"reason":"self_send"}),
                ),
            ]
        }

        #[test]
        fn send_outcome_appends_input_id_and_ordered_delivery_after_its_existing_keys() {
            let message = MessageRecord::fixture(9, 2, 3, "hello");
            for (sent, input_id, delivery, expected) in cases() {
                let output = crate::presentation::send_outcome(&message, sent, input_id, &delivery);
                assert_key_order(
                    &output,
                    &["message", "notification_sent", "input_id", "delivery"],
                );
                assert_key_order(&output["delivery"], &["status", "method", "reason"]);
                assert_eq!(output["notification_sent"], sent);
                assert_eq!(output["input_id"], json!(input_id));
                assert_eq!(output["delivery"], expected);
                assert_eq!(output["message"], crate::presentation::message(&message));
            }
        }

        #[test]
        fn prompt_and_exec_outcomes_append_delivery_and_dispatch_only_on_submission() {
            for (_, input_id, delivery, expected) in cases().into_iter().take(4) {
                let prompt =
                    crate::presentation::member_prompt(7, "%2", "do it", input_id, &delivery);
                assert_key_order(
                    &prompt,
                    &["member_id", "pane_id", "text", "input_id", "delivery"],
                );
                assert_eq!(
                    prompt,
                    json!({"member_id":7,"pane_id":"%2","text":"do it","input_id":input_id,"delivery":expected})
                );
                let exec = crate::presentation::member_exec(
                    7, "%2", "codex", "ls -l", input_id, &delivery,
                );
                assert_key_order(
                    &exec,
                    &[
                        "member_id",
                        "pane_id",
                        "coding_agent",
                        "command",
                        "dispatched",
                        "input_id",
                        "delivery",
                    ],
                );
                assert_eq!(
                    exec["dispatched"],
                    delivery.status == DeliveryStatus::Submitted,
                    "{expected}"
                );
                assert_eq!(exec["delivery"], expected);
            }
        }
    }

    mod runtime {
        use super::*;
        use crate::broker::runtime::test_support::*;
        use crate::broker::runtime::{self as storage, *};
        use rusqlite::params;
        use serde_json::Value;

        #[test]
        fn runtime_permission_presentation_flattens_the_audit_in_shipping_order() {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            let output = crate::presentation::input_operation(&receipt.input);
            assert_key_order(
                &output["input"],
                &[
                    "input_id",
                    "run_id",
                    "operation_key",
                    "kind",
                    "payload_digest",
                    "state",
                    "created_at",
                    "updated_at",
                    "message_id",
                    "disposition",
                    "evidence_ref",
                    "permission_response",
                    "delivery",
                    "deferred_at",
                    "defer_reason",
                    "payload_digest_version",
                ],
            );
            let audit = &output["input"]["permission_response"];
            assert_key_order(
                audit,
                &[
                    "input_id",
                    "event_id",
                    "interaction_id",
                    "expected_owner_epoch",
                    "request_token",
                    "decision",
                    "director_member_id",
                    "director_mode",
                    "decision_source",
                    "mode_evidence_ref",
                    "decision_evidence_ref",
                    "answer_profile_version",
                    "answer_rule_id",
                    "decided_at",
                ],
            );
            assert_eq!(output["input"]["delivery"], Value::Null);
            assert_eq!(output["input"]["deferred_at"], Value::Null);
            assert_eq!(output["input"]["defer_reason"], Value::Null);
            assert_eq!(output["input"]["payload_digest_version"], 1);
            assert_eq!(audit["decision"], "allow");
            assert_eq!(audit["director_mode"], "auto");
            assert_eq!(audit["decided_at"], receipt.response.decided_at);
            assert_eq!(
                crate::output::format_input_operation(&receipt.input),
                format!(
                    "Input: {} run={} kind=permission_response state=in_progress disposition=- message=-\nPermission decision: interaction={} event={} decision=allow source=director-auto mode=auto director={} epoch={}",
                    receipt.input.input_id,
                    fixture.owner.run_id,
                    fixture.request.metadata.interaction_id,
                    fixture.event.event_id,
                    fixture.base.director,
                    fixture.owner.owner_epoch
                )
            );
            assert_no_secret(&fixture.base.conn, SECRET);
            assert_no_secret(&fixture.base.conn, &format!("fingerprint-{SECRET}"));
        }
        #[test]
        fn runtime_presentation_exposes_recorded_owner_snapshot_and_unresolved_inputs() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            let input = fixture.input(&owner, "exec:uncertain", InputKind::Exec, None);
            let queued = fixture.input(&owner, "prompt:queued", InputKind::Prompt, None);
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET state='ready',deferred_at='2026-10-30T10:00:00Z',defer_reason='interaction_open' WHERE input_id=?1",
                    [queued.input_id],
                )
                .unwrap();
            let zero_bytes = fixture.input(&owner, "prompt:zero-bytes", InputKind::Prompt, None);
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET state='ready' WHERE input_id=?1",
                    [zero_bytes.input_id],
                )
                .unwrap();
            let member = crate::broker::get_member(&fixture.conn, fixture.member, fixture.fleet)
                .unwrap()
                .unwrap();
            let output = crate::presentation::member(&member);
            assert_key_order(
                &output,
                &[
                    "member_id",
                    "name",
                    "description",
                    "status",
                    "registered_at",
                    "kind",
                    "placement",
                    "runtime",
                ],
            );
            let runtime = &output["runtime"];
            assert_key_order(
                runtime,
                &[
                    "run_id",
                    "resolved_mode",
                    "status",
                    "reachability",
                    "owner_epoch",
                    "last_health_at",
                    "coverage",
                    "current_interaction_id",
                    "unresolved_input_ids",
                    "owner_kind",
                    "owner_pid",
                    "owner_start_identity",
                    "runner_pid",
                    "runner_start_identity",
                    "child_pid",
                    "child_start_identity",
                    "effective_policy",
                ],
            );
            assert_eq!(runtime["run_id"], owner.run_id);
            assert_eq!(runtime["effective_policy"], Value::Null);
            assert_eq!(
                runtime["current_interaction_id"],
                event.interaction_id.unwrap()
            );
            assert_eq!(
                runtime["unresolved_input_ids"],
                json!([input.input_id, queued.input_id]),
                "a queued ready input is unresolved; a plain zero-byte ready input is not"
            );
            assert_eq!(runtime["owner_start_identity"], "fixture-start-identity");
            assert_eq!(runtime["child_pid"], Value::Null);
            assert_eq!(runtime["child_start_identity"], Value::Null);
            let unwrapped =
                crate::broker::get_member(&fixture.conn, fixture.director, fixture.fleet)
                    .unwrap()
                    .unwrap();
            assert_eq!(
                crate::presentation::member(&unwrapped).get("runtime"),
                Some(&Value::Null)
            );
            let text = crate::output::format_member_detail(&member);
            assert_eq!(
                text,
                format!(
                    "{} worker active\nRuntime: run={} mode=never status=running coverage=unknown review=-",
                    fixture.member, owner.run_id
                )
            );
        }

        #[test]
        fn runtime_presentation_renders_a_profiled_run_without_match_metadata() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET native_version='0.164.0',profile_version='codex-0.163.0-on-request-1',coverage='tested' WHERE run_id=?1",
                    params![owner.run_id],
                )
                .unwrap();
            let member = crate::broker::get_member(&fixture.conn, fixture.member, fixture.fleet)
                .unwrap()
                .unwrap();
            let runtime = &crate::presentation::member(&member)["runtime"];
            assert_eq!(runtime["coverage"], "tested");
            assert!(runtime.get("profile_match").is_none());
            let line = format!(
                "Runtime: run={} mode=never status=running coverage=tested review=-",
                owner.run_id
            );
            assert_eq!(
                crate::output::format_member_detail(&member),
                format!("{} worker active\n{line}", fixture.member)
            );
            let view = storage::get_event(&fixture.conn, event.event_id).unwrap();
            assert!(
                crate::presentation::runtime_event(&view)["run"]
                    .get("profile_match")
                    .is_none()
            );
            assert_eq!(
                crate::output::format_runtime_event(&view)
                    .lines()
                    .nth(1)
                    .unwrap(),
                line
            );
        }
        #[test]
        fn effective_policy_record_is_rendered_last_in_every_runtime_snapshot_surface() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            for (record, review) in [
                (
                    effective_policy("unavailable", None, Some(event.event_id)),
                    "unavailable",
                ),
                (
                    effective_policy("available", Some("auto"), None),
                    "available",
                ),
            ] {
                store_effective_policy(&fixture, &record);
                let run = storage::get_run(&fixture.conn, owner.run_id).unwrap();
                assert_eq!(run.effective_policy_json.as_ref(), Some(&record));
                let member =
                    crate::broker::get_member(&fixture.conn, fixture.member, fixture.fleet)
                        .unwrap()
                        .unwrap();
                let runtime = &crate::presentation::member(&member)["runtime"];
                assert_eq!(
                    runtime.as_object().unwrap().keys().next_back().unwrap(),
                    "effective_policy"
                );
                assert_eq!(runtime["effective_policy"], record);
                let line = format!(
                    "Runtime: run={} mode=never status=running coverage=unknown review={review}",
                    owner.run_id
                );
                assert_eq!(
                    crate::output::format_member_detail(&member),
                    format!("{} worker active\n{line}", fixture.member)
                );
                let view = storage::get_event(&fixture.conn, event.event_id).unwrap();
                let shown = crate::presentation::runtime_event(&view);
                assert_eq!(shown["run"]["effective_policy"], record);
                assert_eq!(
                    shown["run"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .next_back()
                        .unwrap(),
                    "effective_policy"
                );
                assert_eq!(
                    crate::output::format_runtime_event(&view)
                        .lines()
                        .nth(1)
                        .unwrap(),
                    line
                );
                assert_eq!(
                    view.event.payload_json.get("effective_policy"),
                    None,
                    "the event payload is unchanged"
                );
            }
        }
        #[test]
        fn event_and_input_read_wrappers_follow_the_approved_order_and_compact_text() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            let view = storage::get_event(&fixture.conn, event.event_id).unwrap();
            let output = crate::presentation::runtime_event(&view);
            assert_key_order(
                &output,
                &[
                    "event",
                    "notification_state",
                    "message_id",
                    "archive_reason",
                    "run",
                    "interaction",
                ],
            );
            assert_key_order(
                &output["interaction"],
                &[
                    "interaction_id",
                    "run_id",
                    "episode_seq",
                    "classification",
                    "evidence_digest",
                    "opened_at",
                    "evidence_json",
                    "closed_at",
                    "close_reason",
                ],
            );
            assert_eq!(
                crate::output::format_runtime_event(&view),
                format!(
                    "Event: {} run={} seq=1 kind=interaction_opened notification=inbox message={}\nRuntime: run={} mode=never status=running coverage=unknown review=-\nInteraction: {} classification=permission_request closed_at=- close_reason=-",
                    event.event_id,
                    owner.run_id,
                    event.message_id.unwrap(),
                    owner.run_id,
                    event.interaction_id.unwrap()
                )
            );
            let input = fixture.input(&owner, "exec:lost-output", InputKind::Exec, None);
            let output = crate::presentation::input_operation(&input);
            assert_key_order(
                &output["input"],
                &[
                    "input_id",
                    "run_id",
                    "operation_key",
                    "kind",
                    "payload_digest",
                    "state",
                    "created_at",
                    "updated_at",
                    "message_id",
                    "disposition",
                    "evidence_ref",
                    "permission_response",
                    "delivery",
                    "deferred_at",
                    "defer_reason",
                    "payload_digest_version",
                ],
            );
            assert_eq!(output["input"]["permission_response"], Value::Null);
            assert_eq!(
                crate::output::format_input_operation(&input),
                format!(
                    "Input: {} run={} kind=exec state=in_progress disposition=- message=-",
                    input.input_id, owner.run_id
                )
            );
        }

        #[test]
        fn input_presentation_projects_delivery_from_the_stored_receipt_state() {
            const NOW: &str = "2026-10-30T10:00:00.000000+00:00";
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            for (index, (assignment, delivery, deferred)) in [
                (
                    format!("state='ready',deferred_at='{NOW}',defer_reason='interaction_open'"),
                    json!({"status":"queued","method":null,"reason":"interaction_open"}),
                    true,
                ),
                (
                    "state='ready'".to_owned(),
                    json!({"status":"held","method":null,"reason":"zero_bytes_ready"}),
                    false,
                ),
                (
                    "delivery_method='fallback',delivery_reason='submission_plan_unavailable'".to_owned(),
                    json!({"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}),
                    false,
                ),
                (
                    "state='accepted',delivery_method='profile'".to_owned(),
                    json!({"status":"submitted","method":"profile","reason":null}),
                    false,
                ),
                (
                    "state='settled',disposition='submitted',delivery_method='fallback',delivery_reason='submission_plan_unavailable'".to_owned(),
                    json!({"status":"submitted","method":"fallback","reason":"submission_plan_unavailable"}),
                    false,
                ),
                (
                    "state='settled',disposition='not_submitted',delivery_reason='runner_stopped'".to_owned(),
                    json!({"status":"not_submitted","method":null,"reason":"runner_stopped"}),
                    false,
                ),
                (
                    "state='settled',disposition='abandoned_uncertain',delivery_method='profile'".to_owned(),
                    json!({"status":"uncertain","method":"profile","reason":null}),
                    false,
                ),
                (
                    "state='accepted'".to_owned(),
                    json!({"status":"submitted","method":null,"reason":"method_unrecorded"}),
                    false,
                ),
                (
                    "state='uncertain'".to_owned(),
                    json!({"status":"uncertain","method":null,"reason":"method_unrecorded"}),
                    false,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let input = fixture.input(&owner, &format!("prompt:{index}"), InputKind::Prompt, None);
                fixture
                    .conn
                    .execute(
                        &format!("UPDATE member_input_operations SET {assignment} WHERE input_id=?1"),
                        [input.input_id],
                    )
                    .unwrap_or_else(|error| panic!("{assignment}: {error}"));
                let stored = storage::get_input(&fixture.conn, input.input_id)
                    .unwrap_or_else(|error| panic!("{assignment}: {}", error.message()));
                let output = &crate::presentation::input_operation(&stored)["input"];
                assert_eq!(output["delivery"], delivery, "{assignment}");
                assert_key_order(&output["delivery"], &["status", "method", "reason"]);
                assert_eq!(output["deferred_at"].is_null(), !deferred, "{assignment}");
                assert_eq!(output["defer_reason"].is_null(), !deferred, "{assignment}");
                assert_eq!(output["payload_digest_version"], 1);
            }
            for assignment in [
                format!(
                    "state='ready',deferred_at='{NOW}',defer_reason='interaction_open',delivery_method='profile'"
                ),
                "state='settled',disposition='not_submitted',delivery_method='profile'".to_owned(),
            ] {
                let input = fixture.input(
                    &owner,
                    &format!("invalid:{assignment}"),
                    InputKind::Exec,
                    None,
                );
                fixture
                    .conn
                    .execute(
                        &format!(
                            "UPDATE member_input_operations SET {assignment} WHERE input_id=?1"
                        ),
                        [input.input_id],
                    )
                    .unwrap();
                assert!(
                    storage::get_input(&fixture.conn, input.input_id).is_err(),
                    "unknown stored combination {assignment} fails validation"
                );
            }
        }
    }

    mod runner_registry {
        use crate::broker::runtime::{self as storage, RunStatus};
        use crate::runtime::runner::health::test_support::{SilentServer, activate};
        use crate::runtime::runner::test_support::Fixture;
        use rusqlite::Connection;
        use std::time::{Duration, Instant};

        fn counts(conn: &Connection) -> (i64, i64, i64) {
            let scalar = |table| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            };
            (
                scalar("member_interactions"),
                scalar("member_runtime_events"),
                scalar("messages"),
            )
        }
        #[test]
        fn runner_registry_show_returns_historical_health_without_contacting_a_live_endpoint() {
            let mut fixture = Fixture::new();
            let (artifacts, _held) = activate(&mut fixture);
            let _server = SilentServer::start(&artifacts.paths.control_socket);
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
            let before = counts(&fixture.conn);
            let start = Instant::now();
            let member = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap();
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(
                crate::presentation::member(&member)["runtime"]["last_health_at"],
                "2000-01-01T00:00:00Z"
            );
            assert_eq!(counts(&fixture.conn), before);
            assert_eq!(
                storage::get_run(&fixture.conn, fixture.run.run_id)
                    .unwrap()
                    .status,
                RunStatus::Running
            );
        }
    }
}
