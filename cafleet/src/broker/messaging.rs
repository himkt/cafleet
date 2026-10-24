use rusqlite::{Connection, OptionalExtension, params};

use super::members::db_err;
use super::records::{MemberStatus, MessageRecord, MessageStatus};
use crate::error::CafleetError;
use crate::time::{format_utc, now_utc};

pub(crate) const MESSAGE_COLUMNS: &str =
    "message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text";

const PENDING_PREVIEW: &str = "status_state='input_required' AND notified_at IS NULL";

/// Read one full typed-column message row in the pinned key order.
pub(crate) fn message_row(
    conn: &Connection,
    message_id: i64,
) -> Result<Option<MessageRecord>, CafleetError> {
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE message_id=?1"),
        [message_id],
        map_message_row,
    )
    .optional()
    .map_err(db_err)
}

pub(crate) fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        message_id: row.get(0)?,
        from_member_id: row.get(1)?,
        to_member_id: row.get(2)?,
        created_at: row.get(3)?,
        status: row.get(4)?,
        status_timestamp: row.get(5)?,
        text: row.get(6)?,
    })
}

fn required_message_row(conn: &Connection, message_id: i64) -> Result<MessageRecord, CafleetError> {
    message_row(conn, message_id)?.ok_or_else(|| CafleetError::InvalidStoredValue {
        field: "messages.message_id".into(),
        value: message_id.to_string(),
    })
}

fn sender_fleet(conn: &Connection, from_member_id: i64) -> Result<i64, CafleetError> {
    super::members::active_member_fleet(conn, from_member_id)?.ok_or_else(|| {
        CafleetError::Value(format!(
            "Sender member not found or not active: {from_member_id}"
        ))
    })
}

