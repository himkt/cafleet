#[cfg(test)]
use rusqlite::TransactionBehavior;
use rusqlite::{Connection, OptionalExtension, params};

use super::records::{MemberActivity, MemberKind, MemberRecord, Placement, RegisteredMember};
use crate::error::CafleetError;
use crate::time::{now_stamp, parse_lenient};

#[derive(Debug, Clone)]
pub(crate) struct NewPlacement {
    pub(crate) backend: String,
    pub(crate) mux_session: String,
    pub(crate) mux_window_id: String,
    pub(crate) mux_pane_id: Option<String>,
    pub(crate) coding_agent: String,
}

#[cfg(test)]
pub(crate) fn register_member(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    placement: Option<&NewPlacement>,
) -> Result<RegisteredMember, CafleetError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let member = register_member_rows(&tx, fleet_id, name, description, placement)?;
    tx.commit()?;
    Ok(member)
}

pub(crate) fn register_member_rows(
    conn: &Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    placement: Option<&NewPlacement>,
) -> Result<RegisteredMember, CafleetError> {
    let now = now_stamp();
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let member = conn.query_row(
        "INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4) RETURNING member_id, name, registered_at",
        params![fleet_id, name, description, now],
        |row| {
            Ok(RegisteredMember {
                member_id: row.get(0)?,
                name: row.get(1)?,
                registered_at: row.get(2)?,
            })
        },
    )?;
    let member_id = member.member_id;
    if let Some(p) = placement {
        conn.execute(
            "INSERT INTO member_placements
             (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                member_id,
                p.mux_session,
                p.mux_window_id,
                p.mux_pane_id,
                p.backend,
                p.coding_agent,
                now
            ],
        )?;
    }
    Ok(member)
}

pub(crate) fn get_member(
    conn: &Connection,
    member_id: i64,
    fleet_id: i64,
) -> Result<Option<MemberRecord>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let mut member = conn.query_row(
        "SELECT m.name, m.description, m.status, m.registered_at,
                f.director_member_id=m.member_id,
                p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, p.created_at
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         LEFT JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.member_id=?1 AND m.fleet_id=?2 AND m.status='active'",
        params![member_id, fleet_id],
        |row| {
            let backend: Option<String> = row.get(5)?;
            Ok(MemberRecord {
                member_id, fleet_id, name: row.get(0)?, description: row.get(1)?,
                status: row.get(2)?, registered_at: row.get(3)?,
                kind: if row.get(4)? { MemberKind::Director } else { MemberKind::Member },
                placement: backend.map(|_| Placement::from_row(row, 5)).transpose()?,
                runtime: None,
            })
        },
    ).optional()?;
    if let Some(member) = &mut member {
        member.runtime = super::runtime::member_runtime(conn, member_id)?;
    }
    Ok(member)
}

