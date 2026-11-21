use crate::broker::CreatedFleet;
use crate::broker::FleetRecord;
use crate::broker::FleetSummary;
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
            deleted_at: row.get(3)?,
            director_member_id: row
                .get::<_, Option<i64>>(4)?
                .expect("a committed fleet records its root Director"),
        }),
    ).optional().map_err(crate::error::CafleetError::from)
}

pub(crate) fn require_fleet(conn: &Connection, fleet_id: i64) -> Result<FleetRecord, CafleetError> {
    get_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))
}

/// Creates a fleet whose root Director occupies `director`, whose pane is required.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "database-only registration API for fixtures and native verification"
    )
)]
pub(crate) fn create_fleet(
    conn: &mut Connection,
    name: &str,
    director: &NewPlacement,
) -> Result<CreatedFleet, CafleetError> {
    create_fleet_with(conn, name, |_| Ok(director.clone()))
}

pub(crate) fn create_fleet_with(
    conn: &mut Connection,
    name: &str,
    director: impl FnOnce(i64) -> Result<NewPlacement, CafleetError>,
) -> Result<CreatedFleet, CafleetError> {
    let now = now_stamp();
    let tx = conn.transaction()?;
    let result = (|| {
        let fleet_id = tx.query_row(
            "INSERT INTO fleets (name, created_at) VALUES (?1, ?2) RETURNING fleet_id",
            params![name, now],
            |row| row.get(0),
        )?;
        let director = director(fleet_id)?;
        assert!(
            director.pane.is_some()
                && director.window_ordinal == 0
                && director.column_side == crate::multiplexer::layout::ColumnSide::Left,
            "the Director placement records its pane in the Director window's left column"
        );
        let director_id =
            insert_member_row(&tx, fleet_id, DIRECTOR_NAME, DIRECTOR_DESCRIPTION, &now)?;
        insert_placement_row(&tx, director_id, &director, &now)?;
        tx.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            params![director_id, fleet_id],
        )?;
        let director = crate::broker::members::get_member(&tx, director_id, fleet_id)?
            .expect("the created root Director is active");
        let created = CreatedFleet {
            fleet_id,
            name: name.to_owned(),
            created_at: now,
            director,
        };
        tx.execute_batch("COMMIT").map_err(|error| {
            let error = CafleetError::from(error);
            if tx.is_autocommit() {
                error.with_diagnostic(
                    "fleet registration outcome is unconfirmed: SQLite ended the transaction after a failed commit",
                )
            } else {
                error
            }
        })?;
        Ok(created)
    })();
    let error = match result {
        Ok(created) => return Ok(created),
        Err(error) => error,
    };
    let cleanup = if tx.is_autocommit() {
        Ok(())
    } else {
        tx.rollback().map_err(|error| {
            CafleetError::App(format!("rolling back fleet registration failed: {error}"))
        })
    };
    CafleetError::merge_cleanup(Err(error), cleanup)
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
    let director = fleet.director_member_id;
    let director_status: Option<MemberStatus> = conn
        .query_row(
            "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2",
            params![director, fleet_id],
            |row| row.get(0),
        )
        .optional()?;
    match director_status {
        Some(MemberStatus::Active) => Ok(fleet),
        _ => Err(CafleetError::App(format!(
            "fleet {fleet_id}'s root Director (member {director}) is not active."
        ))),
    }
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
            director_member_id: row
                .get::<_, Option<i64>>(3)?
                .expect("a committed fleet records its root Director"),
            member_count: row.get(4)?,
        })
    })?
    .collect::<Result<_, _>>()
    .map_err(crate::error::CafleetError::from)
}

