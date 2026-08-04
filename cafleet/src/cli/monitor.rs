//! The flattened `monitor` command (SPEC §6.3 *monitor*): the live-fleet
//! guard and the in-process heartbeat loop.

use clap::Args;

use super::helpers::connect;
use crate::broker;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;

#[derive(Args)]
pub struct MonitorArgs {
    /// Fleet whose scheduler loop to run.
    #[arg(value_name = "FLEET_ID")]
    fleet_id: i64,
    /// Scan-tick cadence in seconds.
    #[arg(long, default_value_t = crate::monitor::DEFAULT_TICK_SECONDS,
          value_parser = clap::value_parser!(i64).range(1..))]
    tick: i64,
    /// Director wake interval in seconds (0 disables the wake) [default:
    /// CAFLEET_MONITOR_WAKE_INTERVAL, 600].
    #[arg(long)]
    interval: Option<u64>,
}

fn require_live_fleet(conn: &rusqlite::Connection, fleet_id: i64) -> Result<(), CafleetError> {
    match broker::get_fleet(conn, fleet_id)? {
        Some(fleet) if fleet["deleted_at"].is_null() => Ok(()),
        _ => Err(CafleetError::App(format!("fleet {fleet_id} not found"))),
    }
}

/// Requires a live fleet, then the multiplexer; blocks in the loop until
/// stopped or displaced.
pub fn run(settings: &Settings, args: MonitorArgs) -> Result<(), CafleetError> {
    let mut conn = connect(settings)?;
    require_live_fleet(&conn, args.fleet_id)?;
    let mux =
        super::helpers::resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let mut out = std::io::stdout();
    crate::monitor::run_monitor_loop(
        &mut conn,
        &mux,
        &mut out,
        args.fleet_id,
        args.tick,
        args.interval.unwrap_or(settings.monitor_wake_interval),
    )
}
