//! Contracts for shared diagnosis and the real invocation/query observers.
//! These crate unit tests inspect actual connections, statements, and command
//! effects through the approved Step 6 APIs.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, params};
use serde_json::json;

use crate::broker::{self, test_support as common};
use crate::config::Settings;
use crate::config_dir::DirSource;
use crate::diagnosis::{self, AssetMode, AssetState, SchemaState};
use crate::error::CafleetError;

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

mod schema_contracts {
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
            conn.execute_batch("CREATE TEMP TABLE sentinel(x); INSERT INTO sentinel VALUES (1)")
                .unwrap();
            assert!(matches!(
                diagnosis::classify_schema(&conn, 8),
                SchemaState::Missing
            ));
            conn.execute_batch("CREATE TABLE foreign_data(x INTEGER PRIMARY KEY AUTOINCREMENT)")
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

mod asset_contracts {
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
        let report =
            diagnosis::diagnose_assets(Some(&conn), &lookup, dir.path(), "1", AssetMode::Report)
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
            let error = diagnosis::diagnose_assets(Some(&conn), &|_| None, dir.path(), "1", mode)
                .unwrap_err();
            assert!(error.message().contains("database"));
        }
    }
}

mod query_contracts {
    use super::*;

    #[test]
    fn names_observe_completed_bound_queries_at_unique_500_id_boundaries() {
        let mut conn = memory_head();
        let (fleet, _) = common::create_fleet(&mut conn, "names");
        for id in 10000..11001_i64 {
            conn.execute("INSERT INTO members(member_id,fleet_id,name,description,status,registered_at,member_card_json) VALUES (?1,?2,?3,'','deregistered','raw','{}')", params![id,fleet,format!("member-{id}")]).unwrap();
        }
        for count in [0_usize, 1, 500, 501, 1001] {
            let ids: Vec<i64> = (10000..10000 + count as i64).rev().collect();
            let repeated: Vec<i64> = ids.iter().copied().cycle().take(count * 4).collect();
            let queries = RefCell::new(Vec::new());
            let observe = |statement: &rusqlite::Statement<'_>| {
                queries.borrow_mut().push((
                    statement.parameter_count(),
                    statement.expanded_sql().unwrap(),
                ));
            };
            let names =
                broker::members::get_member_names_observed(&conn, &repeated, &observe).unwrap();
            let expected: BTreeMap<_, _> =
                ids.iter().map(|&id| (id, format!("member-{id}"))).collect();
            assert_eq!(names, expected);
            let queries = queries.borrow();
            assert_eq!(queries.len(), count.div_ceil(500));
            assert_eq!(queries.iter().map(|(count, _)| count).sum::<usize>(), count);
            assert!(queries.iter().all(|(count, sql)| *count > 0
                && *count <= 500
                && sql.to_ascii_lowercase().contains("select")
                && !sql.contains('?')));
            if count > 0 {
                assert!(queries.iter().any(|(_, sql)| sql.contains("10000")));
            }
            assert_eq!(broker::get_member_names(&conn, &repeated).unwrap(), names);
        }
    }

    #[test]
    fn names_empty_input_does_not_prepare_sql_and_failed_queries_do_not_emit_completion() {
        let conn = Connection::open_in_memory().unwrap(); // no members table
        let calls = Cell::new(0);
        let observe = |_: &rusqlite::Statement<'_>| calls.set(calls.get() + 1);
        assert!(
            broker::members::get_member_names_observed(&conn, &[], &observe)
                .unwrap()
                .is_empty()
        );
        assert_eq!(calls.get(), 0);
        assert!(broker::members::get_member_names_observed(&conn, &[1], &observe).is_err());
        assert_eq!(calls.get(), 0);
        let mut head = memory_head();
        common::create_fleet(&mut head, "unknown");
        assert!(
            broker::members::get_member_names_observed(&head, &[i64::MAX, i64::MAX], &observe)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            calls.get(),
            1,
            "a successfully executed zero-row query is observed"
        );
    }

    #[test]
    fn roster_is_lean_and_activity_query_alone_computes_the_three_aggregates() {
        let mut conn = memory_head();
        let (fleet, director) = common::create_fleet(&mut conn, "roster");
        let member = common::register(&mut conn, fleet, "worker", None);
        broker::send_message_record(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            200,
            director,
            &member.to_string(),
            "activity",
        )
        .unwrap();
        conn.execute("UPDATE messages SET created_at='2026-01-01T00:00:01Z',status_timestamp='2026-01-01T00:00:01Z'", []).unwrap();
        let queries = RefCell::new(Vec::new());
        let observe = |s: &rusqlite::Statement<'_>| {
            queries.borrow_mut().push(
                s.expanded_sql()
                    .unwrap()
                    .split_whitespace()
                    .collect::<String>()
                    .to_ascii_lowercase(),
            )
        };
        let roster =
            broker::members::list_roster_records_observed(&conn, fleet, true, &observe).unwrap();
        assert_eq!(
            roster,
            broker::list_roster_records(&conn, fleet, true).unwrap()
        );
        assert_eq!(queries.borrow().len(), 1);
        let sql = queries.borrow()[0].clone();
        assert!(
            !sql.contains("max("),
            "lean roster must not compute activity: {sql}"
        );
        assert!(sql.contains("exists("));
        assert!(sql.contains("owner_member_id"));
        assert!(
            !sql.contains("from_member_id"),
            "sender-only history must not enter roster"
        );
        queries.borrow_mut().clear();
        let now = crate::time::parse_lenient("2026-01-01T00:00:00Z").unwrap();
        let activities =
            broker::members::list_member_records_observed(&conn, fleet, now, &observe).unwrap();
        assert_eq!(queries.borrow().len(), 1);
        assert_eq!(queries.borrow()[0].matches("max(").count(), 3);
        assert_eq!(
            activities
                .iter()
                .find(|r| r.member.member_id == member)
                .unwrap()
                .idle,
            Some(0)
        );
        assert_eq!(
            activities
                .iter()
                .map(|r| r.member.clone())
                .collect::<Vec<_>>(),
            roster
        );
    }
}

