use rusqlite::{Connection, OptionalExtension, params};

use super::members::db_err;
use super::records::{
    CreatedFleet, FleetRecord, FleetSummary, MemberKind, MemberRecord, MemberStatus, Placement,
};
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc};

const DIRECTOR_NAME: &str = "Director";
const DIRECTOR_DESCRIPTION: &str = "Root Director for this fleet";

pub(crate) fn fetch_fleet(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<FleetRecord>, CafleetError> {
    conn.query_row(
        "SELECT fleet_id, name, created_at, deleted_at, director_member_id FROM fleets WHERE fleet_id=?1",
        [fleet_id],
        |row| Ok(FleetRecord {
            fleet_id: row.get(0)?, name: row.get(1)?, created_at: row.get(2)?,
            deleted_at: row.get(3)?, director_member_id: row.get(4)?,
        }),
    ).optional().map_err(db_err)
}

#[allow(clippy::too_many_arguments)]
pub fn create_fleet(
    conn: &mut Connection,
    name: Option<&str>,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: &str,
    coding_agent: &str,
    backend: &str,
) -> Result<CreatedFleet, CafleetError> {
    let now = format_utc(now_utc());
    let tx = conn.transaction().map_err(db_err)?;
    let fleet_id = tx
        .query_row(
            "INSERT INTO fleets (name, created_at) VALUES (?1, ?2) RETURNING fleet_id",
            params![name, now],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    let director_id = tx
        .query_row(
            "INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4) RETURNING member_id",
            params![fleet_id, DIRECTOR_NAME, DIRECTOR_DESCRIPTION, now],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    tx.execute(
        "INSERT INTO member_placements
         (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            director_id,
            mux_session,
            mux_window_id,
            mux_pane_id,
            backend,
            coding_agent,
            now
        ],
    )
    .map_err(db_err)?;
    tx.execute(
        "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
        params![director_id, fleet_id],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(CreatedFleet {
        fleet_id,
        name: name.map(str::to_owned),
        created_at: now.clone(),
        director: MemberRecord {
            member_id: director_id,
            fleet_id,
            name: DIRECTOR_NAME.into(),
            description: DIRECTOR_DESCRIPTION.into(),
            registered_at: now.clone(),
            status: MemberStatus::Active,
            kind: MemberKind::Director,
            runtime: None,
            placement: Some(Placement {
                backend: backend.into(),
                mux_session: mux_session.into(),
                mux_window_id: mux_window_id.into(),
                mux_pane_id: Some(mux_pane_id.into()),
                coding_agent: coding_agent.into(),
                created_at: now,
            }),
        },
    })
}

pub(crate) fn require_live_fleet(
    conn: &Connection,
    fleet_id: i64,
) -> Result<FleetRecord, CafleetError> {
    let fleet = fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::App(format!("fleet {fleet_id} is deleted")));
    }
    let director = fleet.director_member_id.ok_or_else(|| CafleetError::App(format!(
        "fleet {fleet_id} has no root Director recorded; re-create the fleet with 'cafleet fleet create'."
    )))?;
    let status: Option<MemberStatus> = conn
        .query_row(
            "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2",
            params![director, fleet_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if status != Some(MemberStatus::Active) {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id}'s root Director (member {director}) is not active."
        )));
    }
    let pane: Option<Option<String>> = conn
        .query_row(
            "SELECT mux_pane_id FROM member_placements WHERE member_id=?1",
            [director],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if !matches!(pane, Some(Some(ref value)) if !value.is_empty()) {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id}'s root Director (member {director}) has no concrete placement."
        )));
    }
    Ok(fleet)
}

pub fn list_fleets(conn: &Connection) -> Result<Vec<FleetSummary>, CafleetError> {
    let mut stmt = conn
        .prepare(
            "SELECT f.fleet_id, f.name, f.created_at, f.director_member_id,
            (SELECT COUNT(*) FROM members m WHERE m.fleet_id=f.fleet_id AND m.status='active')
         FROM fleets f WHERE f.deleted_at IS NULL ORDER BY f.created_at DESC, f.fleet_id DESC",
        )
        .map_err(db_err)?;
    stmt.query_map([], |row| {
        Ok(FleetSummary {
            fleet_id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            director_member_id: row.get(3)?,
            member_count: row.get(4)?,
        })
    })
    .map_err(db_err)?
    .collect::<Result<_, _>>()
    .map_err(db_err)
}

pub fn get_fleet(conn: &Connection, fleet_id: i64) -> Result<Option<FleetRecord>, CafleetError> {
    fetch_fleet(conn, fleet_id)
}

