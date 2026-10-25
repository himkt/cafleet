use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};

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
stored_enum!(MemberKind, "member.kind", {Director => "director", Member => "member"});
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
    pub from_member_id: i64,
    pub to_member_id: i64,
    pub created_at: String,
    pub status: MessageStatus,
    pub status_timestamp: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetRecord {
    pub fleet_id: i64,
    pub name: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
    pub director_member_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSummary {
    pub fleet_id: i64,
    pub name: Option<String>,
    pub created_at: String,
    pub director_member_id: Option<i64>,
    pub member_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreatedFleet {
    pub fleet_id: i64,
    pub name: Option<String>,
    pub created_at: String,
    pub director: MemberRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedFleet {
    pub fleet_id: i64,
    pub deregistered_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{self, tests as common};

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
    fn corrupt_director_status_returns_the_domain_variant_from_a_live_typed_read() {
        let (_dir, conn, fleet, director) = fixture();
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute(
            "UPDATE members SET status='corrupt-status' WHERE member_id=?1",
            [director],
        )
        .unwrap();
        assert!(matches!(broker::get_member(&conn, director, fleet),
            Err(CafleetError::InvalidStoredValue { field, value })
            if field == "members.status" && value == "corrupt-status"));
    }

    #[test]
    fn corrupt_recipient_status_stops_a_send_with_the_domain_cause_before_inserting() {
        let (_dir, mut conn, fleet, director) = fixture();
        let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute(
            "UPDATE members SET status='corrupt-recipient' WHERE member_id=?1",
            [worker],
        )
        .unwrap();
        assert!(
            matches!(broker::send_message(&mut conn, director, worker, "pending assignment"),
            Err(CafleetError::InvalidStoredValue { field, value })
            if field == "members.status" && value == "corrupt-recipient")
        );
        let messages: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(messages, 0);
    }

    #[test]
    fn invalid_required_string_storage_returns_sql_conversion_failures() {
        for damage in [
            "UPDATE members SET name=X'00' WHERE member_id=1",
            "UPDATE member_placements SET mux_session=X'00' WHERE member_id=1",
        ] {
            let (_dir, conn, fleet, director) = fixture();
            conn.execute_batch(damage).unwrap();
            assert!(
                broker::get_member(&conn, director, fleet).is_err(),
                "{damage}"
            );
            assert!(broker::list_members(&conn, fleet).is_err(), "{damage}");
        }
    }
}