mod invocation_contracts {
    use super::*;
    use crate::cli::{
        InvocationEvent, InvocationHooks, InvocationPhase, SchemaPoint, run_with_hooks,
    };

    fn settings(dir: &Path) -> Settings {
        let url = format!("sqlite:///{}", dir.join("database.db").display());
        Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone())).unwrap()
    }

    fn sentinel(conn: &Connection) {
        conn.execute_batch("CREATE TEMP TABLE invocation_sentinel(value TEXT); INSERT INTO invocation_sentinel VALUES ('same connection')").unwrap();
    }

    fn assert_sentinel(conn: &Connection) {
        assert_eq!(
            conn.query_row("SELECT value FROM temp.invocation_sentinel", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "same connection"
        );
    }

    fn prepare_ack(conn: &mut Connection, base: &Path) -> i64 {
        let (_fleet, director) = common::create_fleet(conn, "invocation");
        broker::record_asset_install(
            conn,
            "claude",
            &path_for(base, "CLAUDE_CONFIG_DIR"),
            crate::cli::VERSION,
        )
        .unwrap();
        broker::send_message_record(
            conn,
            &common::FakeNotifier::succeeding(),
            200,
            director,
            &director.to_string(),
            "ack this",
        )
        .unwrap()
        .message
        .message_id
    }

    fn status(conn: &Connection, id: i64) -> String {
        conn.query_row(
            "SELECT status_state FROM messages WHERE message_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn one_successful_open_flows_through_schema_assets_and_the_real_ack_body() {
        let dir = fixture_dir();
        let settings = settings(dir.path());
        let opens = Cell::new(0);
        let id = Cell::new(0_i64);
        let connect = |url: &str| {
            assert_eq!(url, settings.database_url);
            opens.set(opens.get() + 1);
            let mut conn = memory_head();
            id.set(prepare_ack(&mut conn, dir.path()));
            sentinel(&conn);
            Ok(conn)
        };
        // Every fresh DB deterministically allocates delivery 1; the observer
        // verifies the actual generated ID rather than assuming its value.
        let events = RefCell::new(Vec::new());
        let observe = |event: InvocationEvent<'_>| match event {
            InvocationEvent::SchemaInspected {
                point: SchemaPoint::Guard,
                conn,
                state,
            } => {
                assert_sentinel(conn);
                assert!(matches!(state, SchemaState::Head { .. }));
                assert_eq!(id.get(), 1);
                assert_eq!(status(conn, id.get()), "input_required");
                events.borrow_mut().push("schema");
            }
            InvocationEvent::AssetsInspected { conn, result } => {
                assert_sentinel(conn.unwrap());
                assert!(result.is_ok());
                assert_eq!(status(conn.unwrap(), id.get()), "input_required");
                events.borrow_mut().push("assets");
            }
            InvocationEvent::Finished {
                phase: InvocationPhase::CommandBody,
                conn,
                result,
            } => {
                let conn = conn.unwrap();
                assert_sentinel(conn);
                assert!(result.is_ok());
                assert_eq!(
                    status(conn, id.get()),
                    "completed",
                    "the real ACK must use the factory's in-memory DB"
                );
                events.borrow_mut().push("body");
            }
            _ => panic!("unexpected invocation event"),
        };
        let env = |name: &str| Some(path_for(dir.path(), name));
        run_with_hooks(
            &settings,
            &["cafleet", "message", "ack", "1", "--json"],
            &InvocationHooks {
                connect: &connect,
                observe: &observe,
                asset_env: &env,
            },
        )
        .unwrap();
        assert_eq!(opens.get(), 1);
        assert_eq!(*events.borrow(), vec!["schema", "assets", "body"]);
    }

    #[test]
    fn schema_and_asset_refusals_leave_real_message_unchanged_and_never_finish_body() {
        for outdated in [true, false] {
            let dir = fixture_dir();
            let settings = settings(dir.path());
            let mut seed = crate::db::connect(&settings.database_url).unwrap();
            crate::db::migrate_to_head(&mut seed).unwrap();
            let id = prepare_ack(&mut seed, dir.path());
            if outdated {
                seed.execute(
                    "DELETE FROM refinery_schema_history WHERE version=?1",
                    [crate::db::head_version()],
                )
                .unwrap();
            }
            drop(seed);
            let opens = Cell::new(0);
            let connect = |url: &str| {
                opens.set(opens.get() + 1);
                let conn = crate::db::connect(url)?;
                sentinel(&conn);
                Ok(conn)
            };
            let events = RefCell::new(Vec::new());
            let observe = |event: InvocationEvent<'_>| match event {
                InvocationEvent::SchemaInspected {
                    point: SchemaPoint::Guard,
                    conn,
                    state,
                } => {
                    assert_sentinel(conn);
                    assert_eq!(status(conn, id), "input_required");
                    if outdated {
                        assert!(matches!(state, SchemaState::Behind { .. }));
                    } else {
                        assert!(matches!(state, SchemaState::Head { .. }));
                    }
                    events.borrow_mut().push("schema");
                }
                InvocationEvent::AssetsInspected { conn, result } => {
                    assert_sentinel(conn.unwrap());
                    assert!(result.is_err());
                    events.borrow_mut().push("assets");
                }
                _ => panic!("a rejected guard must not run the command body"),
            };
            let path_calls = Cell::new(0);
            let env = |_: &str| {
                path_calls.set(path_calls.get() + 1);
                Some("relative".into())
            };
            let id_text = id.to_string();
            let error = run_with_hooks(
                &settings,
                &["cafleet", "message", "ack", &id_text],
                &InvocationHooks {
                    connect: &connect,
                    observe: &observe,
                    asset_env: &env,
                },
            )
            .unwrap_err();
            if outdated {
                assert!(error.message().contains("database schema is outdated"));
                assert_eq!(
                    path_calls.get(),
                    0,
                    "schema rejection must precede any path resolution"
                );
                assert_eq!(*events.borrow(), vec!["schema"]);
            } else {
                assert!(
                    error
                        .message()
                        .contains("CLAUDE_CONFIG_DIR must be an absolute path")
                );
                assert_eq!(*events.borrow(), vec!["schema", "assets"]);
            }
            assert_eq!(opens.get(), 1);
            let check = crate::db::connect(&settings.database_url).unwrap();
            assert_eq!(
                status(&check, id),
                "input_required",
                "guard refusal has no actual ACK side effect"
            );
        }
    }

    fn setup_observer<'a>(
        events: &'a RefCell<Vec<&'static str>>,
        base: &'a Path,
    ) -> impl for<'e> Fn(InvocationEvent<'e>) + 'a {
        move |event| match event {
            InvocationEvent::SchemaInspected { point, conn, state } => {
                assert_sentinel(conn);
                match point {
                    SchemaPoint::SetupBefore => {
                        assert!(matches!(
                            state,
                            SchemaState::Missing
                                | SchemaState::Head { .. }
                                | SchemaState::Ahead { .. }
                        ));
                        events.borrow_mut().push("before");
                    }
                    SchemaPoint::SetupAfter => {
                        assert!(
                            matches!(state, SchemaState::Head { version } if *version == crate::db::head_version())
                        );
                        assert_eq!(
                            conn.query_row(
                                "SELECT MAX(version) FROM refinery_schema_history",
                                [],
                                |r| r.get::<_, u32>(0)
                            )
                            .unwrap(),
                            crate::db::head_version()
                        );
                        events.borrow_mut().push("after");
                    }
                    _ => panic!("wrong schema point"),
                }
            }
            InvocationEvent::Finished {
                phase: InvocationPhase::SetupDatabase,
                conn,
                result,
            } => {
                if let Some(conn) = conn {
                    assert_sentinel(conn);
                }
                events
                    .borrow_mut()
                    .push(if result.is_ok() { "db ok" } else { "db error" });
            }
            InvocationEvent::Finished {
                phase: InvocationPhase::SetupAssets,
                conn,
                result,
            } => {
                assert!(result.is_ok(), "isolated claude installation must complete");
                let conn = conn.unwrap();
                assert_sentinel(conn);
                let rows = broker::list_asset_installs(conn).unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0]["coding_agent"], "claude");
                assert_eq!(rows[0]["path"], path_for(base, "CLAUDE_CONFIG_DIR"));
                assert_eq!(rows[0]["cafleet_version"], crate::cli::VERSION);
                assert!(
                    base.join("CLAUDE_CONFIG_DIR/skills/cafleet/SKILL.md")
                        .is_file()
                );
                events.borrow_mut().push("assets ok");
            }
            _ => panic!("setup must not eagerly diagnose every agent or run a command body"),
        }
    }

    #[test]
    fn setup_migration_reclassifies_same_connection_before_installing_only_selected_agent() {
        let dir = fixture_dir();
        let settings = settings(dir.path());
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            let conn = Connection::open_in_memory().unwrap();
            sentinel(&conn);
            Ok(conn)
        };
        let events = RefCell::new(Vec::new());
        let observe = setup_observer(&events, dir.path());
        let path_calls = RefCell::new(Vec::new());
        let env = |name: &str| {
            path_calls.borrow_mut().push(name.to_string());
            Some(if name == "CLAUDE_CONFIG_DIR" {
                path_for(dir.path(), name)
            } else {
                "relative-unselected".into()
            })
        };
        run_with_hooks(
            &settings,
            &["cafleet", "setup", "--coding-agent", "claude"],
            &InvocationHooks {
                connect: &connect,
                observe: &observe,
                asset_env: &env,
            },
        )
        .unwrap();
        assert_eq!(opens.get(), 1);
        assert_eq!(
            *events.borrow(),
            vec!["before", "after", "db ok", "assets ok"]
        );
        assert_eq!(*path_calls.borrow(), vec!["CLAUDE_CONFIG_DIR"]);
    }

    #[test]
    fn setup_head_noop_does_not_fabricate_a_post_migration_classification() {
        let dir = fixture_dir();
        let settings = settings(dir.path());
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            let conn = memory_head();
            sentinel(&conn);
            Ok(conn)
        };
        let events = RefCell::new(Vec::new());
        let observe = setup_observer(&events, dir.path());
        let env = |name: &str| {
            assert_eq!(name, "CLAUDE_CONFIG_DIR");
            Some(path_for(dir.path(), name))
        };
        run_with_hooks(
            &settings,
            &["cafleet", "setup", "--coding-agent", "claude"],
            &InvocationHooks {
                connect: &connect,
                observe: &observe,
                asset_env: &env,
            },
        )
        .unwrap();
        assert_eq!(opens.get(), 1);
        assert_eq!(*events.borrow(), vec!["before", "db ok", "assets ok"]);
    }

    #[test]
    fn setup_database_refusal_still_installs_assets_using_the_successfully_opened_connection() {
        let dir = fixture_dir();
        let settings = settings(dir.path());
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            let conn = memory_head();
            conn.execute(
                "UPDATE refinery_schema_history SET version=?1 WHERE version=?2",
                params![crate::db::head_version() + 1, crate::db::head_version()],
            )
            .unwrap();
            sentinel(&conn);
            Ok(conn)
        };
        let events = RefCell::new(Vec::new());
        let observe = setup_observer(&events, dir.path());
        let env = |name: &str| {
            assert_eq!(name, "CLAUDE_CONFIG_DIR");
            Some(path_for(dir.path(), name))
        };
        let error = run_with_hooks(
            &settings,
            &["cafleet", "setup", "--coding-agent", "claude"],
            &InvocationHooks {
                connect: &connect,
                observe: &observe,
                asset_env: &env,
            },
        )
        .unwrap_err();
        assert_eq!(error.message(), "db half failed");
        assert_eq!(opens.get(), 1);
        assert_eq!(*events.borrow(), vec!["before", "db error", "assets ok"]);
    }

    #[test]
    fn setup_retries_a_failed_initial_open_for_the_assets_half_only() {
        let dir = fixture_dir();
        let settings = settings(dir.path());
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            if opens.get() == 1 {
                return Err(CafleetError::App("first open failed".into()));
            }
            let conn = memory_head();
            sentinel(&conn);
            Ok(conn)
        };
        let events = RefCell::new(Vec::new());
        let observe = setup_observer(&events, dir.path());
        let env = |name: &str| {
            assert_eq!(name, "CLAUDE_CONFIG_DIR");
            Some(path_for(dir.path(), name))
        };
        let error = run_with_hooks(
            &settings,
            &["cafleet", "setup", "--coding-agent", "claude"],
            &InvocationHooks {
                connect: &connect,
                observe: &observe,
                asset_env: &env,
            },
        )
        .unwrap_err();
        assert_eq!(error.message(), "db half failed");
        assert_eq!(opens.get(), 2);
        assert_eq!(*events.borrow(), vec!["db error", "assets ok"]);
    }
}

