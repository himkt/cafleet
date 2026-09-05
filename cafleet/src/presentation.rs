//! Explicit CLI/HTTP wire construction, separate from storage records.
use crate::broker::records::*;
use serde_json::{Value, json};

use crate::config_dir::DirSource;
use crate::diagnosis::{AssetReport, AssetState, SchemaState};

pub(crate) fn doctor_database_detail(schema: &SchemaState) -> String {
    match schema {
        SchemaState::Head { version } => format!("schema {version} (head)"),
        SchemaState::Behind { recorded, head } => {
            format!("schema {recorded}, head is {head} — run: cafleet setup")
        }
        SchemaState::Ahead { recorded, head } => {
            format!("schema {recorded} is newer than this CLI (head {head}) — upgrade cafleet")
        }
        SchemaState::Unversioned => {
            "database has tables but no schema history — not a cafleet database?".into()
        }
        SchemaState::Missing => "no database — run: cafleet setup".into(),
        SchemaState::Unreachable { cause } => cause.message().into_owned(),
    }
}

pub(crate) fn doctor_database(schema: &SchemaState, head: u32) -> Value {
    let recorded = match schema {
        SchemaState::Head { version } => Some(*version),
        SchemaState::Behind { recorded, .. } | SchemaState::Ahead { recorded, .. } => {
            Some(*recorded)
        }
        _ => None,
    };
    let ok = matches!(schema, SchemaState::Head { .. });
    json!({"ok":ok,"schema_version":recorded,"head_version":head,
        "error":if ok { None } else { Some(doctor_database_detail(schema)) }})
}

pub(crate) fn doctor_assets(assets: &AssetReport, cli_version: &str) -> Value {
    let agents = assets
        .agents
        .iter()
        .map(|agent| {
            let (identity, install, state, error, source) = match &agent.state {
                AssetState::Current { identity, install } => {
                    (Some(identity), Some(install), "ok", None, None)
                }
                AssetState::Stale { identity, install } => {
                    (Some(identity), Some(install), "stale", None, None)
                }
                AssetState::NotInstalled { identity } => {
                    (Some(identity), None, "not_installed", None, None)
                }
                AssetState::PathError {
                    variable, cause, ..
                } => (None, None, "error", Some(cause.message()), Some(*variable)),
            };
            let source = source.unwrap_or_else(|| {
                match &identity.expect("resolved state has identity").source {
                    DirSource::EnvVar(name) => name,
                    DirSource::Default => "default",
                }
            });
            json!({"coding_agent":agent.coding_agent,
            "path":identity.map(|d| d.path.display().to_string()),"source":source,
            "recorded_version":install.map(|r| &r.cafleet_version),
            "installed_at":install.map(|r| &r.installed_at),"state":state,"error":error})
        })
        .collect::<Vec<_>>();
    let ok = assets.agents.iter().all(|a| {
        matches!(
            a.state,
            AssetState::Current { .. } | AssetState::NotInstalled { .. }
        )
    });
    json!({"ok":ok,"cli_version":cli_version,"agents":agents,
        "superseded":assets.superseded.iter().map(|r| json!({"coding_agent":r.coding_agent,
            "path":r.path,"recorded_version":r.cafleet_version,"installed_at":r.installed_at})).collect::<Vec<_>>()})
}

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
