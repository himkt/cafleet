//! Shared schema and asset facts. CLI policy and display belong to callers.

use std::path::Path;

use rusqlite::Connection;

use crate::broker::members::db_err;
use crate::config_dir::{self, EnvLookup, ResolvedDir};
use crate::error::CafleetError;

#[derive(Debug)]
pub(crate) enum SchemaState {
    Missing,
    Unversioned,
    Behind { recorded: u32, head: u32 },
    Head { version: u32 },
    Ahead { recorded: u32, head: u32 },
    Unreachable { cause: CafleetError },
}

#[derive(Debug, Clone)]
pub(crate) struct AssetInstallRecord {
    pub(crate) coding_agent: String,
    pub(crate) path: String,
    pub(crate) cafleet_version: String,
    pub(crate) installed_at: String,
}

#[derive(Debug)]
pub(crate) enum AssetState {
    Current {
        identity: ResolvedDir,
        install: AssetInstallRecord,
    },
    Stale {
        identity: ResolvedDir,
        install: AssetInstallRecord,
    },
    NotInstalled {
        identity: ResolvedDir,
    },
    PathError {
        variable: &'static str,
        raw_value: String,
        cause: CafleetError,
    },
}

#[derive(Debug)]
pub(crate) struct AgentAsset {
    pub(crate) coding_agent: &'static str,
    pub(crate) state: AssetState,
}

#[derive(Debug)]
pub(crate) struct AssetReport {
    pub(crate) agents: Vec<AgentAsset>,
    pub(crate) superseded: Vec<AssetInstallRecord>,
}

#[derive(Debug)]
pub(crate) struct Diagnosis {
    pub(crate) head_version: u32,
    pub(crate) schema: SchemaState,
    pub(crate) assets: Option<Result<AssetReport, CafleetError>>,
}

#[derive(Debug)]
pub(crate) enum AssetMode {
    Guard,
    Report,
}

pub(crate) fn recorded_version(conn: &Connection) -> Result<Option<u32>, CafleetError> {
    let ledger_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='refinery_schema_history')",
        [], |row| row.get(0),
    ).map_err(db_err)?;
    if !ledger_exists {
        return Ok(None);
    }
    conn.query_row(
        "SELECT MAX(version) FROM refinery_schema_history",
        [],
        |row| row.get(0),
    )
    .map_err(db_err)
}

pub(crate) fn has_foreign_tables(conn: &Connection) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'refinery_schema_history')",
        [], |row| row.get(0),
    ).map_err(db_err)
}

pub(crate) fn classify_schema(conn: &Connection, head: u32) -> SchemaState {
    match recorded_version(conn) {
        Err(cause) => SchemaState::Unreachable { cause },
        Ok(Some(recorded)) if recorded < head => SchemaState::Behind { recorded, head },
        Ok(Some(recorded)) if recorded > head => SchemaState::Ahead { recorded, head },
        Ok(Some(version)) => SchemaState::Head { version },
        Ok(None) => match has_foreign_tables(conn) {
            Ok(true) => SchemaState::Unversioned,
            Ok(false) => SchemaState::Missing,
            Err(cause) => SchemaState::Unreachable { cause },
        },
    }
}

pub(crate) fn asset_table_exists(conn: &Connection) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_installs')",
        [],
        |row| row.get(0),
    )
    .map_err(db_err)
}

pub(crate) fn diagnose_assets(
    conn: Option<&Connection>,
    env: EnvLookup<'_>,
    home: &Path,
    cli_version: &str,
    mode: AssetMode,
) -> Result<AssetReport, CafleetError> {
    let mut agents = Vec::new();
    for (agent, variable) in [
        ("claude", "CLAUDE_CONFIG_DIR"),
        ("codex", "CODEX_HOME"),
        ("opencode", "OPENCODE_CONFIG_DIR"),
    ] {
        // Snapshot each variable once, including its raw value on a path error.
        let raw = env(variable);
        let lookup = |_: &str| raw.clone();
        let identity = match agent {
            "claude" => config_dir::claude_config_dir(&lookup, home),
            "codex" => config_dir::codex_home(&lookup, home),
            _ => config_dir::opencode_preset_base(&lookup, home),
        };
        let state = match identity {
            Ok(identity) => AssetState::NotInstalled { identity },
            Err(cause) => {
                if matches!(mode, AssetMode::Guard) {
                    return Err(cause);
                }
                AssetState::PathError {
                    variable,
                    raw_value: raw.unwrap_or_default(),
                    cause,
                }
            }
        };
        agents.push(AgentAsset {
            coding_agent: agent,
            state,
        });
    }
    let mut records = Vec::new();
    if let Some(conn) = conn
        && asset_table_exists(conn)?
    {
        let mut statement = conn.prepare(
            "SELECT coding_agent, path, cafleet_version, installed_at FROM asset_installs ORDER BY coding_agent, path"
        ).map_err(db_err)?;
        records = statement
            .query_map([], |row| {
                Ok(AssetInstallRecord {
                    coding_agent: row.get(0)?,
                    path: row.get(1)?,
                    cafleet_version: row.get(2)?,
                    installed_at: row.get(3)?,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
    }
    let mut superseded = Vec::new();
    let mut classified = Vec::new();
    for agent in agents {
        let state = match agent.state {
            AssetState::NotInstalled { identity } => {
                let path = identity.path.display().to_string();
                superseded.extend(
                    records
                        .iter()
                        .filter(|r| r.coding_agent == agent.coding_agent && r.path != path)
                        .cloned(),
                );
                match records
                    .iter()
                    .find(|r| r.coding_agent == agent.coding_agent && r.path == path)
                {
                    Some(install) if install.cafleet_version == cli_version => {
                        AssetState::Current {
                            identity,
                            install: install.clone(),
                        }
                    }
                    Some(install) => AssetState::Stale {
                        identity,
                        install: install.clone(),
                    },
                    None => AssetState::NotInstalled { identity },
                }
            }
            other => other,
        };
        classified.push(AgentAsset {
            coding_agent: agent.coding_agent,
            state,
        });
    }
    // Agent order matches the SQL order; each agent's records retain path order.
    Ok(AssetReport {
        agents: classified,
        superseded,
    })
}
