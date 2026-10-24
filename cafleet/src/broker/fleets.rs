use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::members::db_err;
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

#[allow(clippy::too_many_arguments)]
pub fn create_fleet(
    conn: &mut Connection,
    name: Option<&str>,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: &str,
    coding_agent: &str,
    backend: &str,
) -> Result<Value, CafleetError> {
    crate::db::require_current(conn)?;
    let now = format_utc(now_utc());
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO fleets (name, created_at) VALUES (?1, ?2)",
        params![name, now],
    )
    .map_err(db_err)?;
    let fleet_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at)
         VALUES (?1, ?2, ?3, 'active', ?4)",
        params![fleet_id, DIRECTOR_NAME, DIRECTOR_DESCRIPTION, now],
    )
    .map_err(db_err)?;
    let director_id = tx.last_insert_rowid();
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
    Ok(json!({
        "fleet_id": fleet_id, "name": name, "created_at": now,
        "director": {
            "member_id": director_id, "name": DIRECTOR_NAME,
            "description": DIRECTOR_DESCRIPTION, "registered_at": now,
            "placement": {
                "backend": backend, "mux_session": mux_session, "mux_window_id": mux_window_id,
                "mux_pane_id": mux_pane_id, "coding_agent": coding_agent, "created_at": now,
            },
        },
    }))
}

