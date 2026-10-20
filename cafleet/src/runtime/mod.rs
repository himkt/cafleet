//! Concrete process adapters shared by CLI and HTTP.
pub mod system;

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::broker;
use crate::config::Settings;
use crate::delivery::deliver_pane;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, MultiplexerError, resolve_multiplexer};
use crate::time::now_utc;
use system::SystemRunner;

const LOOP_START_TIMEOUT: Duration = Duration::from_secs(5);
const LOOP_START_POLL: Duration = Duration::from_millis(100);

/// The environment snapshot the backends read presence variables from.
fn env_snapshot() -> std::collections::HashMap<String, String> {
    std::env::vars().collect()
}

pub fn resolve_mux(settings: &Settings) -> Result<AnyMultiplexer, MultiplexerError> {
    resolve_multiplexer(
        settings.multiplexer.as_deref(),
        env_snapshot(),
        Rc::new(SystemRunner),
    )
}

/// Whether one delivery attempt keystroked the preview of `message_id` into
/// `member_id`'s pane. Without a multiplexer the attempt is skipped and the
/// preview stays owed.
pub fn deliver_preview(
    conn: &mut Connection,
    settings: &Settings,
    mux: Option<&AnyMultiplexer>,
    member_id: i64,
    message_id: i64,
) -> Result<bool, CafleetError> {
    let Some(mux) = mux else {
        return Ok(false);
    };
    Ok(deliver_pane(conn, mux, settings, member_id, now_utc())?.fired_preview(message_id))
}

/// Ensure the fleet's loop when the fleet has owed work; a fleet that owes
/// nothing needs neither a loop nor a multiplexer.
pub fn ensure_loop_for_owed_work(
    conn: &Connection,
    settings: &Settings,
    fleet_id: i64,
) -> Result<(), CafleetError> {
    if broker::fleet_has_owed_work(conn, fleet_id)? {
        ensure_monitor_loop(conn, settings, fleet_id)?;
    }
    Ok(())
}

/// Where a detached loop for `fleet_id` appends its stdout and stderr.
pub fn monitor_log_path(settings: &Settings, fleet_id: i64) -> Result<PathBuf, CafleetError> {
    let database = PathBuf::from(crate::db::database_path(&settings.database_url)?);
    let directory = database.parent().ok_or_else(|| {
        CafleetError::App(format!(
            "database path '{}' has no parent directory",
            database.display()
        ))
    })?;
    Ok(directory.join(format!("monitor-{fleet_id}.log")))
}

/// Start the fleet's monitor loop as a detached process unless one is live,
/// and return once it is. Two racing callers are safe: the loop's
/// single-instance claim lets one spawned loop win and the other exits.
pub fn ensure_monitor_loop(
    conn: &Connection,
    settings: &Settings,
    fleet_id: i64,
) -> Result<(), CafleetError> {
    if broker::monitor_is_live(conn, fleet_id, now_utc())? {
        return Ok(());
    }
    let log_path = monitor_log_path(settings, fleet_id)?;
    let open_log = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| {
                CafleetError::App(format!(
                    "cannot open monitor log {}: {error}",
                    log_path.display()
                ))
            })
    };
    let cannot_spawn = |error: std::io::Error| {
        CafleetError::App(format!(
            "cannot spawn the monitor loop for fleet {fleet_id}: {error}"
        ))
    };
    Command::new(std::env::current_exe().map_err(cannot_spawn)?)
        .args(["monitor", &fleet_id.to_string()])
        .stdin(Stdio::null())
        .stdout(open_log()?)
        .stderr(open_log()?)
        .process_group(0)
        .spawn()
        .map_err(cannot_spawn)?;

    let deadline = Instant::now() + LOOP_START_TIMEOUT;
    while Instant::now() < deadline {
        std::thread::sleep(LOOP_START_POLL);
        if broker::monitor_is_live(conn, fleet_id, now_utc())? {
            return Ok(());
        }
    }
    Err(CafleetError::App(format!(
        "monitor loop for fleet {fleet_id} did not start; see {}",
        log_path.display()
    )))
}
