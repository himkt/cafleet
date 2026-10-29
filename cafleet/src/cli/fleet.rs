use clap::Subcommand;
use rusqlite::Connection;
use serde_json::Value;

use super::helpers::{emit, resolve_mux};
use crate::broker;
use crate::coding_agent::CodingAgent;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{format_fleet_create, format_fleet_delete};
use crate::presentation;

#[derive(Subcommand)]
pub enum FleetCommand {
    /// Register the calling Director in a new fleet.
    Create {
        /// Fleet name.
        #[arg(long)]
        name: String,
        /// Director's coding agent.
        #[arg(long = "coding-agent", value_enum)]
        coding_agent: CodingAgent,
        /// Output in JSON format.
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
    /// End a fleet's ordinary members, then deregister the fleet.
    Delete {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: FleetCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    match command {
        FleetCommand::Create {
            name,
            coding_agent,
            json,
        } => create(conn, settings, &name, coding_agent, json),
        FleetCommand::List { json } => {
            let fleets = broker::list_fleets(conn)?;
            let payload = Value::Array(fleets.iter().map(presentation::fleet_summary).collect());
            emit(json, &payload, || {
                if fleets.is_empty() {
                    return "No fleets found.".into();
                }
                let mut lines = vec!["FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT".into()];
                for fleet in &fleets {
                    let director = fleet.director_member_id.map(|id| id.to_string());
                    lines.push(format!(
                        "{} {} {} {} {}",
                        fleet.fleet_id,
                        director.as_deref().unwrap_or("-"),
                        fleet.name.as_deref().unwrap_or("-"),
                        fleet.member_count,
                        fleet.created_at
                    ));
                }
                lines.join("\n")
            });
            Ok(())
        }
        FleetCommand::Show { fleet_id, json } => {
            let fleet = broker::get_fleet(conn, fleet_id)?
                .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
            emit(json, &presentation::fleet(&fleet), || {
                let mut text = format!(
                    "fleet_id: {}\nname: {}\ncreated_at: {}",
                    fleet.fleet_id,
                    fleet.name.as_deref().unwrap_or("-"),
                    fleet.created_at
                );
                if let Some(deleted) = &fleet.deleted_at {
                    text.push_str(&format!("\ndeleted_at: {deleted}"));
                }
                text
            });
            Ok(())
        }
        FleetCommand::Delete { fleet_id, json } => {
            let result = crate::input::delete_fleet(conn, settings, fleet_id, deadline)?;
            emit(json, &presentation::deleted_fleet(&result), || {
                format_fleet_delete(&result)
            });
            Ok(())
        }
    }
}

fn create(
    conn: &mut Connection,
    settings: &Settings,
    name: &str,
    agent: CodingAgent,
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
        agent.name(),
        mux.name(),
    )?;
    emit(json, &presentation::created_fleet(&fleet), || {
        format_fleet_create(&fleet)
    });
    Ok(())
}
