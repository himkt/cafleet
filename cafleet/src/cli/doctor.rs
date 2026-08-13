//! `cafleet doctor` — the three-section environment diagnosis (SPEC §6.3
//! *doctor*): multiplexer, database, coding agents, rendered without early
//! abort. Guard-exempt: it reports instead of blocking; exit 1 iff any
//! rendered issue.

use std::path::{Path, PathBuf};

use clap::Args;
use rusqlite::Connection;
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use super::helpers::{emit, resolve_mux, tilde};
use super::setup::{has_foreign_tables, recorded_version};
use crate::broker::{asset_installs_table_exists, list_asset_installs};
use crate::config::Settings;
use crate::config_dir::{DirSource, claude_config_dir, codex_home, opencode_preset_base};
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerContext};

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

enum DbReport {
    Head(u32),
    Behind(u32, u32),
    Ahead(u32, u32),
    Unversioned,
    Missing,
    Unreachable(String),
}

impl DbReport {
    fn ok(&self) -> bool {
        matches!(self, DbReport::Head(_))
    }

    fn detail(&self) -> String {
        match self {
            DbReport::Head(n) => format!("schema {n} (head)"),
            DbReport::Behind(m, n) => format!("schema {m}, head is {n} — run: cafleet setup"),
            DbReport::Ahead(m, n) => {
                format!("schema {m} is newer than this CLI (head {n}) — upgrade cafleet")
            }
            DbReport::Unversioned => {
                "database has tables but no schema history — not a cafleet database?".to_string()
            }
            DbReport::Missing => "no database — run: cafleet setup".to_string(),
            DbReport::Unreachable(error) => error.clone(),
        }
    }

    fn schema_version(&self) -> Value {
        match self {
            DbReport::Head(n) => json!(n),
            DbReport::Behind(m, _) | DbReport::Ahead(m, _) => json!(m),
            _ => Value::Null,
        }
    }
}

struct AgentRow {
    agent: &'static str,
    path_cell: String,
    json_path: Value,
    source_cell: String,
    json_source: String,
    setup_cell: String,
    state: &'static str,
    recorded_version: Value,
    installed_at: Value,
    error: Value,
    is_issue: bool,
}

