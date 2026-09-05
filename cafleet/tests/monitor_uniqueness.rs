//! Active-monitor uniqueness and data-preserving migration contracts (design 176, Step 2).
mod common;

use cafleet::{broker, db};
use common::{Cli, VERSION, code, stderr, stdout};
use rusqlite::{Connection, params, types::Value};

mod embedded {
    refinery::embed_migrations!("migrations");
}

fn old_schema(cli: &Cli, version: i32) -> Connection {
    let mut conn = db::connect(&cli.db_url()).unwrap();
    embedded::migrations::runner()
        .set_target(refinery::Target::Version(version))
        .run(&mut conn)
        .unwrap();
    conn
}

fn head_schema(cli: &Cli) -> Connection {
    let mut conn = db::connect(&cli.db_url()).unwrap();
    db::migrate_to_head(&mut conn).unwrap();
    conn
}

fn fleet(conn: &Connection, id: i64) {
    conn.execute(
        "INSERT INTO fleets(fleet_id, name, created_at) VALUES (?1, 'fixture', '2026-01-01T00:00:00Z')",
        [id],
    ).unwrap();
}

fn member(
    conn: &Connection,
    id: i64,
    fleet: i64,
    status: &str,
    card: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO members(member_id, fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, '', ?4, '2026-01-01T00:00:00Z', ?5)",
        params![id, fleet, format!("member-{id}"), status, card],
    )
}

const MONITOR: &str = r#"{"cafleet":{"kind":"monitor"},"skills":[]}"#;

fn placement() -> broker::NewPlacement {
    broker::NewPlacement {
        backend: "tmux".into(),
        mux_session: "main".into(),
        mux_window_id: "@1".into(),
        mux_pane_id: None,
        coding_agent: "claude".into(),
    }
}

fn rows(conn: &Connection, query: &str) -> Vec<Vec<Value>> {
    let mut statement = conn.prepare(query).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn records(conn: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "fleets",
        "members",
        "member_placements",
        "monitor_runtime",
        "messages",
        "sqlite_sequence",
    ]
    .map(|table| rows(conn, &format!("SELECT * FROM {table} ORDER BY 1")))
    .into()
}

fn schema_and_history(conn: &Connection) -> Vec<Vec<Vec<Value>>> {
    vec![
        rows(
            conn,
            "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name",
        ),
        rows(
            conn,
            "SELECT * FROM refinery_schema_history ORDER BY version",
        ),
    ]
}

fn assert_constraint(error: rusqlite::Error) {
    let rusqlite::Error::SqliteFailure(code, _) = error else {
        panic!("{error:?}");
    };
    assert_eq!(code.extended_code, rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE);
}

#[test]
fn unique_monitor_index_rejects_a_second_active_monitor_insert() {
    let cli = Cli::new();
    let conn = head_schema(&cli);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    let before = records(&conn);
    assert_constraint(member(&conn, 2, 1, "active", MONITOR).unwrap_err());
    assert_eq!(records(&conn), before);
}

#[test]
fn unique_monitor_index_rejects_reactivation_of_a_deregistered_monitor() {
    let cli = Cli::new();
    let conn = head_schema(&cli);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "deregistered", MONITOR).unwrap();
    let before = records(&conn);
    assert_constraint(
        conn.execute("UPDATE members SET status='active' WHERE member_id=2", [])
            .unwrap_err(),
    );
    assert_eq!(records(&conn), before);
}

#[test]
fn unique_monitor_index_rejects_card_conversion_into_a_second_monitor() {
    let cli = Cli::new();
    let conn = head_schema(&cli);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "active", "{}").unwrap();
    let before = records(&conn);
    assert_constraint(
        conn.execute(
            "UPDATE members SET member_card_json=?1 WHERE member_id=2",
            [MONITOR],
        )
        .unwrap_err(),
    );
    assert_eq!(records(&conn), before);
}

#[test]
fn unique_monitor_index_rejects_moving_a_monitor_into_an_occupied_fleet() {
    let cli = Cli::new();
    let conn = head_schema(&cli);
    fleet(&conn, 1);
    fleet(&conn, 2);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 2, "active", MONITOR).unwrap();
    let before = records(&conn);
    assert_constraint(
        conn.execute("UPDATE members SET fleet_id=1 WHERE member_id=2", [])
            .unwrap_err(),
    );
    assert_eq!(records(&conn), before);
}

