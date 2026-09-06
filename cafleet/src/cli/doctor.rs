//! `cafleet doctor` — the three-section environment diagnosis (SPEC §6.3
//! *doctor*): multiplexer, database, coding agents, rendered without early
//! abort. Guard-exempt: it reports instead of blocking; exit 1 iff any
//! rendered issue.

use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use super::helpers::{emit, resolve_mux, tilde};
use super::{InvocationEvent, InvocationHooks, SchemaPoint, inspect_schema};
use crate::config::Settings;
use crate::config_dir::DirSource;
use crate::diagnosis::{self, AssetMode, AssetReport, AssetState, Diagnosis, SchemaState};
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerContext};
use crate::presentation;

#[derive(Args)]
pub struct DoctorArgs {
    /// Output in JSON format.
    #[arg(long)]
    json: bool,
}

struct MuxOk {
    backend: String,
    context: MultiplexerContext,
    presence_var: &'static str,
    presence_value: String,
}

struct AgentRow {
    agent: &'static str,
    path_cell: String,
    source_cell: String,
    setup_cell: String,
    is_issue: bool,
}

pub fn run(
    settings: &Settings,
    args: DoctorArgs,
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".to_string()))?,
    );

    let mux = multiplexer_report(settings);
    let conn = (hooks.connect)(&settings.database_url);
    let mut facts = Diagnosis {
        head_version: crate::db::head_version(),
        schema: match &conn {
            Ok(conn) => inspect_schema(conn, SchemaPoint::Doctor, hooks),
            Err(cause) => SchemaState::Unreachable {
                cause: cause.clone(),
            },
        },
        assets: None,
    };
    let db_ok = matches!(facts.schema, SchemaState::Head { .. });
    let asset_conn = if db_ok { conn.as_ref().ok() } else { None };
    let assets = diagnosis::diagnose_assets(
        asset_conn,
        hooks.asset_env,
        &home,
        super::VERSION,
        AssetMode::Report,
    );
    (hooks.observe)(InvocationEvent::AssetsInspected {
        conn: asset_conn,
        result: &assets,
    });
    facts.assets = Some(assets);
    let assets = facts
        .assets
        .as_ref()
        .expect("assets inspected")
        .as_ref()
        .map_err(Clone::clone)?;
    let agents = agent_rows(&home, assets);
    let superseded = &assets.superseded;

    let issues = usize::from(mux.is_err())
        + usize::from(!db_ok)
        + agents.iter().filter(|row| row.is_issue).count();
    let agents_ok = agents.iter().all(|row| !row.is_issue);

    let payload = json!({
        "multiplexer": match &mux {
            Ok(m) => json!({
                "ok": true,
                "backend": m.backend,
                "session": m.context.session,
                "window_id": m.context.window_id,
                "pane_id": m.context.pane_id,
                "presence_var": m.presence_var,
                "presence_value": m.presence_value,
                "error": Value::Null,
            }),
            Err(error) => json!({
                "ok": false,
                "backend": Value::Null,
                "session": Value::Null,
                "window_id": Value::Null,
                "pane_id": Value::Null,
                "presence_var": Value::Null,
                "presence_value": Value::Null,
                "error": error,
            }),
        },
        "database": presentation::doctor_database(&facts.schema, facts.head_version),
        "coding_agents": presentation::doctor_assets(assets, super::VERSION),
        "issues": issues,
    });

    emit(args.json, &payload, || {
        let mut lines = vec![format!("cafleet {}", super::VERSION)];
        match &mux {
            Ok(m) => {
                lines.push("✓ multiplexer".to_string());
                lines.push(format!("  {:<10} {}", "backend:", m.backend));
                lines.push(format!("  {:<10} {}", "session:", m.context.session));
                lines.push(format!("  {:<10} {}", "window_id:", m.context.window_id));
                lines.push(format!("  {:<10} {}", "pane_id:", m.context.pane_id));
                lines.push(format!(
                    "  {:<10} {}={}",
                    "presence:", m.presence_var, m.presence_value
                ));
            }
            Err(error) => {
                lines.push("✗ multiplexer".to_string());
                lines.push(format!("  {error}"));
            }
        }
        lines.push(format!("{} database", if db_ok { "✓" } else { "✗" }));
        lines.push(format!(
            "  {}",
            presentation::doctor_database_detail(&facts.schema)
        ));
        lines.push(format!(
            "{} coding agents",
            if agents_ok { "✓" } else { "✗" }
        ));
        for line in framed_table(&agents) {
            lines.push(format!("  {line}"));
        }
        for row in superseded {
            lines.push(format!(
                "  note: {} was previously set up at {}",
                row.coding_agent,
                tilde(&row.path, &home)
            ));
        }
        lines.push(match issues {
            0 => "no issues found".to_string(),
            1 => "1 issue found".to_string(),
            n => format!("{n} issues found"),
        });
        lines.join("\n")
    });

    if issues > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn multiplexer_report(settings: &Settings) -> Result<MuxOk, String> {
    let mux = resolve_mux(settings).map_err(|e| e.to_string())?;
    mux.ensure_available().map_err(|e| e.to_string())?;
    let context = mux.context_discovery().map_err(|e| e.to_string())?;
    let presence_var = match mux.name() {
        "herdr" => "HERDR_ENV",
        _ => "TMUX",
    };
    Ok(MuxOk {
        backend: mux.name().to_string(),
        context,
        presence_var,
        presence_value: std::env::var(presence_var).unwrap_or_default(),
    })
}

