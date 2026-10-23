use clap::Subcommand;
use rusqlite::Connection;
use serde_json::Value;

use super::helpers::{emit, resolve_mux};
use crate::broker;
use crate::capture::{CaptureSnapshot, ScanEntry, write_scan};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::output::format_fleet_create;
use crate::time::now_utc;

#[derive(Subcommand)]
pub enum FleetCommand {
    /// Register the calling Director in a new fleet.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "coding-agent", value_parser = ["claude", "codex", "opencode"])]
        coding_agent: String,
        #[arg(long)]
        json: bool,
    },
    /// List non-deleted fleets.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one fleet.
    Show {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Deregister a fleet and its members.
    Delete {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Capture the active fleet's panes once.
    Capture {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(i64).range(1..))]
        lines: i64,
        #[arg(long)]
        ansi: bool,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(
    slot: &mut Option<Connection>,
    settings: &Settings,
    command: FleetCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let conn = slot.as_mut().expect("open invocation connection");
    match command {
        FleetCommand::Create {
            name,
            coding_agent,
            json,
        } => create(conn, settings, &name, &coding_agent, json),
        FleetCommand::List { json } => list(conn, json),
        FleetCommand::Show { fleet_id, json } => show(conn, fleet_id, json),
        FleetCommand::Delete { fleet_id, json } => delete(conn, settings, fleet_id, json, deadline),
        FleetCommand::Capture {
            fleet_id,
            lines,
            ansi,
            json,
        } => capture(conn, settings, fleet_id, lines, ansi, json),
    }
}

fn create(
    conn: &mut Connection,
    settings: &Settings,
    name: &str,
    agent: &str,
    json: bool,
) -> Result<(), CafleetError> {
    let inside_session = |_| {
        CafleetError::App("cafleet fleet create must be run inside a tmux or herdr session".into())
    };
    let mux = resolve_mux(settings).map_err(inside_session)?;
    mux.ensure_available().map_err(inside_session)?;
    let context = mux.context_discovery().map_err(inside_session)?;
    let fleet = broker::create_fleet(
        conn,
        Some(name),
        &context.session,
        &context.window_id,
        &context.pane_id,
        agent,
        mux.name(),
    )?;
    emit(json, &fleet, || format_fleet_create(&fleet));
    Ok(())
}

fn list(conn: &mut Connection, json: bool) -> Result<(), CafleetError> {
    let fleets = broker::list_fleets(conn)?;
    emit(json, &Value::Array(fleets.clone()), || {
        if fleets.is_empty() {
            return "No fleets found.".to_string();
        }
        let cell = |value: &Value| match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut lines = vec![format!(
            "{:<40}{:<40}{:<20}{:<8}{}",
            "FLEET_ID", "DIRECTOR", "NAME", "MEMBERS", "CREATED_AT"
        )];
        for fleet in &fleets {
            lines.push(format!(
                "{:<40}{:<40}{:<20}{:<8}{}",
                cell(&fleet["fleet_id"]),
                cell(&fleet["director_member_id"]),
                cell(&fleet["name"]),
                cell(&fleet["member_count"]),
                cell(&fleet["created_at"]),
            ));
        }
        lines.join("\n")
    });
    Ok(())
}

fn show(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let fleet = broker::get_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    emit(json, &fleet, || {
        let scalar = |value: &Value| match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut lines = vec![
            format!("fleet_id:   {}", scalar(&fleet["fleet_id"])),
            format!("name:       {}", scalar(&fleet["name"])),
            format!("created_at: {}", scalar(&fleet["created_at"])),
        ];
        if !fleet["deleted_at"].is_null() {
            lines.push(format!("deleted_at: {}", scalar(&fleet["deleted_at"])));
        }
        lines.join("\n")
    });
    Ok(())
}

fn delete(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let result = crate::input::delete_fleet(conn, settings, fleet_id, deadline)?;
    emit(json, &result, || {
        format!(
            "Deleted fleet {fleet_id}. Deregistered {} members.",
            result["deregistered_count"]
        )
    });
    Ok(())
}

fn capture(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    lines: i64,
    ansi: bool,
    json_output: bool,
) -> Result<(), CafleetError> {
    let fleet = broker::fleets::fetch_fleet(conn, fleet_id)?
        .filter(|fleet| fleet.deleted_at.is_none())
        .ok_or_else(|| CafleetError::App(format!("fleet {fleet_id} not found")))?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let director_member_id = fleet.director_member_id;
    let members = broker::members::capture_roster(conn, fleet_id)?;
    let mut roster: Vec<_> = members
        .iter()
        .filter(|member| Some(member.member_id) == director_member_id)
        .collect();
    let mut rest: Vec<_> = members
        .iter()
        .filter(|member| Some(member.member_id) != director_member_id && member.placement.is_some())
        .collect();
    rest.sort_by_key(|member| Some(member.member_id));
    roster.extend(rest);

    let mut entries = Vec::new();
    for member in roster {
        let placement =
            member
                .placement
                .as_ref()
                .ok_or_else(|| CafleetError::InvalidStoredValue {
                    field: "member placement".into(),
                    value: member.member_id.to_string(),
                })?;
        let pane_id = placement.mux_pane_id.clone();
        let outcome = match pane_id.as_deref() {
            None => Err("pane not available (pending placement)".to_string()),
            Some(pane) => mux
                .capture_pane(pane, lines)
                .map_err(|error| format!("capture failed: {error}"))
                .map(|raw| CaptureSnapshot::from_raw(&raw, ansi, now_utc())),
        };
        entries.push(ScanEntry {
            member_id: member.member_id,
            name: member.name.clone(),
            kind: member.kind,
            coding_agent: placement.coding_agent.clone(),
            pane_id,
            lines,
            outcome,
        });
    }
    write_scan(&mut std::io::stdout(), &entries, json_output)
}

#[cfg(test)]
mod tests {
    use super::FleetCommand;
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: FleetCommand,
    }

    fn parse(args: &[&str]) -> Result<FleetCommand, clap::Error> {
        Harness::try_parse_from(args).map(|harness| harness.command)
    }

    #[test]
    fn create_parses_registration_inputs_for_the_current_director() {
        let parsed = parse(&[
            "cafleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
            "--json",
        ])
        .unwrap();
        assert!(matches!(parsed, FleetCommand::Create { .. }));
    }

    #[test]
    fn capture_accepts_the_fleet_subject_depth_and_ansi_presentation() {
        parse(&[
            "cafleet", "capture", "1", "--lines", "200", "--ansi", "--json",
        ])
        .unwrap();
    }
}