#[test]
fn uniqueness_predicate_allows_ordinary_deregistered_and_other_fleet_members() {
    let cli = Cli::new();
    let mut conn = head_schema(&cli);
    fleet(&conn, 1);
    fleet(&conn, 2);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 2, "active", MONITOR).unwrap();
    for (id, status, card) in [
        (3, "active", "{}"),
        (4, "active", "{}"),
        (5, "deregistered", MONITOR),
        (6, "deregistered", MONITOR),
        (7, "active", r#"{"cafleet":{"kind":"Monitor"}}"#),
        (8, "active", r#"{"cafleet":{"kind":"monitoring-member"}}"#),
        (9, "active", r#"{"cafleet":{"kind":null}}"#),
    ] {
        member(&conn, id, 1, status, card).unwrap();
    }
    assert_eq!(broker::active_monitor_member_id(&conn, 1).unwrap(), Some(1));
    assert_eq!(broker::active_monitor_member_id(&conn, 2).unwrap(), Some(2));
    broker::deregister_member(&mut conn, 1).unwrap();
    assert_eq!(broker::active_monitor_member_id(&conn, 1).unwrap(), None);
    member(&conn, 10, 1, "active", MONITOR).unwrap();
    assert_eq!(
        broker::active_monitor_member_id(&conn, 1).unwrap(),
        Some(10)
    );
}

#[test]
fn stale_prechecks_on_two_connections_cannot_register_two_monitors() {
    let cli = Cli::new();
    let mut a = head_schema(&cli);
    let mut b = db::connect(&cli.db_url()).unwrap();
    fleet(&a, 1);
    assert_eq!(broker::active_monitor_member_id(&a, 1).unwrap(), None);
    assert_eq!(broker::active_monitor_member_id(&b, 1).unwrap(), None);
    let winner =
        broker::register_member(&mut a, 1, "winner", "", &[], Some(&placement()), true).unwrap();
    let id = winner["member_id"].as_i64().unwrap();
    let before = records(&a);
    let error =
        broker::register_member(&mut b, 1, "loser", "", &[], Some(&placement()), true).unwrap_err();
    assert_eq!(error.exit_code(), 1);
    assert_eq!(
        error.to_string(),
        format!("fleet 1 already has an active monitor member (member {id})")
    );
    assert_eq!(
        records(&a),
        before,
        "loser must not add a member or placement"
    );
    assert!(a.is_autocommit() && b.is_autocommit());
}

#[test]
fn an_unrelated_unique_constraint_is_not_reported_as_an_existing_monitor() {
    let cli = Cli::new();
    let mut conn = head_schema(&cli);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", "{}").unwrap();
    conn.execute_batch("CREATE UNIQUE INDEX fixture_unique_member_name ON members(name)")
        .unwrap();
    let before = records(&conn);
    let error =
        broker::register_member(&mut conn, 1, "member-1", "", &[], Some(&placement()), true)
            .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("UNIQUE constraint failed: members.name"),
        "{error}"
    );
    assert!(!error.to_string().contains("already has an active monitor"));
    assert_eq!(records(&conn), before);
}

#[test]
fn placement_failure_rolls_back_monitor_registration() {
    let cli = Cli::new();
    let mut conn = head_schema(&cli);
    fleet(&conn, 1);
    conn.execute_batch("CREATE TRIGGER fail_placement BEFORE INSERT ON member_placements BEGIN SELECT RAISE(ABORT, 'fixture placement failure'); END").unwrap();
    let before = records(&conn);
    let error = broker::register_member(&mut conn, 1, "monitor", "", &[], Some(&placement()), true)
        .unwrap_err();
    assert!(
        error.to_string().contains("fixture placement failure"),
        "{error}"
    );
    assert_eq!(records(&conn), before);
    assert_eq!(broker::active_monitor_member_id(&conn, 1).unwrap(), None);
}

#[test]
fn bootstrap_director_has_no_monitor_marker_and_director_display_still_wins() {
    let cli = Cli::new();
    let mut conn = head_schema(&cli);
    let created = broker::create_fleet(
        &mut conn,
        Some("fixture"),
        "main",
        "@1",
        "%0",
        "claude",
        "tmux",
        "monitor",
        "",
        |_, _, _| Ok("%1".into()),
    )
    .unwrap();
    let fleet = created["fleet_id"].as_i64().unwrap();
    let director = created["director"]["member_id"].as_i64().unwrap();
    let marker: Option<String> = conn.query_row("SELECT json_extract(member_card_json, '$.cafleet.kind') FROM members WHERE member_id=?1", [director], |r| r.get(0)).unwrap();
    assert_eq!(marker, None);
    let monitor = broker::active_monitor_member_id(&conn, fleet)
        .unwrap()
        .unwrap();
    broker::deregister_member(&mut conn, monitor).unwrap();
    conn.execute(
        "UPDATE members SET member_card_json=?1 WHERE member_id=?2",
        params![MONITOR, director],
    )
    .unwrap();
    assert_eq!(
        broker::get_member(&conn, director, fleet).unwrap().unwrap()["kind"],
        "director"
    );
}

#[test]
fn empty_database_migrates_to_v8_and_reapplying_changes_nothing() {
    let cli = Cli::new();
    let mut conn = db::connect(&cli.db_url()).unwrap();
    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 8);
    let before = schema_and_history(&conn);
    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 8);
    assert_eq!(schema_and_history(&conn), before);
}

