//! Internal records decoded once from storage; wire construction is separate.
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use serde_json::Value;

use crate::error::CafleetError;

macro_rules! stored_enum {
    ($name:ident, $field:literal, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = CafleetError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(CafleetError::InvalidStoredValue {
                        field: $field.into(), value: value.into(),
                    }),
                }
            }
        }
        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                Self::try_from(value.as_str()?)
                    .map_err(|error| FromSqlError::Other(Box::new(error)))
            }
        }
    };
}

stored_enum!(MemberStatus, "members.status", {Active => "active", Deregistered => "deregistered"});
stored_enum!(MemberKind, "member.kind", {Director => "director", Monitor => "monitor", Member => "member"});
stored_enum!(MessageKind, "messages.type", {Unicast => "unicast", BroadcastSummary => "broadcast_summary"});
stored_enum!(MessageStatus, "messages.status_state", {InputRequired => "input_required", Completed => "completed"});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub backend: String,
    pub mux_session: String,
    pub mux_window_id: String,
    pub mux_pane_id: Option<String>,
    pub coding_agent: String,
    pub created_at: String,
}

impl Placement {
    pub(crate) fn from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            backend: row.get(offset)?,
            mux_session: row.get(offset + 1)?,
            mux_window_id: row.get(offset + 2)?,
            mux_pane_id: row.get(offset + 3)?,
            coding_agent: row.get(offset + 4)?,
            created_at: row.get(offset + 5)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberRecord {
    pub member_id: i64,
    pub fleet_id: i64,
    pub name: String,
    pub description: String,
    pub registered_at: String,
    pub status: MemberStatus,
    pub kind: MemberKind,
    pub skills: Vec<Value>,
    pub placement: Option<Placement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberActivity {
    pub member: MemberRecord,
    pub last_sent: Option<String>,
    pub last_recv: Option<String>,
    pub last_ack: Option<String>,
    pub idle: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredMember {
    pub member_id: i64,
    pub name: String,
    pub registered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecord {
    pub message_id: i64,
    pub owner_member_id: i64,
    pub from_member_id: i64,
    pub to_member_id: Option<i64>,
    pub kind: MessageKind,
    pub created_at: String,
    pub status: MessageStatus,
    pub status_timestamp: String,
    pub origin_message_id: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorRuntime {
    pub fleet_id: i64,
    pub pid: Option<i64>,
    pub started_at: Option<String>,
    pub last_tick_at: Option<String>,
    pub tick_seconds: i64,
    pub wake_interval_seconds: Option<i64>,
    pub last_wake_at: Option<String>,
    pub wake_requested_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationAttempt {
    Skipped,
    Sent,
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOutcome {
    pub message: MessageRecord,
    pub notification: NotificationAttempt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastOutcome {
    pub message: MessageRecord,
    pub recipients: usize,
    pub delivered: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeTarget {
    pub member_id: i64,
    pub name: String,
    pub coding_agent: String,
    pub pending_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorMember {
    pub member_id: i64,
    pub name: String,
    pub pending_count: i64,
    pub oldest_pending_ts: Option<String>,
    pub oldest_pending_age_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorRuntimeView {
    pub running: bool,
    pub pid: Option<i64>,
    pub tick_seconds: Option<i64>,
    pub wake_interval_seconds: Option<i64>,
    pub last_tick_at: Option<String>,
    pub last_tick_age_seconds: Option<i64>,
    pub started_at: Option<String>,
    pub last_wake_at: Option<String>,
    pub last_wake_age_seconds: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{self, test_support as common};

    fn fixture() -> (tempfile::TempDir, rusqlite::Connection, i64, i64) {
        let dir = tempfile::Builder::new()
            .prefix(".typed-records-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "typed-records");
        (dir, conn, fleet, director)
    }

    #[test]
    fn corrupt_member_status_returns_the_domain_variant_from_storage() {
        let (_dir, mut conn, fleet, director) = fixture();
        broker::send_message(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            common::MAX_TEXT_LEN,
            director,
            &director.to_string(),
            "retain history",
        )
        .unwrap();
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute(
            "UPDATE members SET status='corrupt-status' WHERE member_id=?1",
            [director],
        )
        .unwrap();
        assert!(matches!(broker::list_roster(&conn, fleet, true),
            Err(CafleetError::InvalidStoredValue { field, value })
            if field == "members.status" && value == "corrupt-status"));
    }
}
