//! Fleet CRUD (SPEC §6.2 *Fleets*) — atomic fleet + Director bootstrap with
//! `director_member_id` backfill, list/get/soft-delete + cascade. The
//! colocated tests pin the contract; see [`super::test_support`] for the API.

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

/// Atomic fleet + root-Director bootstrap: fleet row → Director member →
/// placement → `director_member_id` backfill.
pub fn create_fleet(
    conn: &mut Connection,
    name: Option<&str>,
    mux_session: &str,
    mux_window_id: &str,
    mux_pane_id: &str,
    coding_agent: &str,
    backend: &str,
) -> Result<Value, CafleetError> {
    let now = format_utc(now_utc());
    let card = member_card(DIRECTOR_NAME, DIRECTOR_DESCRIPTION, &[], None);
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO fleets (name, created_at) VALUES (?1, ?2)",
        params![name, now],
    )
    .map_err(db_err)?;
    let fleet_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at, member_card_json) \
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
        params![fleet_id, DIRECTOR_NAME, DIRECTOR_DESCRIPTION, now, card],
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
    tx.commit().map_err(db_err)?;
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
    }))
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
/// (root Director included), drop their placement and monitor-config rows and
/// the fleet's runtime row; messages are untouched. Idempotent.
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
    tx.execute(
        "DELETE FROM monitor_config WHERE member_id IN \
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
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{FakeNotifier, create_fleet, migrated_conn, register};
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn create_fleet_bootstraps_the_fleet_and_backfills_the_director() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");

        let fleet = broker::get_fleet(&conn, fleet_id).unwrap().unwrap();
        assert_eq!(fleet["director_member_id"], director_id);
        assert_eq!(fleet["name"], "alpha");
        assert_eq!(fleet["deleted_at"], serde_json::Value::Null);

        let director = broker::get_member(&conn, director_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(director["name"], "Director");
        assert_eq!(director["description"], "Root Director for this fleet");
        assert_eq!(director["status"], "active");
        assert_eq!(director["kind"], "director");
        assert_eq!(director["placement"]["mux_pane_id"], "%0");
        assert_eq!(director["placement"]["backend"], "tmux");
        assert_eq!(director["placement"]["coding_agent"], "claude");
    }

    #[test]
    fn create_fleet_result_shape_is_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = broker::create_fleet(
            &mut conn,
            Some("alpha"),
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
        )
        .unwrap();
        let fleet_id = fleet["fleet_id"].as_i64().unwrap();
        let director_id = fleet["director"]["member_id"].as_i64().unwrap();
        let ts = fleet["created_at"].as_str().unwrap().to_string();
        let expected = format!(
            r#"{{"fleet_id":{fleet_id},"name":"alpha","created_at":"{ts}","director":{{"member_id":{director_id},"name":"Director","description":"Root Director for this fleet","registered_at":"{ts}","placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%0","coding_agent":"claude","created_at":"{ts}"}}}}}}"#
        );
        assert_eq!(format_json(&fleet), expected);
    }

    #[test]
    fn create_fleet_timestamps_are_canonical() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet = broker::create_fleet(
            &mut conn,
            Some("alpha"),
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
        )
        .unwrap();
        let created_at = fleet["created_at"].as_str().unwrap();
        assert_eq!(created_at.len(), 32, "fixed-width form, got: {created_at}");
        assert!(created_at.ends_with("+00:00"));
        assert!(crate::time::parse_lenient(created_at).is_ok());
    }

    #[test]
    fn list_fleets_counts_active_members_only() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");

        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["member_count"], 1);

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

        broker::delete_fleet(&mut conn, fleet_b).unwrap();
        let rows = broker::list_fleets(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["fleet_id"], fleet_a);
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
        common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let now = crate::time::format_utc(chrono::Utc::now());
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, &now).unwrap());

        let result = broker::delete_fleet(&mut conn, fleet_id).unwrap();
        assert_eq!(result["deregistered_count"], 2);

        let (status, deregistered_at): (String, Option<String>) = conn
            .query_row(
                "SELECT status, deregistered_at FROM members WHERE member_id=?1",
                [member_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "deregistered");
        assert!(deregistered_at.is_some());

        let placements: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM member_placements WHERE member_id IN (?1, ?2)",
                [director_id, member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placements, 0);
        assert!(
            broker::read_monitor_runtime(&conn, fleet_id)
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
}
