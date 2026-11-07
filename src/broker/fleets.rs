use crate::broker::CreatedFleet;
use crate::broker::FleetRecord;
use crate::broker::FleetSummary;
use crate::broker::MemberKind;
use crate::broker::MemberRecord;
use crate::broker::MemberStatus;
use crate::broker::Placement;
use crate::broker::members::NewPlacement;
use crate::broker::members::insert_member_row;
use crate::broker::members::insert_placement_row;
use crate::error::CafleetError;
use crate::time::now_stamp;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;

const DIRECTOR_NAME: &str = "Director";
const DIRECTOR_DESCRIPTION: &str = "Root Director for this fleet";

pub(crate) fn get_fleet(
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
    ).optional().map_err(crate::error::CafleetError::from)
}

pub(crate) fn require_fleet(conn: &Connection, fleet_id: i64) -> Result<FleetRecord, CafleetError> {
    get_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))
}

/// Creates a fleet whose root Director occupies `director`, whose pane is required.
pub(crate) fn create_fleet(
    conn: &mut Connection,
    name: Option<&str>,
    director: &NewPlacement,
) -> Result<CreatedFleet, CafleetError> {
    let mux_pane_id = director
        .mux_pane_id
        .clone()
        .expect("the Director placement records its pane");
    let now = now_stamp();
    let tx = conn.transaction()?;
    let fleet_id = tx.query_row(
        "INSERT INTO fleets (name, created_at) VALUES (?1, ?2) RETURNING fleet_id",
        params![name, now],
        |row| row.get(0),
    )?;
    let director_id = insert_member_row(&tx, fleet_id, DIRECTOR_NAME, DIRECTOR_DESCRIPTION, &now)?;
    insert_placement_row(&tx, director_id, director, &now)?;
    tx.execute(
        "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
        params![director_id, fleet_id],
    )?;
    tx.commit()?;
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
                backend: director.backend.clone(),
                mux_session: director.mux_session.clone(),
                mux_window_id: director.mux_window_id.clone(),
                mux_pane_id: Some(mux_pane_id),
                coding_agent: director.coding_agent.clone(),
                created_at: now,
                mux_server: director.mux_server.clone(),
            }),
        },
    })
}

/// The recorded placement of `fleet_id`'s root Director.
pub(crate) fn director_placement(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Placement, CafleetError> {
    conn.query_row(
        &format!(
            "SELECT {} FROM fleets f JOIN member_placements p ON p.member_id=f.director_member_id
             WHERE f.fleet_id=?1",
            crate::broker::PLACEMENT_COLUMNS
        ),
        [fleet_id],
        |row| Placement::from_row(row, 0),
    )
    .optional()?
    .ok_or_else(|| {
        CafleetError::App(format!(
            "fleet {fleet_id} has no recorded Director placement; re-create the fleet with 'cafleet fleet create'."
        ))
    })
}

pub(crate) fn require_live_fleet(
    conn: &Connection,
    fleet_id: i64,
) -> Result<FleetRecord, CafleetError> {
    let fleet = require_fleet(conn, fleet_id)?;
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
        .optional()?;
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
        .optional()?;
    if !matches!(pane, Some(Some(ref value)) if !value.is_empty()) {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id}'s root Director (member {director}) has no concrete placement."
        )));
    }
    Ok(fleet)
}

pub(crate) fn list_fleets(conn: &Connection) -> Result<Vec<FleetSummary>, CafleetError> {
    let mut stmt = conn.prepare(
        "SELECT f.fleet_id, f.name, f.created_at, f.director_member_id,
            (SELECT COUNT(*) FROM members m WHERE m.fleet_id=f.fleet_id AND m.status='active')
         FROM fleets f WHERE f.deleted_at IS NULL ORDER BY f.created_at DESC, f.fleet_id DESC",
    )?;
    stmt.query_map([], |row| {
        Ok(FleetSummary {
            fleet_id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            director_member_id: row.get(3)?,
            member_count: row.get(4)?,
        })
    })?
    .collect::<Result<_, _>>()
    .map_err(crate::error::CafleetError::from)
}

pub(crate) fn delete_fleet_rows(conn: &Connection, fleet_id: i64) -> Result<usize, CafleetError> {
    let fleet = require_fleet(conn, fleet_id)?;
    let now = now_stamp();
    if fleet.deleted_at.is_none() {
        conn.execute(
            "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
            params![now, fleet_id],
        )?;
    }
    let deregistered_count = conn.execute(
        "UPDATE members SET status='deregistered', deregistered_at=?1 WHERE fleet_id=?2 AND status='active'",
        params![now, fleet_id],
    )?;
    conn.execute("DELETE FROM member_placements WHERE member_id IN (SELECT member_id FROM members WHERE fleet_id=?1)", [fleet_id])?;
    Ok(deregistered_count)
}

