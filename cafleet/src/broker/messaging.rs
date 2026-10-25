use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use super::members::db_err;
use super::records::{MemberStatus, MessageRecord};
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::time::{format_utc, now_utc};

pub(crate) const MESSAGE_COLUMNS: &str =
    "message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text";

pub(crate) const PENDING_PREVIEW: &str =
    "status_state='input_required' AND notified_at IS NULL AND from_member_id!=to_member_id";

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

pub fn get_message(conn: &Connection, message_id: i64) -> Result<MessageRecord, CafleetError> {
    message_row(conn, message_id)?
        .ok_or_else(|| CafleetError::Value(format!("Message {message_id} not found")))
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
    to_member_id: i64,
    text: &str,
) -> Result<MessageRecord, CafleetError> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let fleet_id = sender_fleet(&tx, from_member_id)?;
    super::fleets::require_live_fleet(&tx, fleet_id)?;
    let recipient: Option<(i64, MemberStatus)> = tx
        .query_row(
            "SELECT fleet_id, status FROM members WHERE member_id=?1",
            [to_member_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    let Some((recipient_fleet, recipient_status)) = recipient else {
        return Err(CafleetError::Value(format!(
            "Destination member not found: {to_member_id}"
        )));
    };
    if recipient_status != MemberStatus::Active {
        return Err(CafleetError::Value(format!(
            "Destination member not found: {to_member_id}"
        )));
    }
    if recipient_fleet != fleet_id {
        return Err(CafleetError::Value(format!(
            "members {from_member_id} and {to_member_id} are not in the same fleet."
        )));
    }

    let now = format_utc(now_utc());
    // A self-send owes no keystroke, so it is settled at insert.
    let notified_at = (to_member_id == from_member_id).then_some(&now);
    let message = tx.query_row(
        &format!("INSERT INTO messages (from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at)
         VALUES (?1, ?2, ?3, 'input_required', ?3, ?4, ?5) RETURNING {MESSAGE_COLUMNS}"),
        params![from_member_id, to_member_id, now, text, notified_at],
        map_message_row,
    ).map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(message)
}

pub fn oldest_pending_preview(
    conn: &Connection,
    member_id: i64,
    attempted: &std::collections::BTreeSet<i64>,
) -> Result<Option<MessageRecord>, CafleetError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE to_member_id=?1 AND {PENDING_PREVIEW} \
             ORDER BY created_at, message_id"
        ))
        .map_err(db_err)?;
    let rows = statement
        .query_map([member_id], map_message_row)
        .map_err(db_err)?;
    for row in rows {
        let message = row.map_err(db_err)?;
        if !attempted.contains(&message.message_id) {
            return Ok(Some(message));
        }
    }
    Ok(None)
}

pub(crate) fn stamp_notified_validated(
    conn: &Connection,
    message_id: i64,
    when: &str,
) -> Result<bool, CafleetError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InboxClosure {
    MemberInactive,
    FleetDeleted,
}

#[derive(Debug)]
pub(crate) enum InboxSnapshot {
    Open {
        fleet_id: i64,
        messages: Vec<MessageRecord>,
    },
    Closed {
        fleet_id: i64,
        reason: InboxClosure,
    },
}

pub(crate) fn listen_snapshot(
    conn: &mut Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
) -> Result<InboxSnapshot, CafleetError> {
    listen_snapshot_with_clock(conn, member_id, expected_fleet, deadline, Instant::now)
}

pub(crate) fn listen_snapshot_with_clock(
    conn: &mut Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    now: impl Fn() -> Instant,
) -> Result<InboxSnapshot, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        let interrupted = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&interrupted);
        conn.progress_handler(
            1_000,
            Some(move || {
                let expired = deadline.remaining().is_err();
                if expired {
                    observed.store(true, Ordering::Relaxed);
                }
                expired
            }),
        )
        .map_err(db_err)?;
        let result = read_listen_snapshot(
            conn,
            member_id,
            expected_fleet,
            deadline,
            &now,
            &interrupted,
        );
        let cleanup = conn
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(db_err);
        match (result, cleanup) {
            (Ok(snapshot), Ok(())) => Ok(snapshot),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(error.with_cleanup(format!(
                "removing listener progress handler failed: {cleanup}"
            ))),
        }
    })
}

fn read_listen_snapshot(
    conn: &mut Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    now: &impl Fn() -> Instant,
    interrupted: &AtomicBool,
) -> Result<InboxSnapshot, CafleetError> {
    deadline.remaining_at(now())?;
    let tx = conn.transaction().map_err(db_err)?;
    let result = select_listen_snapshot(&tx, member_id, expected_fleet, deadline, now).and_then(
        |snapshot| {
            deadline.remaining_at(now())?;
            tx.execute_batch("COMMIT").map_err(db_err)?;
            Ok(snapshot)
        },
    );
    let result = match result {
        Err(_) if interrupted.load(Ordering::Relaxed) => Err(CafleetError::DeadlineExceeded),
        result => result,
    };
    match result {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => match tx.rollback().map_err(db_err) {
            Ok(()) => Err(error),
            Err(cleanup) => {
                Err(error.with_cleanup(format!("rolling back listener snapshot failed: {cleanup}")))
            }
        },
    }
}

