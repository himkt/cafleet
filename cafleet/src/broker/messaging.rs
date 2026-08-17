//! Message send/broadcast/poll/ack with the write-then-best-effort-preview
//! ordering (SPEC §6.2 *Messaging*). The colocated tests pin the contract;
//! see [`super::test_support`] for the API.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::members::db_err;
use crate::error::CafleetError;
use crate::output::truncate_text;
use crate::time::{format_utc, now_utc};

/// The broker-side half of the inline-preview overlap point (SPEC §4): the
/// broker truncates and calls this; the keystroke mechanics live behind it.
/// Best-effort — implementations return a boolean and never raise.
pub trait InlinePreviewSender {
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> bool;
}

/// Read one full typed-column message row in the pinned key order.
pub(crate) fn message_row(
    conn: &Connection,
    message_id: i64,
) -> Result<Option<Value>, CafleetError> {
    conn.query_row(
        "SELECT message_id, owner_member_id, from_member_id, to_member_id, type, \
                created_at, status_state, status_timestamp, origin_message_id, text \
         FROM messages WHERE message_id=?1",
        [message_id],
        map_message_row,
    )
    .optional()
    .map_err(db_err)
}

pub(crate) fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "message_id": row.get::<_, i64>(0)?,
        "owner_member_id": row.get::<_, i64>(1)?,
        "from_member_id": row.get::<_, i64>(2)?,
        "to_member_id": row.get::<_, Option<i64>>(3)?,
        "type": row.get::<_, String>(4)?,
        "created_at": row.get::<_, String>(5)?,
        "status_state": row.get::<_, String>(6)?,
        "status_timestamp": row.get::<_, String>(7)?,
        "origin_message_id": row.get::<_, Option<i64>>(8)?,
        "text": row.get::<_, String>(9)?,
    }))
}

/// The sender's fleet, derived from the sender row (SPEC §6.2): no
/// caller-supplied fleet exists.
fn sender_fleet(conn: &Connection, from_member_id: i64) -> Result<i64, CafleetError> {
    super::members::active_member_fleet(conn, from_member_id)?.ok_or_else(|| {
        CafleetError::Value(format!(
            "Sender member not found or not active: {from_member_id}"
        ))
    })
}

