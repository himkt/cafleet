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
                let install = records
                    .iter()
                    .find(|r| r.coding_agent == agent.coding_agent && r.path == path)
                    .cloned();
                match install {
                    Some(install) if install.cafleet_version == cli_version => {
                        AssetState::Current { identity, install }
                    }
                    Some(install) => AssetState::Stale { identity, install },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_states_preserve_versions_and_sql_failures() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(matches!(classify_schema(&conn, 8), SchemaState::Missing));
        conn.execute_batch("CREATE TABLE app_data(value TEXT)")
            .unwrap();
        assert!(matches!(
            classify_schema(&conn, 8),
            SchemaState::Unversioned
        ));
        conn.execute_batch("CREATE TABLE refinery_schema_history(version INTEGER)")
            .unwrap();
        for version in [7, 8, 9] {
            conn.execute("INSERT INTO refinery_schema_history VALUES (?1)", [version])
                .unwrap();
            let state = classify_schema(&conn, 8);
            assert!(matches!(
                (version, state),
                (
                    7,
                    SchemaState::Behind {
                        recorded: 7,
                        head: 8
                    }
                ) | (8, SchemaState::Head { version: 8 })
                    | (
                        9,
                        SchemaState::Ahead {
                            recorded: 9,
                            head: 8
                        }
                    )
            ));
        }
        conn.execute_batch("ALTER TABLE refinery_schema_history RENAME COLUMN version TO broken")
            .unwrap();
        let SchemaState::Unreachable { cause } = classify_schema(&conn, 8) else {
            panic!("expected SQL failure")
        };
        assert!(cause.message().contains("version"));
    }

    #[test]
    fn assets_match_resolved_paths_and_preserve_superseded_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let paths = |name: &str| Some(dir.path().join(name).display().to_string());
        for (agent, path, version) in [
            ("claude", paths("CLAUDE_CONFIG_DIR").unwrap(), "current"),
            ("codex", paths("CODEX_HOME").unwrap(), "old"),
            (
                "claude",
                dir.path().join("previous").display().to_string(),
                "old",
            ),
        ] {
            crate::broker::record_asset_install(&mut conn, agent, &path, version).unwrap();
        }
        let report = diagnose_assets(
            Some(&conn),
            &paths,
            dir.path(),
            "current",
            AssetMode::Report,
        )
        .unwrap();
        assert!(matches!(report.agents[0].state, AssetState::Current { .. }));
        assert!(matches!(report.agents[1].state, AssetState::Stale { .. }));
        assert!(matches!(
            report.agents[2].state,
            AssetState::NotInstalled { .. }
        ));
        assert_eq!(report.superseded.len(), 1);
        assert_eq!(
            report.superseded[0].path,
            dir.path().join("previous").display().to_string()
        );
    }

    #[test]
    fn assets_distinguish_absence_from_path_and_sql_errors() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        for connection in [None, Some(&conn)] {
            let report =
                diagnose_assets(connection, &|_| None, dir.path(), "1", AssetMode::Report).unwrap();
            assert!(
                report
                    .agents
                    .iter()
                    .all(|agent| matches!(agent.state, AssetState::NotInstalled { .. }))
            );
        }
        conn.execute_batch("CREATE TABLE asset_installs(wrong TEXT)")
            .unwrap();
        let error = diagnose_assets(
            Some(&conn),
            &|_| Some("relative".into()),
            dir.path(),
            "1",
            AssetMode::Guard,
        )
        .unwrap_err();
        assert!(error.message().contains("must be an absolute path"));
        for mode in [AssetMode::Guard, AssetMode::Report] {
            assert!(diagnose_assets(Some(&conn), &|_| None, dir.path(), "1", mode).is_err());
        }
    }
}
