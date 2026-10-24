mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use cafleet::broker::records::{MemberStatus, MessageStatus};
use cafleet::error::CafleetError;
use common::{Cli, VERSION, code, stderr, stdout};

const APPLICATION_ID: i64 = 1128351288;

fn format_error(path: &Path) -> String {
    format!(
        "database format is incompatible at '{}'; choose an empty database path and run 'cafleet setup'.",
        path.display()
    )
}

fn asset_snapshot(cli: &Cli) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                assert!(files.insert(name, std::fs::read(path).unwrap()).is_none());
            }
        }
    }
    let mut files = BTreeMap::new();
    for relative in [".claude", ".codex", ".opencode", ".config/opencode"] {
        let path = cli.home.path().join(relative);
        if path.exists() {
            visit(cli.home.path(), &path, &mut files);
        }
    }
    files
}

fn asset_sentinels(cli: &Cli) {
    for relative in [".claude", ".codex", ".opencode", ".config/opencode"] {
        let path = cli.home.path().join(relative);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("operator-file"), relative).unwrap();
    }
}

fn assert_format_rejected(cli: &Cli, path: &Path, args: &[&str]) {
    let database = std::fs::read(path).unwrap();
    let assets = asset_snapshot(cli);
    let output = cli.run(args);
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert_eq!(code(&output), 1, "{args:?}: {combined}");
    assert!(
        combined.contains(&format_error(path)),
        "{args:?}: {combined}"
    );
    assert!(
        std::fs::read(path).unwrap() == database,
        "{args:?}: database bytes changed"
    );
    assert!(
        asset_snapshot(cli) == assets,
        "{args:?}: installed assets changed"
    );
}

fn current_fixture(cli: &Cli) {
    cli.migrate();
    cli.sqlite()
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    cli.seed_asset_row("claude", VERSION);
    asset_sentinels(cli);
}

