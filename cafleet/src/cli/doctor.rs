//! `cafleet doctor` — placement diagnostics + the assets-install report
//! (SPEC §6.3 *doctor*). Guard-exempt: it reports instead of blocking.

use clap::Args;
use serde_json::{Value, json};

use super::helpers::{connect, emit, resolve_mux};
use crate::broker::{asset_installs_table_exists, list_asset_installs};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;

#[derive(Args)]
pub struct DoctorArgs {
    /// Output in JSON format.
    #[arg(long)]
    json: bool,
}

pub fn run(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let presence_var = match mux.name() {
        "herdr" => "HERDR_ENV",
        _ => "TMUX",
    };
    let presence_value = std::env::var(presence_var).unwrap_or_default();

    let installs: Vec<Value> = match connect(settings) {
        Ok(conn) if asset_installs_table_exists(&conn) => list_asset_installs(&conn)?
            .into_iter()
            .map(|row| {
                let current = row["cafleet_version"] == super::VERSION;
                json!({
                    "coding_agent": row["coding_agent"],
                    "cafleet_version": row["cafleet_version"],
                    "installed_at": row["installed_at"],
                    "current": current,
                })
            })
            .collect(),
        _ => Vec::new(),
    };

    let payload = json!({
        "multiplexer": {
            "backend": mux.name(),
            "session": context.session,
            "window_id": context.window_id,
            "pane_id": context.pane_id,
            "presence_var": presence_var,
            "presence_value": presence_value,
        },
        "assets": {
            "cli_version": super::VERSION,
            "installs": installs,
        },
    });
    emit(args.json, &payload, || {
        let mut lines = vec![
            "multiplexer:".to_string(),
            format!("  {:<14} {}", "backend:", mux.name()),
            format!("  {:<14} {}", "session:", context.session),
            format!("  {:<14} {}", "window_id:", context.window_id),
            format!("  {:<14} {}", "pane_id:", context.pane_id),
            format!("  {:<14} {presence_var}={presence_value}", "presence:"),
            "assets:".to_string(),
        ];
        if installs.is_empty() {
            lines.push("  (no assets install recorded; run 'cafleet setup')".to_string());
        } else {
            lines.push(format!("  {:<12} {}", "cli_version:", super::VERSION));
            for install in &installs {
                let agent = install["coding_agent"]
                    .as_str()
                    .expect("rows carry the agent");
                let version = install["cafleet_version"]
                    .as_str()
                    .expect("rows carry the version");
                let installed_at = install["installed_at"]
                    .as_str()
                    .expect("rows carry the timestamp");
                let state = if install["current"] == true {
                    "ok"
                } else {
                    "STALE"
                };
                lines.push(format!(
                    "  {:<12} {version} ({installed_at}) {state}",
                    format!("{agent}:")
                ));
            }
        }
        lines.join("\n")
    });
    Ok(())
}
