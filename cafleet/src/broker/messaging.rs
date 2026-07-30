//! Message send/broadcast/poll/ack with the write-then-best-effort-preview
//! ordering (SPEC §6.2 *Messaging*). The colocated tests pin the contract;
//! see [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, MAX_TEXT_LEN, create_fleet, migrated_conn, placement, register,
    };
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn send_message_persists_the_full_row_and_notifies() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();

        let result = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        assert_eq!(result["notification_sent"], true);

        let message = &result["message"];
        let message_id = message["message_id"].as_i64().unwrap();
        let ts = message["created_at"].as_str().unwrap().to_string();
        let expected = format!(
            r#"{{"message_id":{message_id},"owner_member_id":{member_id},"from_member_id":{director_id},"to_member_id":{member_id},"type":"unicast","created_at":"{ts}","status_state":"input_required","status_timestamp":"{ts}","origin_message_id":null,"text":"hi"}}"#
        );
        assert_eq!(format_json(message), expected);

        let calls = notifier.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target_pane_id, "%2");
        assert_eq!(calls[0].message_id, message_id);
        assert_eq!(calls[0].sender_id, director_id);
        assert_eq!(calls[0].ts, ts);
        assert_eq!(calls[0].text, "hi");
    }

    #[test]
    fn send_message_truncates_the_preview_but_persists_full_text() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let text = "a".repeat(10);

        let result = broker::send_message(
            &mut conn,
            &notifier,
            5,
            fleet_id,
            director_id,
            &member_id.to_string(),
            &text,
        )
        .unwrap();

        assert_eq!(
            result["message"]["text"], text,
            "persisted text is never truncated"
        );
        let calls = notifier.calls.borrow();
        assert_eq!(calls[0].text, "aaaaa…", "preview truncated broker-side");
    }

    #[test]
    fn send_message_to_self_skips_the_preview() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let result = common::send(
            &mut conn,
            &notifier,
            fleet_id,
            director_id,
            director_id,
            "note",
        );
        assert_eq!(result["notification_sent"], false);
        assert!(notifier.calls.borrow().is_empty());
    }

    #[test]
    fn send_message_to_a_paneless_recipient_skips_the_preview() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let pending_id = register(&mut conn, fleet_id, "pending", None);
        let notifier = FakeNotifier::succeeding();
        let result = common::send(&mut conn, &notifier, fleet_id, director_id, pending_id, "hi");
        assert_eq!(result["notification_sent"], false);
        assert!(notifier.calls.borrow().is_empty());
    }

    #[test]
    fn send_message_survives_a_failed_preview() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::failing();

        let result = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        assert_eq!(result["notification_sent"], false);
        assert_eq!(notifier.calls.borrow().len(), 1);

        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(pending.len(), 1, "the persisted message is never rolled back");
    }

    #[test]
    fn send_message_rejects_a_non_integer_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            director_id,
            "abc",
            "hi",
        )
        .expect_err("a non-integer destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Invalid destination format: abc");
    }

    #[test]
    fn send_message_rejects_an_inactive_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(&mut conn, &notifier, MAX_TEXT_LEN, fleet_id, 999, "1", "hi")
            .expect_err("an unknown sender must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(
            err.message(),
            "Sender member not found or not active in fleet: 999"
        );
    }

    #[test]
    fn send_message_rejects_a_missing_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            director_id,
            "999",
            "hi",
        )
        .expect_err("a missing destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Destination member not found: 999");
    }

    #[test]
    fn send_message_rejects_a_cross_fleet_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_a, director_a) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let stranger_id = register(&mut conn, fleet_b, "stranger", Some("%5"));
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_a,
            director_a,
            &stranger_id.to_string(),
            "hi",
        )
        .expect_err("a cross-fleet destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(
            err.message(),
            format!("Destination member not in fleet: {stranger_id}")
        );
    }

    #[test]
    fn broadcast_writes_a_summary_and_one_delivery_per_peer() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_a = register(&mut conn, fleet_id, "a", Some("%2"));
        let member_b = register(&mut conn, fleet_id, "b", Some("%3"));
        let notifier = FakeNotifier::succeeding();

        let result = broker::broadcast_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            director_id,
            "all hands",
        )
        .unwrap();
        assert_eq!(result.len(), 1, "single-element result list");
        let envelope = &result[0];
        assert_eq!(envelope["recipients"], 2);
        assert_eq!(envelope["delivered"], 2);

        let summary = &envelope["message"];
        let summary_id = summary["message_id"].as_i64().unwrap();
        assert_eq!(summary["owner_member_id"], director_id);
        assert_eq!(summary["from_member_id"], director_id);
        assert_eq!(summary["to_member_id"], Value::Null);
        assert_eq!(summary["type"], "broadcast_summary");
        assert_eq!(summary["status_state"], "completed");
        assert_eq!(
            summary["origin_message_id"], summary_id,
            "self-referential origin"
        );
        assert_eq!(summary["text"], "Broadcast sent to 2 recipients");

        for recipient in [member_a, member_b] {
            let pending = broker::poll_messages(&conn, recipient).unwrap();
            assert_eq!(pending.len(), 1);
            let delivery = &pending[0];
            assert_eq!(delivery["type"], "unicast");
            assert_eq!(delivery["status_state"], "input_required");
            assert_eq!(delivery["origin_message_id"], summary_id);
            assert_eq!(delivery["text"], "all hands");
        }
        assert_eq!(notifier.calls.borrow().len(), 2);
    }

    #[test]
    fn broadcast_excludes_the_sender_and_counts_only_landed_previews() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let sender_id = register(&mut conn, fleet_id, "sender", Some("%2"));
        broker::register_member(
            &mut conn,
            fleet_id,
            "watch",
            "d",
            &[],
            Some(&placement(Some("%3"))),
            Some("monitoring-member"),
        )
        .unwrap();
        register(&mut conn, fleet_id, "pending", None);
        let notifier = FakeNotifier::succeeding();

        let result = broker::broadcast_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            fleet_id,
            sender_id,
            "hello",
        )
        .unwrap();
        let envelope = &result[0];
        assert_eq!(
            envelope["recipients"], 3,
            "director + monitoring member + pending peer; sender excluded"
        );
        assert_eq!(
            envelope["delivered"], 2,
            "the paneless peer contributes no preview"
        );
        assert_eq!(envelope["message"]["text"], "Broadcast sent to 3 recipients");
    }

    #[test]
    fn broadcast_rejects_an_inactive_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::broadcast_message(&mut conn, &notifier, MAX_TEXT_LEN, fleet_id, 999, "hi")
            .expect_err("an unknown sender must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(
            err.message(),
            "Sender member not found or not active in fleet: 999"
        );
    }

    #[test]
    fn poll_returns_unacked_deliveries_newest_first_without_summaries() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();

        let first = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "one");
        let second = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let second_id = second["message"]["message_id"].as_i64().unwrap();

        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0]["message_id"], second_id, "newest first");
        assert_eq!(pending[1]["message_id"], first_id);

        broker::ack_message(&mut conn, member_id, first_id).unwrap();
        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["message_id"], second_id);

        broker::broadcast_message(&mut conn, &notifier, 200, fleet_id, member_id, "fanout")
            .unwrap();
        let sender_pending = broker::poll_messages(&conn, member_id).unwrap();
        assert!(
            sender_pending.iter().all(|m| m["type"] == "unicast"),
            "broadcast_summary rows never appear in poll"
        );
    }

    #[test]
    fn ack_transitions_the_message_and_restamps_status_timestamp() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();
        let sent_ts = sent["message"]["status_timestamp"]
            .as_str()
            .unwrap()
            .to_string();

        let acked = broker::ack_message(&mut conn, member_id, message_id).unwrap();
        let message = &acked["message"];
        assert_eq!(message["status_state"], "completed");
        let acked_ts = message["status_timestamp"].as_str().unwrap();
        assert!(
            acked_ts >= sent_ts.as_str(),
            "status_timestamp restamped on ack"
        );
    }

    #[test]
    fn ack_error_surfaces_are_pinned() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, fleet_id, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let err = broker::ack_message(&mut conn, member_id, 999).expect_err("missing message");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Message 999 not found");

        let err = broker::ack_message(&mut conn, director_id, message_id)
            .expect_err("only the recipient may ack");
        assert!(matches!(err, CafleetError::Permission(_)));
        assert_eq!(err.message(), "Only the recipient can ACK a message");

        broker::ack_message(&mut conn, member_id, message_id).unwrap();
        let err = broker::ack_message(&mut conn, member_id, message_id)
            .expect_err("a completed message cannot be acked again");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Cannot ACK message in state completed");
    }
}
