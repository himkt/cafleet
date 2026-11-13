use crate::broker::MemberActivity;
use crate::broker::MemberKind;
use crate::broker::MemberRecord;
use crate::broker::MemberStatus;
use crate::broker::Placement;
use crate::broker::RegisteredMember;
use crate::error::CafleetError;
use crate::multiplexer::layout::ColumnSide;
use crate::time::now_stamp;
use crate::time::parse_lenient;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;

pub(crate) fn missing_run(member_id: i64) -> CafleetError {
    CafleetError::App(format!("member {member_id} has no recorded run"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlacementPane {
    pub(crate) mux_window_id: String,
    pub(crate) mux_window_name: String,
    pub(crate) mux_pane_id: String,
}

/// `pane: None` registers a pending placement whose window and pane are published later.
#[derive(Debug, Clone)]
pub(crate) struct NewPlacement {
    pub(crate) backend: String,
    pub(crate) mux_session: String,
    pub(crate) window_ordinal: i64,
    pub(crate) column_side: ColumnSide,
    pub(crate) coding_agent: String,
    pub(crate) mux_server: String,
    pub(crate) pane: Option<PlacementPane>,
}

impl NewPlacement {
    pub(crate) fn director(
        backend: String,
        mux_session: String,
        coding_agent: String,
        mux_server: String,
        pane: PlacementPane,
    ) -> Self {
        Self {
            backend,
            mux_session,
            window_ordinal: 0,
            column_side: ColumnSide::Left,
            coding_agent,
            mux_server,
            pane: Some(pane),
        }
    }
}

/// An overflow or Director window holding at least one active concrete ordinary member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OccupiedWindow {
    pub(crate) window_ordinal: i64,
    pub(crate) mux_window_id: String,
    pub(crate) mux_window_name: String,
    pub(crate) left: Vec<String>,
    pub(crate) right: Vec<String>,
}

pub(crate) fn concrete_occupancy(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<OccupiedWindow>, CafleetError> {
    let mut stmt = conn.prepare(
        "SELECT p.window_ordinal, p.mux_window_id, p.mux_window_name, p.column_side, p.mux_pane_id
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL
           AND f.director_member_id IS NOT m.member_id
         ORDER BY p.window_ordinal, m.member_id",
    )?;
    let rows = stmt.query_map([fleet_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, ColumnSide>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut windows: Vec<OccupiedWindow> = Vec::new();
    for row in rows {
        let (window_ordinal, mux_window_id, mux_window_name, side, pane) = row?;
        if windows.last().map(|window| window.window_ordinal) != Some(window_ordinal) {
            windows.push(OccupiedWindow {
                window_ordinal,
                mux_window_id,
                mux_window_name,
                left: Vec::new(),
                right: Vec::new(),
            });
        }
        let window = windows
            .last_mut()
            .expect("the row's window was just pushed");
        match side {
            ColumnSide::Left => window.left.push(pane),
            ColumnSide::Right => window.right.push(pane),
        }
    }
    Ok(windows)
}

pub(crate) fn window_member_ids(
    conn: &Connection,
    fleet_id: i64,
    window_ordinal: i64,
) -> Result<Vec<i64>, CafleetError> {
    let mut stmt = conn.prepare(
        "SELECT m.member_id
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active' AND p.window_ordinal=?2
           AND f.director_member_id IS NOT m.member_id
         ORDER BY m.member_id",
    )?;
    stmt.query_map(params![fleet_id, window_ordinal], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()
        .map_err(CafleetError::from)
}

pub(crate) fn register_member_rows(
    conn: &Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    placement: &NewPlacement,
) -> Result<RegisteredMember, CafleetError> {
    let now = now_stamp();
    crate::broker::fleets::require_live_fleet(conn, fleet_id)?;
    let member_id = insert_member_row(conn, fleet_id, name, description, &now)?;
    insert_placement_row(conn, member_id, placement, &now)?;
    Ok(RegisteredMember {
        member_id,
        name: name.into(),
        registered_at: now,
    })
}

pub(super) fn insert_member_row(
    conn: &Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    now: &str,
) -> Result<i64, CafleetError> {
    conn.query_row(
        "INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4) RETURNING member_id",
        params![fleet_id, name, description, now],
        |row| row.get(0),
    )
    .map_err(CafleetError::from)
}

pub(super) fn insert_placement_row(
    conn: &Connection,
    member_id: i64,
    placement: &NewPlacement,
    now: &str,
) -> Result<(), CafleetError> {
    let pane = placement.pane.as_ref();
    conn.execute(
        "INSERT INTO member_placements
         (member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side,
          mux_pane_id, backend, mux_server, coding_agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            member_id,
            placement.mux_session,
            pane.map(|pane| &pane.mux_window_id),
            pane.map(|pane| &pane.mux_window_name),
            placement.window_ordinal,
            placement.column_side.as_str(),
            pane.map(|pane| &pane.mux_pane_id),
            placement.backend,
            placement.mux_server,
            placement.coding_agent,
            now
        ],
    )?;
    Ok(())
}

const MEMBER_FIELDS: usize = 7;
const ACTIVITY_OFFSET: usize = MEMBER_FIELDS + crate::broker::PLACEMENT_COLUMN_COUNT;

fn member_columns() -> String {
    format!(
        "m.member_id, m.fleet_id, m.name, m.description, m.status,
         m.registered_at, f.director_member_id=m.member_id, {}",
        crate::broker::PLACEMENT_COLUMNS
    )
}

const MEMBER_FROM: &str = "FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
    JOIN member_placements p ON p.member_id=m.member_id";

fn member_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemberRecord> {
    Ok(MemberRecord {
        member_id: row.get(0)?,
        fleet_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        status: row.get(4)?,
        registered_at: row.get(5)?,
        kind: if row.get(6)? {
            MemberKind::Director
        } else {
            MemberKind::Member
        },
        placement: Placement::from_row(row, MEMBER_FIELDS)?,
        runtime: None,
    })
}

pub(crate) fn get_member(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<Option<MemberRecord>, CafleetError> {
    crate::broker::fleets::require_live_fleet(conn, fleet_id)?;
    let mut member = conn
        .query_row(
            &format!(
                "SELECT {} {MEMBER_FROM}
                 WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'",
                member_columns()
            ),
            params![member_id, fleet_id],
            member_row,
        )
        .optional()?;
    if let Some(member) = &mut member {
        member.runtime = crate::broker::runtime::member_runtime(conn, member_id)?;
    }
    Ok(member)
}

pub(crate) fn member_fleet_status(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<(i64, MemberStatus)>, CafleetError> {
    conn.query_row(
        "SELECT fleet_id, status FROM members WHERE member_id=?1",
        [member_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(CafleetError::from)
}

pub(crate) fn active_member_fleet(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<i64>, CafleetError> {
    Ok(member_fleet_status(conn, member_id)?
        .filter(|(_, status)| *status == MemberStatus::Active)
        .map(|(fleet_id, _)| fleet_id))
}

pub(crate) fn deregister_member_rows(
    conn: &Connection,
    member_id: i64,
) -> Result<bool, CafleetError> {
    let is_root: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM fleets \
             WHERE director_member_id=?1 AND deleted_at IS NULL)",
        [member_id],
        |row| row.get(0),
    )?;
    if is_root {
        return Err(CafleetError::App(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead".to_string(),
        ));
    }
    let changed = conn.execute(
        "UPDATE members SET status='deregistered' WHERE member_id=?1 AND status='active'",
        [member_id],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    conn.execute(
        "DELETE FROM member_placements WHERE member_id=?1",
        [member_id],
    )?;
    Ok(true)
}

fn idle_seconds(
    row: &MemberActivity,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<i64>, CafleetError> {
    let latest = [
        ("messages.created_at", &row.last_sent),
        ("messages.created_at", &row.last_recv),
        ("messages.status_timestamp", &row.last_ack),
    ]
    .into_iter()
    .filter_map(|(field, value)| value.as_ref().map(|value| (field, value)))
    .max_by_key(|(_, value)| *value);
    let Some((field, latest)) = latest else {
        return Ok(None);
    };
    let parsed = parse_lenient(latest).ok_or_else(|| CafleetError::InvalidStoredValue {
        field: field.into(),
        value: latest.clone(),
    })?;
    Ok(Some((now - parsed).num_seconds().max(0)))
}

pub(crate) fn list_members(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<MemberActivity>, CafleetError> {
    crate::broker::fleets::require_live_fleet(conn, fleet_id)?;
    let now = chrono::Utc::now();
    let mut stmt = conn.prepare(&format!(
        "SELECT {},
                (SELECT MAX(created_at) FROM messages WHERE from_member_id=m.member_id),
                (SELECT MAX(created_at) FROM messages WHERE to_member_id=m.member_id),
                (SELECT MAX(status_timestamp) FROM messages WHERE to_member_id=m.member_id AND status_state='completed')
         {MEMBER_FROM}
         WHERE m.fleet_id=?1 AND m.status='active'
         ORDER BY m.member_id",
        member_columns()
    ))?;
    let rows = stmt.query_map([fleet_id], |row| {
        Ok(MemberActivity {
            member: member_row(row)?,
            last_sent: row.get(ACTIVITY_OFFSET)?,
            last_recv: row.get(ACTIVITY_OFFSET + 1)?,
            last_ack: row.get(ACTIVITY_OFFSET + 2)?,
            idle: None,
        })
    })?;
    rows.map(|row| {
        let mut activity = row?;
        activity.idle = idle_seconds(&activity, now)?;
        Ok(activity)
    })
    .collect()
}

pub(crate) fn active_member(
    conn: &Connection,
    member_id: i64,
) -> Result<MemberRecord, CafleetError> {
    let missing = || CafleetError::App(format!("Member {member_id} not found"));
    let fleet_id = crate::broker::active_member_fleet(conn, member_id)?.ok_or_else(missing)?;
    crate::broker::get_member(conn, member_id, fleet_id)?.ok_or_else(missing)
}
pub(crate) fn require_pinned_fleet(
    member_id: i64,
    actual_fleet: i64,
    expected_fleet: Option<i64>,
    operation: &str,
) -> Result<(), CafleetError> {
    if expected_fleet.is_some_and(|expected| expected != actual_fleet) {
        return Err(CafleetError::App(format!(
            "Member {member_id} changed fleets during {operation}"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::broker::members::*;
    use rusqlite::TransactionBehavior;
    pub(crate) fn register_member(
        conn: &mut Connection,
        fleet_id: i64,
        name: &str,
        description: &str,
        placement: &NewPlacement,
    ) -> Result<RegisteredMember, CafleetError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let member = register_member_rows(&tx, fleet_id, name, description, placement)?;
        tx.commit()?;
        Ok(member)
    }
    pub(crate) fn deregister_member(
        conn: &mut Connection,
        member_id: i64,
    ) -> Result<bool, CafleetError> {
        let tx = conn.transaction()?;
        let changed = deregister_member_rows(&tx, member_id)?;
        tx.commit()?;
        Ok(changed)
    }
    pub(crate) fn publish_placement_pane(
        conn: &mut Connection,
        member_id: i64,
        pane_id: &str,
    ) -> Result<Option<Placement>, CafleetError> {
        conn.execute(
            "UPDATE member_placements SET mux_pane_id=?1,
                mux_window_id=COALESCE(mux_window_id, ?3),
                mux_window_name=COALESCE(mux_window_name, ?4)
             WHERE member_id=?2",
            params![pane_id, member_id, TEST_WINDOW_ID, TEST_WINDOW_NAME],
        )?;
        conn.query_row(
            &format!(
                "SELECT {} FROM member_placements p WHERE p.member_id=?1",
                crate::broker::PLACEMENT_COLUMNS
            ),
            [member_id],
            |row| Placement::from_row(row, 0),
        )
        .optional()
        .map_err(crate::error::CafleetError::from)
    }
    use rusqlite::Connection;

    use crate::broker::members::NewPlacement;
    use crate::broker::members::PlacementPane;
    use crate::multiplexer::layout::ColumnSide;

    pub(crate) const TEST_WINDOW_ID: &str = "@1";
    pub(crate) const TEST_WINDOW_NAME: &str = "main";
    const TEST_SESSION: &str = "main";
    const TEST_SERVER: &str = "/nonexistent/cafleet-test/tmux.sock";

    fn test_pane(pane: &str) -> PlacementPane {
        PlacementPane {
            mux_window_id: TEST_WINDOW_ID.into(),
            mux_window_name: TEST_WINDOW_NAME.into(),
            mux_pane_id: pane.into(),
        }
    }

    /// A Director-window member placement, concrete in window `@1` when `pane` is given.
    pub(crate) fn placement(pane: Option<&str>) -> NewPlacement {
        NewPlacement {
            backend: "tmux".into(),
            mux_session: TEST_SESSION.into(),
            window_ordinal: 0,
            column_side: ColumnSide::Right,
            coding_agent: "claude".into(),
            mux_server: TEST_SERVER.into(),
            pane: pane.map(test_pane),
        }
    }

    pub(crate) fn director_new_placement(pane: &str) -> NewPlacement {
        NewPlacement::director(
            "tmux".into(),
            TEST_SESSION.into(),
            "claude".into(),
            TEST_SERVER.into(),
            test_pane(pane),
        )
    }

    /// A pending placement in the given slot.
    pub(crate) fn slot_placement(window_ordinal: i64, column_side: ColumnSide) -> NewPlacement {
        NewPlacement {
            window_ordinal,
            column_side,
            ..placement(None)
        }
    }

    /// Registers a pending member in the given slot and, when `concrete` is given, publishes
    /// its `(window_id, window_name, pane_id)`.
    pub(crate) fn register_in_slot(
        conn: &mut Connection,
        fleet: i64,
        name: &str,
        window_ordinal: i64,
        column_side: ColumnSide,
        concrete: Option<(&str, &str, &str)>,
    ) -> i64 {
        let member = register_member(
            conn,
            fleet,
            name,
            "assignment",
            &slot_placement(window_ordinal, column_side),
        )
        .unwrap()
        .member_id;
        if let Some((window_id, window_name, pane_id)) = concrete {
            assert_eq!(
                conn.execute(
                    "UPDATE member_placements SET mux_pane_id=?1, mux_window_id=?2, mux_window_name=?3
                     WHERE member_id=?4 AND mux_pane_id IS NULL",
                    params![pane_id, window_id, window_name, member],
                )
                .unwrap(),
                1
            );
        }
        member
    }

    pub(crate) fn register(
        conn: &mut Connection,
        fleet: i64,
        name: &str,
        pane: Option<&str>,
    ) -> i64 {
        crate::broker::members::test_support::register_member(
            conn,
            fleet,
            name,
            "assignment",
            &placement(pane),
        )
        .unwrap()
        .member_id
    }
}

#[cfg(test)]
mod tests {
    use crate::broker;
    use crate::broker::MemberStatus;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::deregister_member;
    use crate::broker::members::test_support::placement;
    use crate::broker::members::test_support::register;
    use crate::broker::members::test_support::register_member;
    use crate::broker::members::*;
    use crate::db::test_support::migrated_conn;

    #[test]
    fn registration_and_placement_updates_preserve_required_fields_and_valid_absence() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, _) = create_fleet_fixture(&mut conn, "alpha");
        let registered = register_member(
            &mut conn,
            fleet,
            "worker",
            "literal assignment",
            &placement(None),
        )
        .unwrap();
        assert_eq!(registered.name, "worker");
        assert!(parse_lenient(&registered.registered_at).is_some());
        let member = get_member(&conn, registered.member_id, fleet)
            .unwrap()
            .unwrap();
        assert_eq!(member.description, "literal assignment");
        assert_eq!(member.kind, MemberKind::Member);
        assert_eq!(member.placement.mux_pane_id, None);
    }

    #[test]
    fn registration_rolls_back_member_and_identifier_when_placement_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        conn.execute_batch("CREATE TRIGGER fail_placement BEFORE INSERT ON member_placements BEGIN SELECT RAISE(ABORT, 'placement failure'); END").unwrap();
        assert!(
            register_member(&mut conn, fleet, "worker", "assignment", &placement(None))
                .unwrap_err()
                .message()
                .contains("placement failure")
        );
        assert_eq!(list_members(&conn, fleet).unwrap().len(), 1);
        conn.execute_batch("DROP TRIGGER fail_placement").unwrap();
        assert_eq!(register(&mut conn, fleet, "worker", None), director + 1);
    }

    #[test]
    fn live_fleet_and_director_checks_guard_member_registration() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        assert!(register_member(&mut conn, 999, "x", "assignment", &placement(None)).is_err());
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        assert!(deregister_member(&mut conn, director).is_err());
        crate::broker::fleets::delete_fleet_rows(&conn, fleet).unwrap();
        assert!(
            register_member(&mut conn, fleet, "worker", "assignment", &placement(None)).is_err()
        );
    }

    #[test]
    fn deregistration_is_fleet_scoped_and_keeps_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        let (other, _) = create_fleet_fixture(&mut conn, "other");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = broker::send_message(&mut conn, director, worker, "history").unwrap();
        assert!(get_member(&conn, worker, other).unwrap().is_none());
        assert!(deregister_member(&mut conn, worker).unwrap());
        assert!(!deregister_member(&mut conn, worker).unwrap());
        assert!(get_member(&conn, worker, fleet).unwrap().is_none());
        assert_eq!(
            broker::get_message(&conn, message.message_id).unwrap(),
            message
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM member_placements WHERE member_id=?1",
                [worker],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT status FROM members WHERE member_id=?1",
                [worker],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "deregistered"
        );
    }

    #[test]
    fn roster_preserves_all_placement_states_and_activity() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "activity");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let pending = register(&mut conn, fleet, "pending", None);
        let message = broker::send_message(&mut conn, director, worker, "hello").unwrap();
        let rows = list_members(&conn, fleet).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.member.member_id)
                .collect::<Vec<_>>(),
            [director, worker, pending]
        );
        let sender = &rows[0];
        assert_eq!(sender.member.kind, MemberKind::Director);
        assert_eq!(
            sender.last_sent.as_deref(),
            Some(message.created_at.as_str())
        );
        assert_eq!(sender.last_recv, None);
        assert!((0..=5).contains(&sender.idle.unwrap()));
        assert_eq!(
            rows[1].last_recv.as_deref(),
            Some(message.created_at.as_str())
        );
        assert_eq!(rows[1].last_ack, None);
        assert_eq!(rows[2].member.placement.mux_pane_id, None);
        assert_eq!(
            (
                rows[2].last_sent.clone(),
                rows[2].last_recv.clone(),
                rows[2].idle
            ),
            (None, None, None)
        );
        let ack = broker::ack_message(&mut conn, message.message_id).unwrap();
        assert_eq!(
            list_members(&conn, fleet).unwrap()[1].last_ack,
            Some(ack.status_timestamp)
        );
        conn.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            params![worker, fleet],
        )
        .unwrap();
        let rows = list_members(&conn, fleet).unwrap();
        assert_eq!(rows[0].member.kind, MemberKind::Member);
        assert_eq!(rows[1].member.kind, MemberKind::Director);
        deregister_member(&mut conn, pending).unwrap();
        assert_eq!(list_members(&conn, fleet).unwrap().len(), 2);
    }

    fn activity(sent: Option<&str>, received: Option<&str>, ack: Option<&str>) -> MemberActivity {
        MemberActivity {
            member: MemberRecord {
                member_id: 1,
                fleet_id: 1,
                name: "worker".into(),
                description: String::new(),
                registered_at: String::new(),
                status: MemberStatus::Active,
                kind: MemberKind::Member,
                placement: Placement {
                    backend: "tmux".into(),
                    mux_session: "main".into(),
                    mux_window_id: None,
                    mux_window_name: None,
                    window_ordinal: 0,
                    column_side: crate::multiplexer::layout::ColumnSide::Right,
                    mux_pane_id: None,
                    coding_agent: "claude".into(),
                    created_at: String::new(),
                    mux_server: "/nonexistent/cafleet-test/tmux.sock".into(),
                },
                runtime: None,
            },
            last_sent: sent.map(str::to_owned),
            last_recv: received.map(str::to_owned),
            last_ack: ack.map(str::to_owned),
            idle: None,
        }
    }

    #[test]
    fn idle_clamps_future_activity_to_zero() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        let row = activity(None, None, Some("2026-01-01T00:01:00Z"));
        assert_eq!(idle_seconds(&row, now).unwrap(), Some(0));
    }

    #[test]
    fn idle_is_absent_without_activity_and_refuses_a_corrupt_latest_timestamp() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(
            idle_seconds(&activity(None, None, None), now).unwrap(),
            None
        );
        for (row, field) in [
            (
                activity(Some("z-invalid"), Some("2025-12-31T23:59:50Z"), None),
                "messages.created_at",
            ),
            (
                activity(None, Some("2025-12-31T23:59:50Z"), Some("z-invalid")),
                "messages.status_timestamp",
            ),
        ] {
            assert!(
                matches!(
                    idle_seconds(&row, now),
                    Err(CafleetError::InvalidStoredValue { field: actual, value })
                        if actual == field && value == "z-invalid"
                ),
                "{field}"
            );
        }
    }

    #[test]
    fn idle_selects_raw_lexicographic_maximum_before_parsing_offsets() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        let row = activity(
            Some("2026-01-01T08:00:00+09:00"),
            Some("2025-12-31T23:59:50Z"),
            None,
        );
        assert_eq!(idle_seconds(&row, now).unwrap(), Some(3600));
    }

    mod occupancy {
        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::members::test_support::deregister_member;
        use crate::broker::members::test_support::register_in_slot;
        use crate::broker::members::*;
        use crate::db::test_support::migrated_conn;
        use crate::multiplexer::layout::ColumnSide;

        /// `(ordinal, window ID, window name, left panes, right panes)`.
        type WindowSummary<'a> = (i64, &'a str, &'a str, Vec<&'a str>, Vec<&'a str>);

        fn summary(windows: &[OccupiedWindow]) -> Vec<WindowSummary<'_>> {
            windows
                .iter()
                .map(|window| {
                    (
                        window.window_ordinal,
                        window.mux_window_id.as_str(),
                        window.mux_window_name.as_str(),
                        window.left.iter().map(String::as_str).collect(),
                        window.right.iter().map(String::as_str).collect(),
                    )
                })
                .collect()
        }

        #[test]
        fn occupancy_groups_concrete_placements_by_ordinal_and_side_in_registry_order() {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "layout");
            for (name, ordinal, side, concrete) in [
                ("a", 0, ColumnSide::Right, ("@1", "main", "%2")),
                ("b", 1, ColumnSide::Left, ("@7", "cafleet-1-1", "%10")),
                ("c", 0, ColumnSide::Right, ("@1", "main", "%3")),
                ("d", 1, ColumnSide::Right, ("@7", "cafleet-1-1", "%12")),
                ("e", 1, ColumnSide::Left, ("@7", "cafleet-1-1", "%11")),
                ("f", 3, ColumnSide::Left, ("@9", "cafleet-1-3", "%20")),
            ] {
                register_in_slot(&mut conn, fleet, name, ordinal, side, Some(concrete));
            }
            let windows = concrete_occupancy(&conn, fleet).unwrap();
            assert_eq!(
                summary(&windows),
                vec![
                    (0, "@1", "main", vec![], vec!["%2", "%3"]),
                    (1, "@7", "cafleet-1-1", vec!["%10", "%11"], vec!["%12"]),
                    (3, "@9", "cafleet-1-3", vec!["%20"], vec![]),
                ]
            );
        }

        #[test]
        fn occupancy_excludes_the_director_pending_deregistered_and_other_fleet_rows() {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "layout");
            let (other, _) = create_fleet_fixture(&mut conn, "other");
            assert!(concrete_occupancy(&conn, fleet).unwrap().is_empty());
            register_in_slot(&mut conn, fleet, "pending", 1, ColumnSide::Left, None);
            let departed = register_in_slot(
                &mut conn,
                fleet,
                "departed",
                2,
                ColumnSide::Left,
                Some(("@8", "cafleet-1-2", "%30")),
            );
            deregister_member(&mut conn, departed).unwrap();
            register_in_slot(
                &mut conn,
                other,
                "foreign",
                1,
                ColumnSide::Left,
                Some(("@5", "cafleet-2-1", "%40")),
            );
            assert!(concrete_occupancy(&conn, fleet).unwrap().is_empty());
            register_in_slot(
                &mut conn,
                fleet,
                "live",
                0,
                ColumnSide::Right,
                Some(("@1", "main", "%2")),
            );
            assert_eq!(
                summary(&concrete_occupancy(&conn, fleet).unwrap()),
                vec![(0, "@1", "main", vec![], vec!["%2"])]
            );
        }

        #[test]
        fn window_members_lists_every_active_member_of_one_ordinal_ascending() {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "layout");
            let (other, _) = create_fleet_fixture(&mut conn, "other");
            let window = ("@7", "cafleet-1-1");
            let first = register_in_slot(
                &mut conn,
                fleet,
                "first",
                1,
                ColumnSide::Right,
                Some((window.0, window.1, "%11")),
            );
            register_in_slot(
                &mut conn,
                fleet,
                "director-window",
                0,
                ColumnSide::Right,
                Some(("@1", "main", "%2")),
            );
            let second = register_in_slot(
                &mut conn,
                fleet,
                "second",
                1,
                ColumnSide::Left,
                Some((window.0, window.1, "%10")),
            );
            let departed = register_in_slot(
                &mut conn,
                fleet,
                "departed",
                1,
                ColumnSide::Left,
                Some((window.0, window.1, "%12")),
            );
            deregister_member(&mut conn, departed).unwrap();
            register_in_slot(
                &mut conn,
                other,
                "foreign",
                1,
                ColumnSide::Left,
                Some(("@5", "cafleet-2-1", "%40")),
            );
            assert_eq!(
                window_member_ids(&conn, fleet, 1).unwrap(),
                vec![first, second]
            );
            assert!(
                !window_member_ids(&conn, fleet, 0)
                    .unwrap()
                    .contains(&director)
            );
            assert!(window_member_ids(&conn, fleet, 2).unwrap().is_empty());
        }

        #[test]
        fn a_stored_column_side_outside_left_and_right_fails_decoding() {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "layout");
            let worker = register_in_slot(
                &mut conn,
                fleet,
                "worker",
                0,
                ColumnSide::Right,
                Some(("@1", "main", "%2")),
            );
            conn.pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            conn.execute(
                "UPDATE member_placements SET column_side='middle' WHERE member_id=?1",
                [worker],
            )
            .unwrap();
            conn.pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
            for error in [
                get_member(&conn, worker, fleet).unwrap_err(),
                list_members(&conn, fleet).unwrap_err(),
            ] {
                assert!(
                    matches!(
                        &error,
                        CafleetError::InvalidStoredValue { field, value }
                            if field == "member_placements.column_side" && value == "middle"
                    ),
                    "{error:?}"
                );
            }
        }

        #[test]
        fn member_reads_decode_the_recorded_slot_and_window() {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "layout");
            let pending = register_in_slot(&mut conn, fleet, "pending", 2, ColumnSide::Left, None);
            let placed = register_in_slot(
                &mut conn,
                fleet,
                "placed",
                1,
                ColumnSide::Right,
                Some(("@7", "cafleet-1-1", "%11")),
            );
            let slot = |member: i64| {
                let placement = get_member(&conn, member, fleet).unwrap().unwrap().placement;
                (
                    placement.window_ordinal,
                    placement.column_side,
                    placement.mux_window_id,
                    placement.mux_window_name,
                    placement.mux_pane_id,
                )
            };
            assert_eq!(
                slot(director),
                (
                    0,
                    ColumnSide::Left,
                    Some("@1".into()),
                    Some("main".into()),
                    Some("%0".into())
                )
            );
            assert_eq!(slot(pending), (2, ColumnSide::Left, None, None, None));
            assert_eq!(
                slot(placed),
                (
                    1,
                    ColumnSide::Right,
                    Some("@7".into()),
                    Some("cafleet-1-1".into()),
                    Some("%11".into())
                )
            );
        }
    }

    #[test]
    fn idle_preserves_fractional_second_truncation() {
        let now = parse_lenient("2026-01-01T00:00:01.900000+00:00").unwrap();
        let row = activity(Some("2026-01-01T00:00:00.950000+00:00"), None, None);
        assert_eq!(idle_seconds(&row, now).unwrap(), Some(0));
        let row = activity(None, None, Some("2025-12-31T23:59:59.950000+00:00"));
        assert_eq!(idle_seconds(&row, now).unwrap(), Some(1));
    }
}