fn pane_of(conn: &Connection, member_id: i64) -> Result<Option<String>, CafleetError> {
    Ok(conn
        .query_row(
            "SELECT mux_pane_id FROM member_placements WHERE member_id=?1",
            [member_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(db_err)?
        .flatten())
}

fn preview_text(text: &str, max_text_len: usize) -> String {
    truncate_text(Some(text), max_text_len).expect("Some input yields Some")
}

pub fn send_message(
    conn: &mut Connection,
    notifier: &dyn InlinePreviewSender,
    max_text_len: usize,
    from_member_id: i64,
    to: &str,
    text: &str,
) -> Result<Value, CafleetError> {
    let fleet_id = sender_fleet(conn, from_member_id)?;
    let to_id: i64 = to
        .parse()
        .map_err(|_| CafleetError::Value(format!("Invalid destination format: {to}")))?;
    let recipient: Option<(i64, String)> = conn
        .query_row(
            "SELECT fleet_id, status FROM members WHERE member_id=?1",
            [to_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    let Some((recipient_fleet, recipient_status)) = recipient else {
        return Err(CafleetError::Value(format!(
            "Destination member not found: {to_id}"
        )));
    };
    if recipient_status != "active" {
        return Err(CafleetError::Value(format!(
            "Destination member not found: {to_id}"
        )));
    }
    if recipient_fleet != fleet_id {
        return Err(CafleetError::Value(format!(
            "members {from_member_id} and {to_id} are not in the same fleet."
        )));
    }

    let now = format_utc(now_utc());
    conn.execute(
        "INSERT INTO messages (owner_member_id, from_member_id, to_member_id, type, \
         created_at, status_state, status_timestamp, origin_message_id, text) \
         VALUES (?1, ?2, ?3, 'unicast', ?4, 'input_required', ?4, NULL, ?5)",
        params![to_id, from_member_id, to_id, now, text],
    )
    .map_err(db_err)?;
    let message_id = conn.last_insert_rowid();

    let mut notification_sent = false;
    if to_id != from_member_id
        && let Some(pane) = pane_of(conn, to_id)?
    {
        notification_sent = notifier.send_inline_preview(
            &pane,
            message_id,
            from_member_id,
            &now,
            &preview_text(text, max_text_len),
        );
    }
    let message = message_row(conn, message_id)?.expect("the just-inserted message exists");
    Ok(json!({"message": message, "notification_sent": notification_sent}))
}

pub fn broadcast_message(
    conn: &mut Connection,
    notifier: &dyn InlinePreviewSender,
    max_text_len: usize,
    from_member_id: i64,
    text: &str,
) -> Result<Vec<Value>, CafleetError> {
    let fleet_id = sender_fleet(conn, from_member_id)?;
    let mut stmt = conn
        .prepare(
            "SELECT m.member_id, p.mux_pane_id \
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id \
             WHERE m.fleet_id=?1 AND m.status='active' AND m.member_id != ?2 \
             ORDER BY m.member_id",
        )
        .map_err(db_err)?;
    let recipients: Vec<(i64, Option<String>)> = stmt
        .query_map(params![fleet_id, from_member_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    drop(stmt);

    let now = format_utc(now_utc());
    let summary_text = format!("Broadcast sent to {} recipients", recipients.len());
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO messages (owner_member_id, from_member_id, to_member_id, type, \
         created_at, status_state, status_timestamp, origin_message_id, text) \
         VALUES (?1, ?1, NULL, 'broadcast_summary', ?2, 'completed', ?2, NULL, ?3)",
        params![from_member_id, now, summary_text],
    )
    .map_err(db_err)?;
    let summary_id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE messages SET origin_message_id=?1 WHERE message_id=?1",
        [summary_id],
    )
    .map_err(db_err)?;
    let mut deliveries: Vec<(i64, Option<String>)> = Vec::with_capacity(recipients.len());
    for (recipient_id, pane) in recipients.iter().cloned() {
        tx.execute(
            "INSERT INTO messages (owner_member_id, from_member_id, to_member_id, type, \
             created_at, status_state, status_timestamp, origin_message_id, text) \
             VALUES (?1, ?2, ?1, 'unicast', ?3, 'input_required', ?3, ?4, ?5)",
            params![recipient_id, from_member_id, now, summary_id, text],
        )
        .map_err(db_err)?;
        deliveries.push((tx.last_insert_rowid(), pane));
    }
    tx.commit().map_err(db_err)?;

    let preview = preview_text(text, max_text_len);
    let mut delivered = 0i64;
    for (delivery_id, pane) in &deliveries {
        if let Some(pane) = pane
            && notifier.send_inline_preview(pane, *delivery_id, from_member_id, &now, &preview)
        {
            delivered += 1;
        }
    }
    let summary = message_row(conn, summary_id)?.expect("the just-inserted summary exists");
    Ok(vec![json!({
        "message": summary,
        "recipients": recipients.len(),
        "delivered": delivered,
    })])
}

pub fn poll_messages(conn: &Connection, member_id: i64) -> Result<Vec<Value>, CafleetError> {
    if super::members::active_member_fleet(conn, member_id)?.is_none() {
        return Err(CafleetError::Value(format!("Member {member_id} not found")));
    }
    let mut stmt = conn
        .prepare(
            "SELECT message_id, owner_member_id, from_member_id, to_member_id, type, \
                    created_at, status_state, status_timestamp, origin_message_id, text \
             FROM messages \
             WHERE owner_member_id=?1 AND status_state='input_required' AND type='unicast' \
             ORDER BY status_timestamp DESC, message_id DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([member_id], map_message_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn ack_message(conn: &mut Connection, message_id: i64) -> Result<Value, CafleetError> {
    let row: Option<String> = conn
        .query_row(
            "SELECT status_state FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let Some(status) = row else {
        return Err(CafleetError::Value(format!(
            "Message {message_id} not found"
        )));
    };
    if status != "input_required" {
        return Err(CafleetError::Value(format!(
            "Cannot ACK message in state {status}"
        )));
    }
    let now = format_utc(now_utc());
    conn.execute(
        "UPDATE messages SET status_state='completed', status_timestamp=?1 WHERE message_id=?2",
        params![now, message_id],
    )
    .map_err(db_err)?;
    let message = message_row(conn, message_id)?.expect("the just-acked message exists");
    Ok(json!({"message": message}))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{
        FakeNotifier, MAX_TEXT_LEN, MONITOR_PANE, bootstrap_monitor, create_fleet, migrated_conn,
        register,
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

        let result = common::send(&mut conn, &notifier, director_id, member_id, "hi");
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
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let result = common::send(&mut conn, &notifier, director_id, director_id, "note");
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
        let result = common::send(&mut conn, &notifier, director_id, pending_id, "hi");
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

        let result = common::send(&mut conn, &notifier, director_id, member_id, "hi");
        assert_eq!(result["notification_sent"], false);
        assert_eq!(notifier.calls.borrow().len(), 1);

        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(
            pending.len(),
            1,
            "the persisted message is never rolled back"
        );
    }

    #[test]
    fn send_message_rejects_a_non_integer_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err =
            broker::send_message(&mut conn, &notifier, MAX_TEXT_LEN, director_id, "abc", "hi")
                .expect_err("a non-integer destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Invalid destination format: abc");
    }

    #[test]
    fn send_message_rejects_an_inactive_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(&mut conn, &notifier, MAX_TEXT_LEN, 999, "1", "hi")
            .expect_err("an unknown sender must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Sender member not found or not active: 999");
    }

    #[test]
    fn send_message_rejects_a_missing_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err =
            broker::send_message(&mut conn, &notifier, MAX_TEXT_LEN, director_id, "999", "hi")
                .expect_err("a missing destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Destination member not found: 999");
    }

    #[test]
    fn send_message_rejects_a_cross_fleet_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_a) = create_fleet(&mut conn, "alpha");
        let (fleet_b, _) = create_fleet(&mut conn, "beta");
        let stranger_id = register(&mut conn, fleet_b, "stranger", Some("%5"));
        let notifier = FakeNotifier::succeeding();
        let err = broker::send_message(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            director_a,
            &stranger_id.to_string(),
            "hi",
        )
        .expect_err("a cross-fleet destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(
            err.message(),
            format!("members {director_a} and {stranger_id} are not in the same fleet.")
        );
    }

    #[test]
    fn broadcast_writes_a_summary_and_one_delivery_per_peer() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let monitor_id = bootstrap_monitor(&conn, fleet_id);
        let member_a = register(&mut conn, fleet_id, "a", Some("%2"));
        let member_b = register(&mut conn, fleet_id, "b", Some("%3"));
        let notifier = FakeNotifier::succeeding();

        let result =
            broker::broadcast_message(&mut conn, &notifier, MAX_TEXT_LEN, director_id, "all hands")
                .unwrap();
        assert_eq!(result.len(), 1, "single-element result list");
        let envelope = &result[0];
        assert_eq!(envelope["recipients"], 3, "monitor + two workers");
        assert_eq!(envelope["delivered"], 3);

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
        assert_eq!(summary["text"], "Broadcast sent to 3 recipients");

        let calls = notifier.calls.borrow();
        assert_eq!(calls.len(), 3);
        for (recipient, pane) in [
            (monitor_id, MONITOR_PANE),
            (member_a, "%2"),
            (member_b, "%3"),
        ] {
            let pending = broker::poll_messages(&conn, recipient).unwrap();
            assert_eq!(pending.len(), 1);
            let delivery = &pending[0];
            assert_eq!(delivery["type"], "unicast");
            assert_eq!(delivery["status_state"], "input_required");
            assert_eq!(delivery["origin_message_id"], summary_id);
            assert_eq!(delivery["text"], "all hands");

            let call = calls
                .iter()
                .find(|c| c.target_pane_id == pane)
                .expect("one preview per recipient pane");
            assert_eq!(
                call.message_id,
                delivery["message_id"].as_i64().unwrap(),
                "the preview carries the recipient's own delivery id — \
                 the id the recipient acks"
            );
            assert_eq!(call.sender_id, director_id);
        }
    }

    #[test]
    fn broadcast_excludes_the_sender_and_counts_only_landed_previews() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, _) = create_fleet(&mut conn, "alpha");
        let sender_id = register(&mut conn, fleet_id, "sender", Some("%2"));
        register(&mut conn, fleet_id, "helper", Some("%3"));
        register(&mut conn, fleet_id, "pending", None);
        let notifier = FakeNotifier::succeeding();

        let result =
            broker::broadcast_message(&mut conn, &notifier, MAX_TEXT_LEN, sender_id, "hello")
                .unwrap();
        let envelope = &result[0];
        assert_eq!(
            envelope["recipients"], 4,
            "director + monitor + pane-bound peer + pending peer; sender excluded"
        );
        assert_eq!(
            envelope["delivered"], 3,
            "the paneless peer contributes no preview"
        );
        assert_eq!(
            envelope["message"]["text"],
            "Broadcast sent to 4 recipients"
        );
    }

    #[test]
    fn broadcast_rejects_an_inactive_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");
        let notifier = FakeNotifier::succeeding();
        let err = broker::broadcast_message(&mut conn, &notifier, MAX_TEXT_LEN, 999, "hi")
            .expect_err("an unknown sender must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Sender member not found or not active: 999");
    }

    #[test]
    fn poll_returns_unacked_deliveries_newest_first_without_summaries() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();

        let first = common::send(&mut conn, &notifier, director_id, member_id, "one");
        let second = common::send(&mut conn, &notifier, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let second_id = second["message"]["message_id"].as_i64().unwrap();

        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0]["message_id"], second_id, "newest first");
        assert_eq!(pending[1]["message_id"], first_id);

        broker::ack_message(&mut conn, first_id).unwrap();
        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["message_id"], second_id);

        broker::broadcast_message(&mut conn, &notifier, 200, member_id, "fanout").unwrap();
        let sender_pending = broker::poll_messages(&conn, member_id).unwrap();
        assert!(
            sender_pending.iter().all(|m| m["type"] == "unicast"),
            "broadcast_summary rows never appear in poll"
        );
    }

    #[test]
    fn poll_rejects_an_unknown_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");
        let err = broker::poll_messages(&conn, 999).expect_err("an unknown member must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Member 999 not found");
    }

    #[test]
    fn ack_transitions_the_message_and_restamps_status_timestamp() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let sent = common::send(&mut conn, &notifier, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();
        let sent_ts = sent["message"]["status_timestamp"]
            .as_str()
            .unwrap()
            .to_string();

        let acked = broker::ack_message(&mut conn, message_id).unwrap();
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
        let sent = common::send(&mut conn, &notifier, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let err = broker::ack_message(&mut conn, 999).expect_err("missing message");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Message 999 not found");

        broker::ack_message(&mut conn, message_id).unwrap();
        let err = broker::ack_message(&mut conn, message_id)
            .expect_err("a completed message cannot be acked again");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Cannot ACK message in state completed");
    }
}
