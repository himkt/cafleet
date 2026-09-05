//! The two-form `monitor` command (SPEC §6.3 *monitor*): the live-fleet
//! guard, the in-process heartbeat loop, and the one-shot `scan` batch
//! capture.

use clap::{Args, Subcommand};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::helpers::{connect, emit, resolve_mux};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::output::strip_ansi;
use crate::time::{format_utc, now_utc};

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct MonitorArgs {
    #[command(subcommand)]
    command: Option<MonitorCommand>,
    /// Fleet whose scheduler loop to run.
    #[arg(value_name = "FLEET_ID", required = true)]
    fleet_id: Option<i64>,
    /// Scan-tick cadence in seconds.
    #[arg(long, default_value_t = crate::monitor::DEFAULT_TICK_SECONDS,
          value_parser = clap::value_parser!(i64).range(1..))]
    tick: i64,
    /// Wake interval in seconds (0 disables the wake) [default:
    /// CAFLEET_MONITOR_WAKE_INTERVAL, 600].
    #[arg(long, value_parser = clap::value_parser!(i64).range(0..))]
    interval: Option<i64>,
}

#[derive(Subcommand)]
enum MonitorCommand {
    /// Capture the Director's pane and every active member's pane once.
    Scan {
        /// Fleet whose panes to scan.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Trailing lines captured per pane.
        #[arg(long, default_value_t = 20,
              value_parser = clap::value_parser!(i64).range(1..))]
        lines: i64,
        /// Preserve ANSI escapes in every captured content.
        #[arg(long)]
        ansi: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

fn require_live_fleet(
    conn: &rusqlite::Connection,
    fleet_id: i64,
) -> Result<broker::fleets::FleetRow, CafleetError> {
    match broker::fleets::fetch_fleet(conn, fleet_id)? {
        Some(fleet) if fleet.deleted_at.is_none() => Ok(fleet),
        _ => Err(CafleetError::App(format!("fleet {fleet_id} not found"))),
    }
}

/// Dispatch the two forms: the `scan` subcommand, else the loop (clap
/// guarantees the loop positional whenever no subcommand is given).
pub fn run(settings: &Settings, args: MonitorArgs) -> Result<(), CafleetError> {
    match args.command {
        Some(MonitorCommand::Scan {
            fleet_id,
            lines,
            ansi,
            json,
        }) => scan(settings, fleet_id, lines, ansi, json),
        None => run_loop(
            settings,
            args.fleet_id
                .expect("clap guarantees the loop positional when no subcommand is given"),
            args.tick,
            args.interval,
        ),
    }
}

/// Requires a live fleet, then the multiplexer; blocks in the loop until
/// stopped or displaced.
fn run_loop(
    settings: &Settings,
    fleet_id: i64,
    tick: i64,
    interval: Option<i64>,
) -> Result<(), CafleetError> {
    let mut conn = connect(settings)?;
    require_live_fleet(&conn, fleet_id)?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let mut out = std::io::stdout();
    crate::monitor::run_monitor_loop(
        &mut conn,
        &mux,
        &mut out,
        fleet_id,
        tick,
        interval.unwrap_or(settings.monitor_wake_interval),
    )
}

/// One-shot batch capture (SPEC §6.3 *monitor scan*): the Director's pane
/// first, then every other active placement-owning member ascending by
/// member id. An annotated entry never aborts the scan; no DB writes.
fn scan(
    settings: &Settings,
    fleet_id: i64,
    lines: i64,
    ansi: bool,
    json_output: bool,
) -> Result<(), CafleetError> {
    let conn = connect(settings)?;
    let fleet = require_live_fleet(&conn, fleet_id)?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let director_member_id = fleet.director_member_id;
    let members = broker::list_member_records(&conn, fleet_id)?
        .into_iter()
        .map(|row| row.member)
        .collect::<Vec<_>>();
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

    let mut sections = Vec::new();
    let mut entries = Vec::new();
    for member in roster {
        let member_id = member.member_id;
        let name = &member.name;
        let kind = member.kind.as_str();
        let placement =
            member
                .placement
                .as_ref()
                .ok_or_else(|| CafleetError::InvalidStoredValue {
                    field: "member placement".into(),
                    value: member_id.to_string(),
                })?;
        let coding_agent = &placement.coding_agent;
        let pane_id = placement.mux_pane_id.as_deref();

        let outcome = match pane_id {
            None => Err("pane not available (pending placement)".to_string()),
            Some(pane) => mux
                .capture_pane(pane, lines)
                .map_err(|error| format!("capture failed: {error}"))
                .map(|raw| if ansi { raw } else { strip_ansi(&raw) }),
        };
        match outcome {
            Ok(content) => {
                let pane = pane_id.expect("a successful capture has a pane");
                let captured_at = format_utc(now_utc());
                let digest = Sha256::digest(content.as_bytes());
                let content_sha256: String =
                    digest.iter().map(|byte| format!("{byte:02x}")).collect();
                sections.push(format!(
                    "=== {member_id} ({name}; kind={kind}; coding_agent={coding_agent}; \
                     pane={pane}; captured_at={captured_at}) ===\n{content}"
                ));
                entries.push(json!({
                    "member_id": member_id,
                    "name": name,
                    "kind": kind,
                    "coding_agent": coding_agent,
                    "pane_id": pane,
                    "lines": lines,
                    "content": content,
                    "captured_at": captured_at,
                    "content_sha256": content_sha256,
                    "error": Value::Null,
                }));
            }
            Err(annotation) => {
                let pane_token = pane_id.unwrap_or("—");
                sections.push(format!(
                    "=== {member_id} ({name}; kind={kind}; coding_agent={coding_agent}; \
                     pane={pane_token}) ===\n{annotation}"
                ));
                entries.push(json!({
                    "member_id": member_id,
                    "name": name,
                    "kind": kind,
                    "coding_agent": coding_agent,
                    "pane_id": pane_id,
                    "lines": lines,
                    "content": Value::Null,
                    "captured_at": Value::Null,
                    "content_sha256": Value::Null,
                    "error": annotation,
                }));
            }
        }
    }

    if json_output {
        emit(true, &Value::Array(entries), String::new);
    } else {
        println!("{}", sections.join("\n\n"));
    }
    Ok(())
}