fn select_listen_snapshot(
    conn: &Connection,
    member_id: i64,
    expected_fleet: Option<i64>,
    deadline: Deadline,
    now: &impl Fn() -> Instant,
) -> Result<InboxSnapshot, CafleetError> {
    deadline.remaining_at(now())?;
    let subject: Option<(i64, MemberStatus)> = conn
        .query_row(
            "SELECT fleet_id, status FROM members WHERE member_id=?1",
            [member_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    deadline.remaining_at(now())?;
    let Some((fleet_id, status)) = subject else {
        return Err(if expected_fleet.is_some() {
            CafleetError::InvalidStoredValue {
                field: "members.member_id".into(),
                value: member_id.to_string(),
            }
        } else {
            CafleetError::Value(format!("Member {member_id} not found"))
        });
    };
    if expected_fleet.is_some_and(|expected| expected != fleet_id) {
        return Err(CafleetError::App(format!(
            "Member {member_id} changed fleets during listen"
        )));
    }
    if expected_fleet.is_none() && status != MemberStatus::Active {
        return Err(CafleetError::Value(format!("Member {member_id} not found")));
    }
    deadline.remaining_at(now())?;
    let fleet = super::fleets::fetch_fleet(conn, fleet_id)?.ok_or_else(|| {
        CafleetError::InvalidStoredValue {
            field: "fleets.fleet_id".into(),
            value: fleet_id.to_string(),
        }
    })?;
    deadline.remaining_at(now())?;
    if expected_fleet.is_some() {
        if fleet.deleted_at.is_some() {
            return Ok(InboxSnapshot::Closed {
                fleet_id,
                reason: InboxClosure::FleetDeleted,
            });
        }
        if status != MemberStatus::Active {
            return Ok(InboxSnapshot::Closed {
                fleet_id,
                reason: InboxClosure::MemberInactive,
            });
        }
    }
    deadline.remaining_at(now())?;
    let live = super::fleets::require_live_fleet(conn, fleet_id)?;
    deadline.remaining_at(now())?;
    let root = live
        .director_member_id
        .expect("a live fleet has its root Director");
    for id in [member_id, root] {
        deadline.remaining_at(now())?;
        super::members::get_member(conn, id, fleet_id)?.ok_or_else(|| {
            CafleetError::InvalidStoredValue {
                field: "members.member_id".into(),
                value: id.to_string(),
            }
        })?;
        deadline.remaining_at(now())?;
        if member_id == root {
            break;
        }
    }
    deadline.remaining_at(now())?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages \
             WHERE to_member_id=?1 AND status_state='input_required' \
             ORDER BY created_at ASC, message_id ASC"
        ))
        .map_err(db_err)?;
    deadline.remaining_at(now())?;
    let mut rows = statement.query([member_id]).map_err(db_err)?;
    deadline.remaining_at(now())?;
    let mut messages = Vec::new();
    loop {
        deadline.remaining_at(now())?;
        let row = rows.next().map_err(db_err)?;
        deadline.remaining_at(now())?;
        let Some(row) = row else { break };
        let message = map_message_row(row).map_err(db_err)?;
        deadline.remaining_at(now())?;
        messages.push(message);
    }
    Ok(InboxSnapshot::Open { fleet_id, messages })
}

