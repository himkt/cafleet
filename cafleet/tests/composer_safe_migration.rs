use std::collections::BTreeMap;

use cafleet::{broker, db};
use rusqlite::{Connection, types::Value};

mod embedded {
    refinery::embed_migrations!("migrations");
}

const PLACEMENT_COLUMNS: [&str; 9] = [
    "member_id",
    "mux_session",
    "mux_window_id",
    "mux_pane_id",
    "backend",
    "coding_agent",
    "created_at",
    "keystroke_at",
    "silence_notice_at",
];

fn rows(conn: &Connection, query: &str) -> Vec<Vec<Value>> {
    let mut statement = conn.prepare(query).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| (0..width).map(|column| row.get(column)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn records(conn: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name != 'refinery_schema_history' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables
        .into_iter()
        .map(|table| {
            let columns = if table == "member_placements" {
                PLACEMENT_COLUMNS.join(",")
            } else {
                "*".to_string()
            };
            let values = rows(
                conn,
                &format!("SELECT {columns} FROM \"{table}\" ORDER BY 1"),
            );
            (table, values)
        })
        .collect()
}

fn ledger(conn: &Connection) -> Vec<Vec<Value>> {
    rows(
        conn,
        "SELECT * FROM refinery_schema_history ORDER BY version",
    )
}

fn assert_current_schema(conn: &Connection) {
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('member_placements') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, PLACEMENT_COLUMNS);
    assert_eq!(
        rows(conn, "PRAGMA integrity_check"),
        vec![vec![Value::Text("ok".into())]]
    );
    assert!(rows(conn, "PRAGMA foreign_key_check").is_empty());
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

fn connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    conn
}

#[test]
fn fresh_setup_provides_the_current_placement_schema_and_idempotent_ledger() {
    let mut conn = connection();
    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 10);
    assert_current_schema(&conn);
    let data = records(&conn);
    let history = ledger(&conn);
    assert_eq!(history.len(), 10);
    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 10);
    assert_eq!(records(&conn), data);
    assert_eq!(ledger(&conn), history);
    assert_current_schema(&conn);
}

