//! The two-form `monitor` command (SPEC §6.3 *monitor*): the live-fleet
//! guard, the in-process heartbeat loop, and the one-shot `scan` batch
//! capture.

use rusqlite::Connection;

use clap::{Args, Subcommand};

use super::helpers::resolve_mux;
use crate::broker;
use crate::capture::{CaptureSnapshot, ScanEntry, write_scan};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::time::now_utc;

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
pub fn run(
    conn: &mut Connection,
    settings: &Settings,
    args: MonitorArgs,
) -> Result<(), CafleetError> {
    match args.command {
        Some(MonitorCommand::Scan {
            fleet_id,
            lines,
            ansi,
            json,
        }) => scan(conn, settings, fleet_id, lines, ansi, json),
        None => run_loop(
            conn,
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
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    tick: i64,
    interval: Option<i64>,
) -> Result<(), CafleetError> {
    require_live_fleet(conn, fleet_id)?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let mut out = std::io::stdout();
    crate::monitor::run_monitor_loop(
        conn,
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
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    lines: i64,
    ansi: bool,
    json_output: bool,
) -> Result<(), CafleetError> {
    let fleet = require_live_fleet(conn, fleet_id)?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let director_member_id = fleet.director_member_id;
    let members = broker::list_members(conn, fleet_id)?
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