// Step 10 compatibility checks use only the existing public installer adapter.
mod step10_existing_installer {
    use super::*;
    use crate::assets::{agent_paths, install_agent};
    use crate::embedded::{PRESETS, SKILLS, lookup};

    #[test]
    fn same_version_reinstall_replaces_different_bytes_and_preserves_unrelated_siblings() {
        let dir = fixture_dir();
        let base = dir.path().join("codex");
        let lookup_env = |_: &str| Some(base.display().to_string());
        let paths = agent_paths(&lookup_env, dir.path(), "codex").unwrap();
        let mut conn = Connection::open(dir.path().join("assets.sqlite3")).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        install_agent(&mut conn, "codex", &paths, "same").unwrap();
        for skill in ["cafleet", "cafleet-design-doc"] {
            std::fs::write(
                base.join("skills").join(skill).join("SKILL.md"),
                b"old bytes, same version",
            )
            .unwrap();
            std::fs::write(
                base.join("skills").join(skill).join("obsolete.txt"),
                b"old extra",
            )
            .unwrap();
        }
        std::fs::write(base.join("rules/cafleet.rules"), b"old preset").unwrap();
        std::fs::create_dir_all(base.join("skills/unrelated")).unwrap();
        std::fs::write(base.join("skills/unrelated/keep"), b"keep").unwrap();
        std::fs::write(base.join("rules/unrelated.rules"), b"keep rule").unwrap();
        install_agent(&mut conn, "codex", &paths, "same").unwrap();
        for (relative, bytes) in SKILLS {
            assert_eq!(
                std::fs::read(base.join("skills").join(relative)).unwrap(),
                *bytes,
                "{relative}"
            );
        }
        for skill in ["cafleet", "cafleet-design-doc"] {
            assert!(
                !base
                    .join("skills")
                    .join(skill)
                    .join("obsolete.txt")
                    .exists()
            );
        }
        assert_eq!(
            std::fs::read(base.join("rules/cafleet.rules")).unwrap(),
            lookup(PRESETS, "codex/cafleet.rules").unwrap()
        );
        assert_eq!(
            std::fs::read(base.join("skills/unrelated/keep")).unwrap(),
            b"keep"
        );
        assert_eq!(
            std::fs::read(base.join("rules/unrelated.rules")).unwrap(),
            b"keep rule"
        );
        assert_eq!(broker::list_asset_installs(&conn).unwrap().len(), 1);
    }

