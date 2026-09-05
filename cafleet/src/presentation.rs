//! Explicit CLI/HTTP wire construction, separate from storage records.
use crate::broker::records::*;
use serde_json::{Value, json};

pub fn placement(row: &Placement) -> Value {
    json!({"backend":row.backend,"mux_session":row.mux_session,
        "mux_window_id":row.mux_window_id,"mux_pane_id":row.mux_pane_id,
        "coding_agent":row.coding_agent,"created_at":row.created_at})
}

pub fn registered_member(row: &RegisteredMember) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"registered_at":row.registered_at})
}

pub fn member(row: &MemberRecord) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"description":row.description,
        "status":row.status.as_str(),"registered_at":row.registered_at,
        "kind":row.kind.as_str(),"skills":row.skills,
        "placement":row.placement.as_ref().map(placement)})
}

pub fn roster_member(row: &MemberRecord) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"description":row.description,
        "status":row.status.as_str(),"registered_at":row.registered_at,
        "kind":row.kind.as_str(),"placement":row.placement.as_ref().map(placement)})
}

pub fn member_activity(row: &MemberActivity) -> Value {
    json!({"member_id":row.member.member_id,"name":row.member.name,
        "kind":row.member.kind.as_str(),"placement":row.member.placement.as_ref().map(placement),
        "last_sent":row.last_sent,"last_recv":row.last_recv,"last_ack":row.last_ack,"idle":row.idle})
}

pub fn message(row: &MessageRecord) -> Value {
    json!({"message_id":row.message_id,"owner_member_id":row.owner_member_id,
        "from_member_id":row.from_member_id,"to_member_id":row.to_member_id,
        "type":row.kind.as_str(),"created_at":row.created_at,"status_state":row.status.as_str(),
        "status_timestamp":row.status_timestamp,"origin_message_id":row.origin_message_id,"text":row.text})
}

pub fn message_envelope(row: &MessageRecord) -> Value {
    json!({"message":message(row)})
}

pub fn send_outcome(outcome: &SendOutcome) -> Value {
    json!({"message":message(&outcome.message),"notification_sent":outcome.notification == NotificationAttempt::Sent})
}

pub fn broadcast_outcome(outcome: &BroadcastOutcome) -> Value {
    json!({"message":message(&outcome.message),"recipients":outcome.recipients,"delivered":outcome.delivered})
}

pub fn monitor_runtime(row: &MonitorRuntime) -> Value {
    json!({"fleet_id":row.fleet_id,"pid":row.pid,"started_at":row.started_at,
        "last_tick_at":row.last_tick_at,"tick_seconds":row.tick_seconds,
        "wake_interval_seconds":row.wake_interval_seconds,"last_wake_at":row.last_wake_at,
        "wake_requested_at":row.wake_requested_at})
}

pub fn monitor_runtime_view(row: &MonitorRuntimeView) -> Value {
    json!({"running":row.running,"pid":row.pid,"tick_seconds":row.tick_seconds,
        "wake_interval_seconds":row.wake_interval_seconds,"last_tick_at":row.last_tick_at,
        "last_tick_age_seconds":row.last_tick_age_seconds,"started_at":row.started_at,
        "last_wake_at":row.last_wake_at,"last_wake_age_seconds":row.last_wake_age_seconds})
}

pub fn wake_target(row: &WakeTarget) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"coding_agent":row.coding_agent,"pending_count":row.pending_count})
}

pub fn monitor_member(row: &MonitorMember) -> Value {
    json!({"member_id":row.member_id,"name":row.name,"pending_count":row.pending_count,
        "oldest_pending_ts":row.oldest_pending_ts,"oldest_pending_age_seconds":row.oldest_pending_age_seconds})
}