pub fn send_message(
    conn: &mut Connection,
    from_member_id: i64,
    to: &str,
    text: &str,
) -> Result<MessageRecord, CafleetError> {
    crate::db::require_current(conn)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let fleet_id = sender_fleet(&tx, from_member_id)?;
    super::fleets::require_live_fleet(&tx, fleet_id)?;
    let to_id: i64 = to
        .parse()
        .map_err(|_| CafleetError::Value(format!("Invalid destination format: {to}")))?;
    let recipient: Option<(i64, MemberStatus)> = tx
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
    if recipient_status != MemberStatus::Active {
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
    // A self-send owes no keystroke, so it is settled at insert.
    let notified_at = (to_id == from_member_id).then_some(&now);
    tx.execute(
        "INSERT INTO messages (from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at)
         VALUES (?1, ?2, ?3, 'input_required', ?3, ?4, ?5)",
        params![from_member_id, to_id, now, text, notified_at],
    )
    .map_err(db_err)?;
    let message = required_message_row(&tx, tx.last_insert_rowid())?;
    tx.commit().map_err(db_err)?;
    Ok(message)
}

pub fn oldest_pending_preview(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<MessageRecord>, CafleetError> {
    conn.query_row(
        &format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE to_member_id=?1 AND {PENDING_PREVIEW} \
             ORDER BY created_at, message_id LIMIT 1"
        ),
        [member_id],
        map_message_row,
    )
    .optional()
    .map_err(db_err)
}

pub(crate) fn is_preview_pending(conn: &Connection, message_id: i64) -> Result<bool, CafleetError> {
    conn.query_row(
        "SELECT status_state, notified_at FROM messages WHERE message_id=?1",
        [message_id],
        |row| {
            Ok((
                row.get::<_, MessageStatus>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        },
    )
    .map(|(status, notified)| status == MessageStatus::InputRequired && notified.is_none())
    .map_err(db_err)
}

/// Stamp a pending preview as keystroked; `false` when the row is no longer
/// pending, because it was ACKed or another process stamped it first.
pub fn stamp_notified(
    conn: &mut Connection,
    message_id: i64,
    when: &str,
) -> Result<bool, CafleetError> {
    crate::db::require_current(conn)?;
    let changed = conn
        .execute(
            &format!(
                "UPDATE messages SET notified_at=?1 WHERE message_id=?2 AND {PENDING_PREVIEW}"
            ),
            params![when, message_id],
        )
        .map_err(db_err)?;
    Ok(changed == 1)
}

pub fn poll_messages(
    conn: &Connection,
    member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError> {
    if super::members::active_member_fleet(conn, member_id)?.is_none() {
        return Err(CafleetError::Value(format!("Member {member_id} not found")));
    }
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE to_member_id=?1 AND status_state='input_required' \
             ORDER BY status_timestamp DESC, message_id DESC"
        ))
        .map_err(db_err)?;
    let rows = stmt
        .query_map([member_id], map_message_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn ack_message(conn: &mut Connection, message_id: i64) -> Result<MessageRecord, CafleetError> {
    crate::db::require_current(conn)?;
    let now = format_utc(now_utc());
    let updated = conn
        .query_row(
            &format!(
                "UPDATE messages SET status_state='completed', status_timestamp=?1 \
                WHERE message_id=?2 AND status_state='input_required' RETURNING {MESSAGE_COLUMNS}"
            ),
            params![now, message_id],
            map_message_row,
        )
        .optional()
        .map_err(db_err)?;
    if let Some(message) = updated {
        return Ok(message);
    }
    let Some(message) = message_row(conn, message_id)? else {
        return Err(CafleetError::Value(format!(
            "Message {message_id} not found"
        )));
    };
    Err(CafleetError::Value(format!(
        "Cannot ACK message in state {}",
        message.status.as_str()
    )))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::broker;
    use crate::broker::test_support as common;
    use crate::broker::test_support::{create_fleet, migrated_conn, register};
    use crate::error::CafleetError;
    use crate::output::format_json;

    #[test]
    fn an_ack_that_loses_the_pending_transition_reports_completed() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "ack-race");
        let member = register(&mut conn, fleet, "worker", Some("%2"));
        let message =
            broker::send_message(&mut conn, director, &member.to_string(), "ready").unwrap();
        let path = conn.path().unwrap().to_owned();
        let won = Arc::new(AtomicBool::new(false));
        let observed = won.clone();
        let mut intercepted = false;
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if !intercepted
                && matches!(
                    context.action,
                    AuthAction::Update {
                        table_name: "messages",
                        ..
                    }
                )
            {
                intercepted = true;
                let mut other = rusqlite::Connection::open(&path).unwrap();
                broker::ack_message(&mut other, message.message_id).unwrap();
                observed.store(true, Ordering::SeqCst);
            }
            Authorization::Allow
        }))
        .unwrap();
        let error = broker::ack_message(&mut conn, message.message_id).unwrap_err();
        assert!(won.load(Ordering::SeqCst));
        assert_eq!(error.message(), "Cannot ACK message in state completed");
    }

    fn notified_at(conn: &rusqlite::Connection, message_id: i64) -> Option<String> {
        conn.query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn send_message_persists_the_full_row_and_leaves_the_preview_owed() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        let text = "界".repeat(250);
        let sent =
            broker::send_message(&mut conn, director_id, &member_id.to_string(), &text).unwrap();
        let message = crate::presentation::message(&sent);
        let message_id = sent.message_id;
        let ts = sent.created_at.clone();
        let expected = format!(
            r#"{{"message_id":{message_id},"from_member_id":{director_id},"to_member_id":{member_id},"created_at":"{ts}","status_state":"input_required","status_timestamp":"{ts}","text":"{text}"}}"#
        );
        assert_eq!(format_json(&message), expected);

        let stored = broker::get_message(&conn, message_id).unwrap();
        assert_eq!(crate::presentation::message(&stored), message);
        assert_eq!(stored.text, text);
        assert_eq!(
            notified_at(&conn, message_id),
            None,
            "the preview is owed from the insert"
        );
    }

    #[test]
    fn a_self_send_is_settled_at_insert_and_a_paneless_send_persists() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let pending_id = register(&mut conn, fleet_id, "pending", None);

        let self_send =
            broker::send_message(&mut conn, director_id, &director_id.to_string(), "note").unwrap();
        assert_eq!(
            notified_at(&conn, self_send.message_id),
            Some(self_send.created_at.clone()),
            "a self-send owes no keystroke"
        );

        let pending_send =
            broker::send_message(&mut conn, director_id, &pending_id.to_string(), "hi").unwrap();
        for (sent, recipient, text) in [
            (&self_send, director_id, "note"),
            (&pending_send, pending_id, "hi"),
        ] {
            let stored = broker::get_message(&conn, sent.message_id).unwrap();
            assert_eq!(stored.from_member_id, director_id);
            assert_eq!(stored.to_member_id, recipient);
            assert_eq!(stored.text, text);
            assert_eq!(stored.status.as_str(), "input_required");
            assert_eq!(
                crate::presentation::message(&stored),
                crate::presentation::message(sent)
            );
        }
        assert_ne!(self_send.message_id, pending_send.message_id);
    }

    #[test]
    fn send_message_rejects_a_non_integer_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let err = broker::send_message(&mut conn, director_id, "abc", "hi")
            .expect_err("a non-integer destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Invalid destination format: abc");
    }

    #[test]
    fn send_message_rejects_an_inactive_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");
        let err = broker::send_message(&mut conn, 999, "1", "hi")
            .expect_err("an unknown sender must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Sender member not found or not active: 999");
    }

    #[test]
    fn send_message_rejects_a_missing_destination() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director_id) = create_fleet(&mut conn, "alpha");
        let err = broker::send_message(&mut conn, director_id, "999", "hi")
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
        let err = broker::send_message(&mut conn, director_a, &stranger_id.to_string(), "hi")
            .expect_err("a cross-fleet destination must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(
            err.message(),
            format!("members {director_a} and {stranger_id} are not in the same fleet.")
        );
    }

    #[test]
    fn poll_returns_only_the_recipients_unacked_messages_newest_first() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));

        let first = common::send(&mut conn, director_id, member_id, "one");
        let second = common::send(&mut conn, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let second_id = second["message"]["message_id"].as_i64().unwrap();

        let pending = broker::poll_messages(&conn, member_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0]["message_id"], second_id, "newest first");
        assert_eq!(pending[1]["message_id"], first_id);

        broker::ack_message(&mut conn, first_id).unwrap();
        let pending = broker::poll_messages(&conn, member_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["message_id"], second_id);

        common::send(&mut conn, member_id, director_id, "outgoing");
        let pending = broker::poll_messages(&conn, member_id).unwrap();
        assert_eq!(
            pending.iter().map(|row| row.message_id).collect::<Vec<_>>(),
            [second_id]
        );
    }

    #[test]
    fn poll_scopes_by_recipient_and_orders_by_status_timestamp_then_descending_id() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "ordering");
        let worker = register(&mut conn, fleet, "worker", None);
        let peer = register(&mut conn, fleet, "peer", None);
        let mut ids = Vec::new();
        for text in ["first", "second", "third"] {
            ids.push(
                broker::send_message(&mut conn, director, &worker.to_string(), text)
                    .unwrap()
                    .message_id,
            );
        }
        broker::send_message(&mut conn, director, &peer.to_string(), "other inbox").unwrap();
        broker::send_message(&mut conn, worker, &director.to_string(), "outgoing").unwrap();
        conn.execute("UPDATE messages SET status_timestamp='2026-02-01T00:00:00+00:00',created_at='2020-01-01T00:00:00+00:00' WHERE message_id IN (?1,?2)",
            rusqlite::params![ids[0], ids[1]]).unwrap();
        conn.execute("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00',created_at='2099-01-01T00:00:00+00:00' WHERE message_id=?1", [ids[2]]).unwrap();
        let rows = broker::poll_messages(&conn, worker).unwrap();
        assert_eq!(
            rows.iter().map(|row| row.message_id).collect::<Vec<_>>(),
            [ids[1], ids[0], ids[2]]
        );
        assert!(rows.iter().all(|row| row.to_member_id == worker));
    }
    #[test]
    fn poll_rejects_an_unknown_member() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");
        let err = broker::poll_messages(&conn, 999)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .expect_err("an unknown member must error");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Member 999 not found");
    }

    #[test]
    fn ack_transitions_the_message_and_restamps_status_timestamp() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        let member_id = register(&mut conn, fleet_id, "worker", Some("%2"));
        let sent = common::send(&mut conn, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();
        let sent_ts = sent["message"]["status_timestamp"]
            .as_str()
            .unwrap()
            .to_string();

        let acked = broker::ack_message(&mut conn, message_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
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
        let sent = common::send(&mut conn, director_id, member_id, "hi");
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
    #[test]
    fn send_failure_rolls_back_the_message_without_leaving_an_identifier_gap() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "atomic");
        let worker = register(&mut conn, fleet, "worker", None);
        conn.execute_batch(
            "CREATE TRIGGER reject_message BEFORE INSERT ON messages
            BEGIN SELECT RAISE(ABORT, 'injected message failure'); END;",
        )
        .unwrap();
        let error =
            broker::send_message(&mut conn, director, &worker.to_string(), "first").unwrap_err();
        assert!(
            error.message().contains("injected message failure"),
            "{error}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        conn.execute_batch("DROP TRIGGER reject_message").unwrap();
        let message =
            broker::send_message(&mut conn, director, &worker.to_string(), "second").unwrap();
        assert_eq!(message.message_id, 1);
    }

    #[test]
    fn oldest_pending_preview_uses_creation_order_and_skips_acked_and_notified_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "preview");
        let worker = register(&mut conn, fleet, "worker", None);
        let mut ids = Vec::new();
        for text in ["acked", "notified", "oldest", "newest"] {
            ids.push(
                broker::send_message(&mut conn, director, &worker.to_string(), text)
                    .unwrap()
                    .message_id,
            );
        }
        broker::ack_message(&mut conn, ids[0]).unwrap();
        conn.execute(
            "UPDATE messages SET notified_at='2026-10-23T00:00:00+00:00' WHERE message_id=?1",
            [ids[1]],
        )
        .unwrap();
        conn.execute_batch("UPDATE messages SET created_at='2026-01-01T00:00:00+00:00', status_timestamp='2099-01-01T00:00:00+00:00' WHERE message_id=3;
            UPDATE messages SET created_at='2026-02-01T00:00:00+00:00', status_timestamp='2020-01-01T00:00:00+00:00' WHERE message_id=4;").unwrap();
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker)
                .unwrap()
                .unwrap()
                .message_id,
            ids[2]
        );
    }
}