pub(crate) fn require_live_fleet(
    conn: &Connection,
    fleet_id: i64,
) -> Result<FleetRow, CafleetError> {
    let fleet = fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::App(format!("fleet {fleet_id} is deleted")));
    }
    let director = fleet.director_member_id.ok_or_else(|| CafleetError::App(format!(
        "fleet {fleet_id} has no root Director recorded; re-create the fleet with 'cafleet fleet create'."
    )))?;
    let status: Option<super::records::MemberStatus> = conn
        .query_row(
            "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2",
            params![director, fleet_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if status != Some(super::records::MemberStatus::Active) {
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

pub(crate) fn delete_fleet_rows(conn: &Connection, fleet_id: i64) -> Result<Value, CafleetError> {
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
    let deregistered = conn
        .execute(
            "UPDATE members SET status='deregistered', deregistered_at=?1 \
             WHERE fleet_id=?2 AND status='active'",
            params![now, fleet_id],
        )
        .map_err(db_err)?;
    conn.execute(
        "DELETE FROM member_placements WHERE member_id IN \
         (SELECT member_id FROM members WHERE fleet_id=?1)",
        [fleet_id],
    )
    .map_err(db_err)?;
    Ok(json!({
        "fleet_id": fleet_id,
        "deregistered_count": deregistered,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{create_fleet, migrated_conn, register};
    use crate::output::format_json;
    use tempfile::TempDir;

    fn bootstrap(conn: &mut Connection) -> Result<Value, CafleetError> {
        broker::create_fleet(conn, Some("alpha"), "main", "@1", "%0", "claude", "tmux")
    }

    fn row_count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn create_fleet_registers_the_current_director_and_backfills_its_identity() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "alpha");
        let row = broker::get_fleet(&conn, fleet).unwrap().unwrap();
        assert_eq!(row["director_member_id"], director);
        assert_eq!(row["name"], "alpha");
        let member = broker::get_member(&conn, director, fleet).unwrap().unwrap();
        let member = crate::presentation::member(&member);
        assert_eq!(member["name"], "Director");
        assert_eq!(member["description"], "Root Director for this fleet");
        assert_eq!(member["kind"], "director");
        assert_eq!(member["placement"]["mux_pane_id"], "%0");
        assert_eq!(row_count(&conn, "members"), 1);
        assert_eq!(row_count(&conn, "member_placements"), 1);
    }

    #[test]
    fn create_fleet_result_shape_is_ordered_and_registration_only() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = bootstrap(&mut conn).unwrap();
        let id = fleet["fleet_id"].as_i64().unwrap();
        let director = fleet["director"]["member_id"].as_i64().unwrap();
        let ts = fleet["created_at"].as_str().unwrap();
        assert_eq!(
            format_json(&fleet),
            format!(
                r#"{{"fleet_id":{id},"name":"alpha","created_at":"{ts}","director":{{"member_id":{director},"name":"Director","description":"Root Director for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{ts}"}}}}}}"#
            )
        );
        assert_eq!(ts.len(), 32);
        assert!(ts.ends_with("+00:00"));
        assert!(crate::time::parse_lenient(ts).is_ok());
    }

    #[test]
    fn registration_rolls_back_all_rows_and_ids_on_each_database_failure() {
        for trigger in [
            "BEFORE INSERT ON fleets",
            "BEFORE INSERT ON members",
            "BEFORE INSERT ON member_placements",
            "BEFORE UPDATE OF director_member_id ON fleets",
        ] {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            conn.execute_batch(&format!("CREATE TRIGGER fail_registration {trigger} BEGIN SELECT RAISE(ABORT, 'registration failure'); END;")).unwrap();
            let error = bootstrap(&mut conn).unwrap_err();
            assert!(error.message().contains("registration failure"));
            for table in ["fleets", "members", "member_placements"] {
                assert_eq!(row_count(&conn, table), 0, "{table}");
            }
            conn.execute_batch("DROP TRIGGER fail_registration;")
                .unwrap();
            let fleet = bootstrap(&mut conn).unwrap();
            assert_eq!(fleet["fleet_id"], 1);
            assert_eq!(fleet["director"]["member_id"], 1);
        }
    }

    #[test]
    fn list_fleets_counts_active_members_only() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");

        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["member_count"], 1, "Director");

        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        assert_eq!(broker::list_fleets(&conn).unwrap()[0]["member_count"], 2);

        broker::deregister_member(&mut conn, member_id).unwrap();
        assert_eq!(broker::list_fleets(&conn).unwrap()[0]["member_count"], 1);
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

        common::guarded_delete_fleet(&mut conn, fleet_b).unwrap();
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
        common::guarded_delete_fleet(&mut conn, fleet_id).unwrap();
        let fleet = broker::get_fleet(&conn, fleet_id).unwrap().unwrap();
        assert!(fleet["deleted_at"].is_string());
    }

    #[test]
    fn delete_fleet_soft_deletes_and_cascades() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let message =
            broker::send_message(&mut conn, director_id, &member_id.to_string(), "hi").unwrap();
        let result = common::guarded_delete_fleet(&mut conn, fleet_id).unwrap();
        assert_eq!(
            result["deregistered_count"], 2,
            "Director and ordinary member"
        );

        for member in [director_id, member_id] {
            let (status, deregistered_at): (String, String) = conn
                .query_row(
                    "SELECT status, deregistered_at FROM members WHERE member_id=?1",
                    [member],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "deregistered");
            assert!(crate::time::parse_lenient(&deregistered_at).is_ok());
        }

        let placements = row_count(&conn, "member_placements");
        assert_eq!(placements, 0, "deregistration removes every placement");
        let messages: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(messages, 1);
        assert_eq!(
            broker::messaging::message_row(&conn, message.message_id).unwrap(),
            Some(message)
        );
    }

    #[test]
    fn delete_fleet_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        common::guarded_delete_fleet(&mut conn, fleet_id).unwrap();
        let deleted = broker::get_fleet(&conn, fleet_id).unwrap().unwrap();
        let second = common::guarded_delete_fleet(&mut conn, fleet_id).unwrap();
        assert_eq!(second["deregistered_count"], 0);
        assert_eq!(
            broker::get_fleet(&conn, fleet_id).unwrap().unwrap(),
            deleted
        );
    }

    #[test]
    fn delete_fleet_missing_is_an_application_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err =
            common::guarded_delete_fleet(&mut conn, 999).expect_err("missing fleet must error");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(err.message(), "fleet '999' not found.");
    }

    #[test]
    fn guarded_fleet_deletion_rolls_back_every_stage_and_releases_ownership_on_error() {
        for trigger in [
            "BEFORE UPDATE OF deleted_at ON fleets",
            "BEFORE UPDATE OF status ON members",
            "BEFORE DELETE ON member_placements",
        ] {
            let dir = TempDir::new().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet(&mut conn, "alpha");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let message =
                broker::send_message(&mut conn, director, &worker.to_string(), "retained history")
                    .unwrap();
            let before_fleet = broker::get_fleet(&conn, fleet).unwrap();
            let before_members =
                [director, worker].map(|member| broker::get_member(&conn, member, fleet).unwrap());
            conn.execute_batch(&format!("CREATE TRIGGER fail_deletion {trigger} BEGIN SELECT RAISE(ABORT, 'fleet deletion failure'); END;")).unwrap();

            let error = common::guarded_delete_fleet(&mut conn, fleet).unwrap_err();
            assert!(matches!(error, CafleetError::App(_)));
            assert!(error.message().contains("fleet deletion failure"));
            assert_eq!(
                broker::get_fleet(&conn, fleet).unwrap(),
                before_fleet,
                "{trigger}"
            );
            for (member, before) in [director, worker].into_iter().zip(before_members) {
                assert_eq!(
                    broker::get_member(&conn, member, fleet).unwrap(),
                    before,
                    "{trigger}"
                );
                let deregistered_at: Option<String> = conn
                    .query_row(
                        "SELECT deregistered_at FROM members WHERE member_id=?1",
                        [member],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(deregistered_at, None);
            }
            assert_eq!(row_count(&conn, "member_placements"), 2);
            assert_eq!(
                broker::messaging::message_row(&conn, message.message_id).unwrap(),
                Some(message.clone())
            );

            conn.execute_batch("DROP TRIGGER fail_deletion").unwrap();
            let deleted = common::guarded_delete_fleet(&mut conn, fleet).unwrap();
            assert_eq!(deleted["deregistered_count"], 2);
            assert_eq!(row_count(&conn, "member_placements"), 0);
            assert_eq!(
                broker::messaging::message_row(&conn, message.message_id).unwrap(),
                Some(message)
            );
        }
    }
}