#[test]
fn populated_v9_upgrade_preserves_retained_records_claims_and_migration_history() {
    let mut conn = connection();
    embedded::migrations::runner()
        .set_target(refinery::Target::Version(9))
        .run(&mut conn)
        .unwrap();
    conn.execute_batch(
        r#"
        INSERT INTO fleets(fleet_id,name,created_at,deleted_at) VALUES
            (1,'active fleet','2026-01-01T00:00:00Z',NULL),
            (2,'closed fleet','2025-01-01T00:00:00Z','2026-01-01T00:00:00Z');
        INSERT INTO members(member_id,fleet_id,name,description,status,registered_at,deregistered_at,member_card_json) VALUES
            (1,1,'Director','coordinates','active','2026-01-01T00:00:00Z',NULL,'{"cafleet":{"kind":"director"},"skills":[]}'),
            (2,1,'worker','works','active','2026-01-02T00:00:00Z',NULL,'{"skills":["fixture"]}'),
            (3,1,'monitor','observes','active','2026-01-03T00:00:00Z',NULL,'{"cafleet":{"kind":"monitor"},"skills":[]}'),
            (4,2,'former worker','finished','deregistered','2025-01-02T00:00:00Z','2026-01-01T00:00:00Z','{}');
        UPDATE fleets SET director_member_id=1 WHERE fleet_id=1;
        INSERT INTO member_placements(member_id,mux_session,mux_window_id,mux_pane_id,backend,coding_agent,created_at,keystroke_at,forced_at,silence_notice_at) VALUES
            (1,'main','@1','%0','tmux','claude','2026-01-01T00:00:01Z','2026-10-17T00:00:01.123456+00:00','2026-01-01T00:00:02Z','2026-01-01T00:00:03Z'),
            (2,'herdr-session','w1:t2','w1:p2','herdr','codex','2026-01-02T00:00:01Z','2026-10-17T00:00:02.654321+00:00',NULL,NULL),
            (3,'main','@1',NULL,'tmux','opencode','2026-01-03T00:00:01Z',NULL,'2026-01-03T00:00:02Z','2026-01-03T00:00:03Z');
        INSERT INTO monitor_runtime(fleet_id,pid,started_at,last_tick_at,tick_seconds,last_wake_at,wake_interval_seconds,wake_requested_at) VALUES
            (1,12345,'2026-10-17T00:00:00Z','2026-10-17T00:00:05Z',7,'2026-10-16T23:50:00Z',900,'2026-10-17T00:00:03Z'),
            (2,NULL,NULL,NULL,5,'2025-12-31T23:50:00Z',NULL,NULL);
        INSERT INTO asset_installs(coding_agent,path,cafleet_version,installed_at) VALUES
            ('claude','/fixture/claude','fixture-version','2026-01-01T00:00:00Z'),
            ('claude','/fixture/other-claude','older-version','2025-01-01T00:00:00Z');
        INSERT INTO messages(message_id,owner_member_id,from_member_id,to_member_id,type,created_at,status_state,status_timestamp,origin_message_id,text,notified_at) VALUES
            (10,2,1,2,'unicast','2026-01-01T00:00:00Z','input_required','2026-01-01T00:00:00Z',NULL,'pending preview',NULL),
            (11,1,2,1,'unicast','2026-01-02T00:00:00Z','input_required','2026-01-02T00:00:00Z',NULL,'delivered preview','2026-01-02T00:00:01Z'),
            (12,2,1,2,'unicast','2026-01-03T00:00:00Z','completed','2026-01-03T00:00:02Z',NULL,'acknowledged preview','2026-01-03T00:00:01Z'),
            (13,1,1,NULL,'broadcast_summary','2026-01-04T00:00:00Z','completed','2026-01-04T00:00:00Z',13,'Broadcast sent to 1 recipients','2026-01-04T00:00:00Z'),
            (14,2,1,2,'unicast','2026-01-04T00:00:00Z','input_required','2026-01-04T00:00:00Z',13,'broadcast delivery',NULL);
        INSERT INTO member_execs(exec_id,member_id,command,created_at,dispatched_at,started_at,pid,finished_at,exit_code,resumed_at) VALUES
            (20,2,'queued command','2026-01-01T00:00:00Z',NULL,NULL,NULL,NULL,NULL,NULL),
            (21,1,'running command','2026-01-02T00:00:00Z','2026-01-02T00:00:01Z','2026-01-02T00:00:02Z',23456,NULL,NULL,NULL),
            (22,2,'finished command','2026-01-03T00:00:00Z','2026-01-03T00:00:01Z','2026-01-03T00:00:02Z',34567,'2026-01-03T00:00:03Z',7,NULL),
            (23,4,'resumed command','2025-01-03T00:00:00Z','2025-01-03T00:00:01Z','2025-01-03T00:00:02Z',45678,'2025-01-03T00:00:03Z',0,'2025-01-03T00:00:04Z');
        "#,
    ).unwrap();
    let before = records(&conn);
    let old_history = ledger(&conn);
    let retained_columns_query = format!(
        "SELECT name,type,\"notnull\",dflt_value,pk FROM pragma_table_info('member_placements') WHERE name IN ({}) ORDER BY name",
        PLACEMENT_COLUMNS.map(|name| format!("'{name}'")).join(",")
    );
    let placement_columns = rows(&conn, &retained_columns_query);
    let placement_foreign_keys = rows(&conn, "PRAGMA foreign_key_list('member_placements')");
    let indexes = rows(
        &conn,
        "SELECT name,sql FROM sqlite_master WHERE type='index' ORDER BY name",
    );

    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 10);
    assert_current_schema(&conn);
    assert_eq!(records(&conn), before);
    assert_eq!(rows(&conn, &retained_columns_query), placement_columns);
    assert_eq!(
        rows(&conn, "PRAGMA foreign_key_list('member_placements')"),
        placement_foreign_keys
    );
    assert_eq!(
        rows(
            &conn,
            "SELECT name,sql FROM sqlite_master WHERE type='index' ORDER BY name"
        ),
        indexes
    );
    let history = ledger(&conn);
    assert_eq!(history.len(), 10);
    assert_eq!(&history[..9], old_history);
    let target = broker::pane_target(&conn, 2).unwrap().unwrap();
    assert_eq!(target.pane_id, "w1:p2");
    assert_eq!(target.coding_agent, "codex");
    assert_eq!(broker::poll_messages(&conn, 2).unwrap().len(), 2);

    assert_eq!(db::migrate_to_head(&mut conn).unwrap(), 10);
    assert_eq!(records(&conn), before);
    assert_eq!(ledger(&conn), history);
    assert_current_schema(&conn);
}
