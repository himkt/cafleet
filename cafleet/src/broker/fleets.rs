//! Fleet CRUD (SPEC §6.2 *Fleets*) — atomic fleet + Director + monitor
//! bootstrap with `director_member_id` backfill and a caller-supplied monitor
//! spawn callback, list/get/soft-delete + cascade. The colocated tests pin
//! the contract; see [`super::test_support`] for the API.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::members::{db_err, member_card};
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc};

const DIRECTOR_NAME: &str = "Director";
const DIRECTOR_DESCRIPTION: &str = "Root Director for this fleet";

pub(crate) struct FleetRow {
    pub fleet_id: i64,
    pub name: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
    pub director_member_id: Option<i64>,
}

pub(crate) fn fetch_fleet(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Option<FleetRow>, CafleetError> {
    conn.query_row(
        "SELECT fleet_id, name, created_at, deleted_at, director_member_id \
         FROM fleets WHERE fleet_id=?1",
        [fleet_id],
        |row| {
            Ok(FleetRow {
                fleet_id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                deleted_at: row.get(3)?,
                director_member_id: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

/// Per-call transaction observations. Events report real operation results;
/// `after_rollback` can add a diagnostic only after successful recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BootstrapEvent {
    Begun,
    CommitFinished {
        fleet_id: i64,
        error: Option<String>,
    },
    RollbackFinished {
        fleet_id: Option<i64>,
        error: Option<String>,
        autocommit: bool,
    },
}

pub(crate) trait BootstrapHooks {
    fn observe(&self, _event: BootstrapEvent) {}
    fn after_rollback(&self, _fleet_id: Option<i64>) -> Result<(), CafleetError> {
        Ok(())
    }
}

struct NoopBootstrapHooks;
impl BootstrapHooks for NoopBootstrapHooks {}

/// Bootstrap DB rows in one transaction around the caller's monitor spawn.
/// Failures explicitly attempt rollback and preserve any recovery diagnostic.
#[allow(clippy::too_many_arguments)]
pub fn create_fleet(
    conn: &mut Connection,
    name: Option<&str>,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: &str,
    coding_agent: &str,
    backend: &str,
    monitor_name: &str,
    monitor_description: &str,
    spawn_monitor: impl FnOnce(i64, i64, i64) -> Result<String, CafleetError>,
) -> Result<Value, CafleetError> {
    create_fleet_with_hooks(
        conn,
        name,
        mux_session,
        mux_window_id,
        mux_pane_id,
        coding_agent,
        backend,
        monitor_name,
        monitor_description,
        spawn_monitor,
        &NoopBootstrapHooks,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_fleet_with_hooks(
    conn: &mut Connection,
    name: Option<&str>,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: &str,
    coding_agent: &str,
    backend: &str,
    monitor_name: &str,
    monitor_description: &str,
    spawn_monitor: impl FnOnce(i64, i64, i64) -> Result<String, CafleetError>,
    hooks: &dyn BootstrapHooks,
) -> Result<Value, CafleetError> {
    let now = format_utc(now_utc());
    let director_card = member_card(DIRECTOR_NAME, DIRECTOR_DESCRIPTION, &[], false);
    let monitor_card = member_card(monitor_name, monitor_description, &[], true);
    let mut tx = conn.transaction().map_err(db_err)?;
    hooks.observe(BootstrapEvent::Begun);
    let mut allocated_fleet_id = None;
    let result: Result<Value, CafleetError> = (|| {
        tx.execute(
            "INSERT INTO fleets (name, created_at) VALUES (?1, ?2)",
            params![name, now],
        )
        .map_err(db_err)?;
        let fleet_id = tx.last_insert_rowid();
        allocated_fleet_id = Some(fleet_id);
        tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
        params![
            fleet_id,
            DIRECTOR_NAME,
            DIRECTOR_DESCRIPTION,
            now,
            director_card
        ],
    )
    .map_err(db_err)?;
        let director_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO member_placements \
         (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) \
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
        tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
        params![fleet_id, monitor_name, monitor_description, now, monitor_card],
    )
    .map_err(db_err)?;
        let monitor_id = tx.last_insert_rowid();
        let monitor_pane_id = spawn_monitor(fleet_id, director_id, monitor_id)?;
        tx.execute(
            "INSERT INTO member_placements \
         (member_id, mux_session, mux_window_id, mux_pane_id, backend, coding_agent, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                monitor_id,
                mux_session,
                mux_window_id,
                monitor_pane_id,
                backend,
                coding_agent,
                now
            ],
        )
        .map_err(db_err)?;
        let commit = tx.execute_batch("COMMIT").map_err(db_err);
        hooks.observe(BootstrapEvent::CommitFinished {
            fleet_id,
            error: commit.as_ref().err().map(ToString::to_string),
        });
        commit?;
        Ok(json!({
            "fleet_id": fleet_id,
            "name": name,
            "created_at": now,
            "director": {
                "member_id": director_id,
                "name": DIRECTOR_NAME,
                "description": DIRECTOR_DESCRIPTION,
                "registered_at": now,
                "placement": {
                    "backend": backend,
                    "mux_session": mux_session,
                    "mux_window_id": mux_window_id,
                    "mux_pane_id": mux_pane_id,
                    "coding_agent": coding_agent,
                    "created_at": now,
                },
            },
            "monitor": {
                "member_id": monitor_id,
                "name": monitor_name,
                "description": monitor_description,
                "registered_at": now,
                "placement": {
                    "backend": backend,
                    "mux_session": mux_session,
                    "mux_window_id": mux_window_id,
                    "mux_pane_id": monitor_pane_id,
                    "coding_agent": coding_agent,
                    "created_at": now,
                },
            },
        }))
    })();
    match result {
        Ok(value) => {
            tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
            Ok(value)
        }
        Err(mut primary) => {
            // SQLite may already have rolled back (e.g. RAISE(ROLLBACK)).
            let rollback = if tx.is_autocommit() {
                Ok(())
            } else {
                tx.execute_batch("ROLLBACK").map_err(db_err)
            };
            tx.set_drop_behavior(rusqlite::DropBehavior::Ignore);
            drop(tx);
            let autocommit = conn.is_autocommit();
            hooks.observe(BootstrapEvent::RollbackFinished {
                fleet_id: allocated_fleet_id,
                error: rollback.as_ref().err().map(ToString::to_string),
                autocommit,
            });
            let cleanup = match rollback {
                Ok(()) if autocommit => hooks.after_rollback(allocated_fleet_id),
                Ok(()) => Err(CafleetError::App(
                    "transaction remains open after rollback".into(),
                )),
                Err(error) => Err(error),
            };
            if let Err(error) = cleanup {
                let id = allocated_fleet_id.map_or_else(|| "unknown".into(), |id| id.to_string());
                primary = primary.with_cleanup(format!(
                    "cleanup failed for fleet {id} transaction: {error}"
                ));
            }
            Err(primary)
        }
    }
}

pub fn list_fleets(conn: &Connection) -> Result<Vec<Value>, CafleetError> {
    let mut stmt = conn
        .prepare(
            "SELECT f.fleet_id, f.name, f.created_at, f.director_member_id, \
                    (SELECT COUNT(*) FROM members m \
                     WHERE m.fleet_id=f.fleet_id AND m.status='active') \
             FROM fleets f WHERE f.deleted_at IS NULL \
             ORDER BY f.created_at DESC, f.fleet_id DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "fleet_id": row.get::<_, i64>(0)?,
                "name": row.get::<_, Option<String>>(1)?,
                "created_at": row.get::<_, String>(2)?,
                "director_member_id": row.get::<_, Option<i64>>(3)?,
                "member_count": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn get_fleet(conn: &Connection, fleet_id: i64) -> Result<Option<Value>, CafleetError> {
    Ok(fetch_fleet(conn, fleet_id)?.map(|fleet| {
        json!({
            "fleet_id": fleet.fleet_id,
            "name": fleet.name,
            "created_at": fleet.created_at,
            "deleted_at": fleet.deleted_at,
            "director_member_id": fleet.director_member_id,
        })
    }))
}

/// Soft-delete + cascade: stamp `deleted_at`, deregister every active member
/// (root Director included), drop their placement rows and the fleet's
/// runtime row; messages are untouched. Idempotent.
pub fn delete_fleet(conn: &mut Connection, fleet_id: i64) -> Result<Value, CafleetError> {
    let fleet = fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    let now = format_utc(now_utc());
    let tx = conn.transaction().map_err(db_err)?;
    if fleet.deleted_at.is_none() {
        tx.execute(
            "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
            params![now, fleet_id],
        )
        .map_err(db_err)?;
    }
    let deregistered = tx
        .execute(
            "UPDATE members SET status='deregistered', deregistered_at=?1 \
             WHERE fleet_id=?2 AND status='active'",
            params![now, fleet_id],
        )
        .map_err(db_err)?;
    tx.execute(
        "DELETE FROM member_placements WHERE member_id IN \
         (SELECT member_id FROM members WHERE fleet_id=?1)",
        [fleet_id],
    )
    .map_err(db_err)?;
    tx.execute("DELETE FROM monitor_runtime WHERE fleet_id=?1", [fleet_id])
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(json!({
        "fleet_id": fleet_id,
        "deregistered_count": deregistered,
    }))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, MONITOR_DESCRIPTION, MONITOR_NAME, MONITOR_PANE, bootstrap_monitor,
        create_fleet, migrated_conn, register,
    };
    use crate::error::CafleetError;
    use crate::output::format_json;
    use crate::spawn_prompt::substitute_spawn_placeholders;

    fn bootstrap(
        conn: &mut Connection,
        spawn_monitor: impl FnOnce(i64, i64, i64) -> Result<String, CafleetError>,
    ) -> Result<Value, CafleetError> {
        broker::create_fleet(
            conn,
            Some("alpha"),
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
            MONITOR_NAME,
            MONITOR_DESCRIPTION,
            spawn_monitor,
        )
    }

    fn row_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn assert_no_rows_persisted(conn: &Connection) {
        for table in ["fleets", "members", "member_placements"] {
            assert_eq!(row_count(conn, table), 0, "{table} must hold zero rows");
        }
    }

    #[test]
    fn create_fleet_bootstraps_the_fleet_the_director_and_the_monitor() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");

        let fleet = broker::get_fleet(&conn, fleet_id).unwrap().unwrap();
        assert_eq!(fleet["director_member_id"], director_id);
        assert_eq!(fleet["name"], "alpha");
        assert_eq!(fleet["deleted_at"], serde_json::Value::Null);

        let director = broker::get_member_record(&conn, director_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(director["name"], "Director");
        assert_eq!(director["description"], "Root Director for this fleet");
        assert_eq!(director["status"], "active");
        assert_eq!(director["kind"], "director");
        assert_eq!(director["placement"]["mux_pane_id"], "%0");
        assert_eq!(director["placement"]["backend"], "tmux");
        assert_eq!(director["placement"]["coding_agent"], "claude");

        let monitor_id = bootstrap_monitor(&conn, fleet_id);
        let monitor = broker::get_member_record(&conn, monitor_id, fleet_id)
            .map(|record| record.as_ref().map(crate::presentation::member))
            .unwrap()
            .unwrap();
        assert_eq!(monitor["name"], "monitor");
        assert_eq!(monitor["description"], "Monitor member for this fleet");
        assert_eq!(monitor["status"], "active");
        assert_eq!(monitor["kind"], "monitor");
        assert_eq!(
            monitor["placement"]["mux_pane_id"], MONITOR_PANE,
            "the placement carries the pane id the spawn callback returned"
        );
        assert_eq!(monitor["placement"]["mux_session"], "main");
        assert_eq!(monitor["placement"]["mux_window_id"], "@1");
        assert_eq!(monitor["placement"]["backend"], "tmux");
        assert_eq!(monitor["placement"]["coding_agent"], "claude");

        let card_kind: Option<String> = conn
            .query_row(
                "SELECT json_extract(member_card_json, '$.cafleet.kind') \
                 FROM members WHERE member_id=?1",
                [monitor_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            card_kind.as_deref(),
            Some("monitor"),
            "the bootstrap writes the same card marker as member create --role monitor"
        );
    }

    #[test]
    fn create_fleet_invokes_the_spawn_callback_with_the_three_allocated_ids() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let mut seen = None;
        let fleet = bootstrap(&mut conn, |fleet_id, director_id, monitor_id| {
            seen = Some((fleet_id, director_id, monitor_id));
            Ok(MONITOR_PANE.to_string())
        })
        .unwrap();

        let (fleet_id, director_id, monitor_id) = seen.expect("the callback ran");
        assert_eq!(fleet["fleet_id"], fleet_id);
        assert_eq!(fleet["director"]["member_id"], director_id);
        assert_eq!(fleet["monitor"]["member_id"], monitor_id);
        assert_eq!(bootstrap_monitor(&conn, fleet_id), monitor_id);
    }

    #[test]
    fn create_fleet_result_shape_is_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = bootstrap(&mut conn, |_, _, _| Ok(MONITOR_PANE.to_string())).unwrap();
        let fleet_id = fleet["fleet_id"].as_i64().unwrap();
        let director_id = fleet["director"]["member_id"].as_i64().unwrap();
        let monitor_id = fleet["monitor"]["member_id"].as_i64().unwrap();
        let ts = fleet["created_at"].as_str().unwrap().to_string();
        let expected = format!(
            r#"{{"fleet_id":{fleet_id},"name":"alpha","created_at":"{ts}","director":{{"member_id":{director_id},"name":"Director","description":"Root Director for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{ts}"}}}},"monitor":{{"member_id":{monitor_id},"name":"monitor","description":"Monitor member for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%1","coding_agent":"claude","created_at":"{ts}"}}}}}}"#
        );
        assert_eq!(format_json(&fleet), expected);
    }

    #[test]
    fn create_fleet_timestamps_are_canonical() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = bootstrap(&mut conn, |_, _, _| Ok(MONITOR_PANE.to_string())).unwrap();
        let created_at = fleet["created_at"].as_str().unwrap();
        assert_eq!(created_at.len(), 32, "fixed-width form, got: {created_at}");
        assert!(created_at.ends_with("+00:00"));
        assert!(crate::time::parse_lenient(created_at).is_ok());
    }

    #[test]
    fn create_fleet_rolls_back_everything_on_a_spawn_callback_failure() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = bootstrap(&mut conn, |_, _, _| {
            Err(CafleetError::App(
                "tmux split-window failed: boom".to_string(),
            ))
        })
        .expect_err("a callback failure must fail the bootstrap");
        assert_eq!(
            err.message(),
            "tmux split-window failed: boom",
            "the callback error surfaces verbatim"
        );
        assert_no_rows_persisted(&conn);
    }

    #[test]
    fn create_fleet_rolls_back_everything_on_a_substitution_failure() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = bootstrap(&mut conn, |fleet_id, director_id, monitor_id| {
            substitute_spawn_placeholders(
                "hello {typo}",
                fleet_id,
                monitor_id,
                director_id,
                "claude",
            )
            .map(|_| MONITOR_PANE.to_string())
        })
        .expect_err("a substitution failure must fail the bootstrap");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.exit_code(), 2);
        assert_eq!(
            err.message(),
            "Unknown placeholder 'typo' in custom prompt. \
             Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, \
             {coding_agent}. Double literal braces ({{, }}) to keep them as text.",
            "the existing substitution error string is re-raised unchanged"
        );
        assert_no_rows_persisted(&conn);
    }

    #[test]
    fn create_fleet_is_retryable_as_is_after_a_rollback() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        bootstrap(&mut conn, |_, _, _| {
            Err(CafleetError::App(
                "tmux split-window failed: boom".to_string(),
            ))
        })
        .expect_err("the first attempt fails");
        assert_no_rows_persisted(&conn);

        let fleet = bootstrap(&mut conn, |_, _, _| Ok(MONITOR_PANE.to_string()))
            .expect("the retry succeeds with the same arguments");
        assert_eq!(row_count(&conn, "fleets"), 1);
        assert_eq!(row_count(&conn, "members"), 2, "Director + monitor");
        assert_eq!(row_count(&conn, "member_placements"), 2);
        let fleet_id = fleet["fleet_id"].as_i64().unwrap();
        assert_eq!(
            fleet["monitor"]["member_id"],
            bootstrap_monitor(&conn, fleet_id)
        );
    }

    #[test]
    fn list_fleets_counts_active_members_only() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");

        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["member_count"], 2, "Director + bootstrap monitor");

        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        assert_eq!(broker::list_fleets(&conn).unwrap()[0]["member_count"], 3);

        broker::deregister_member(&mut conn, member_id).unwrap();
        assert_eq!(broker::list_fleets(&conn).unwrap()[0]["member_count"], 2);
    }

    #[test]
    fn list_fleets_orders_newest_first_and_excludes_soft_deleted() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "older");
        let (fleet_b, _) = create_fleet(&mut conn, "newer");

        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["fleet_id"], fleet_b);
        assert_eq!(rows[1]["fleet_id"], fleet_a);
        assert_eq!(rows[1]["director_member_id"], director_a);
        assert_eq!(rows[1]["name"], "older");

        broker::delete_fleet(&mut conn, fleet_b).unwrap();
        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["fleet_id"], fleet_a);
    }

    #[test]
    fn step6_fleet_timestamp_ties_use_descending_id_after_primary_time_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let (first, _) = create_fleet(&mut conn, "first");
        let (second, _) = create_fleet(&mut conn, "second");
        let (third, _) = create_fleet(&mut conn, "third");
        conn.execute("UPDATE fleets SET created_at='2026-01-01T00:00:00Z'", [])
            .unwrap();
        conn.execute(
            "UPDATE fleets SET created_at='2026-01-02T00:00:00Z' WHERE fleet_id=?1",
            [first],
        )
        .unwrap();
        assert_eq!(
            broker::list_fleets(&conn)
                .unwrap()
                .iter()
                .map(|r| r["fleet_id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![first, third, second]
        );
    }

    #[test]
    fn get_fleet_returns_none_for_a_missing_id() {
        let dir = TempDir::new().unwrap();
        let conn = migrated_conn(&dir);
        assert!(broker::get_fleet(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn get_fleet_returns_soft_deleted_rows_with_deleted_at() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::delete_fleet(&mut conn, fleet_id).unwrap();
        let fleet = broker::get_fleet(&conn, fleet_id).unwrap().unwrap();
        assert!(fleet["deleted_at"].is_string());
    }

    #[test]
    fn delete_fleet_soft_deletes_and_cascades() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        common::send(&mut conn, &notifier, director_id, member_id, "hi");
        let now = crate::time::format_utc(chrono::Utc::now());
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, 600, &now).unwrap());

        let result = broker::delete_fleet(&mut conn, fleet_id).unwrap();
        assert_eq!(
            result["deregistered_count"], 3,
            "Director + bootstrap monitor + worker"
        );

        let (status, deregistered_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, deregistered_at FROM members WHERE member_id=?1",
                [member_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "deregistered");
        assert!(deregistered_at.is_some());

        let placements = row_count(&conn, "member_placements");
        assert_eq!(
            placements, 0,
            "the cascade drops the Director, monitor, and worker placements"
        );
        assert!(
            broker::read_monitor_runtime_record(&conn, fleet_id)
                .map(|record| record.as_ref().map(crate::presentation::monitor_runtime))
                .unwrap()
                .is_none()
        );

        let messages: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert!(messages > 0, "messages are never deleted");
    }

    #[test]
    fn delete_fleet_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::delete_fleet(&mut conn, fleet_id).unwrap();
        let second = broker::delete_fleet(&mut conn, fleet_id).unwrap();
        assert_eq!(second["deregistered_count"], 0);
    }

    #[test]
    fn delete_fleet_missing_is_an_application_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = broker::delete_fleet(&mut conn, 999).expect_err("missing fleet must error");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(err.message(), "fleet '999' not found.");
    }
    #[test]
    fn creation_null_name_preserves_complete_success_wire_shape_and_key_order() {
        let dir = tempfile::Builder::new()
            .prefix(".bootstrap-wire-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = super::create_fleet(
            &mut conn,
            None,
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
            "monitor",
            "Monitor member for this fleet",
            |_, _, _| Ok("%1".into()),
        )
        .unwrap();
        let ts = fleet["created_at"].as_str().unwrap();
        assert_eq!(
            format_json(&fleet),
            format!(
                r#"{{"fleet_id":1,"name":null,"created_at":"{ts}","director":{{"member_id":1,"name":"Director","description":"Root Director for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{ts}"}}}},"monitor":{{"member_id":2,"name":"monitor","description":"Monitor member for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%1","coding_agent":"claude","created_at":"{ts}"}}}}}}"#
            )
        );
    }
}
