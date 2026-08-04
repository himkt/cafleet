//! The `fleet` group (SPEC §6.3 *fleet group*).

use clap::Subcommand;
use serde_json::Value;

use super::helpers::{connect, emit, resolve_mux};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::output::format_fleet_create;

#[derive(Subcommand)]
pub enum FleetCommand {
    /// Create a fleet with its root Director.
    Create {
        /// Human-readable name for the fleet.
        #[arg(long)]
        name: String,
        /// The backend the Director is actually running on.
        #[arg(long = "coding-agent", value_parser = ["claude", "codex", "opencode"])]
        coding_agent: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// List non-deleted fleets.
    List {
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one fleet (soft-deleted included).
    Show {
        /// The fleet to show.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Soft-delete a fleet and deregister its members.
    Delete {
        /// The fleet to delete.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub fn run(settings: &Settings, command: FleetCommand) -> Result<(), CafleetError> {
    match command {
        FleetCommand::Create {
            name,
            coding_agent,
            json,
        } => create(settings, &name, &coding_agent, json),
        FleetCommand::List { json } => list(settings, json),
        FleetCommand::Show { fleet_id, json } => show(settings, fleet_id, json),
        FleetCommand::Delete { fleet_id, json } => delete(settings, fleet_id, json),
    }
}

fn create(
    settings: &Settings,
    name: &str,
    coding_agent: &str,
    json: bool,
) -> Result<(), CafleetError> {
    let inside_session = || {
        CafleetError::App(
            "cafleet fleet create must be run inside a tmux or herdr session".to_string(),
        )
    };
    let mux = resolve_mux(settings).map_err(|_| inside_session())?;
    mux.ensure_available().map_err(|_| inside_session())?;
    let context = mux.context_discovery().map_err(|_| inside_session())?;
    let mut conn = connect(settings)?;
    let fleet = broker::create_fleet(
        &mut conn,
        Some(name),
        &context.session,
        &context.window_id,
        &context.pane_id,
        coding_agent,
        mux.name(),
    )?;
    emit(json, &fleet, || format_fleet_create(&fleet, false));
    Ok(())
}

fn list(settings: &Settings, json: bool) -> Result<(), CafleetError> {
    let conn = connect(settings)?;
    let fleets = broker::list_fleets(&conn)?;
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

fn show(settings: &Settings, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let conn = connect(settings)?;
    let fleet = broker::get_fleet(&conn, fleet_id)?
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

fn delete(settings: &Settings, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let mut conn = connect(settings)?;
    let result = broker::delete_fleet(&mut conn, fleet_id)?;
    emit(json, &result, || {
        format!(
            "Deleted fleet {fleet_id}. Deregistered {} members.",
            result["deregistered_count"]
        )
    });
    Ok(())
}