pub(crate) struct MemberRunRegistry {
    pub(crate) run_id: i64,
    pub(crate) member_id: i64,
    pub(crate) run_status: crate::broker::runtime::RunStatus,
    pub(crate) cleanup_state: crate::broker::runtime::CleanupState,
    pub(crate) member_status: String,
    pub(crate) director_member_id: Option<i64>,
}

impl MemberRunRegistry {
    pub(crate) fn is_settled(&self) -> bool {
        self.run_status.terminal()
            && self.cleanup_state == crate::broker::runtime::CleanupState::Complete
    }
}

pub(crate) fn member_run_registry(
    conn: &Connection,
) -> Result<Vec<MemberRunRegistry>, CafleetError> {
    let mut statement = conn.prepare(
        "SELECT r.run_id, r.member_id, r.status, r.cleanup_state, m.status, f.director_member_id \
         FROM member_runs r JOIN members m ON m.member_id = r.member_id \
         JOIN fleets f ON f.fleet_id = m.fleet_id ORDER BY r.run_id",
    )?;
    let runs = statement
        .query_map([], |row| {
            Ok(MemberRunRegistry {
                run_id: row.get(0)?,
                member_id: row.get(1)?,
                run_status: row.get(2)?,
                cleanup_state: row.get(3)?,
                member_status: row.get(4)?,
                director_member_id: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(runs)
}

pub(crate) fn recorded_asset_install(
    conn: &Connection,
    coding_agent: &str,
    path: &str,
) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT cafleet_version, installed_at FROM asset_installs WHERE coding_agent=?1 AND path=?2",
        params![coding_agent, path],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional()
}

pub(crate) fn record_asset_install(
    conn: &mut Connection,
    coding_agent: &str,
    path: &str,
    cafleet_version: &str,
) -> Result<(), CafleetError> {
    let now = now_stamp();
    conn.execute(
        "INSERT INTO asset_installs (coding_agent, path, cafleet_version, installed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(coding_agent, path) DO UPDATE SET
             cafleet_version=excluded.cafleet_version, installed_at=excluded.installed_at",
        params![coding_agent, path, cafleet_version, now],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;

    pub(crate) fn create_fleet_fixture(conn: &mut Connection, name: &str) -> (i64, i64) {
        let fleet = crate::broker::fleets::create_fleet(
            conn,
            Some(name),
            &crate::broker::members::test_support::placement(Some("%0")),
        )
        .unwrap();
        (fleet.fleet_id, fleet.director.member_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::fleets::*;
    use crate::broker::members::test_support::placement;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;

    fn row_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn registration_is_atomic_at_every_stage_and_preserves_identity() {
        let trigger = "BEFORE UPDATE OF director_member_id ON fleets";
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        conn.execute_batch(&format!("CREATE TRIGGER fail_registration {trigger} BEGIN SELECT RAISE(ABORT, 'registration failure'); END;")).unwrap();
        let error =
            broker::create_fleet(&mut conn, Some("alpha"), &placement(Some("%0"))).unwrap_err();
        assert!(error.message().contains("registration failure"));
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
        conn.execute_batch("DROP TRIGGER fail_registration")
            .unwrap();
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        assert_eq!((fleet, director), (1, 1));
        let stored = get_fleet(&conn, fleet).unwrap().unwrap();
        assert_eq!(stored.director_member_id, Some(director));
        let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
        assert_eq!(member.kind, MemberKind::Director);
        assert_eq!(member.placement.unwrap().mux_pane_id.as_deref(), Some("%0"));
    }

    #[test]
    fn fleet_listing_uses_time_then_id_and_counts_only_active_members() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (first, _) = create_fleet_fixture(&mut conn, "first");
        let (second, _) = create_fleet_fixture(&mut conn, "second");
        let (third, _) = create_fleet_fixture(&mut conn, "third");
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
        crate::broker::members::test_support::deregister_member(&mut conn, worker).unwrap();
        assert_eq!(list_fleets(&conn).unwrap()[0].member_count, 1);
        delete_fleet_rows(&conn, second).unwrap();
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
    fn install_record_upsert_changes_only_the_selected_agent_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&dir);
        for (agent, path) in [("codex", "/a"), ("codex", "/b"), ("claude", "/a")] {
            record_asset_install(&mut conn, agent, path, "old").unwrap();
        }
        conn.execute("UPDATE asset_installs SET installed_at='old-time'", [])
            .unwrap();
        record_asset_install(&mut conn, "codex", "/a", "new").unwrap();
        let rows = conn.prepare("SELECT coding_agent, path, cafleet_version, installed_at FROM asset_installs ORDER BY coding_agent, path")
            .unwrap().query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)))
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            (
                "claude".into(),
                "/a".into(),
                "old".into(),
                "old-time".into()
            )
        );
        assert_eq!(
            (&rows[1].0, &rows[1].1, &rows[1].2),
            (&"codex".into(), &"/a".into(), &"new".into())
        );
        assert!(crate::time::parse_lenient(&rows[1].3).is_ok());
        assert_eq!(
            rows[2],
            ("codex".into(), "/b".into(), "old".into(), "old-time".into())
        );
    }

    #[test]
    fn member_run_registry_reads_run_and_member_states_in_ascending_run_order() {
        use crate::broker::runtime::CleanupState;
        use crate::broker::runtime::RunStatus;
        use crate::broker::runtime::test_support::register_member_run;

        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "registry-read");
        let first = register_member_run(&mut conn, fleet, director, "first");
        let second = register_member_run(&mut conn, fleet, director, "second");
        conn.execute(
            "UPDATE member_runs SET status='exited', cleanup_state='complete' WHERE run_id=?1",
            [first.run_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
            [second.run_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [second.member_id],
        )
        .unwrap();
        let rows = member_run_registry(&conn).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.run_id).collect::<Vec<_>>(),
            [first.run_id, second.run_id]
        );
        assert_eq!(rows[0].member_id, first.member_id);
        assert_eq!(rows[0].run_status, RunStatus::Exited);
        assert_eq!(rows[0].cleanup_state, CleanupState::Complete);
        assert_eq!(rows[0].member_status, "active");
        assert_eq!(rows[1].member_id, second.member_id);
        assert_eq!(rows[1].run_status, RunStatus::Starting);
        assert_eq!(rows[1].cleanup_state, CleanupState::Incomplete);
        assert_eq!(rows[1].member_status, "deregistered");
        assert!(
            rows.iter()
                .all(|row| row.director_member_id == Some(director))
        );
        let (other_fleet, other_director) = create_fleet_fixture(&mut conn, "other");
        let third = register_member_run(&mut conn, other_fleet, other_director, "third");
        let rows = member_run_registry(&conn).unwrap();
        assert_eq!(rows[2].run_id, third.run_id);
        assert_eq!(rows[2].director_member_id, Some(other_director));
        assert!(
            member_run_registry(&Connection::open_in_memory().unwrap()).is_err(),
            "a registry without member runs surfaces the query error"
        );
    }

    #[test]
    fn require_live_fleet_refuses_a_deleted_fleet_or_a_damaged_root_director() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        assert_eq!(require_live_fleet(&conn, fleet).unwrap().fleet_id, fleet);
        for (damage, expected) in [
            (
                "UPDATE fleets SET director_member_id=NULL".to_owned(),
                format!(
                    "fleet {fleet} has no root Director recorded; re-create the fleet with 'cafleet fleet create'."
                ),
            ),
            (
                format!("UPDATE members SET status='deregistered' WHERE member_id={director}"),
                format!("fleet {fleet}'s root Director (member {director}) is not active."),
            ),
            (
                format!("DELETE FROM member_placements WHERE member_id={director}"),
                format!(
                    "fleet {fleet}'s root Director (member {director}) has no concrete placement."
                ),
            ),
            (
                format!("UPDATE member_placements SET mux_pane_id=NULL WHERE member_id={director}"),
                format!(
                    "fleet {fleet}'s root Director (member {director}) has no concrete placement."
                ),
            ),
            (
                "UPDATE fleets SET deleted_at='2026-01-01T00:00:00Z'".to_owned(),
                format!("fleet {fleet} is deleted"),
            ),
        ] {
            conn.execute_batch("SAVEPOINT damage").unwrap();
            conn.execute_batch(&damage).unwrap();
            assert_eq!(
                require_live_fleet(&conn, fleet).unwrap_err().message(),
                expected,
                "{damage}"
            );
            conn.execute_batch("ROLLBACK TO damage; RELEASE damage")
                .unwrap();
        }
    }

    #[test]
    fn director_placement_reads_the_recorded_director_pane_and_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, _) = create_fleet_fixture(&mut conn, "alpha");
        register(&mut conn, fleet, "worker", Some("%5"));
        let expected = placement(Some("%0"));
        let recorded = director_placement(&conn, fleet).unwrap();
        assert_eq!(
            (
                recorded.backend.as_str(),
                recorded.mux_session.as_str(),
                recorded.mux_window_id.as_str(),
                recorded.mux_pane_id.as_deref(),
                recorded.coding_agent.as_str(),
                recorded.mux_server.as_str(),
            ),
            (
                expected.backend.as_str(),
                expected.mux_session.as_str(),
                expected.mux_window_id.as_str(),
                Some("%0"),
                expected.coding_agent.as_str(),
                expected.mux_server.as_str(),
            )
        );
        assert_eq!(
            director_placement(&conn, 999).unwrap_err().message(),
            "fleet 999 has no recorded Director placement; re-create the fleet with 'cafleet fleet create'."
        );
    }
}
