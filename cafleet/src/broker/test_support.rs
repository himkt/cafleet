//! Shared fixtures for typed broker contract tests (SPEC §6.2).
//!
//! Storage APIs return broker::records types: register_member returns
//! RegisteredMember; get_member/list_roster return MemberRecord;
//! list_members adds MemberActivity; update_placement_pane_id returns
//! Placement. Message reads and ACK use MessageRecord, send_message
//! returns SendOutcome with NotificationAttempt, and broadcast_message
//! returns BroadcastOutcome. Monitor queries distinguish optional raw
//! MonitorRuntime from MonitorRuntimeView, MonitorMember, and WakeTarget.
//!
//! Tests call explicit crate::presentation functions when asserting wire JSON.
//! This fixture's send helper also presents the typed outcome for existing
//! message setup; notification policy tests inspect NotificationAttempt directly.
//! No fixture depends on the temporary production Value wrappers.
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

pub const MONITOR_NAME: &str = "monitor";
pub const MONITOR_DESCRIPTION: &str = "Monitor member for this fleet";
pub const MONITOR_PANE: &str = "%1";

/// Atomic bootstrap on the canned tmux context: Director on `%0`, monitor
/// member on [`MONITOR_PANE`] via an always-succeeding spawn callback.
pub fn create_fleet(conn: &mut Connection, name: &str) -> (i64, i64) {
    let fleet = broker::create_fleet(
        conn,
        Some(name),
        "main",
        "@1",
        "%0",
        "claude",
        "tmux",
        MONITOR_NAME,
        MONITOR_DESCRIPTION,
        |_, _, _| Ok(MONITOR_PANE.to_string()),
    )
    .unwrap();
    (
        fleet["fleet_id"].as_i64().unwrap(),
        fleet["director"]["member_id"].as_i64().unwrap(),
    )
}

/// The monitor member the atomic bootstrap registered for `fleet_id`.
pub fn bootstrap_monitor(conn: &Connection, fleet_id: i64) -> i64 {
    broker::active_monitor_member_id(conn, fleet_id)
        .unwrap()
        .expect("the atomic bootstrap registers the monitor member")
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
    .unwrap()
    .member_id
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
    .unwrap()
    .member_id
}

pub struct NotifyCall {
    pub target_pane_id: String,
    pub message_id: i64,
    pub sender_id: i64,
    pub ts: String,
    pub text: String,
}

/// The raw error string [`FakeNotifier::failing`] returns from every attempt.
pub const PREVIEW_ERROR: &str = "tmux command failed: tmux send-keys -t %2 Escape\nstderr: boom";

pub struct FakeNotifier {
    pub result: Result<(), String>,
    pub calls: RefCell<Vec<NotifyCall>>,
}

impl FakeNotifier {
    pub fn succeeding() -> Self {
        FakeNotifier {
            result: Ok(()),
            calls: RefCell::new(Vec::new()),
        }
    }

    pub fn failing() -> Self {
        FakeNotifier {
            result: Err(PREVIEW_ERROR.to_string()),
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
    ) -> Result<(), String> {
        self.calls.borrow_mut().push(NotifyCall {
            target_pane_id: target_pane_id.to_string(),
            message_id,
            sender_id,
            ts: ts.to_string(),
            text: text.to_string(),
        });
        self.result.clone()
    }
}

/// Send and return the outcome's `{message, notification_sent}` payload;
/// tests that assert NotificationAttempt call broker::send_message directly.
pub fn send(
    conn: &mut Connection,
    notifier: &FakeNotifier,
    from: i64,
    to: i64,
    text: &str,
) -> Value {
    let outcome =
        broker::send_message(conn, notifier, MAX_TEXT_LEN, from, &to.to_string(), text).unwrap();
    crate::presentation::send_outcome(&outcome)
}