    #[test]
    fn obsolete_research_symlink_removal_does_not_follow_its_external_target() {
        let dir = fixture_dir();
        let base = dir.path().join("claude");
        let paths = agent_paths(
            &|_: &str| Some(base.display().to_string()),
            dir.path(),
            "claude",
        )
        .unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"untouched").unwrap();
        std::fs::create_dir_all(base.join("skills")).unwrap();
        let research = base.join("skills/cafleet-research");
        std::os::unix::fs::symlink(&outside, &research).unwrap();
        let mut conn = Connection::open(dir.path().join("assets.sqlite3")).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        install_agent(&mut conn, "claude", &paths, "new").unwrap();
        assert!(std::fs::symlink_metadata(&research).is_err());
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"untouched");
    }

    #[test]
    fn later_backend_failure_preserves_earlier_backend_bytes_and_exact_record() {
        let dir = fixture_dir();
        let mut conn = Connection::open(dir.path().join("assets.sqlite3")).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let claude = agent_paths(&|_| None, dir.path(), "claude").unwrap();
        install_agent(&mut conn, "claude", &claude, "new").unwrap();
        let rows = broker::list_asset_installs(&conn).unwrap();
        let before = std::fs::read(dir.path().join(".claude/skills/cafleet/SKILL.md")).unwrap();
        let codex = agent_paths(&|_| None, dir.path(), "codex").unwrap();
        std::fs::create_dir_all(dir.path().join(".codex")).unwrap();
        std::fs::write(dir.path().join(".codex/skills"), b"not a directory").unwrap();
        assert!(install_agent(&mut conn, "codex", &codex, "new").is_err());
        assert_eq!(broker::list_asset_installs(&conn).unwrap(), rows);
        assert_eq!(
            std::fs::read(dir.path().join(".claude/skills/cafleet/SKILL.md")).unwrap(),
            before
        );
    }
}
