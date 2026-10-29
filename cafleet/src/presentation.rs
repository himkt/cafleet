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

pub fn placement(row: &Placement) -> Value {
    json!({"backend":row.backend,"mux_session":row.mux_session,
        "mux_window_id":row.mux_window_id,"mux_pane_id":row.mux_pane_id,
        "coding_agent":row.coding_agent,"created_at":row.created_at})
}

pub fn member(row: &MemberRecord) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"description":row.description,
        "status":row.status.as_str(),"registered_at":row.registered_at,
        "kind":row.kind.as_str(),
        "placement":row.placement.as_ref().map(placement),
        "runtime":row.runtime.as_ref().map(runtime_snapshot)})
}

pub fn runtime_snapshot(row: &crate::broker::runtime::RuntimeSnapshot) -> Value {
    let run = &row.run;
    json!({"run_id":run.run_id,"resolved_mode":run.resolved_mode,"status":run.status.as_str(),
        "reachability":run.reachability.as_str(),"owner_epoch":run.owner_epoch,"last_health_at":run.last_health_at,
        "coverage":run.coverage.as_str(),"profile_match":run.profile_match.map(|profile_match| profile_match.as_str()),
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

pub fn runtime_event(view: &crate::broker::runtime::RuntimeEventView) -> Value {
    let event = &view.event;
    json!({"event":event.payload_json,"notification_state":event.notification_state.as_str(),
        "message_id":event.message_id,"archive_reason":event.archive_reason.map(|reason| reason.as_str()),
        "run":runtime_snapshot(&view.run),"interaction":view.interaction.as_ref().map(runtime_interaction)})
}

pub fn input_operation(row: &crate::broker::runtime::InputOperation) -> Value {
    json!({"input":{"input_id":row.input_id,"run_id":row.run_id,"operation_key":row.operation_key,
        "kind":row.kind.as_str(),"payload_digest":row.payload_digest,"state":row.state.as_str(),
        "created_at":row.created_at,"updated_at":row.updated_at,"message_id":row.message_id,
        "disposition":row.disposition.map(|disposition| disposition.as_str()),"evidence_ref":row.evidence_ref,"permission_response":row.permission_response.as_ref().map(permission_response)}})
}

pub fn member_activity(row: &MemberActivity) -> Value {
    json!({"member_id":row.member.member_id,"name":row.member.name,
        "kind":row.member.kind.as_str(),"placement":row.member.placement.as_ref().map(placement),
        "last_sent":row.last_sent,"last_recv":row.last_recv,"last_ack":row.last_ack,"idle":row.idle})
}

pub fn message(row: &MessageRecord) -> Value {
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

pub fn message_envelope(row: &MessageRecord) -> Value {
    json!({"message":message(row)})
}

pub fn send_outcome(row: &MessageRecord, notification_sent: bool) -> Value {
    json!({"message":message(row),"notification_sent":notification_sent})
}

pub(crate) fn member_capture(capture: &crate::capture::MemberCapture) -> Value {
    let snapshot = &capture.snapshot;
    json!({"member_id":capture.member_id,"pane_id":capture.pane_id,"lines":capture.lines,
        "content":snapshot.content,"captured_at":snapshot.captured_at,"content_sha256":snapshot.content_sha256})
}

pub fn fleet(row: &FleetRecord) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "deleted_at":row.deleted_at, "director_member_id":row.director_member_id})
}

pub fn fleet_summary(row: &FleetSummary) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "director_member_id":row.director_member_id, "member_count":row.member_count})
}

pub fn created_fleet(row: &CreatedFleet) -> Value {
    json!({"fleet_id":row.fleet_id, "name":row.name, "created_at":row.created_at,
        "director":member(&row.director)})
}

pub fn deleted_fleet(row: &DeletedFleet) -> Value {
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
