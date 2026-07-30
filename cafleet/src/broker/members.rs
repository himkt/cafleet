//! Member registry, placement, roster, and activity proxies (SPEC §6.2
//! *Members*). The colocated tests pin the contract; see
//! [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, create_fleet, migrated_conn, placement, register,
    };
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn register_member_returns_the_registration_summary_and_enrolls_at_720() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        assert_eq!(result["name"], "analyst");
        assert!(crate::time::parse_lenient(result["registered_at"].as_str().unwrap()).is_ok());

        let config = broker::get_monitor_config(&conn, fleet_id, member_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["interval_seconds"], 720);
        assert_eq!(config["enabled"], true);
    }

    #[test]
    fn get_member_shape_is_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let result = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "test member",
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap();
        let member_id = result["member_id"].as_i64().unwrap();
        let ts = result["registered_at"].as_str().unwrap().to_string();

        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        let expected = format!(
            r#"{{"member_id":{member_id},"name":"analyst","description":"test member","status":"active","registered_at":"{ts}","kind":"member","skills":[],"placement":{{"backend":"tmux","mux_session":"main","mux_window_id":"@1","mux_pane_id":"%2","coding_agent":"claude","created_at":"{ts}"}}}}"#
        );
        assert_eq!(format_json(&member), expected);
    }

    #[test]
    fn register_member_carries_skills_verbatim() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let skills = [json!("python"), json!("sql")];
        let member_id = broker::register_member(
            &mut conn,
            fleet_id,
            "analyst",
            "d",
            &skills,
            Some(&placement(Some("%2"))),
            None,
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["skills"], json!(["python", "sql"]));
    }

    #[test]
    fn placementless_member_is_not_enrolled_and_has_null_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id =
            broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None).unwrap()
                ["member_id"]
                .as_i64()
                .unwrap();
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"], Value::Null);
        assert!(
            broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .is_none(),
            "only pane-bound members are enrolled"
        );
    }

    #[test]
    fn register_member_unknown_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let err = broker::register_member(&mut conn, 999, "x", "d", &[], None, None)
            .expect_err("unknown fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), "Fleet '999' not found.");
    }

    #[test]
    fn register_member_deleted_fleet_is_a_usage_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::delete_fleet(&mut conn, fleet_id).unwrap();
        let err = broker::register_member(&mut conn, fleet_id, "x", "d", &[], None, None)
            .expect_err("deleted fleet must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.message(), format!("fleet {fleet_id} is deleted"));
    }

    #[test]
    fn monitoring_member_requires_a_placement() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            None,
            Some("monitoring-member"),
        )
        .expect_err("a placementless monitoring member must be rejected");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            "a monitoring member must be pane-bound; register it via \
             'cafleet member create --role monitor' (placement required)."
        );
    }

    #[test]
    fn only_one_active_monitoring_member_per_fleet() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let first = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();
        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "watch2",
            "d",
            &[],
            Some(&placement(Some("%4"))),
            Some("monitoring-member"),
        )
        .expect_err("a second monitoring member must be rejected");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!(
                "fleet {fleet_id} already has an active monitoring member (member {first}); only one is allowed."
            )
        );
    }

    #[test]
    fn monitoring_member_is_unenrolled_with_kind_monitor() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let monitor_id = broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();

        let member = broker::get_member(&conn, monitor_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["kind"], "monitor");
        assert!(
            broker::get_monitor_config(&conn, fleet_id, monitor_id)
                .unwrap()
                .is_none(),
            "the monitoring member is the unenrolled watcher"
        );

        let found = broker::find_monitoring_member(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(found["member_id"], monitor_id);
        assert_eq!(found["name"], "watch");
        assert_eq!(found["pane_id"], "%3");
    }

    #[test]
    fn find_monitoring_member_treats_a_pending_pane_as_absent() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(None)),
            Some("monitoring-member"),
        )
        .unwrap();
        assert!(
            broker::find_monitoring_member(&conn, fleet_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn root_director_invariant_guard_fires_only_for_placed_registrations() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [director_id],
        )
        .unwrap();

        let err = broker::register_member(
            &mut conn,
            fleet_id,
            "worker",
            "d",
            &[],
            Some(&placement(Some("%2"))),
            None,
        )
        .expect_err("a placed registration under an inactive Director must fail loudly");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!("fleet {fleet_id}'s root Director (member {director_id}) is not active.")
        );

        broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None)
            .expect("a placementless registration skips the invariant guard");
    }

    #[test]
    fn deregister_root_director_is_rejected() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let err = broker::deregister_member(&mut conn, director_id)
            .expect_err("the root Director must not be deregisterable");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            "cannot deregister the root Director; use 'cafleet fleet delete' instead"
        );
    }

    #[test]
    fn deregister_member_flips_cleans_and_is_not_repeatable() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        assert!(broker::deregister_member(&mut conn, member_id).unwrap());

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
                "SELECT COUNT(*) FROM member_placements WHERE member_id=?1",
                [member_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placements, 0);
        assert!(
            broker::get_monitor_config(&conn, fleet_id, member_id)
                .unwrap()
                .is_none()
        );

        assert!(!broker::deregister_member(&mut conn, member_id).unwrap());
    }

    #[test]
    fn get_member_is_active_only_and_fleet_scoped() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, _) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let member_id = register(&mut conn, fleet_a, "worker", Some("%2"));

        assert!(
            broker::get_member(&conn, member_id, fleet_b)
                .unwrap()
                .is_none()
        );
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(
            broker::get_member(&conn, member_id, fleet_a)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn non_object_card_kind_collapses_to_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        conn.execute(
            r#"UPDATE members SET member_card_json='{"name":"worker","description":"d","skills":[],"cafleet":"weird"}' WHERE member_id=?1"#,
            [member_id],
        )
        .unwrap();

        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["kind"], "member");
        let listed = broker::list_members(&conn, fleet_id).unwrap();
        let row = listed.iter().find(|r| r["member_id"] == member_id).unwrap();
        assert_eq!(row["kind"], "member");
    }

    #[test]
    fn update_placement_pane_id_patches_the_pending_pane() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", None);

        let updated = broker::update_placement_pane_id(&mut conn, member_id, "%9")
            .unwrap()
            .unwrap();
        assert_eq!(updated["mux_pane_id"], "%9");
        let member = broker::get_member(&conn, member_id, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["placement"]["mux_pane_id"], "%9");

        let placementless =
            broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None).unwrap()
                ["member_id"]
                .as_i64()
                .unwrap();
        assert!(
            broker::update_placement_pane_id(&mut conn, placementless, "%1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn verify_member_fleet_is_status_agnostic() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, _) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let member_id = register(&mut conn, fleet_a, "worker", Some("%2"));

        assert!(broker::verify_member_fleet(&conn, member_id, fleet_a).unwrap());
        assert!(!broker::verify_member_fleet(&conn, member_id, fleet_b).unwrap());
        broker::deregister_member(&mut conn, member_id).unwrap();
        assert!(broker::verify_member_fleet(&conn, member_id, fleet_a).unwrap());
    }

    #[test]
    fn get_member_names_batches_and_includes_deregistered() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        broker::deregister_member(&mut conn, member_id).unwrap();

        assert!(broker::get_member_names(&conn, &[]).unwrap().is_empty());
        let names = broker::get_member_names(&conn, &[director_id, member_id]).unwrap();
        assert_eq!(
            names.get(&director_id).map(String::as_str),
            Some("Director")
        );
        assert_eq!(names.get(&member_id).map(String::as_str), Some("worker"));
    }

    #[test]
    fn list_members_rows_carry_kind_placement_and_activity_proxies() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let ghost_id =
            broker::register_member(&mut conn, fleet_id, "ghost", "d", &[], None, None).unwrap()
                ["member_id"]
                .as_i64()
                .unwrap();

        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let rows = broker::list_members(&conn, fleet_id).unwrap();
        assert_eq!(rows.len(), 3);
        let by_id = |id: i64| rows.iter().find(|r| r["member_id"] == id).unwrap();

        let director = by_id(director_id);
        assert_eq!(director["kind"], "director");
        assert!(director["last_sent"].is_string());
        assert_eq!(director["last_recv"], Value::Null);
        let idle = director["idle"].as_i64().unwrap();
        assert!((0..=5).contains(&idle), "fresh activity, got idle {idle}");

        let worker = by_id(member_id);
        assert_eq!(worker["kind"], "member");
        assert!(worker["last_recv"].is_string());
        assert_eq!(worker["last_ack"], Value::Null);

        let ghost = by_id(ghost_id);
        assert_eq!(ghost["placement"], Value::Null);
        assert_eq!(ghost["last_sent"], Value::Null);
        assert_eq!(ghost["last_recv"], Value::Null);
        assert_eq!(ghost["idle"], Value::Null);

        broker::ack_message(&mut conn, member_id, message_id).unwrap();
        let rows = broker::list_members(&conn, fleet_id).unwrap();
        let worker = rows.iter().find(|r| r["member_id"] == member_id).unwrap();
        assert!(worker["last_ack"].is_string());
    }

    #[test]
    fn list_members_excludes_deregistered_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        broker::deregister_member(&mut conn, member_id).unwrap();
        let rows = broker::list_members(&conn, fleet_id).unwrap();
        assert!(rows.iter().all(|r| r["member_id"] != member_id));
    }

    #[test]
    fn list_roster_surfaces_deregistered_message_holders_only_on_request() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let holder_id = register(&mut conn, fleet_id, "holder", Some("%2"));
        let silent_id = register(&mut conn, fleet_id, "silent", Some("%3"));
        let notifier = FakeNotifier::succeeding();
        common::send(&mut conn, &notifier, fleet_id, director_id, holder_id, "hi");
        broker::deregister_member(&mut conn, holder_id).unwrap();
        broker::deregister_member(&mut conn, silent_id).unwrap();

        let active_only = broker::list_roster(&conn, fleet_id, false).unwrap();
        assert!(active_only.iter().all(|r| r["member_id"] != holder_id));

        let with_holders = broker::list_roster(&conn, fleet_id, true).unwrap();
        let holder = with_holders
            .iter()
            .find(|r| r["member_id"] == holder_id)
            .expect("a deregistered member owning messages stays visible");
        assert_eq!(holder["status"], "deregistered");
        assert_eq!(holder["placement"], Value::Null);
        assert!(
            with_holders.iter().all(|r| r["member_id"] != silent_id),
            "a deregistered member without messages stays hidden"
        );
    }
}
