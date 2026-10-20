//! Active-monitor uniqueness contracts.
mod common;

use cafleet::{broker, db};
use common::Cli;
use rusqlite::{Connection, params, types::Value};

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
    let winner = broker::register_member(&mut a, 1, "winner", "", &[], Some(&placement()), true)
        .map(|record| cafleet::presentation::registered_member(&record))
        .unwrap();
    let id = winner["member_id"].as_i64().unwrap();
    let before = records(&a);
    let error = broker::register_member(&mut b, 1, "loser", "", &[], Some(&placement()), true)
        .map(|record| cafleet::presentation::registered_member(&record))
        .unwrap_err();
    assert!(matches!(
        &error,
        cafleet::error::CafleetError::ActiveMonitorExists { fleet_id: 1, member_id }
            if *member_id == id
    ));
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
            .map(|record| cafleet::presentation::registered_member(&record))
            .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("UNIQUE constraint failed: members.name"),
        "{error}"
    );
    assert!(!matches!(
        error,
        cafleet::error::CafleetError::ActiveMonitorExists { .. }
    ));
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
        .map(|record| cafleet::presentation::registered_member(&record))
        .unwrap_err();
    assert!(
        error.to_string().contains("fixture placement failure"),
        "{error}"
    );
    assert_eq!(records(&conn), before);
    assert_eq!(broker::active_monitor_member_id(&conn, 1).unwrap(), None);
}
