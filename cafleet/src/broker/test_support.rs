//! Shared fixtures for the broker's colocated contract tests (SPEC §6.2).
//!
//! Expected public API pinned by the submodule test suites (all result shapes
//! are `serde_json::Value` dicts with insertion-order keys; errors are
//! `crate::error::CafleetError` `Value(String)` — translated by callers:
//! CLI → exit 1, WebUI → HTTP status):
//!
//! ```text
//! pub struct NewPlacement { pub backend: String, pub mux_session: String,
//!     pub mux_window_id: String, pub mux_pane_id: Option<String>,
//!     pub coding_agent: String }
//! pub trait InlinePreviewSender {
//!     fn send_inline_preview(&self, target_pane_id: &str, message_id: i64,
//!         sender_id: i64, ts: &str, text: &str) -> bool;
//! }
//! // fleets
//! create_fleet(conn: &mut Connection, name: Option<&str>, mux_session: &str,
//!     mux_window_id: &str, mux_pane_id: &str, coding_agent: &str,
//!     backend: &str) -> Result<Value>
//! list_fleets(conn: &Connection) -> Result<Vec<Value>>
//! get_fleet(conn: &Connection, fleet_id: i64) -> Result<Option<Value>>
//! delete_fleet(conn: &mut Connection, fleet_id: i64) -> Result<Value>
//! // members
//! register_member(conn: &mut Connection, fleet_id: i64, name: &str,
//!     description: &str, skills: &[Value], placement: Option<&NewPlacement>,
//!     monitor: bool) -> Result<Value>  // {member_id, name, registered_at}
//! get_member(conn: &Connection, member_id: i64, fleet_id: i64) -> Result<Option<Value>>
//! active_monitor_member_id(conn: &Connection, fleet_id: i64) -> Result<Option<i64>>
//! deregister_member(conn: &mut Connection, member_id: i64) -> Result<bool>
//! update_placement_pane_id(conn: &mut Connection, member_id: i64, pane_id: &str)
//!     -> Result<Option<Value>>
//! verify_member_fleet(conn: &Connection, member_id: i64, fleet_id: i64) -> Result<bool>
//! get_member_names(conn: &Connection, member_ids: &[i64]) -> Result<BTreeMap<i64, String>>
//! list_members(conn: &Connection, fleet_id: i64) -> Result<Vec<Value>>
//! list_roster(conn: &Connection, fleet_id: i64, include_message_holders: bool)
//!     -> Result<Vec<Value>>
//! // messaging
//! send_message(conn: &mut Connection, notifier: &dyn InlinePreviewSender,
//!     max_text_len: usize, from_member_id: i64, to: &str, text: &str)
//!     -> Result<Value>  // {message, notification_sent}
//! broadcast_message(conn: &mut Connection, notifier: &dyn InlinePreviewSender,
//!     max_text_len: usize, from_member_id: i64, text: &str)
//!     -> Result<Vec<Value>>  // [{message, recipients, delivered}]
//! poll_messages(conn: &Connection, member_id: i64) -> Result<Vec<Value>>
//! ack_message(conn: &mut Connection, message_id: i64) -> Result<Value>  // {message}
//! // queries
//! list_inbox(conn: &Connection, member_id: i64) -> Result<Vec<Value>>
//! list_sent(conn: &Connection, member_id: i64) -> Result<Vec<Value>>
//! list_timeline(conn: &Connection, fleet_id: i64, limit: usize) -> Result<Vec<Value>>
//! get_message(conn: &Connection, message_id: i64) -> Result<Value> // {message}
//! // monitor
//! record_monitor_wake(conn: &mut Connection, fleet_id: i64, when: &str) -> Result<()>
//! list_fleet_wake_targets(conn: &Connection, fleet_id: i64) -> Result<Vec<Value>>
//! fleet_wake_director(conn: &Connection, fleet_id: i64) -> Result<Value>
//! claim_monitor_runtime(conn: &mut Connection, fleet_id: i64, pid: i64,
//!     tick_seconds: i64, wake_interval: i64, when: &str) -> Result<bool>
//! set_monitor_wake_interval(conn: &mut Connection, fleet_id: i64,
//!     wake_interval: i64) -> Result<bool>  // false ⇔ no row
//! heartbeat_monitor_runtime(conn: &mut Connection, fleet_id: i64, pid: i64,
//!     when: &str) -> Result<bool>
//! clear_monitor_runtime(conn: &mut Connection, fleet_id: i64, pid: i64) -> Result<()>
//! read_monitor_runtime(conn: &Connection, fleet_id: i64) -> Result<Option<Value>>
//! monitor_is_live(conn: &Connection, fleet_id: i64, now: DateTime<Utc>) -> Result<bool>
//! monitor_runtime_payload(conn: &Connection, fleet_id: i64, now: DateTime<Utc>) -> Result<Value>
//! monitor_members_payload(conn: &Connection, fleet_id: i64, now: DateTime<Utc>)
//!     -> Result<Vec<Value>>
//! // asset_installs
//! asset_installs_table_exists(conn: &Connection) -> bool
//! list_asset_installs(conn: &Connection) -> Result<Vec<Value>>
//! record_asset_install(conn: &mut Connection, coding_agent: &str,
//!     cafleet_version: &str) -> Result<()>
//! ```
#![allow(dead_code)]

use std::cell::RefCell;

use rusqlite::Connection;
use serde_json::Value;
use tempfile::TempDir;

use crate::broker::{self, InlinePreviewSender, NewPlacement};

pub const MAX_TEXT_LEN: usize = 200;

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
    broker::register_member(
        conn,
        fleet_id,
        name,
        "test member",
        &[],
        Some(&placement(pane)),
        false,
    )
    .unwrap()["member_id"]
        .as_i64()
        .unwrap()
}

pub fn register_monitor(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    pane: Option<&str>,
) -> i64 {
    broker::register_member(
        conn,
        fleet_id,
        name,
        "monitor member",
        &[],
        Some(&placement(pane)),
        true,
    )
    .unwrap()["member_id"]
        .as_i64()
        .unwrap()
}

pub struct NotifyCall {
    pub target_pane_id: String,
    pub message_id: i64,
    pub sender_id: i64,
    pub ts: String,
    pub text: String,
}

pub struct FakeNotifier {
    pub result: bool,
    pub calls: RefCell<Vec<NotifyCall>>,
}

impl FakeNotifier {
    pub fn succeeding() -> Self {
        FakeNotifier {
            result: true,
            calls: RefCell::new(Vec::new()),
        }
    }

    pub fn failing() -> Self {
        FakeNotifier {
            result: false,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl InlinePreviewSender for FakeNotifier {
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> bool {
        self.calls.borrow_mut().push(NotifyCall {
            target_pane_id: target_pane_id.to_string(),
            message_id,
            sender_id,
            ts: ts.to_string(),
            text: text.to_string(),
        });
        self.result
    }
}

pub fn send(
    conn: &mut Connection,
    notifier: &FakeNotifier,
    from: i64,
    to: i64,
    text: &str,
) -> Value {
    broker::send_message(conn, notifier, MAX_TEXT_LEN, from, &to.to_string(), text).unwrap()
}