pub(crate) fn delete_fleet_rows(conn: &Connection, fleet_id: i64) -> Result<usize, CafleetError> {
    let fleet = fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    let now = format_utc(now_utc());
    if fleet.deleted_at.is_none() {
        conn.execute(
            "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
            params![now, fleet_id],
        )
        .map_err(db_err)?;
    }
    let deregistered_count = conn.execute(
        "UPDATE members SET status='deregistered', deregistered_at=?1 WHERE fleet_id=?2 AND status='active'",
        params![now, fleet_id],
    ).map_err(db_err)?;
    conn.execute("DELETE FROM member_placements WHERE member_id IN (SELECT member_id FROM members WHERE fleet_id=?1)", [fleet_id]).map_err(db_err)?;
    Ok(deregistered_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker;
    use crate::broker::tests::{create_fleet, guarded_delete_fleet, migrated_conn, register};

    fn row_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn registration_is_atomic_at_every_stage_and_preserves_identity() {
        for trigger in [
            "BEFORE INSERT ON fleets",
            "BEFORE INSERT ON members",
            "BEFORE INSERT ON member_placements",
            "BEFORE UPDATE OF director_member_id ON fleets",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            conn.execute_batch(&format!("CREATE TRIGGER fail_registration {trigger} BEGIN SELECT RAISE(ABORT, 'registration failure'); END;")).unwrap();
            let error = broker::create_fleet(
                &mut conn,
                Some("alpha"),
                "main",
                "@1",
                "%0",
                "claude",
                "tmux",
            )
            .unwrap_err();
            assert!(error.message().contains("registration failure"));
            for table in ["fleets", "members", "member_placements"] {
                assert_eq!(row_count(&conn, table), 0);
            }
            conn.execute_batch("DROP TRIGGER fail_registration")
                .unwrap();
            let (fleet, director) = create_fleet(&mut conn, "alpha");
            assert_eq!((fleet, director), (1, 1));
            let stored = get_fleet(&conn, fleet).unwrap().unwrap();
            assert_eq!(stored.director_member_id, Some(director));
            let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
            assert_eq!(member.kind, MemberKind::Director);
            assert_eq!(member.placement.unwrap().mux_pane_id.as_deref(), Some("%0"));
        }
    }

    #[test]
    fn fleet_listing_uses_time_then_id_and_counts_only_active_members() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (first, _) = create_fleet(&mut conn, "first");
        let (second, _) = create_fleet(&mut conn, "second");
        let (third, _) = create_fleet(&mut conn, "third");
        let worker = register(&mut conn, first, "worker", None);
        conn.execute("UPDATE fleets SET created_at='2026-01-01T00:00:00Z'", [])
            .unwrap();
        conn.execute(
            "UPDATE fleets SET created_at='2026-01-02T00:00:00Z' WHERE fleet_id=?1",
            [first],
        )
        .unwrap();
        let rows = list_fleets(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.fleet_id).collect::<Vec<_>>(),
            [first, third, second]
        );
        assert_eq!(rows[0].member_count, 2);
        broker::deregister_member(&mut conn, worker).unwrap();
        assert_eq!(list_fleets(&conn).unwrap()[0].member_count, 1);
        guarded_delete_fleet(&mut conn, second).unwrap();
        assert_eq!(list_fleets(&conn).unwrap().len(), 2);
        assert!(
            get_fleet(&conn, second)
                .unwrap()
                .unwrap()
                .deleted_at
                .is_some()
        );
        assert!(get_fleet(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn deletion_is_atomic_soft_idempotent_and_preserves_messages() {
        for trigger in [
            "BEFORE UPDATE OF deleted_at ON fleets",
            "BEFORE UPDATE OF status ON members",
            "BEFORE DELETE ON member_placements",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet(&mut conn, "alpha");
            let worker = register(&mut conn, fleet, "worker", None);
            let message =
                broker::send_message(&mut conn, director, worker, "retained history").unwrap();
            let before = get_fleet(&conn, fleet).unwrap();
            let members =
                [director, worker].map(|id| broker::get_member(&conn, id, fleet).unwrap());
            conn.execute_batch(&format!("CREATE TRIGGER fail_deletion {trigger} BEGIN SELECT RAISE(ABORT, 'deletion failure'); END;")).unwrap();
            assert!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap_err()
                    .message()
                    .contains("deletion failure")
            );
            assert_eq!(get_fleet(&conn, fleet).unwrap(), before);
            for (id, member) in [director, worker].into_iter().zip(members) {
                assert_eq!(broker::get_member(&conn, id, fleet).unwrap(), member);
                assert_eq!(
                    conn.query_row(
                        "SELECT deregistered_at FROM members WHERE member_id=?1",
                        [id],
                        |row| row.get::<_, Option<String>>(0)
                    )
                    .unwrap(),
                    None
                );
            }
            conn.execute_batch("DROP TRIGGER fail_deletion").unwrap();
            assert_eq!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap()
                    .deregistered_count,
                2
            );
            assert_eq!(row_count(&conn, "member_placements"), 0);
            let deleted = get_fleet(&conn, fleet).unwrap();
            assert_eq!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap()
                    .deregistered_count,
                0
            );
            assert_eq!(get_fleet(&conn, fleet).unwrap(), deleted);
            assert_eq!(
                broker::get_message(&conn, message.message_id).unwrap(),
                message
            );
            assert!(guarded_delete_fleet(&mut conn, 999).is_err());
        }
    }
}