pub fn run(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    let home = PathBuf::from(
        std::env::var("HOME").map_err(|_| CafleetError::App("HOME is not set".to_string()))?,
    );

    let mux = multiplexer_report(settings);
    let conn = crate::db::connect(&settings.database_url);
    let db = match &conn {
        Ok(conn) => database_report(conn),
        Err(error) => DbReport::Unreachable(error.message().to_string()),
    };
    let rows: Vec<Value> = match &conn {
        Ok(conn) if asset_installs_table_exists(conn) => list_asset_installs(conn)?,
        _ => Vec::new(),
    };
    let agents = agent_rows(&home, &rows);
    let superseded = superseded_rows(&agents, &rows);

    let issues = usize::from(mux.is_err())
        + usize::from(!db.ok())
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
        "database": {
            "ok": db.ok(),
            "schema_version": db.schema_version(),
            "head_version": crate::db::head_version(),
            "error": if db.ok() { Value::Null } else { json!(db.detail()) },
        },
        "coding_agents": {
            "ok": agents_ok,
            "cli_version": super::VERSION,
            "agents": agents.iter().map(|row| json!({
                "coding_agent": row.agent,
                "path": row.json_path,
                "source": row.json_source,
                "recorded_version": row.recorded_version,
                "installed_at": row.installed_at,
                "state": row.state,
                "error": row.error,
            })).collect::<Vec<_>>(),
            "superseded": superseded.iter().map(|row| json!({
                "coding_agent": row["coding_agent"],
                "path": row["path"],
                "recorded_version": row["cafleet_version"],
                "installed_at": row["installed_at"],
            })).collect::<Vec<_>>(),
        },
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
        lines.push(format!("{} database", if db.ok() { "✓" } else { "✗" }));
        lines.push(format!("  {}", db.detail()));
        lines.push(format!(
            "{} coding agents",
            if agents_ok { "✓" } else { "✗" }
        ));
        for line in framed_table(&agents) {
            lines.push(format!("  {line}"));
        }
        for row in &superseded {
            lines.push(format!(
                "  note: {} was previously set up at {}",
                row["coding_agent"].as_str().expect("rows carry the agent"),
                tilde(row["path"].as_str().expect("rows carry the path"), &home)
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

fn database_report(conn: &Connection) -> DbReport {
    let head = crate::db::head_version();
    match recorded_version(conn) {
        Err(error) => DbReport::Unreachable(error.message().to_string()),
        Ok(Some(recorded)) if recorded == head => DbReport::Head(recorded),
        Ok(Some(recorded)) if recorded < head => DbReport::Behind(recorded, head),
        Ok(Some(recorded)) => DbReport::Ahead(recorded, head),
        Ok(None) => match has_foreign_tables(conn) {
            Err(error) => DbReport::Unreachable(error.message().to_string()),
            Ok(true) => DbReport::Unversioned,
            Ok(false) => DbReport::Missing,
        },
    }
}

/// One row per agent in the fixed order, resolution errors caught per agent
/// and rendered as the `error` state instead of aborting.
fn agent_rows(home: &Path, rows: &[Value]) -> Vec<AgentRow> {
    let env = |name: &str| std::env::var(name).ok();
    ["claude", "codex", "opencode"]
        .into_iter()
        .map(|agent| {
            let (var, resolved) = match agent {
                "claude" => ("CLAUDE_CONFIG_DIR", claude_config_dir(&env, home)),
                "codex" => ("CODEX_HOME", codex_home(&env, home)),
                _ => ("OPENCODE_CONFIG_DIR", opencode_preset_base(&env, home)),
            };
            match resolved {
                Err(error) => AgentRow {
                    agent,
                    path_cell: env(var).unwrap_or_default(),
                    json_path: Value::Null,
                    source_cell: format!("${var}"),
                    json_source: var.to_string(),
                    setup_cell: format!("✗ {var} is not an absolute path"),
                    state: "error",
                    recorded_version: Value::Null,
                    installed_at: Value::Null,
                    error: json!(error.message()),
                    is_issue: true,
                },
                Ok(resolved) => {
                    let identity = resolved.path.display().to_string();
                    let (source_cell, json_source) = match resolved.source {
                        DirSource::EnvVar(name) => (format!("${name}"), name.to_string()),
                        DirSource::Default => ("default".to_string(), "default".to_string()),
                    };
                    let current = rows.iter().find(|row| {
                        row["coding_agent"] == agent && row["path"] == identity.as_str()
                    });
                    let (setup_cell, state, recorded, installed_at, is_issue) = match current {
                        Some(row) if row["cafleet_version"] == super::VERSION => (
                            format!("✓ {}", super::VERSION),
                            "ok",
                            row["cafleet_version"].clone(),
                            row["installed_at"].clone(),
                            false,
                        ),
                        Some(row) => (
                            format!(
                                "✗ {} → cafleet setup --coding-agent {agent}",
                                row["cafleet_version"]
                                    .as_str()
                                    .expect("rows carry the version")
                            ),
                            "stale",
                            row["cafleet_version"].clone(),
                            row["installed_at"].clone(),
                            true,
                        ),
                        None => (
                            format!("– cafleet setup --coding-agent {agent}"),
                            "not_installed",
                            Value::Null,
                            Value::Null,
                            false,
                        ),
                    };
                    AgentRow {
                        agent,
                        path_cell: tilde(&identity, home),
                        json_path: json!(identity),
                        source_cell,
                        json_source,
                        setup_cell,
                        state,
                        recorded_version: recorded,
                        installed_at,
                        error: Value::Null,
                        is_issue,
                    }
                }
            }
        })
        .collect()
}

/// The recorded rows at paths other than their agent's resolved identity
/// path, in the stored ascending `(coding_agent, path)` order. Rows of an
/// agent in the `error` state stay unclassified and are omitted.
fn superseded_rows<'a>(agents: &[AgentRow], rows: &'a [Value]) -> Vec<&'a Value> {
    rows.iter()
        .filter(|row| {
            agents.iter().any(|agent_row| {
                row["coding_agent"] == agent_row.agent
                    && agent_row.json_path != Value::Null
                    && row["path"] != agent_row.json_path
            })
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