#[test]
fn setup_initializes_an_empty_sqlite_file_and_is_idempotent() {
    let cli = Cli::new();
    drop(cli.sqlite());
    let output = cli.run(&["setup"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let conn = cli.sqlite();
    assert_eq!(
        conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        APPLICATION_ID
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*), MAX(version) FROM refinery_schema_history",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        )
        .unwrap(),
        (
            i64::from(cafleet::db::head_version()),
            i64::from(cafleet::db::head_version())
        )
    );
    drop(conn);
    let tables = cli.asset_rows();
    let assets = asset_snapshot(&cli);
    let again = cli.run(&["setup"]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(cli.asset_rows(), tables);
    assert!(
        asset_snapshot(&cli) == assets,
        "idempotent setup preserves installed asset bytes"
    );
}

#[test]
fn default_setup_uses_the_current_file_and_preserves_the_predecessor_and_sidecars() {
    let cli = Cli::new();
    let directory = cli.home.path().join(".local/share/cafleet");
    std::fs::create_dir_all(&directory).unwrap();
    let predecessor = directory.join("cafleet_v7.db");
    let conn = rusqlite::Connection::open(&predecessor).unwrap();
    conn.execute_batch(
        "CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('history');",
    )
    .unwrap();
    drop(conn);
    let mut before = BTreeMap::new();
    before.insert(predecessor.clone(), std::fs::read(&predecessor).unwrap());
    for suffix in ["-wal", "-shm", "-journal"] {
        let path = directory.join(format!("cafleet_v7.db{suffix}"));
        let bytes = format!("preserve {suffix}").into_bytes();
        std::fs::write(&path, &bytes).unwrap();
        before.insert(path, bytes);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_cafleet"))
        .args(["setup", "--coding-agent", "codex"])
        .env_clear()
        .env("HOME", cli.home.path())
        .env("PATH", &cli.shim_dir)
        .output()
        .unwrap();
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    let current = rusqlite::Connection::open(directory.join("cafleet_v8.db")).unwrap();
    assert_eq!(
        current
            .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        APPLICATION_ID
    );
    for (path, bytes) in before {
        assert!(
            std::fs::read(&path).unwrap() == bytes,
            "{} changed",
            path.display()
        );
    }
}

#[test]
fn explicit_empty_target_is_initialized_at_its_selected_path() {
    let mut cli = Cli::new();
    let path = cli.home.path().join("selected/nested/current.db");
    cli.set_env(
        "CAFLEET_DATABASE_URL",
        &format!("sqlite:///{}", path.display()),
    );
    let output = cli.run(&["setup", "--coding-agent", "codex"]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    let conn = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        APPLICATION_ID
    );
    assert!(!cli.db_path().exists());
}

#[test]
fn incompatible_formats_preserve_database_and_assets_for_both_setup_paths() {
    for sql in [
        "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('keep');",
        "PRAGMA application_id=42;",
        "PRAGMA application_id=42; CREATE TABLE foreign_data(value TEXT);",
        "PRAGMA application_id=1128351288;",
        "CREATE TABLE refinery_schema_history(version INTEGER);",
        "PRAGMA application_id=1128351288; CREATE TABLE refinery_schema_history(version INTEGER); INSERT INTO refinery_schema_history VALUES(1);",
    ] {
        let cli = Cli::new();
        cli.sqlite().execute_batch(sql).unwrap();
        asset_sentinels(&cli);
        assert_format_rejected(&cli, &cli.db_path(), &["setup"]);
        assert_format_rejected(&cli, &cli.db_path(), &["setup", "--coding-agent", "codex"]);
    }
}

#[test]
fn identity_is_checked_before_an_otherwise_valid_ledger() {
    for identity in [0, 42] {
        let cli = Cli::new();
        current_fixture(&cli);
        let conn = cli.sqlite();
        conn.pragma_update(None, "application_id", identity)
            .unwrap();
        conn.execute("UPDATE refinery_schema_history SET version=99 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)", [])
            .unwrap();
        drop(conn);
        assert_format_rejected(&cli, &cli.db_path(), &["setup"]);
        assert_format_rejected(&cli, &cli.db_path(), &["fleet", "list"]);
    }
}

#[test]
fn malformed_current_shapes_gate_setup_and_normal_writes() {
    for mutation in [
        "DROP TABLE asset_installs;",
        "DROP TABLE messages;",
        "ALTER TABLE members RENAME COLUMN name TO missing_name;",
        "ALTER TABLE member_placements RENAME COLUMN coding_agent TO missing_agent;",
        "ALTER TABLE members ADD COLUMN unexpected TEXT;",
        "ALTER TABLE refinery_schema_history RENAME COLUMN version TO missing_version;",
        "DELETE FROM refinery_schema_history;",
        "DROP TABLE messages; CREATE TABLE messages(message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, from_member_id TEXT NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT, to_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT, created_at TEXT NOT NULL, status_state TEXT NOT NULL, status_timestamp TEXT NOT NULL, text TEXT NOT NULL, notified_at TEXT);",
        "DROP TABLE messages; CREATE TABLE messages(message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, from_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT, to_member_id INTEGER REFERENCES members(member_id) ON DELETE RESTRICT, created_at TEXT NOT NULL, status_state TEXT NOT NULL, status_timestamp TEXT NOT NULL, text TEXT NOT NULL, notified_at TEXT);",
    ] {
        let cli = Cli::new();
        current_fixture(&cli);
        cli.sqlite().execute_batch(mutation).unwrap();
        assert_format_rejected(&cli, &cli.db_path(), &["setup"]);
        for args in [
            ["fleet", "delete", "1"].as_slice(),
            ["member", "delete", "1"].as_slice(),
            ["message", "ack", "1"].as_slice(),
        ] {
            assert_format_rejected(&cli, &cli.db_path(), args);
        }
        assert!(
            cli.shim_calls().is_empty(),
            "format rejection precedes pane operations"
        );
    }
}

#[test]
fn compatible_current_format_accepts_auxiliary_tables_indexes_and_triggers() {
    let cli = Cli::new();
    current_fixture(&cli);
    cli.sqlite()
        .execute_batch(
            "CREATE TABLE auxiliary_audit(event TEXT);
         CREATE INDEX auxiliary_event ON auxiliary_audit(event);
         CREATE TRIGGER auxiliary_insert AFTER INSERT ON auxiliary_audit BEGIN SELECT 1; END;
         INSERT INTO auxiliary_audit VALUES('keep');",
        )
        .unwrap();
    for args in [
        ["fleet", "list"].as_slice(),
        ["setup"].as_slice(),
        ["doctor", "--json"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(
            code(&output),
            0,
            "{args:?}: {}{}",
            stdout(&output),
            stderr(&output)
        );
    }
    assert_eq!(
        cli.sqlite()
            .query_row("SELECT event FROM auxiliary_audit", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn explicit_incompatible_target_is_preserved_without_using_another_database() {
    let mut cli = Cli::new();
    let path = cli.home.path().join("custom.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE operator_data(value TEXT); INSERT INTO operator_data VALUES('keep');",
    )
    .unwrap();
    drop(conn);
    cli.set_env(
        "CAFLEET_DATABASE_URL",
        &format!("sqlite:///{}", path.display()),
    );
    assert_format_rejected(&cli, &path, &["setup"]);
    assert_format_rejected(&cli, &path, &["fleet", "list"]);
    assert!(!cli.db_path().exists());
    assert!(
        !cli.home
            .path()
            .join(".local/share/cafleet/cafleet_v8.db")
            .exists()
    );
}

#[test]
fn doctor_reports_incompatible_format_and_finishes_its_other_read_only_sections() {
    let cli = Cli::new();
    cli.sqlite()
        .execute_batch("CREATE TABLE operator_data(value TEXT);")
        .unwrap();
    let before = std::fs::read(cli.db_path()).unwrap();
    let output = cli.run(&["doctor", "--json"]);
    assert_eq!(code(&output), 1);
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["database"]["ok"], false);
    assert_eq!(payload["database"]["error"], format_error(&cli.db_path()));
    assert!(payload["multiplexer"].is_object());
    assert!(payload["coding_agents"]["agents"].is_array());
    assert!(payload["member_permissions"].is_object());
    assert!(
        std::fs::read(cli.db_path()).unwrap() == before,
        "doctor changed database bytes"
    );
    assert!(asset_snapshot(&cli).is_empty());
}

#[test]
fn doctor_and_invocation_agree_on_malformed_current_format() {
    let cli = Cli::new();
    current_fixture(&cli);
    cli.sqlite()
        .execute_batch("DROP TABLE asset_installs;")
        .unwrap();
    assert_format_rejected(&cli, &cli.db_path(), &["doctor", "--json"]);
    assert_format_rejected(&cli, &cli.db_path(), &["fleet", "list"]);
}

#[test]
fn ahead_current_schema_preserves_both_setup_write_paths() {
    let cli = Cli::new();
    current_fixture(&cli);
    cli.sqlite()
        .execute("UPDATE refinery_schema_history SET version=version+1 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)", [])
        .unwrap();
    let database = std::fs::read(cli.db_path()).unwrap();
    let assets = asset_snapshot(&cli);
    let records = cli.asset_rows();
    for args in [
        ["setup"].as_slice(),
        ["setup", "--coding-agent", "codex"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 1);
        let combined = format!("{}{}", stdout(&output), stderr(&output));
        assert!(
            combined.contains("Refusing to downgrade automatically."),
            "{combined}"
        );
        assert!(
            std::fs::read(cli.db_path()).unwrap() == database,
            "ahead schema changed database bytes"
        );
        assert!(
            asset_snapshot(&cli) == assets,
            "ahead schema changed asset bytes"
        );
        assert_eq!(cli.asset_rows(), records);
    }
}

#[test]
fn corrupt_database_bytes_preserve_the_actual_sqlite_cause() {
    let cli = Cli::new();
    let bytes = b"operator data that is not a SQLite database";
    std::fs::write(cli.db_path(), bytes).unwrap();
    for args in [
        ["setup"].as_slice(),
        ["fleet", "list"].as_slice(),
        ["doctor", "--json"].as_slice(),
    ] {
        let output = cli.run(args);
        assert_eq!(code(&output), 1);
        let combined = format!("{}{}", stdout(&output), stderr(&output));
        assert!(combined.contains("file is not a database"), "{combined}");
        assert!(
            std::fs::read(cli.db_path()).unwrap() == bytes,
            "corrupt file bytes changed"
        );
        assert!(asset_snapshot(&cli).is_empty());
    }
}

fn seed_live_registry(cli: &Cli) {
    cli.ready();
    cli.sqlite().execute_batch(
        "INSERT INTO fleets(fleet_id, name, created_at) VALUES(1, 'test', '2026-10-23T00:00:00Z');
         INSERT INTO members(member_id, fleet_id, name, description, status, registered_at)
         VALUES(1, 1, 'Director', 'test', 'active', '2026-10-23T00:00:00Z');
         INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
         VALUES(1, 'main', '@1', '%0', 'tmux', 'claude', '2026-10-23T00:00:00Z');
         UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;"
    ).unwrap();
}

#[test]
fn live_fleet_reads_fail_at_missing_director_or_placement_state() {
    for mutation in [
        "UPDATE fleets SET director_member_id=NULL WHERE fleet_id=1;",
        "UPDATE members SET status='deregistered' WHERE member_id=1;",
        "DELETE FROM member_placements WHERE member_id=1;",
        "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=1;",
    ] {
        let cli = Cli::new();
        seed_live_registry(&cli);
        cli.sqlite().execute_batch(mutation).unwrap();
        let output = cli.run(&["member", "list", "1", "--json"]);
        assert_eq!(
            code(&output),
            1,
            "{mutation}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        assert!(!stderr(&output).trim().is_empty());
    }
}

#[test]
fn a_pending_ordinary_placement_remains_a_valid_diagnostic_state() {
    let cli = Cli::new();
    seed_live_registry(&cli);
    cli.sqlite().execute_batch(
        "INSERT INTO members(member_id, fleet_id, name, description, status, registered_at)
         VALUES(2, 1, 'pending', 'test', 'active', '2026-10-23T00:00:00Z');
         INSERT INTO member_placements(member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
         VALUES(2, 'main', '@1', NULL, 'tmux', 'codex', '2026-10-23T00:00:00Z');"
    ).unwrap();
    let output = cli.run(&["member", "show", "2", "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["member_id"], 2);
    let placement = payload
        .get("placement")
        .expect("pending member retains its placement");
    assert!(placement.is_object());
    assert_eq!(
        placement.get("mux_pane_id").expect("pane key is present"),
        &serde_json::Value::Null
    );
}

#[test]
fn member_output_uses_registration_fields_and_the_director_back_reference() {
    let cli = Cli::new();
    seed_live_registry(&cli);
    let output = cli.run(&["member", "show", "1", "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let member: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        member
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "member_id",
            "name",
            "description",
            "status",
            "registered_at",
            "kind",
            "placement",
            "unconfirmed_previews",
        ]
    );
    assert_eq!(member["name"], "Director");
    assert_eq!(member["description"], "test");
    assert_eq!(member["kind"], "director");
    assert_eq!(member["placement"]["coding_agent"], "claude");
}

#[test]
fn message_output_uses_required_recipient_and_exact_stored_text() {
    let cli = Cli::new();
    seed_live_registry(&cli);
    let text = "stored\ntext\twith controls";
    cli.sqlite().execute(
        "INSERT INTO messages(message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text)
         VALUES(1, 1, 1, '2026-10-23T00:00:00Z', 'input_required', '2026-10-23T00:00:00Z', ?1)",
        [text],
    ).unwrap();
    let output = cli.run(&["message", "show", "1", "--json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let message = payload
        .get("message")
        .expect("show returns a message envelope");
    assert_eq!(
        message
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "message_id",
            "from_member_id",
            "to_member_id",
            "created_at",
            "status_state",
            "status_timestamp",
            "text",
        ]
    );
    assert_eq!(message["to_member_id"], 1);
    assert_eq!(message["text"], text);
}

#[test]
fn stored_status_decoders_return_typed_errors_for_corrupt_values() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for (field, sql) in [
        ("members.status", "SELECT 'corrupt-member'"),
        ("messages.status_state", "SELECT 'corrupt-message'"),
    ] {
        let error = if field == "members.status" {
            conn.query_row(sql, [], |row| row.get::<_, MemberStatus>(0))
                .unwrap_err()
        } else {
            conn.query_row(sql, [], |row| row.get::<_, MessageStatus>(0))
                .unwrap_err()
        };
        let rusqlite::Error::FromSqlConversionFailure(_, _, cause) = error else {
            panic!("invalid stored enum must preserve its typed conversion cause")
        };
        assert!(matches!(cause.downcast_ref::<CafleetError>(),
            Some(CafleetError::InvalidStoredValue { field: actual, value })
            if actual == field && value.starts_with("corrupt-")));
    }
}
