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
    Incomplete {
        identity: ResolvedDir,
        install: Option<AssetInstallRecord>,
        recovery: crate::assets::IncompleteInstall,
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
                // Reuse the resolved identity; do not reread environment variables.
                let paths =
                    crate::assets::agent_paths(&|_| Some(path.clone()), home, agent.coding_agent)?;
                if let Some(recovery) = crate::assets::inspect_install(&paths)? {
                    AssetState::Incomplete {
                        identity,
                        install,
                        recovery,
                    }
                } else {
                    match install {
                        Some(install) if install.cafleet_version == cli_version => {
                            AssetState::Current { identity, install }
                        }
                        Some(install) => AssetState::Stale { identity, install },
                        None => AssetState::NotInstalled { identity },
                    }
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
    use crate::broker;
    use crate::config_dir::DirSource;
    use crate::diagnosis::{self, AssetMode, AssetState, SchemaState};
    use crate::error::CafleetError;
    use rusqlite::{Connection, params};
    use serde_json::json;
    use std::cell::RefCell;
    use std::path::Path;
    fn memory_head() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        conn
    }

    fn fixture_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(".step6-contract-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
    }

    fn path_for(base: &Path, variable: &str) -> String {
        base.join(variable).display().to_string()
    }

    mod schema {
        use super::*;

        #[test]
        fn schema_missing_and_unversioned_include_absent_and_empty_ledgers() {
            for empty_ledger in [false, true] {
                let conn = Connection::open_in_memory().unwrap();
                if empty_ledger {
                    conn.execute_batch("CREATE TABLE refinery_schema_history(version INTEGER)")
                        .unwrap();
                }
                assert!(matches!(
                    diagnosis::classify_schema(&conn, 8),
                    SchemaState::Missing
                ));
                // SQLite bookkeeping and TEMP state do not turn this into a foreign DB.
                conn.execute_batch(
                    "CREATE TEMP TABLE sentinel(x); INSERT INTO sentinel VALUES (1)",
                )
                .unwrap();
                assert!(matches!(
                    diagnosis::classify_schema(&conn, 8),
                    SchemaState::Missing
                ));
                conn.execute_batch(
                    "CREATE TABLE foreign_data(x INTEGER PRIMARY KEY AUTOINCREMENT)",
                )
                .unwrap();
                assert!(matches!(
                    diagnosis::classify_schema(&conn, 8),
                    SchemaState::Unversioned
                ));
                conn.execute_batch("DROP TABLE foreign_data").unwrap();
                assert!(
                    matches!(diagnosis::classify_schema(&conn, 8), SchemaState::Missing),
                    "sqlite_sequence alone is bookkeeping"
                );
            }
        }

        #[test]
        fn schema_classifies_recorded_maximum_as_behind_head_or_ahead() {
            for version in [7, 8, 9] {
                let conn = Connection::open_in_memory().unwrap();
                conn.execute_batch("CREATE TABLE refinery_schema_history(version INTEGER); INSERT INTO refinery_schema_history VALUES (1)").unwrap();
                conn.execute("INSERT INTO refinery_schema_history VALUES (?1)", [version])
                    .unwrap();
                let state = diagnosis::classify_schema(&conn, 8);
                match (version, state) {
                    (
                        7,
                        SchemaState::Behind {
                            recorded: 7,
                            head: 8,
                        },
                    )
                    | (8, SchemaState::Head { version: 8 })
                    | (
                        9,
                        SchemaState::Ahead {
                            recorded: 9,
                            head: 8,
                        },
                    ) => {}
                    other => panic!("wrong classification: {other:?}"),
                }
            }
        }

        #[test]
        fn schema_sql_failure_is_unreachable_with_the_original_cause() {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE refinery_schema_history(wrong_column INTEGER)")
                .unwrap();
            let state = diagnosis::classify_schema(&conn, 8);
            let SchemaState::Unreachable { cause } = &state else {
                panic!("{state:?}")
            };
            assert!(cause.message().contains("version"));
            let guard = crate::cli::helpers::schema_guard(&state).unwrap_err();
            assert_eq!(guard.message(), cause.message());
            assert_eq!(guard.exit_code(), cause.exit_code());
            let wire = crate::presentation::doctor_database(&state, 8);
            assert_eq!(
                wire,
                json!({"ok":false,"schema_version":null,"head_version":8,"error":cause.message()})
            );
        }

        #[test]
        fn shared_schema_facts_keep_distinct_guard_and_doctor_messages_and_wire_order() {
            let cases = [
                (
                    SchemaState::Missing,
                    "no cafleet database; run 'cafleet setup'",
                    "no database — run: cafleet setup",
                    None,
                ),
                (
                    SchemaState::Unversioned,
                    "database has tables but no schema history — not a cafleet database?",
                    "database has tables but no schema history — not a cafleet database?",
                    None,
                ),
                (
                    SchemaState::Behind {
                        recorded: 7,
                        head: 8,
                    },
                    "database schema is outdated (schema 7, head 8); run 'cafleet setup'",
                    "schema 7, head is 8 — run: cafleet setup",
                    Some(7),
                ),
                (
                    SchemaState::Ahead {
                        recorded: 9,
                        head: 8,
                    },
                    "database schema 9 is newer than this cafleet (head 8); upgrade cafleet",
                    "schema 9 is newer than this CLI (head 8) — upgrade cafleet",
                    Some(9),
                ),
            ];
            for (state, guard_detail, doctor_detail, recorded) in cases {
                assert_eq!(
                    crate::cli::helpers::schema_guard(&state)
                        .unwrap_err()
                        .message(),
                    guard_detail
                );
                assert_eq!(
                    crate::presentation::doctor_database_detail(&state),
                    doctor_detail
                );
                let expected = json!({"ok":false,"schema_version":recorded,"head_version":8,"error":doctor_detail});
                assert_eq!(
                    crate::output::format_json(&crate::presentation::doctor_database(&state, 8)),
                    crate::output::format_json(&expected)
                );
            }
            let head = SchemaState::Head { version: 8 };
            assert!(crate::cli::helpers::schema_guard(&head).is_ok());
            assert_eq!(
                crate::presentation::doctor_database_detail(&head),
                "schema 8 (head)"
            );
            assert_eq!(
                crate::output::format_json(&crate::presentation::doctor_database(&head, 8)),
                r#"{"ok":true,"schema_version":8,"head_version":8,"error":null}"#
            );
            let unreachable = SchemaState::Unreachable {
                cause: CafleetError::Usage("raw cause".into()),
            };
            assert!(
                matches!(crate::cli::helpers::schema_guard(&unreachable), Err(CafleetError::Usage(message)) if message == "raw cause")
            );
        }
    }

    mod assets {
        use super::*;

        #[test]
        fn assets_decode_current_stale_and_missing_with_sorted_superseded_records() {
            let dir = fixture_dir();
            let conn = memory_head();
            let lookup = |name: &str| Some(path_for(dir.path(), name));
            let claude = lookup("CLAUDE_CONFIG_DIR").unwrap();
            let codex = lookup("CODEX_HOME").unwrap();
            for (agent, path, version) in [
                ("codex", format!("{codex}/z-old"), "old"),
                ("claude", claude.clone(), "1.0.0"),
                ("codex", codex.clone(), "1.0"),
                ("claude", format!("{claude}/a-old"), "old"),
            ] {
                conn.execute("INSERT INTO asset_installs(coding_agent,cafleet_version,installed_at,path) VALUES (?1,?2,?3,?4)", params![agent,version,"raw-installed-at",path]).unwrap();
            }
            let report = diagnosis::diagnose_assets(
                Some(&conn),
                &lookup,
                dir.path(),
                "1.0.0",
                AssetMode::Report,
            )
            .unwrap();
            assert_eq!(
                report
                    .agents
                    .iter()
                    .map(|a| a.coding_agent)
                    .collect::<Vec<_>>(),
                vec!["claude", "codex", "opencode"]
            );
            assert!(
                matches!(&report.agents[0].state, AssetState::Current { identity, install }
            if identity.path == Path::new(&claude) && matches!(identity.source, DirSource::EnvVar("CLAUDE_CONFIG_DIR")) && install.coding_agent == "claude" && install.path == claude && install.cafleet_version == "1.0.0" && install.installed_at == "raw-installed-at")
            );
            assert!(
                matches!(&report.agents[1].state, AssetState::Stale { identity, install }
            if identity.path == Path::new(&codex) && install.cafleet_version == "1.0")
            );
            assert!(matches!(
                &report.agents[2].state,
                AssetState::NotInstalled { .. }
            ));
            assert_eq!(
                report
                    .superseded
                    .iter()
                    .map(|r| (r.coding_agent.clone(), r.path.clone()))
                    .collect::<Vec<_>>(),
                vec![
                    ("claude".to_string(), format!("{claude}/a-old")),
                    ("codex".to_string(), format!("{codex}/z-old"))
                ]
            );
            let guard = crate::cli::helpers::stale_assets_guard(&report, "1.0.0").unwrap_err();
            assert_eq!(
                guard.message(),
                "stale assets detected (codex=1.0; CLI 1.0.0); run 'cafleet setup' to reinstall"
            );
            let wire = crate::presentation::doctor_assets(&report, "1.0.0");
            assert_eq!(wire["agents"][0]["state"], "ok");
            assert_eq!(wire["agents"][1]["state"], "stale");
            assert_eq!(wire["agents"][2]["state"], "not_installed");
            assert_eq!(wire["agents"][0]["installed_at"], "raw-installed-at");
            assert_eq!(
                wire.as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                vec!["ok", "cli_version", "agents", "superseded"]
            );
            assert_eq!(
                wire["agents"][0]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                vec![
                    "coding_agent",
                    "path",
                    "source",
                    "recorded_version",
                    "installed_at",
                    "state",
                    "error"
                ]
            );
        }

        #[test]
        fn assets_no_connection_and_absent_table_are_not_installed_but_not_sql_errors() {
            let dir = fixture_dir();
            let conn = Connection::open_in_memory().unwrap();
            assert!(!diagnosis::asset_table_exists(&conn).unwrap());
            for connection in [None, Some(&conn)] {
                let report = diagnosis::diagnose_assets(
                    connection,
                    &|_| None,
                    dir.path(),
                    "1",
                    AssetMode::Report,
                )
                .unwrap();
                assert!(report.superseded.is_empty());
                assert!(report.agents.iter().all(|agent| matches!(&agent.state, AssetState::NotInstalled { identity } if matches!(identity.source, DirSource::Default))));
                assert_eq!(
                    crate::cli::helpers::stale_assets_guard(&report, "1")
                        .unwrap_err()
                        .message(),
                    "no assets install is recorded at the resolved paths; run 'cafleet setup' to install"
                );
                let wire = crate::presentation::doctor_assets(&report, "1");
                assert_eq!(wire["ok"], true);
                for agent in wire["agents"].as_array().unwrap() {
                    assert!(agent["recorded_version"].is_null());
                    assert!(agent["installed_at"].is_null());
                    assert!(agent["error"].is_null());
                }
            }
        }

        #[test]
        fn report_keeps_path_errors_and_continues_without_superseding_the_invalid_agent() {
            let dir = fixture_dir();
            let mut conn = memory_head();
            broker::record_asset_install(&mut conn, "claude", "/old/claude", "old").unwrap();
            let lookups = RefCell::new(Vec::new());
            let lookup = |name: &str| {
                lookups.borrow_mut().push(name.to_string());
                Some(if name == "CLAUDE_CONFIG_DIR" {
                    "relative".into()
                } else {
                    path_for(dir.path(), name)
                })
            };
            let report = diagnosis::diagnose_assets(
                Some(&conn),
                &lookup,
                dir.path(),
                "1",
                AssetMode::Report,
            )
            .unwrap();
            assert_eq!(
                *lookups.borrow(),
                vec!["CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_CONFIG_DIR"]
            );
            assert!(
                matches!(&report.agents[0].state, AssetState::PathError { variable: "CLAUDE_CONFIG_DIR", raw_value, cause }
            if raw_value == "relative" && cause.message() == "CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')")
            );
            assert!(matches!(
                &report.agents[1].state,
                AssetState::NotInstalled { .. }
            ));
            assert!(matches!(
                &report.agents[2].state,
                AssetState::NotInstalled { .. }
            ));
            assert!(report.superseded.is_empty());
            let wire = crate::presentation::doctor_assets(&report, "1");
            assert_eq!(wire["ok"], false);
            assert_eq!(
                wire["agents"][0],
                json!({"coding_agent":"claude","path":null,"source":"CLAUDE_CONFIG_DIR","recorded_version":null,"installed_at":null,"state":"error","error":"CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')"})
            );
        }

        #[test]
        fn guard_stops_at_first_path_error_before_attempting_malformed_install_sql() {
            let dir = fixture_dir();
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE asset_installs(wrong_column TEXT)")
                .unwrap();
            let calls = RefCell::new(Vec::new());
            let lookup = |name: &str| {
                calls.borrow_mut().push(name.to_string());
                Some("relative".into())
            };
            let error =
                diagnosis::diagnose_assets(Some(&conn), &lookup, dir.path(), "1", AssetMode::Guard)
                    .unwrap_err();
            assert_eq!(
                error.message(),
                "CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')"
            );
            assert_eq!(*calls.borrow(), vec!["CLAUDE_CONFIG_DIR"]);
        }

        #[test]
        fn malformed_install_table_is_an_error_in_both_modes_not_an_empty_report() {
            let dir = fixture_dir();
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE asset_installs(wrong_column TEXT)")
                .unwrap();
            assert!(diagnosis::asset_table_exists(&conn).unwrap());
            for mode in [AssetMode::Guard, AssetMode::Report] {
                let error =
                    diagnosis::diagnose_assets(Some(&conn), &|_| None, dir.path(), "1", mode)
                        .unwrap_err();
                assert!(error.message().contains("database"));
            }
        }
    }
}
