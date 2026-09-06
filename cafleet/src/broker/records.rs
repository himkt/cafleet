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
    use serde_json::json;

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
    fn stored_enums_decode_exact_values_and_report_typed_integrity_errors() {
        macro_rules! check {
            ($ty:ident, $field:literal, [$($variant:ident => $wire:literal),+]) => {{
                $(assert_eq!($ty::try_from($wire).unwrap(), $ty::$variant);
                  assert_eq!($ty::$variant.as_str(), $wire);)+
                for bad in ["", "UNKNOWN", " active", "null"] {
                    assert!(matches!($ty::try_from(bad),
                        Err(CafleetError::InvalidStoredValue { field, value })
                        if field == $field && value == bad));
                }
            }};
        }
        check!(MemberStatus, "members.status", [Active => "active", Deregistered => "deregistered"]);
        check!(MemberKind, "member.kind", [Director => "director", Monitor => "monitor", Member => "member"]);
        check!(MessageKind, "messages.type", [Unicast => "unicast", BroadcastSummary => "broadcast_summary"]);
        check!(MessageStatus, "messages.status_state", [InputRequired => "input_required", Completed => "completed"]);
    }

    #[test]
    fn member_records_preserve_roles_skills_and_optional_placement() {
        let (_dir, mut conn, fleet, director) = fixture();
        let skills = vec![json!({"nested": [null, true, 7]}), json!(["free", "form"])];
        let registered = broker::register_member(
            &mut conn,
            fleet,
            "unplaced",
            "description",
            &skills,
            None,
            false,
        )
        .unwrap();
        assert_eq!(registered.name, "unplaced");
        assert!(registered.member_id > 0);
        assert!(!registered.registered_at.is_empty());
        let row = broker::get_member(&conn, registered.member_id, fleet)
            .unwrap()
            .unwrap();
        assert_eq!(row.member_id, registered.member_id);
        assert_eq!(row.fleet_id, fleet);
        assert_eq!(row.registered_at, registered.registered_at);
        assert_eq!(row.description, "description");
        assert_eq!(row.status, MemberStatus::Active);
        assert_eq!(row.kind, MemberKind::Member);
        assert_eq!(row.skills, skills);
        assert_eq!(row.placement, None);
        assert_eq!(
            broker::get_member(&conn, director, fleet)
                .unwrap()
                .unwrap()
                .kind,
            MemberKind::Director
        );
        let monitor = common::bootstrap_monitor(&conn, fleet);
        assert_eq!(
            broker::get_member(&conn, monitor, fleet)
                .unwrap()
                .unwrap()
                .kind,
            MemberKind::Monitor
        );
        let pending = common::register(&mut conn, fleet, "pending", None);
        let placement = broker::get_member(&conn, pending, fleet)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        assert_eq!(placement.backend, "tmux");
        assert_eq!(placement.mux_session, "main");
        assert_eq!(placement.mux_window_id, "@1");
        assert_eq!(placement.mux_pane_id, None);
        assert_eq!(placement.coding_agent, "claude");
        let activities = broker::list_members(&conn, fleet).unwrap();
        let activity = activities
            .iter()
            .find(|a| a.member.member_id == registered.member_id)
            .unwrap();
        assert_eq!(activity.member, row);
        assert_eq!(
            (&activity.last_sent, &activity.last_recv, &activity.last_ack),
            (&None, &None, &None)
        );
        assert_eq!(
            broker::list_roster(&conn, fleet, false)
                .unwrap()
                .into_iter()
                .find(|m| m.member_id == row.member_id)
                .unwrap(),
            row
        );
        broker::send_message(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            common::MAX_TEXT_LEN,
            director,
            &registered.member_id.to_string(),
            "retain history",
        )
        .unwrap();
        broker::deregister_member(&mut conn, registered.member_id).unwrap();
        assert!(
            broker::get_member(&conn, registered.member_id, fleet)
                .unwrap()
                .is_none()
        );
        let historical = broker::list_roster(&conn, fleet, true)
            .unwrap()
            .into_iter()
            .find(|m| m.member_id == registered.member_id)
            .unwrap();
        assert_eq!(historical.status, MemberStatus::Deregistered);
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

    #[test]
    fn failed_notification_retains_the_persisted_typed_delivery_and_ack_lifecycle() {
        let (_dir, mut conn, fleet, director) = fixture();
        let recipient = common::register(&mut conn, fleet, "recipient", Some("%9"));
        let notifier = common::FakeNotifier::failing();
        let outcome = broker::send_message(
            &mut conn,
            &notifier,
            common::MAX_TEXT_LEN,
            director,
            &recipient.to_string(),
            "work",
        )
        .unwrap();
        assert_eq!(
            outcome.notification,
            NotificationAttempt::Failed {
                error: common::PREVIEW_ERROR.into()
            }
        );
        let message = outcome.message;
        assert_eq!(message.owner_member_id, recipient);
        assert_eq!(message.from_member_id, director);
        assert_eq!(message.to_member_id, Some(recipient));
        assert_eq!(message.kind, MessageKind::Unicast);
        assert_eq!(message.status, MessageStatus::InputRequired);
        assert_eq!(message.origin_message_id, None);
        assert_eq!(message.text, "work");
        assert_eq!(message.status_timestamp, message.created_at);
        assert_eq!(
            broker::get_message(&conn, message.message_id).unwrap(),
            message
        );
        assert_eq!(
            broker::poll_messages(&conn, recipient).unwrap(),
            vec![message.clone()]
        );
        let ack = broker::ack_message(&mut conn, message.message_id).unwrap();
        assert_eq!(ack.status, MessageStatus::Completed);
        assert_eq!(ack.message_id, message.message_id);
        assert_eq!(ack.created_at, message.created_at);
        assert_eq!(ack.to_member_id, message.to_member_id);
        assert!(broker::poll_messages(&conn, recipient).unwrap().is_empty());
        assert_eq!(notifier.calls.borrow().len(), 1);
    }

    #[test]
    fn broadcast_records_distinguish_summary_metadata_from_pending_deliveries() {
        let (_dir, mut conn, fleet, director) = fixture();
        let recipient = common::register(&mut conn, fleet, "recipient", Some("%9"));
        let outcome = broker::broadcast_message(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            common::MAX_TEXT_LEN,
            director,
            "broadcast",
        )
        .unwrap();
        assert_eq!(outcome.recipients, 2); // ordinary member and bootstrap monitor
        assert_eq!(outcome.delivered, 2);
        assert_eq!(outcome.message.kind, MessageKind::BroadcastSummary);
        assert_eq!(outcome.message.status, MessageStatus::Completed);
        assert_eq!(outcome.message.owner_member_id, director);
        assert_eq!(outcome.message.from_member_id, director);
        assert_eq!(outcome.message.to_member_id, None);
        assert_eq!(
            outcome.message.origin_message_id,
            Some(outcome.message.message_id)
        );
        assert_eq!(
            broker::get_message(&conn, outcome.message.message_id).unwrap(),
            outcome.message
        );
        let deliveries = broker::poll_messages(&conn, recipient).unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].kind, MessageKind::Unicast);
        assert_eq!(deliveries[0].status, MessageStatus::InputRequired);
        assert_eq!(
            deliveries[0].origin_message_id,
            Some(outcome.message.message_id)
        );
        assert_eq!(deliveries[0].to_member_id, Some(recipient));
        assert_eq!(deliveries[0].owner_member_id, recipient);
    }

    #[test]
    fn typed_wake_and_monitor_rows_preserve_pending_counts_and_member_identity() {
        let (_dir, mut conn, fleet, director) = fixture();
        let member = common::register(&mut conn, fleet, "worker name", Some("%9"));
        let notifier = common::FakeNotifier::succeeding();
        for _ in 0..2 {
            broker::send_message(
                &mut conn,
                &notifier,
                common::MAX_TEXT_LEN,
                director,
                &member.to_string(),
                "work",
            )
            .unwrap();
        }
        let targets = broker::list_fleet_wake_targets(&conn, fleet).unwrap();
        assert_eq!(
            targets,
            vec![WakeTarget {
                member_id: member,
                name: "worker name".into(),
                coding_agent: "claude".into(),
                pending_count: 2
            }]
        );
        let root = broker::fleet_wake_director(&conn, fleet).unwrap();
        assert_eq!(root.member_id, director);
        assert_eq!(root.coding_agent, "claude");
        assert_eq!(root.pending_count, 0);
        let rows = broker::monitor_member_records(&conn, fleet, crate::time::now_utc()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].member_id, member);
        assert_eq!(rows[0].name, "worker name");
        assert_eq!(rows[0].pending_count, 2);
        assert!(rows[0].oldest_pending_ts.is_some());
        assert!(rows[0].oldest_pending_age_seconds.is_some());
    }
}