#[test]
fn populated_v7_upgrade_preserves_records_and_adds_the_unique_index() {
    let cli = Cli::new();
    let mut conn = old_schema(&cli, 7);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "deregistered", MONITOR).unwrap();
    member(&conn, 3, 1, "active", "{}").unwrap();
    let before = records(&conn);
    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 8);
    assert_eq!(records(&conn), before);
    let index: (i64, i64) = conn.query_row("SELECT \"unique\", partial FROM pragma_index_list('members') WHERE name='idx_members_one_active_monitor_per_fleet'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(index, (1, 1));
    assert_constraint(member(&conn, 4, 1, "active", MONITOR).unwrap_err());
}

#[test]
fn duplicate_v7_migration_preserves_schema_history_and_all_records() {
    let cli = Cli::new();
    let mut conn = old_schema(&cli, 7);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "active", MONITOR).unwrap();
    let data = records(&conn);
    let schema = schema_and_history(&conn);
    assert!(db::migrate_to_head(&mut conn).is_err());
    assert_eq!(records(&conn), data);
    assert_eq!(schema_and_history(&conn), schema);
    assert!(conn.is_autocommit());
}

#[test]
fn failure_of_v8_rolls_back_all_pending_migrations_from_v5() {
    let cli = Cli::new();
    let mut conn = old_schema(&cli, 5);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "active", MONITOR).unwrap();
    conn.execute_batch("INSERT INTO asset_installs VALUES ('claude', 'old-version', 'old-time')")
        .unwrap();
    let data = records(&conn);
    let assets = rows(&conn, "SELECT * FROM asset_installs");
    let schema = schema_and_history(&conn);
    assert!(db::migrate_to_head(&mut conn).is_err());
    assert_eq!(schema_and_history(&conn), schema);
    assert_eq!(records(&conn), data);
    assert_eq!(rows(&conn, "SELECT * FROM asset_installs"), assets);
    assert!(conn.is_autocommit());
}

#[test]
fn unrelated_ddl_failure_preserves_original_cause_and_pending_schema() {
    let cli = Cli::new();
    let mut conn = old_schema(&cli, 5);
    conn.execute_batch("CREATE TABLE idx_members_one_active_monitor_per_fleet (fixture TEXT)")
        .unwrap();
    let schema = schema_and_history(&conn);
    let error = db::migrate_to_head(&mut conn).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("idx_members_one_active_monitor_per_fleet"),
        "{error}"
    );
    assert!(!error.to_string().contains("active monitor duplicates"));
    assert_eq!(schema_and_history(&conn), schema);
}

#[test]
fn duplicate_committed_after_clean_precheck_is_rejected_by_migration() {
    let cli = Cli::new();
    let mut a = old_schema(&cli, 7);
    let b = db::connect(&cli.db_url()).unwrap();
    fleet(&a, 1);
    member(&a, 1, 1, "active", MONITOR).unwrap();
    let duplicates = rows(
        &a,
        "SELECT fleet_id FROM members WHERE status='active' AND json_extract(member_card_json, '$.cafleet.kind')='monitor' GROUP BY fleet_id HAVING count(*)>1",
    );
    assert!(duplicates.is_empty());
    member(&b, 2, 1, "active", MONITOR).unwrap();
    let data = records(&a);
    let schema = schema_and_history(&a);
    assert!(db::migrate_to_head(&mut a).is_err());
    assert_eq!(records(&a), data);
    assert_eq!(schema_and_history(&a), schema);
}

