//! Read-only message queries — inbox / sent / timeline / `get_message`
//! visibility rule (SPEC §6.2 *Queries*). The colocated tests pin the
//! contract; see [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, MAX_TEXT_LEN, create_fleet, migrated_conn, register,
    };
    use crate::error::CafleetError;

    #[test]
    fn list_inbox_keeps_acked_rows_and_hides_summaries() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();

        let first = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "one");
        let second = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let second_id = second["message"]["message_id"].as_i64().unwrap();
        broker::ack_message(&mut conn, member_id, first_id).unwrap();
        broker::broadcast_message(&mut conn, &notifier, MAX_TEXT_LEN, fleet_id, member_id, "x")
            .unwrap();

        let inbox = broker::list_inbox(&conn, member_id).unwrap();
        assert_eq!(
            inbox.len(),
            2,
            "acked rows stay; the sender's summary is not a delivery"
        );
        assert!(inbox.iter().all(|m| m["type"] == "unicast"));
        let acked = inbox.iter().find(|m| m["message_id"] == first_id).unwrap();
        assert_eq!(acked["status_state"], "completed");
        assert!(inbox.iter().any(|m| m["message_id"] == second_id));
    }

    #[test]
    fn list_sent_includes_broadcast_deliveries_but_not_the_summary() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        register(&mut conn, fleet_id, "a", Some("%2"));
        register(&mut conn, fleet_id, "b", Some("%3"));
        let notifier = FakeNotifier::succeeding();
        broker::broadcast_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            director_id,
            "fanout",
        )
        .unwrap();

        let sent = broker::list_sent(&conn, director_id).unwrap();
        assert_eq!(sent.len(), 2, "one delivery per peer; summary excluded");
        assert!(sent.iter().all(|m| m["type"] == "unicast"));
        assert!(sent.iter().all(|m| m["from_member_id"] == director_id));
    }

    #[test]
    fn list_timeline_is_fleet_scoped_capped_and_newest_first() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "alpha");
        let member_a = register(&mut conn, fleet_a, "worker", Some("%2"));
        let (fleet_b, director_b) = create_fleet(&mut conn, "beta");
        let member_b = register(&mut conn, fleet_b, "stranger", Some("%5"));
        let notifier = FakeNotifier::succeeding();

        common::send(&mut conn, &notifier, fleet_a, director_a, member_a, "one");
        let second = common::send(&mut conn, &notifier, fleet_a, director_a, member_a, "two");
        let third = common::send(&mut conn, &notifier, fleet_a, member_a, director_a, "three");
        let foreign = common::send(&mut conn, &notifier, fleet_b, director_b, member_b, "other");

        let timeline = broker::list_timeline(&conn, fleet_a, 2).unwrap();
        assert_eq!(timeline.len(), 2, "capped at the supplied limit");
        assert_eq!(
            timeline[0]["message_id"],
            third["message"]["message_id"].as_i64().unwrap()
        );
        assert_eq!(
            timeline[1]["message_id"],
            second["message"]["message_id"].as_i64().unwrap()
        );

        let full = broker::list_timeline(&conn, fleet_a, 200).unwrap();
        assert_eq!(full.len(), 3);
        let foreign_id = foreign["message"]["message_id"].as_i64().unwrap();
        assert!(full.iter().all(|m| m["message_id"] != foreign_id));
    }

    #[test]
    fn get_message_returns_the_message_envelope() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let result = broker::get_message(&conn, fleet_id, message_id).unwrap();
        assert_eq!(result["message"]["message_id"], message_id);
        assert_eq!(result["message"]["text"], "hi");
    }

    #[test]
    fn get_message_hides_missing_and_out_of_fleet_rows_identically() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "alpha");
        let member_a = register(&mut conn, fleet_a, "worker", Some("%2"));
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_a, director_a, member_a, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let err = broker::get_message(&conn, fleet_a, 999).expect_err("missing message");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Message 999 not found");

        let err = broker::get_message(&conn, fleet_b, message_id)
            .expect_err("the out-of-fleet gate hides as not-found");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), format!("Message {message_id} not found"));
    }

    #[test]
    fn get_message_resolves_a_broadcast_summary_via_its_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let result = broker::broadcast_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            director_id,
            "fanout",
        )
        .unwrap();
        let summary_id = result[0]["message"]["message_id"].as_i64().unwrap();

        let fetched = broker::get_message(&conn, fleet_id, summary_id).unwrap();
        assert_eq!(fetched["message"]["type"], "broadcast_summary");
        assert_eq!(
            fetched["message"]["to_member_id"],
            serde_json::Value::Null,
            "the NULL recipient endpoint is dropped, not dereferenced"
        );
    }
}
