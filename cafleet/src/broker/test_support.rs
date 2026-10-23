#![allow(dead_code)]

use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::broker::{self, NewPlacement};

pub fn migrated_conn(dir: &TempDir) -> Connection {
    let url = format!("sqlite:///{}", dir.path().join("broker_test.db").display());
    let mut conn = crate::db::connect(&url).unwrap();
    crate::db::migrate_to_head(&mut conn).unwrap();
    conn
}

pub fn placement(pane: Option<&str>) -> NewPlacement {
    NewPlacement {
        backend: "tmux".to_string(),
        mux_session: "main".to_string(),
        mux_window_id: "@1".to_string(),
        mux_pane_id: pane.map(str::to_string),
        coding_agent: "claude".to_string(),
    }
}

pub fn create_fleet(conn: &mut Connection, name: &str) -> (i64, i64) {
    let fleet =
        broker::create_fleet(conn, Some(name), "main", "@1", "%0", "claude", "tmux").unwrap();
    (
        fleet["fleet_id"].as_i64().unwrap(),
        fleet["director"]["member_id"].as_i64().unwrap(),
    )
}

pub fn register(conn: &mut Connection, fleet_id: i64, name: &str, pane: Option<&str>) -> i64 {
    broker::register_member(conn, fleet_id, name, "test member", Some(&placement(pane)))
        .unwrap()
        .member_id
}

/// Persist a point-to-point message and return `{"message": <presented row>}`.
pub fn send(conn: &mut Connection, from: i64, to: i64, text: &str) -> Value {
    let message = broker::send_message(conn, from, &to.to_string(), text).unwrap();
    json!({"message": crate::presentation::message(&message)})
}
