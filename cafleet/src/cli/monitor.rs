//! The `monitor` group (SPEC §6.3 *monitor group*): the shared live-fleet
//! guard and `capture`; `start` arrives with the heartbeat loop.

use clap::Subcommand;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::FleetIdArg;
use super::helpers::{connect, emit, require_fleet_id};
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;
use crate::output::strip_ansi;
use crate::time::{format_utc, now_utc};

#[derive(Subcommand)]
pub enum MonitorCommand {
    /// Run the per-fleet scheduler loop in-process.
    Start {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Scan-tick cadence in seconds.
        #[arg(long, default_value_t = crate::monitor::DEFAULT_TICK_SECONDS,
              value_parser = clap::value_parser!(i64).range(1..))]
        tick: i64,
        /// Director wake interval in seconds (0 disables the wake) [default:
        /// CAFLEET_MONITOR_WAKE_INTERVAL, 600].
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Capture the tail of a member's pane.
    Capture {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Target member's ID.
        #[arg(long = "member-id")]
        member_id: i64,
        /// Number of trailing lines to capture.
        #[arg(long, default_value_t = 20)]
        lines: i64,
        /// Emit the raw capture, ANSI escapes preserved.
        #[arg(long, overrides_with = "no_ansi")]
        ansi: bool,
        /// Strip ANSI escapes and clean carriage-return redraws.
        #[arg(long = "no-ansi", overrides_with = "ansi")]
        no_ansi: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

fn require_live_fleet(conn: &rusqlite::Connection, fleet_id: i64) -> Result<(), CafleetError> {
    match broker::get_fleet(conn, fleet_id)? {
        Some(fleet) if fleet["deleted_at"].is_null() => Ok(()),
        _ => Err(CafleetError::App(format!("fleet {fleet_id} not found"))),
    }
}

pub fn run(settings: &Settings, command: MonitorCommand) -> Result<(), CafleetError> {
    match command {
        MonitorCommand::Start {
            fleet,
            tick,
            interval,
        } => start(settings, fleet.fleet_id, tick, interval),
        MonitorCommand::Capture {
            fleet,
            member_id,
            lines,
            ansi,
            no_ansi: _,
            json,
        } => capture(settings, fleet.fleet_id, member_id, lines, ansi, json),
    }
}

/// Requires a live fleet, then the multiplexer; blocks in the loop until
/// stopped or displaced.
fn start(
    settings: &Settings,
    fleet_id: Option<i64>,
    tick: i64,
    interval: Option<u64>,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let mut conn = connect(settings)?;
    require_live_fleet(&conn, fleet_id)?;
    let mux =
        super::helpers::resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
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

fn capture(
    settings: &Settings,
    fleet_id: Option<i64>,
    member_id: i64,
    lines: i64,
    ansi: bool,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    {
        let conn = connect(settings)?;
        require_live_fleet(&conn, fleet_id)?;
    }
    let (mux, pane_id) =
        super::member::load_member_with_pane(settings, fleet_id, member_id, "capture")?;
    let raw = mux
        .capture_pane(&pane_id, lines)
        .map_err(|e| CafleetError::App(format!("capture failed: {e}")))?;
    let content = if ansi { raw } else { strip_ansi(&raw) };
    let captured_at = format_utc(now_utc());
    let digest = Sha256::digest(content.as_bytes());
    let content_sha256: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let payload = json!({
        "member_id": member_id,
        "pane_id": pane_id,
        "lines": lines,
        "content": content,
        "captured_at": captured_at,
        "content_sha256": content_sha256,
    });
    if json {
        emit(true, &payload, String::new);
    } else {
        print!("{content}");
    }
    Ok(())
}