/// Render typed asset facts using the existing text-only cells.
fn agent_rows(home: &Path, report: &AssetReport) -> Vec<AgentRow> {
    report
        .agents
        .iter()
        .map(|agent| {
            let (identity, setup_cell, is_issue) = match &agent.state {
                AssetState::Current { identity, install } => {
                    (identity, format!("✓ {}", install.cafleet_version), false)
                }
                AssetState::Stale { identity, install } => (
                    identity,
                    format!(
                        "✗ {} → cafleet setup --coding-agent {}",
                        install.cafleet_version, agent.coding_agent
                    ),
                    true,
                ),
                AssetState::NotInstalled { identity } => (
                    identity,
                    format!("– cafleet setup --coding-agent {}", agent.coding_agent),
                    false,
                ),
                AssetState::Incomplete {
                    identity, recovery, ..
                } => (identity, format!("✗ {}", recovery.diagnostic()), true),
                AssetState::PathError {
                    variable,
                    raw_value,
                    ..
                } => {
                    return AgentRow {
                        agent: agent.coding_agent,
                        path_cell: raw_value.clone(),
                        source_cell: format!("${variable}"),
                        setup_cell: format!("✗ {variable} is not an absolute path"),
                        is_issue: true,
                    };
                }
            };
            AgentRow {
                agent: agent.coding_agent,
                path_cell: tilde(&identity.path.display().to_string(), home),
                source_cell: match identity.source {
                    DirSource::EnvVar(name) => format!("${name}"),
                    DirSource::Default => "default".into(),
                },
                setup_cell,
                is_issue,
            }
        })
        .collect()
}

/// The light box-drawing framed table, aligned by display width.
fn framed_table(agents: &[AgentRow]) -> Vec<String> {
    const HEADER: [&str; 4] = ["coding agent", "path", "source", "setup"];
    let body: Vec<[&str; 4]> = agents
        .iter()
        .map(|row| {
            [
                row.agent,
                row.path_cell.as_str(),
                row.source_cell.as_str(),
                row.setup_cell.as_str(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..HEADER.len())
        .map(|column| {
            body.iter()
                .map(|row| row[column].width())
                .chain(std::iter::once(HEADER[column].width()))
                .max()
                .expect("the header supplies at least one width")
        })
        .collect();

    let bar = |left: char, mid: char, right: char| {
        let segments: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{left}{}{right}", segments.join(&mid.to_string()))
    };
    let line = |cells: &[&str; 4]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!(" {cell}{} ", " ".repeat(width - cell.width())))
            .collect();
        format!("│{}│", padded.join("│"))
    };

    let mut out = vec![bar('┌', '┬', '┐'), line(&HEADER), bar('├', '┼', '┤')];
    for row in &body {
        out.push(line(row));
    }
    out.push(bar('└', '┴', '┘'));
    out
}
