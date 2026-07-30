//! Fleet CRUD (SPEC §6.2 *Fleets*) — atomic fleet + Director bootstrap with
//! `director_member_id` backfill, list/get/soft-delete + cascade. The
//! colocated tests pin the contract; see [`super::test_support`] for the API.

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
        let fleet =
            broker::create_fleet(&mut conn, Some("alpha"), "main", "@1", "%0", "claude", "tmux")
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
    fn create_fleet_enrolls_the_director_at_180() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let config = broker::get_monitor_config(&conn, fleet_id, director_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["interval_seconds"], 180);
        assert_eq!(config["enabled"], true);
        assert_eq!(config["last_ping_at"], serde_json::Value::Null);
    }

    #[test]
    fn create_fleet_timestamps_are_canonical() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let fleet =
            broker::create_fleet(&mut conn, Some("alpha"), "main", "@1", "%0", "claude", "tmux")
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

        let configs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM monitor_config WHERE member_id IN (?1, ?2)",
                [director_id, member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(configs, 0);
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
