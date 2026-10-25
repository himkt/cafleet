use crate::broker::records::*;
use serde_json::{Value, json};

pub fn placement(row: &Placement) -> Value {
    json!({"backend":row.backend,"mux_session":row.mux_session,
        "mux_window_id":row.mux_window_id,"mux_pane_id":row.mux_pane_id,
        "coding_agent":row.coding_agent,"created_at":row.created_at})
}

pub fn member(row: &MemberRecord) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"description":row.description,
        "status":row.status.as_str(),"registered_at":row.registered_at,
        "kind":row.kind.as_str(),
        "placement":row.placement.as_ref().map(placement)})
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
    json!({"fleet_id":row.fleet_id, "deregistered_count":row.deregistered_count})
}