pub(crate) fn active_member_fleet(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<i64>, CafleetError> {
    conn.query_row(
        "SELECT fleet_id FROM members WHERE member_id=?1 AND status='active'",
        [member_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(crate::error::CafleetError::from)
}

#[cfg(test)]
pub(crate) fn deregister_member(
    conn: &mut Connection,
    member_id: i64,
) -> Result<bool, CafleetError> {
    let tx = conn.transaction()?;
    let changed = deregister_member_rows(&tx, member_id)?;
    tx.commit()?;
    Ok(changed)
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
    let now = now_stamp();
    let changed = conn.execute(
        "UPDATE members SET status='deregistered', deregistered_at=?1 \
             WHERE member_id=?2 AND status='active'",
        params![now, member_id],
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

#[cfg(test)]
pub(crate) fn update_placement_pane_id(
    conn: &mut Connection,
    member_id: i64,
    pane_id: &str,
) -> Result<Option<Placement>, CafleetError> {
    conn.query_row(
        "UPDATE member_placements SET mux_pane_id=?1 WHERE member_id=?2
         RETURNING backend, mux_session, mux_window_id, mux_pane_id, coding_agent, created_at",
        params![pane_id, member_id],
        |row| Placement::from_row(row, 0),
    )
    .optional()
    .map_err(crate::error::CafleetError::from)
}

fn roster_rows(conn: &Connection, fleet_id: i64) -> Result<Vec<MemberActivity>, CafleetError> {
    let mut stmt = conn.prepare(
        "SELECT m.member_id, m.name, m.status, f.director_member_id=m.member_id,
                p.backend, p.mux_session, p.mux_window_id, p.mux_pane_id, p.coding_agent, p.created_at,
                m.description, m.registered_at,
                (SELECT MAX(created_at) FROM messages WHERE from_member_id=m.member_id),
                (SELECT MAX(created_at) FROM messages WHERE to_member_id=m.member_id),
                (SELECT MAX(status_timestamp) FROM messages WHERE to_member_id=m.member_id AND status_state='completed')
         FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
         LEFT JOIN member_placements p ON p.member_id=m.member_id
         WHERE m.fleet_id=?1 AND m.status='active'
         ORDER BY m.member_id"
    )?;
    stmt.query_map([fleet_id], |row| {
        let backend: Option<String> = row.get(4)?;
        Ok(MemberActivity {
            member: MemberRecord {
                member_id: row.get(0)?,
                fleet_id,
                name: row.get(1)?,
                status: row.get(2)?,
                kind: if row.get(3)? {
                    MemberKind::Director
                } else {
                    MemberKind::Member
                },
                placement: backend.map(|_| Placement::from_row(row, 4)).transpose()?,
                runtime: None,
                description: row.get(10)?,
                registered_at: row.get(11)?,
            },
            last_sent: row.get(12)?,
            last_recv: row.get(13)?,
            last_ack: row.get(14)?,
            idle: None,
        })
    })?
    .collect::<Result<Vec<_>, _>>()
    .map_err(crate::error::CafleetError::from)
}

fn idle_seconds(row: &MemberActivity, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
    let latest = [&row.last_sent, &row.last_recv, &row.last_ack]
        .into_iter()
        .flatten()
        .max()?;
    let parsed = parse_lenient(latest).ok()?;
    Some((now - parsed).num_seconds().max(0))
}

pub(crate) fn list_members(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<MemberActivity>, CafleetError> {
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let now = chrono::Utc::now();
    let mut rows = roster_rows(conn, fleet_id)?;
    for row in &mut rows {
        row.idle = idle_seconds(row, now);
    }
    Ok(rows)
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;

    use super::NewPlacement;

    pub(crate) fn placement(pane: Option<&str>) -> NewPlacement {
        NewPlacement {
            backend: "tmux".into(),
            mux_session: "main".into(),
            mux_window_id: "@1".into(),
            mux_pane_id: pane.map(str::to_owned),
            coding_agent: "claude".into(),
        }
    }

    pub(crate) fn register(
        conn: &mut Connection,
        fleet: i64,
        name: &str,
        pane: Option<&str>,
    ) -> i64 {
        super::register_member(conn, fleet, name, "assignment", Some(&placement(pane)))
            .unwrap()
            .member_id
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{placement, register};
    use super::*;
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::records::MemberStatus;
    use crate::db::test_support::migrated_conn;
    use crate::deletion::test_support::guarded_delete_fleet;

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
            Some(&placement(None)),
        )
        .unwrap();
        assert_eq!(registered.name, "worker");
        assert!(parse_lenient(&registered.registered_at).is_ok());
        let member = get_member(&conn, registered.member_id, fleet)
            .unwrap()
            .unwrap();
        assert_eq!(member.description, "literal assignment");
        assert_eq!(member.kind, MemberKind::Member);
        assert_eq!(member.placement.unwrap().mux_pane_id, None);
        let updated = update_placement_pane_id(&mut conn, registered.member_id, "%9")
            .unwrap()
            .unwrap();
        assert_eq!(updated.mux_pane_id.as_deref(), Some("%9"));
        assert_eq!(
            get_member(&conn, registered.member_id, fleet)
                .unwrap()
                .unwrap()
                .placement,
            Some(updated)
        );
        let ghost = register_member(&mut conn, fleet, "ghost", "paneless", None)
            .unwrap()
            .member_id;
        assert!(
            get_member(&conn, ghost, fleet)
                .unwrap()
                .unwrap()
                .placement
                .is_none()
        );
        assert!(
            update_placement_pane_id(&mut conn, ghost, "%1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn registration_rolls_back_member_and_identifier_when_placement_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        conn.execute_batch("CREATE TRIGGER fail_placement BEFORE INSERT ON member_placements BEGIN SELECT RAISE(ABORT, 'placement failure'); END").unwrap();
        assert!(
            register_member(
                &mut conn,
                fleet,
                "worker",
                "assignment",
                Some(&placement(None))
            )
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
        assert!(register_member(&mut conn, 999, "x", "assignment", None).is_err());
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        for damage in [
            "UPDATE fleets SET director_member_id=NULL",
            "UPDATE members SET status='deregistered' WHERE member_id=1",
            "DELETE FROM member_placements WHERE member_id=1",
            "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=1",
        ] {
            conn.execute_batch("SAVEPOINT damage").unwrap();
            conn.execute_batch(damage).unwrap();
            assert!(
                register_member(&mut conn, fleet, "worker", "assignment", None).is_err(),
                "{damage}"
            );
            conn.execute_batch("ROLLBACK TO damage; RELEASE damage")
                .unwrap();
        }
        assert!(deregister_member(&mut conn, director).is_err());
        guarded_delete_fleet(&mut conn, fleet).unwrap();
        assert!(register_member(&mut conn, fleet, "worker", "assignment", None).is_err());
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
        assert!(
            conn.query_row(
                "SELECT deregistered_at FROM members WHERE member_id=?1",
                [worker],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn roster_preserves_all_placement_states_and_activity() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "activity");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let pending = register(&mut conn, fleet, "pending", None);
        let ghost = register_member(&mut conn, fleet, "ghost", "paneless", None)
            .unwrap()
            .member_id;
        let message = broker::send_message(&mut conn, director, worker, "hello").unwrap();
        let rows = list_members(&conn, fleet).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.member.member_id)
                .collect::<Vec<_>>(),
            [director, worker, pending, ghost]
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
        assert_eq!(rows[2].member.placement.as_ref().unwrap().mux_pane_id, None);
        assert!(rows[3].member.placement.is_none());
        assert_eq!(
            (
                rows[3].last_sent.clone(),
                rows[3].last_recv.clone(),
                rows[3].idle
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
        assert_eq!(list_members(&conn, fleet).unwrap().len(), 3);
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
                placement: None,
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
        for row in [
            activity(Some("2026-01-01T00:01:00Z"), None, None),
            activity(None, Some("2026-01-01T00:01:00Z"), None),
            activity(None, None, Some("2026-01-01T00:01:00Z")),
        ] {
            assert_eq!(idle_seconds(&row, now), Some(0));
        }
    }

    #[test]
    fn idle_is_absent_when_the_latest_timestamp_is_absent_or_invalid() {
        let now = parse_lenient("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(idle_seconds(&activity(None, None, None), now), None);
        for row in [
            activity(Some("z-invalid"), Some("2025-12-31T23:59:50Z"), None),
            activity(None, Some("z-invalid"), Some("2025-12-31T23:59:50Z")),
        ] {
            assert_eq!(idle_seconds(&row, now), None);
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
        assert_eq!(idle_seconds(&row, now), Some(3600));
    }

    #[test]
    fn idle_preserves_fractional_second_truncation() {
        let now = parse_lenient("2026-01-01T00:00:01.900000+00:00").unwrap();
        let row = activity(Some("2026-01-01T00:00:00.950000+00:00"), None, None);
        assert_eq!(idle_seconds(&row, now), Some(0));
        let row = activity(None, None, Some("2025-12-31T23:59:59.950000+00:00"));
        assert_eq!(idle_seconds(&row, now), Some(1));
    }
}