pub(crate) fn delete_fleet_rows(conn: &Connection, fleet_id: i64) -> Result<usize, CafleetError> {
    require_fleet(conn, fleet_id)?;
    let now = now_stamp();
    conn.execute(
        "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2 AND deleted_at IS NULL",
        params![now, fleet_id],
    )?;
    let deregistered_count = conn.execute(
        "UPDATE members SET status='deregistered' WHERE fleet_id=?1 AND status='active'",
        [fleet_id],
    )?;
    conn.execute("DELETE FROM member_placements WHERE member_id IN (SELECT member_id FROM members WHERE fleet_id=?1)", [fleet_id])?;
    Ok(deregistered_count)
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;

    pub(crate) fn create_fleet_fixture(conn: &mut Connection, name: &str) -> (i64, i64) {
        let fleet = crate::broker::fleets::create_fleet(
            conn,
            name,
            &crate::broker::members::test_support::director_new_placement("%0"),
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
    use crate::broker::members::test_support::director_new_placement;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use crate::multiplexer::layout::ColumnSide;

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
            let calls = std::cell::Cell::new(0);
            let error = create_fleet_with(&mut conn, "alpha", |fleet_id| {
                assert_eq!(fleet_id, 1);
                calls.set(calls.get() + 1);
                Ok(director_new_placement("%0"))
            })
            .unwrap_err();
            assert!(error.message().contains("registration failure"), "{error}");
            assert_eq!(
                calls.get(),
                usize::from(trigger != "BEFORE INSERT ON fleets")
            );
            assert!(conn.is_autocommit());
            for table in ["fleets", "members", "member_placements"] {
                assert_eq!(row_count(&conn, table), 0, "{trigger}: {table}");
            }
            conn.execute_batch("DROP TRIGGER fail_registration")
                .unwrap();
            let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
            assert_eq!((fleet, director), (1, 1));
            let stored = get_fleet(&conn, fleet).unwrap().unwrap();
            assert_eq!(stored.director_member_id, director);
            let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
            assert_eq!(member.kind, crate::broker::MemberKind::Director);
            assert_eq!(member.placement.mux_pane_id.as_deref(), Some("%0"));
        }
    }

    #[test]
    fn placement_factory_receives_the_allocated_id_once_and_preserves_existing_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (legacy, _) = create_fleet_fixture(&mut conn, "legacy");
        let seen = std::cell::RefCell::new(Vec::new());
        let created = create_fleet_with(&mut conn, "alpha", |fleet_id| {
            seen.borrow_mut().push(fleet_id);
            let mut placement = director_new_placement("%5");
            placement.pane.as_mut().unwrap().mux_window_name = format!("cafleet-{fleet_id}-0");
            Ok(placement)
        })
        .unwrap();
        assert_eq!(*seen.borrow(), [2]);
        assert_eq!(created.fleet_id, 2);
        assert_eq!(created.director.name, "Director");
        assert_eq!(
            created.director.placement.mux_window_name.as_deref(),
            Some("cafleet-2-0")
        );
        assert_eq!(
            director_placement(&conn, created.fleet_id).unwrap(),
            created.director.placement
        );
        assert_eq!(
            director_placement(&conn, legacy)
                .unwrap()
                .mux_window_name
                .as_deref(),
            Some("main")
        );
        assert!(conn.is_autocommit());
    }

    #[test]
    fn placement_factory_failure_rolls_back_registration_and_allows_id_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let calls = std::cell::Cell::new(0);
        let error = create_fleet_with(&mut conn, "alpha", |fleet_id| {
            assert_eq!(fleet_id, 1);
            calls.set(calls.get() + 1);
            Err(CafleetError::App("placement factory failure".into()))
        })
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert!(error.message().contains("placement factory failure"));
        assert!(conn.is_autocommit());
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
        assert_eq!(create_fleet_fixture(&mut conn, "retry").0, 1);
    }

    #[test]
    fn both_registration_entry_points_assert_the_director_placement_contract() {
        for factory in [false, true] {
            for invalid in 0..3 {
                let dir = tempfile::tempdir().unwrap();
                let mut conn = migrated_conn(&dir);
                let mut placement = director_new_placement("%0");
                match invalid {
                    0 => placement.pane = None,
                    1 => placement.window_ordinal = 1,
                    2 => placement.column_side = ColumnSide::Right,
                    _ => unreachable!(),
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if factory {
                        create_fleet_with(&mut conn, "alpha", |_| Ok(placement))
                    } else {
                        create_fleet(&mut conn, "alpha", &placement)
                    }
                }));
                assert!(result.is_err(), "factory={factory}, invalid={invalid}");
                assert!(conn.is_autocommit());
                for table in ["fleets", "members", "member_placements"] {
                    assert_eq!(row_count(&conn, table), 0);
                }
            }
        }
    }

    #[test]
    fn transaction_acquisition_failure_keeps_the_placement_factory_uninvoked() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        conn.authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Begin
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        let result = create_fleet_with(&mut conn, "alpha", |_| {
            panic!("a transaction is acquired before invoking the placement factory")
        });
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        let error = result.unwrap_err();
        assert!(error.message().contains("not authorized"), "{error}");
        assert!(conn.is_autocommit());
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
    }

    #[test]
    fn placement_failure_and_rollback_failure_both_remain_visible() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        conn.authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        let result = create_fleet_with(&mut conn, "alpha", |_| {
            Err(CafleetError::App("placement factory failure".into()))
        });
        let transaction_open = !conn.is_autocommit();
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        if transaction_open {
            conn.execute_batch("ROLLBACK").unwrap();
        }
        let error = result.unwrap_err();
        assert!(
            error.message().contains("placement factory failure"),
            "{error}"
        );
        assert!(error.message().contains("not authorized"), "{error}");
        assert!(transaction_open);
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
    }

    #[test]
    fn result_construction_failure_rolls_back_before_any_commit_attempt() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let commits = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&commits);
        conn.commit_hook(Some(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            false
        }))
        .unwrap();
        conn.authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "member_placements",
                    column_name: "backend"
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        let result = create_fleet_with(&mut conn, "alpha", |_| Ok(director_new_placement("%0")));
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        conn.commit_hook(None::<fn() -> bool>).unwrap();
        let error = result.unwrap_err();
        assert!(error.message().contains("prohibited"), "{error}");
        assert_eq!(commits.load(Ordering::SeqCst), 0);
        assert!(conn.is_autocommit());
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
    }

    #[test]
    fn a_failed_commit_resolves_an_active_transaction_and_preserves_rollback_errors() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        use std::sync::{Arc, Mutex};

        for deny_rollback in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&attempts);
            conn.authorizer(Some(move |context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Unknown,
                } => {
                    observed.lock().unwrap().push("commit");
                    Authorization::Deny
                }
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => {
                    observed.lock().unwrap().push("rollback");
                    if deny_rollback {
                        Authorization::Deny
                    } else {
                        Authorization::Allow
                    }
                }
                _ => Authorization::Allow,
            }))
            .unwrap();
            let result =
                create_fleet_with(&mut conn, "alpha", |_| Ok(director_new_placement("%0")));
            let transaction_open = !conn.is_autocommit();
            conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            if transaction_open {
                conn.execute_batch("ROLLBACK").unwrap();
            }
            let error = result.unwrap_err();
            let diagnostic = error.message();
            assert!(diagnostic.contains("not authorized"), "{diagnostic}");
            let attempts = attempts.lock().unwrap();
            assert_eq!(attempts.first(), Some(&"commit"));
            assert!(attempts[1..].contains(&"rollback"), "{attempts:?}");
            assert_eq!(transaction_open, deny_rollback);
            if deny_rollback {
                assert!(
                    diagnostic.to_lowercase().contains("rollback")
                        || diagnostic.contains("rolling back"),
                    "{diagnostic}"
                );
                assert!(
                    diagnostic.matches("not authorized").count() >= 2,
                    "{diagnostic}"
                );
            }
            for table in ["fleets", "members", "member_placements"] {
                assert_eq!(row_count(&conn, table), 0);
            }
        }
    }

    #[test]
    fn a_failed_commit_with_an_ended_transaction_reports_an_unconfirmed_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        conn.commit_hook(Some(|| true)).unwrap();
        let result = create_fleet_with(&mut conn, "alpha", |_| Ok(director_new_placement("%0")));
        conn.commit_hook(None::<fn() -> bool>).unwrap();
        let error = result.unwrap_err();
        assert!(error.message().contains("unconfirmed"), "{error}");
        assert!(conn.is_autocommit());
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(&conn, table), 0);
        }
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
    fn require_live_fleet_refuses_a_deleted_fleet_or_an_inactive_root_director() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        assert_eq!(require_live_fleet(&conn, fleet).unwrap().fleet_id, fleet);
        for (damage, expected) in [
            (
                format!("UPDATE members SET status='deregistered' WHERE member_id={director}"),
                format!("fleet {fleet}'s root Director (member {director}) is not active."),
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
        let expected = director_new_placement("%0");
        let recorded = director_placement(&conn, fleet).unwrap();
        assert_eq!(
            (
                recorded.backend.as_str(),
                recorded.mux_session.as_str(),
                recorded.mux_window_id.as_deref(),
                recorded.mux_window_name.as_deref(),
                recorded.window_ordinal,
                recorded.column_side,
                recorded.mux_pane_id.as_deref(),
                recorded.coding_agent.as_str(),
                recorded.mux_server.as_str(),
            ),
            (
                expected.backend.as_str(),
                expected.mux_session.as_str(),
                Some("@1"),
                Some("main"),
                0,
                ColumnSide::Left,
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

    #[test]
    fn bootstrap_records_the_director_window_as_ordinal_zero_left_and_starts_the_ordinal_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let director = NewPlacement::director(
            "herdr".into(),
            "w4".into(),
            "codex".into(),
            "work".into(),
            crate::broker::members::PlacementPane {
                mux_window_id: "w4:t1".into(),
                mux_window_name: "agents".into(),
                mux_pane_id: "w4:p3N".into(),
            },
        );
        assert_eq!(
            (director.window_ordinal, director.column_side),
            (0, ColumnSide::Left)
        );
        let fleet = create_fleet(&mut conn, "alpha", &director).unwrap();
        let placement = &fleet.director.placement;
        assert_eq!(
            (
                placement.window_ordinal,
                placement.column_side,
                placement.mux_window_id.as_deref(),
                placement.mux_window_name.as_deref(),
                placement.mux_pane_id.as_deref(),
            ),
            (
                0,
                ColumnSide::Left,
                Some("w4:t1"),
                Some("agents"),
                Some("w4:p3N")
            )
        );
        assert_eq!(
            conn.query_row(
                "SELECT last_window_ordinal FROM fleets WHERE fleet_id=?1",
                [fleet.fleet_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
