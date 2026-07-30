//! Monitor schedule + runtime DB layer (SPEC §6.2 *Monitor*). The colocated
//! tests pin the contract; see [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{FakeNotifier, create_fleet, migrated_conn, register};
    use crate::error::CafleetError;
    use crate::time::format_utc;

    fn own_pid() -> i64 {
        i64::from(std::process::id())
    }

    // A PID far above any real process id on macOS/Linux test hosts, so the
    // signal-0 probe reports no-such-process.
    const DEAD_PID: i64 = 999_999_999;

    fn base_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, 10, 0, 0).unwrap()
    }

    #[test]
    fn get_monitor_config_is_none_for_unenrolled_or_cross_fleet_members() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let monitor_id = broker::register_member(
            &mut conn,
            fleet_a,
            "watch",
            "d",
            &[],
            Some(&common::placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap()["member_id"]
            .as_i64()
            .unwrap();

        assert!(
            broker::get_monitor_config(&conn, fleet_a, monitor_id)
                .unwrap()
                .is_none()
        );
        assert!(
            broker::get_monitor_config(&conn, fleet_b, director_a)
                .unwrap()
                .is_none(),
            "the fleet gate hides an enrolled member of another fleet"
        );
    }

    #[test]
    fn update_monitor_config_applies_partial_updates() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");

        let updated =
            broker::update_monitor_config(&mut conn, fleet_id, director_id, Some(300), None)
                .unwrap();
        assert_eq!(updated["interval_seconds"], 300);
        assert_eq!(updated["enabled"], true, "unspecified field untouched");

        let updated =
            broker::update_monitor_config(&mut conn, fleet_id, director_id, None, Some(false))
                .unwrap();
        assert_eq!(updated["interval_seconds"], 300);
        assert_eq!(updated["enabled"], false);
    }

    #[test]
    fn disabling_clears_the_stall_check_stamp() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::record_monitor_dispatch(&mut conn, &[], &[director_id], &when).unwrap();
        let config = broker::get_monitor_config(&conn, fleet_id, director_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["last_stall_check_at"], when);

        broker::update_monitor_config(&mut conn, fleet_id, director_id, None, Some(false)).unwrap();
        let config = broker::get_monitor_config(&conn, fleet_id, director_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["last_stall_check_at"], Value::Null);
    }

    #[test]
    fn update_monitor_config_not_enrolled_is_an_application_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let err = broker::update_monitor_config(&mut conn, fleet_id, 999, Some(60), None)
            .expect_err("an unenrolled member must be rejected");
        assert!(matches!(err, CafleetError::App(_)));
        assert_eq!(
            err.message(),
            format!("member 999 is not enrolled in monitoring for fleet {fleet_id}.")
        );
    }

    #[test]
    fn list_monitor_configs_returns_every_enrolled_member_with_bool_enabled() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        let configs = broker::list_monitor_configs(&conn, fleet_id).unwrap();
        assert_eq!(configs.len(), 2);
        for config in &configs {
            assert!(config["enabled"].is_boolean());
        }
        let intervals: Vec<i64> = configs
            .iter()
            .map(|c| c["interval_seconds"].as_i64().unwrap())
            .collect();
        assert!(intervals.contains(&180) && intervals.contains(&720));
        assert!(configs.iter().any(|c| c["member_id"] == director_id));
        assert!(configs.iter().any(|c| c["member_id"] == member_id));
    }

    #[test]
    fn record_pings_stamps_last_ping_at_and_ignores_an_empty_list() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());

        broker::record_pings(&mut conn, &[], &when).unwrap();
        broker::record_pings(&mut conn, &[director_id], &when).unwrap();
        let config = broker::get_monitor_config(&conn, fleet_id, director_id)
            .unwrap()
            .unwrap();
        assert_eq!(config["last_ping_at"], when);
    }

    #[test]
    fn record_monitor_dispatch_commits_both_cadences_atomically() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let when = format_utc(base_time());

        broker::record_monitor_dispatch(&mut conn, &[director_id], &[member_id], &when).unwrap();

        let director = broker::get_monitor_config(&conn, fleet_id, director_id)
            .unwrap()
            .unwrap();
        assert_eq!(director["last_ping_at"], when);
        assert_eq!(director["last_stall_check_at"], Value::Null);
        let member = broker::get_monitor_config(&conn, fleet_id, member_id)
            .unwrap()
            .unwrap();
        assert_eq!(member["last_ping_at"], Value::Null);
        assert_eq!(member["last_stall_check_at"], when);
    }

    #[test]
    fn reconcile_monitor_lifecycle_clears_stamps_for_listed_fleet_members_only() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "alpha");
        let member_a = register(&mut conn, fleet_a, "worker", Some("%2"));
        let (fleet_b, director_b) = create_fleet(&mut conn, "beta");
        let when = format_utc(base_time());
        broker::record_monitor_dispatch(&mut conn, &[], &[director_a, member_a, director_b], &when)
            .unwrap();

        broker::reconcile_monitor_lifecycle(&mut conn, fleet_a, &[member_a, director_b]).unwrap();

        let cleared = broker::get_monitor_config(&conn, fleet_a, member_a)
            .unwrap()
            .unwrap();
        assert_eq!(cleared["last_stall_check_at"], Value::Null);
        let kept = broker::get_monitor_config(&conn, fleet_a, director_a)
            .unwrap()
            .unwrap();
        assert_eq!(
            kept["last_stall_check_at"], when,
            "unlisted members keep their stamp"
        );
        let foreign = broker::get_monitor_config(&conn, fleet_b, director_b)
            .unwrap()
            .unwrap();
        assert_eq!(
            foreign["last_stall_check_at"], when,
            "the fleet filter protects other fleets' rows"
        );

        broker::reconcile_monitor_lifecycle(&mut conn, fleet_a, &[]).unwrap();
    }

    #[test]
    fn list_monitor_targets_returns_the_watched_set_with_the_scan_row_shape() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&common::placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap();

        let targets = broker::list_monitor_targets(&conn, fleet_id).unwrap();
        assert_eq!(targets.len(), 2, "the monitoring member is never a target");

        let director = targets
            .iter()
            .find(|t| t["member_id"] == director_id)
            .unwrap();
        assert_eq!(director["is_director"], true);
        assert_eq!(director["name"], "Director");
        assert_eq!(director["pane_id"], "%0");
        assert_eq!(director["coding_agent"], "claude");
        assert_eq!(director["interval_seconds"], 180);
        assert_eq!(director["last_ping_at"], Value::Null);
        assert_eq!(director["enabled"], true);
        assert_eq!(director["last_stall_check_at"], Value::Null);
        assert_eq!(director["pending_count"], 0);
        assert_eq!(director["oldest_pending_ts"], Value::Null);

        let worker = targets.iter().find(|t| t["member_id"] == member_id).unwrap();
        assert_eq!(worker["is_director"], false);
        assert_eq!(worker["interval_seconds"], 720);
    }

    #[test]
    fn list_monitor_targets_counts_pending_deliveries() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let first = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "one");
        common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let first_ts = first["message"]["status_timestamp"]
            .as_str()
            .unwrap()
            .to_string();

        let targets = broker::list_monitor_targets(&conn, fleet_id).unwrap();
        let worker = targets.iter().find(|t| t["member_id"] == member_id).unwrap();
        assert_eq!(worker["pending_count"], 2);
        assert_eq!(worker["oldest_pending_ts"], first_ts);

        broker::ack_message(&mut conn, member_id, first_id).unwrap();
        let targets = broker::list_monitor_targets(&conn, fleet_id).unwrap();
        let worker = targets.iter().find(|t| t["member_id"] == member_id).unwrap();
        assert_eq!(worker["pending_count"], 1);
        assert_ne!(worker["oldest_pending_ts"], first_ts);
    }

    #[test]
    fn claim_inserts_a_fresh_slot_and_refuses_a_live_one() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());

        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["fleet_id"], fleet_id);
        assert_eq!(row["pid"], own_pid());
        assert_eq!(row["started_at"], when);
        assert_eq!(row["last_tick_at"], when);
        assert_eq!(row["tick_seconds"], 5);

        let refused = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid() + 1,
            5,
            &format_utc(base_time() + Duration::seconds(1)),
        )
        .unwrap();
        assert!(!refused, "a live slot is never stolen");
    }

    #[test]
    fn claim_reclaims_a_stale_heartbeat() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        assert!(
            broker::claim_monitor_runtime(
                &mut conn,
                fleet_id,
                own_pid(),
                5,
                &format_utc(base_time())
            )
            .unwrap()
        );

        // stale_after = max(3 * 5, 15) = 15; a 100-second-old heartbeat is
        // stale even though the owning process (this test) is alive.
        let later = format_utc(base_time() + Duration::seconds(100));
        assert!(broker::claim_monitor_runtime(&mut conn, fleet_id, 4242, 5, &later).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], 4242);
        assert_eq!(row["started_at"], later);
    }

    #[test]
    fn claim_reclaims_a_dead_process_despite_a_fresh_heartbeat() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        assert!(
            broker::claim_monitor_runtime(
                &mut conn,
                fleet_id,
                DEAD_PID,
                5,
                &format_utc(base_time())
            )
            .unwrap()
        );

        let reclaimed = broker::claim_monitor_runtime(
            &mut conn,
            fleet_id,
            own_pid(),
            5,
            &format_utc(base_time() + Duration::seconds(1)),
        )
        .unwrap();
        assert!(reclaimed, "signal-0 no-such-process corroborates dead");
    }

    #[test]
    fn heartbeat_is_ownership_checked() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();

        let tick = format_utc(base_time() + Duration::seconds(2));
        assert!(broker::heartbeat_monitor_runtime(&mut conn, fleet_id, own_pid(), &tick).unwrap());
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["last_tick_at"], tick);

        let displaced = broker::heartbeat_monitor_runtime(
            &mut conn,
            fleet_id,
            4242,
            &format_utc(base_time() + Duration::seconds(3)),
        )
        .unwrap();
        assert!(!displaced, "a non-owner heartbeat matches zero rows");
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["last_tick_at"], tick, "the owner's heartbeat survives");
    }

    #[test]
    fn clear_is_ownership_checked_and_preserves_tick_seconds() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let when = format_utc(base_time());
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 7, &when).unwrap();

        broker::clear_monitor_runtime(&mut conn, fleet_id, 4242).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], own_pid(), "a loser's clear is a no-op");

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        let row = broker::read_monitor_runtime(&conn, fleet_id)
            .unwrap()
            .unwrap();
        assert_eq!(row["pid"], Value::Null);
        assert_eq!(row["started_at"], Value::Null);
        assert_eq!(row["last_tick_at"], Value::Null);
        assert_eq!(row["tick_seconds"], 7);
    }

    #[test]
    fn monitor_is_live_tracks_the_slot_lifecycle() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let now = base_time();
        assert!(
            !broker::monitor_is_live(&conn, fleet_id, now).unwrap(),
            "no row"
        );

        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &format_utc(now)).unwrap();
        assert!(broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(2)).unwrap());
        assert!(
            !broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(100)).unwrap(),
            "a stale heartbeat reads as dead"
        );

        broker::clear_monitor_runtime(&mut conn, fleet_id, own_pid()).unwrap();
        assert!(!broker::monitor_is_live(&conn, fleet_id, now + Duration::seconds(2)).unwrap());
    }

    #[test]
    fn runtime_payload_reports_live_fields_and_masks_stale_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let now = base_time();

        let absent = broker::monitor_runtime_payload(&conn, fleet_id, now).unwrap();
        assert_eq!(absent["running"], false);
        assert_eq!(absent["pid"], Value::Null);
        assert_eq!(absent["tick_seconds"], Value::Null);

        let when = format_utc(now);
        broker::claim_monitor_runtime(&mut conn, fleet_id, own_pid(), 5, &when).unwrap();
        let live =
            broker::monitor_runtime_payload(&conn, fleet_id, now + Duration::seconds(2)).unwrap();
        assert_eq!(live["running"], true);
        assert_eq!(live["pid"], own_pid());
        assert_eq!(live["tick_seconds"], 5);
        assert_eq!(live["last_tick_at"], when);
        assert_eq!(live["last_tick_age_seconds"], 2);
        assert_eq!(live["started_at"], when);

        let stale =
            broker::monitor_runtime_payload(&conn, fleet_id, now + Duration::seconds(100)).unwrap();
        assert_eq!(stale["running"], false);
        assert_eq!(stale["pid"], Value::Null, "a stale row never leaks its pid");
        assert_eq!(
            stale["tick_seconds"], 5,
            "tick_seconds survives from the stale row"
        );
        assert_eq!(stale["last_tick_at"], Value::Null);
        assert_eq!(stale["last_tick_age_seconds"], Value::Null);
        assert_eq!(stale["started_at"], Value::Null);
    }

    #[test]
    fn members_payload_labels_roles_and_truncates_ages() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");

        let now = Utc::now() + Duration::seconds(10);
        let ping_when = format_utc(now - Duration::seconds(30));
        broker::record_pings(&mut conn, &[director_id], &ping_when).unwrap();

        let rows = broker::monitor_members_payload(&conn, fleet_id, now).unwrap();
        assert_eq!(rows.len(), 2);

        let director = rows.iter().find(|r| r["member_id"] == director_id).unwrap();
        assert_eq!(director["role"], "director");
        assert_eq!(director["last_ping_at"], ping_when);
        assert_eq!(director["last_ping_age_seconds"], 30);
        assert_eq!(director["pending_count"], 0);
        assert_eq!(director["oldest_pending_ts"], Value::Null);
        assert_eq!(director["oldest_pending_age_seconds"], Value::Null);

        let worker = rows.iter().find(|r| r["member_id"] == member_id).unwrap();
        assert_eq!(worker["role"], "member");
        assert_eq!(worker["last_ping_at"], Value::Null);
        assert_eq!(worker["last_ping_age_seconds"], Value::Null);
        assert_eq!(worker["pending_count"], 1);
        assert!(worker["oldest_pending_ts"].is_string());
        let pending_age = worker["oldest_pending_age_seconds"].as_i64().unwrap();
        assert!(
            (8..=11).contains(&pending_age),
            "whole-second age against the supplied now, got {pending_age}"
        );
    }
}
