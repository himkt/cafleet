//! CLI output construction from storage records.
use crate::broker::records::*;
use serde_json::{Value, json};

use crate::config_dir::DirSource;
use crate::diagnosis::{AssetReport, AssetState, SchemaState};
use crate::member_permissions::{Finding, FindingKind};

pub(crate) fn doctor_database_detail(schema: &SchemaState) -> String {
    match schema {
        SchemaState::Head { version } => format!("schema {version} (head)"),
        SchemaState::Behind { recorded, head } => {
            format!("schema {recorded}, head is {head} — run: cafleet setup")
        }
        SchemaState::Ahead { recorded, head } => {
            format!("schema {recorded} is newer than this CLI (head {head}) — upgrade cafleet")
        }
        SchemaState::Incompatible { path } => crate::diagnosis::incompatible_format(path)
            .message()
            .into_owned(),
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

pub(crate) fn doctor_member_permissions(findings: &[Finding]) -> Value {
    let findings = findings
        .iter()
        .map(|finding| {
            let (list, rule, command) = match &finding.kind {
                FindingKind::BlockingRule {
                    list,
                    rule,
                    command,
                } => (Some(*list), Some(rule.as_str()), Some(*command)),
                FindingKind::ManagedRulesOnly { command } => (
                    Some("allowManagedPermissionRulesOnly"),
                    None,
                    Some(*command),
                ),
                FindingKind::Unreadable => (None, None, None),
            };
            json!({"coding_agent":"claude","file":finding.file.display().to_string(),
                "list":list,"rule":rule,"command":command})
        })
        .collect::<Vec<_>>();
    json!({"ok":findings.is_empty(),"findings":findings})
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
        "kind":row.kind.as_str(),
        "placement":row.placement.as_ref().map(placement)})
}

pub fn member_activity(row: &MemberActivity) -> Value {
    json!({"member_id":row.member.member_id,"name":row.member.name,
        "kind":row.member.kind.as_str(),"placement":row.member.placement.as_ref().map(placement),
        "last_sent":row.last_sent,"last_recv":row.last_recv,"last_ack":row.last_ack,"idle":row.idle,
        "unconfirmed_previews":row.unconfirmed_previews})
}

pub fn member_detail(row: &MemberRecord, unconfirmed_previews: &[i64]) -> Value {
    let mut value = member(row);
    value
        .as_object_mut()
        .expect("a member presents as an object")
        .insert("unconfirmed_previews".into(), json!(unconfirmed_previews));
    value
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

pub(crate) fn scan_json(entries: &[crate::capture::ScanEntry]) -> Value {
    Value::Array(entries.iter().map(|entry| {
        let snapshot = entry.outcome.as_ref().ok();
        json!({"member_id":entry.member_id,"name":entry.name,"kind":entry.kind.as_str(),
            "coding_agent":entry.coding_agent,"pane_id":entry.pane_id,"lines":entry.lines,
            "content":snapshot.map(|s| &s.content),"captured_at":snapshot.map(|s| &s.captured_at),
            "content_sha256":snapshot.map(|s| &s.content_sha256),"error":entry.outcome.as_ref().err()})
    }).collect())
}

pub(crate) fn scan_text(entries: &[crate::capture::ScanEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            let pane = entry.pane_id.as_deref().unwrap_or("—");
            let (stamp, body) = match &entry.outcome {
                Ok(snapshot) => (
                    format!("; captured_at={}", snapshot.captured_at),
                    snapshot.content.as_str(),
                ),
                Err(error) => (String::new(), error.as_str()),
            };
            format!(
                "=== {} ({}; kind={}; coding_agent={}; pane={pane}{stamp}) ===\n{body}",
                entry.member_id,
                entry.name,
                entry.kind.as_str(),
                entry.coding_agent
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
