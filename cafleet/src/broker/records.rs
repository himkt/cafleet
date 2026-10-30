macro_rules! stored_enum {
    (values $name:ident, $field:literal, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        stored_enum!(@enum $name, $field, {$($variant => $wire),+});
        impl $name {
            pub(crate) const VALUES: &'static [&'static str] = &[$($wire),+];
        }
    };
    (test_values $name:ident, $field:literal, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        stored_enum!(@enum $name, $field, {$($variant => $wire),+});
        #[cfg(test)]
        impl $name {
            pub(crate) const VALUES: &'static [&'static str] = &[$($wire),+];
        }
    };
    (@enum $name:ident, $field:literal, {$($variant:ident => $wire:literal),+}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub(crate) enum $name { $(#[serde(rename = $wire)] $variant),+ }
        impl $name {
            pub(crate) fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = $crate::error::CafleetError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err($crate::error::CafleetError::InvalidStoredValue {
                        field: $field.into(), value: value.into(),
                    }),
                }
            }
        }
        impl rusqlite::types::FromSql for $name {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                Self::try_from(value.as_str()?)
                    .map_err(|error| rusqlite::types::FromSqlError::Other(Box::new(error)))
            }
        }
    };
    ($name:ident, $field:literal, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        stored_enum!(@enum $name, $field, {$($variant => $wire),+});
    };
}
pub(crate) use stored_enum;

stored_enum!(MemberStatus, "members.status", {Active => "active", Deregistered => "deregistered"});
stored_enum!(MemberKind, "member.kind", {Director => "director", Member => "member"});
stored_enum!(MessageStatus, "messages.status_state", {InputRequired => "input_required", Completed => "completed"});

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    pub(crate) backend: String,
    pub(crate) mux_session: String,
    pub(crate) mux_window_id: String,
    pub(crate) mux_pane_id: Option<String>,
    pub(crate) coding_agent: String,
    pub(crate) created_at: String,
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
pub(crate) struct MemberRecord {
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) registered_at: String,
    pub(crate) status: MemberStatus,
    pub(crate) kind: MemberKind,
    pub(crate) placement: Option<Placement>,
    pub(crate) runtime: Option<super::runtime::RuntimeSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MemberActivity {
    pub(crate) member: MemberRecord,
    pub(crate) last_sent: Option<String>,
    pub(crate) last_recv: Option<String>,
    pub(crate) last_ack: Option<String>,
    pub(crate) idle: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredMember {
    pub(crate) member_id: i64,
    pub(crate) name: String,
    pub(crate) registered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageRecord {
    pub(crate) message_id: i64,
    pub(crate) from_member_id: i64,
    pub(crate) to_member_id: i64,
    pub(crate) created_at: String,
    pub(crate) status: MessageStatus,
    pub(crate) status_timestamp: String,
    pub(crate) text: String,
}

#[cfg(test)]
impl MessageRecord {
    pub(crate) const FIXTURE_STAMP: &str = "2026-10-25T01:00:00.000000+00:00";

    pub(crate) fn fixture(id: i64, from: i64, to: i64, text: &str) -> Self {
        Self {
            message_id: id,
            from_member_id: from,
            to_member_id: to,
            created_at: Self::FIXTURE_STAMP.into(),
            status: MessageStatus::InputRequired,
            status_timestamp: Self::FIXTURE_STAMP.into(),
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetRecord {
    pub(crate) fleet_id: i64,
    pub(crate) name: Option<String>,
    pub(crate) created_at: String,
    pub(crate) deleted_at: Option<String>,
    pub(crate) director_member_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetSummary {
    pub(crate) fleet_id: i64,
    pub(crate) name: Option<String>,
    pub(crate) created_at: String,
    pub(crate) director_member_id: Option<i64>,
    pub(crate) member_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CreatedFleet {
    pub(crate) fleet_id: i64,
    pub(crate) name: Option<String>,
    pub(crate) created_at: String,
    pub(crate) director: MemberRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeletedFleet {
    pub(crate) fleet_id: i64,
    pub(crate) deregistered_count: usize,
    pub(crate) members: Vec<DeletedMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeletedMember {
    pub(crate) member_id: i64,
    pub(crate) pane_status: String,
}

#[cfg(test)]
mod tests {
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::error::CafleetError;

    fn fixture() -> (tempfile::TempDir, rusqlite::Connection, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "typed-records");
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
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
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