#[test]
fn setup_sorts_duplicate_diagnostics_preserves_data_and_still_installs_assets() {
    let cli = Cli::new();
    let conn = old_schema(&cli, 7);
    for id in [20, 3] {
        fleet(&conn, id);
    }
    for (id, fleet) in [(90, 20), (8, 3), (30, 20), (2, 3)] {
        member(&conn, id, fleet, "active", MONITOR).unwrap();
    }
    member(&conn, 99, 3, "deregistered", MONITOR).unwrap();
    member(&conn, 100, 3, "active", "{}").unwrap();
    let data = records(&conn);
    let schema = schema_and_history(&conn);
    let output = cli.run(&["setup", "--coding-agent", "claude"]);
    let out = stdout(&output);
    assert_eq!(code(&output), 1, "{out}{}", stderr(&output));
    assert!(out.contains("active monitor duplicates prevent migration: fleet 3: members 2, 8; fleet 20: members 30, 90"), "{out}");
    assert_eq!(records(&conn), data);
    assert_eq!(schema_and_history(&conn), schema);
    assert!(
        cli.asset_rows()
            .iter()
            .any(|(agent, _, version)| agent == "claude" && version == VERSION)
    );
    assert!(
        cli.home
            .path()
            .join(".claude/skills/cafleet/SKILL.md")
            .is_file()
    );
    assert!(!cli.shim_log.exists(), "setup must not operate on panes");
}

#[test]
fn behind_schema_blocks_new_cli_delete_until_legacy_data_repair_and_setup_retry() {
    let cli = Cli::new();
    let mut conn = old_schema(&cli, 7);
    fleet(&conn, 1);
    member(&conn, 1, 1, "active", MONITOR).unwrap();
    member(&conn, 2, 1, "active", MONITOR).unwrap();
    let failed = cli.run(&["setup", "--coding-agent", "claude"]);
    assert_eq!(code(&failed), 1);
    let before = records(&conn);
    let deleted = cli.run(&["member", "delete", "2"]);
    assert_eq!(code(&deleted), 1);
    assert!(
        stderr(&deleted).contains("database schema is outdated (schema 7, head 8)"),
        "{}",
        stderr(&deleted)
    );
    assert_eq!(records(&conn), before);
    assert!(!cli.shim_log.exists());
    // Models the persisted result of the documented old-version cleanup, not an old binary run.
    broker::deregister_member(&mut conn, 2).unwrap();
    let repaired = records(&conn);
    let retried = cli.run(&["setup", "--coding-agent", "claude"]);
    assert_eq!(
        code(&retried),
        0,
        "{}{}",
        stdout(&retried),
        stderr(&retried)
    );
    assert!(stdout(&retried).contains("Upgraded from 7 to 8."));
    assert_eq!(records(&conn), repaired);
    assert_eq!(broker::active_monitor_member_id(&conn, 1).unwrap(), Some(1));
}

#[test]
fn duplicate_cli_guard_keeps_error_and_performs_no_registration_or_pane_creation() {
    let cli = Cli::new();
    let (fleet, _) = cli.with_fleet();
    let conn = cli.sqlite();
    let before = records(&conn);
    let log = std::fs::read_to_string(&cli.shim_log).unwrap();
    let output = cli.run(&[
        "member",
        "create",
        "--fleet-id",
        &fleet.to_string(),
        "--role",
        "monitor",
        "--name",
        "duplicate",
        "--description",
        "fixture",
        "follow the role",
    ]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains(&format!(
            "fleet {fleet} already has an active monitor member (member 2)"
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(records(&conn), before);
    let after = std::fs::read_to_string(&cli.shim_log).unwrap();
    assert_eq!(
        after.matches("split-window").count(),
        log.matches("split-window").count()
    );
    assert_eq!(
        after.matches("kill-pane").count(),
        log.matches("kill-pane").count()
    );
}