pub fn ack_message(conn: &mut Connection, message_id: i64) -> Result<MessageRecord, CafleetError> {
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
    use super::*;
    use crate::broker;
    use crate::broker::tests::{create_fleet, migrated_conn, register};
    use tempfile::TempDir;

    #[test]
    fn send_poll_show_and_strict_ack_preserve_the_complete_body() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "messages");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let text = format!("Unicode 世界\n{}\n\"quoted\" \u{1b}", "界".repeat(4096));
        let first = send_message(&mut conn, director, worker, &text).unwrap();
        let second = send_message(&mut conn, director, worker, "next").unwrap();
        assert_eq!(first.text, text);
        assert_eq!(
            (first.from_member_id, first.to_member_id),
            (director, worker)
        );
        assert_eq!(
            first.status,
            super::super::records::MessageStatus::InputRequired
        );
        assert_eq!(first.created_at, first.status_timestamp);
        assert_eq!(get_message(&conn, first.message_id).unwrap(), first);
        assert_eq!(
            poll_messages(&conn, worker).unwrap(),
            [second.clone(), first.clone()]
        );
        assert_eq!(notified_at(&conn, first.message_id), None);
        let ack = ack_message(&mut conn, first.message_id).unwrap();
        assert_eq!(ack.status, super::super::records::MessageStatus::Completed);
        assert!(ack.status_timestamp >= first.status_timestamp);
        assert_eq!(ack.text, text);
        assert!(ack_message(&mut conn, first.message_id).is_err());
        assert_eq!(poll_messages(&conn, worker).unwrap(), [second]);
        assert!(get_message(&conn, 999).is_err());
        assert!(ack_message(&mut conn, 999).is_err());
        assert!(poll_messages(&conn, 999).is_err());
    }

    #[test]
    fn self_and_pending_placement_messages_have_distinct_preview_states() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "preview");
        let pending = register(&mut conn, fleet, "pending", None);
        let own = send_message(&mut conn, director, director, "self").unwrap();
        let assignment = send_message(&mut conn, director, pending, "pending").unwrap();
        assert_eq!(
            notified_at(&conn, own.message_id),
            Some(own.created_at.clone())
        );
        assert_eq!(notified_at(&conn, assignment.message_id), None);
        assert_eq!(get_message(&conn, own.message_id).unwrap(), own);
        assert_eq!(poll_messages(&conn, pending).unwrap(), [assignment]);
    }

    #[test]
    fn sending_requires_active_same_fleet_members_and_a_live_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "alpha");
        let worker = register(&mut conn, fleet, "worker", None);
        let (_, other) = create_fleet(&mut conn, "other");
        for (sender, recipient) in [(999, worker), (director, 999), (director, other)] {
            assert!(matches!(
                send_message(&mut conn, sender, recipient, "invalid"),
                Err(CafleetError::Value(_))
            ));
        }
        broker::deregister_member(&mut conn, worker).unwrap();
        assert!(send_message(&mut conn, director, worker, "inactive recipient").is_err());
        assert!(send_message(&mut conn, worker, director, "inactive sender").is_err());
        conn.execute(
            "UPDATE fleets SET director_member_id=NULL WHERE fleet_id=?1",
            [fleet],
        )
        .unwrap();
        assert!(send_message(&mut conn, director, director, "missing root").is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn corrupt_required_message_values_surface_their_decode_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (_, director) = create_fleet(&mut conn, "corrupt");
        let message = send_message(&mut conn, director, director, "body").unwrap();
        conn.execute(
            "UPDATE messages SET status_state='corrupt' WHERE message_id=?1",
            [message.message_id],
        )
        .unwrap();
        for result in [
            get_message(&conn, message.message_id),
            ack_message(&mut conn, message.message_id),
        ] {
            assert!(
                matches!(result, Err(CafleetError::InvalidStoredValue { field, value }) if field == "messages.status_state" && value == "corrupt")
            );
        }
        assert!(poll_messages(&conn, director).unwrap().is_empty());
        conn.execute(
            "UPDATE messages SET status_state='input_required', text=X'00' WHERE message_id=?1",
            [message.message_id],
        )
        .unwrap();
        assert!(get_message(&conn, message.message_id).is_err());
        assert!(poll_messages(&conn, director).is_err());
    }

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
        let message = broker::send_message(&mut conn, director, member, "ready").unwrap();
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
    fn poll_scopes_by_recipient_and_orders_by_status_timestamp_then_descending_id() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "ordering");
        let worker = register(&mut conn, fleet, "worker", None);
        let peer = register(&mut conn, fleet, "peer", None);
        let mut ids = Vec::new();
        for text in ["first", "second", "third"] {
            ids.push(
                broker::send_message(&mut conn, director, worker, text)
                    .unwrap()
                    .message_id,
            );
        }
        broker::send_message(&mut conn, director, peer, "other inbox").unwrap();
        broker::send_message(&mut conn, worker, director, "outgoing").unwrap();
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
        let error = broker::send_message(&mut conn, director, worker, "first").unwrap_err();
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
        let message = broker::send_message(&mut conn, director, worker, "second").unwrap();
        assert_eq!(message.message_id, 1);
    }

    #[test]
    fn preview_selection_skips_attempted_ids_and_self_messages_without_hiding_pending_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet(&mut conn, "selection");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let self_message = broker::send_message(&mut conn, worker, worker, "self").unwrap();
        conn.execute(
            "UPDATE messages SET notified_at=NULL WHERE message_id=?1",
            [self_message.message_id],
        )
        .unwrap();
        let previous = broker::send_message(&mut conn, director, worker, "previous").unwrap();
        let next = broker::send_message(&mut conn, director, worker, "next").unwrap();
        let excluded = std::collections::BTreeSet::from([previous.message_id]);
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker, &excluded)
                .unwrap()
                .unwrap()
                .message_id,
            next.message_id
        );
        assert_eq!(
            broker::oldest_pending_preview(&conn, worker, &std::collections::BTreeSet::new())
                .unwrap()
                .unwrap()
                .message_id,
            previous.message_id
        );
        assert!(
            broker::oldest_pending_preview(
                &conn,
                worker,
                &std::collections::BTreeSet::from([previous.message_id, next.message_id])
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            broker::panes::pending_previews(&conn, fleet, director).unwrap(),
            vec![(worker, previous.message_id), (worker, next.message_id)]
        );
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
                broker::send_message(&mut conn, director, worker, text)
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
            broker::oldest_pending_preview(&conn, worker, &std::collections::BTreeSet::new())
                .unwrap()
                .unwrap()
                .message_id,
            ids[2]
        );
    }
}
