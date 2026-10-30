use clap::Subcommand;
use rusqlite::Connection;
use serde_json::Value;

use super::helpers::emit;
use crate::broker;
use crate::coding_agent::CodingAgent;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::{
    format_fleet_create, format_fleet_delete, format_fleet_list, format_fleet_show,
};
use crate::presentation;
use crate::runtime::resolve_mux;

#[derive(Subcommand)]
pub(crate) enum FleetCommand {
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

pub(crate) fn run(
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
            emit(json, &payload, || format_fleet_list(&fleets));
            Ok(())
        }
        FleetCommand::Show { fleet_id, json } => {
            let fleet = broker::require_fleet(conn, fleet_id)?;
            emit(json, &presentation::fleet(&fleet), || {
                format_fleet_show(&fleet)
            });
            Ok(())
        }
        FleetCommand::Delete { fleet_id, json } => {
            let result = crate::deletion::delete_fleet(conn, settings, fleet_id, deadline)?;
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
