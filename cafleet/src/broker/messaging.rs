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
    let message = insert_message_validated(&tx, from_member_id, to_member_id, text)?;
    tx.commit().map_err(db_err)?;
    Ok(message)
}

pub(crate) fn insert_message_validated(
    conn: &Connection,
    from_member_id: i64,
    to_member_id: i64,
    text: &str,
) -> Result<MessageRecord, CafleetError> {
    let fleet_id = sender_fleet(conn, from_member_id)?;
    super::fleets::require_live_fleet(conn, fleet_id)?;
    let recipient: Option<(i64, MemberStatus)> = conn
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
    let message = conn.query_row(
        &format!("INSERT INTO messages (from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at)
         VALUES (?1, ?2, ?3, 'input_required', ?3, ?4, ?5) RETURNING {MESSAGE_COLUMNS}"),
        params![from_member_id, to_member_id, now, text, notified_at],
        map_message_row,
    ).map_err(db_err)?;
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

fn listen_snapshot_with_clock(
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
pub(crate) mod test_support {
    use rusqlite::Connection;

    use crate::broker;
    pub(crate) fn broker_fixture() -> (tempfile::TempDir, Connection, i64, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = crate::db::connect(&format!(
            "sqlite:///{}",
            dir.path().join("listen.db").display()
        ))
        .unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let created = broker::create_fleet(
            &mut conn,
            Some("listen"),
            "main",
            "@1",
            "%0",
            "claude",
            "tmux",
        )
        .unwrap();
        let fleet = created.fleet_id;
        let director = created.director.member_id;
        let worker = broker::register_member(&mut conn, fleet, "worker", "listener sender", None)
            .unwrap()
            .member_id;
        (dir, conn, fleet, director, worker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::migrated_conn;
    use tempfile::TempDir;

    #[test]
    fn send_poll_show_and_strict_ack_preserve_the_complete_body() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "messages");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "preview");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        let worker = register(&mut conn, fleet, "worker", None);
        let (_, other) = create_fleet_fixture(&mut conn, "other");
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
        let (_, director) = create_fleet_fixture(&mut conn, "corrupt");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "ack-race");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "ordering");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "atomic");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "selection");
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
        let (fleet, director) = create_fleet_fixture(&mut conn, "preview");
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

    mod listen_snapshot {
        use super::super::test_support::broker_fixture;
        use super::*;
        use std::cell::Cell;
        use std::time::Duration;

        fn snapshot_messages(snapshot: InboxSnapshot, expected_fleet: i64) -> Vec<MessageRecord> {
            match snapshot {
                InboxSnapshot::Open { fleet_id, messages } => {
                    assert_eq!(fleet_id, expected_fleet);
                    messages
                }
                InboxSnapshot::Closed { .. } => panic!("the live inbox remains open"),
            }
        }

        fn assert_handler_cleared(conn: &Connection) {
            let result: i64 = conn.query_row(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT sum(x) FROM n", [], |row| row.get(0),
    ).expect("a later SQL query runs after the listener-owned deadline expires");
            assert_eq!(result, 50_005_000);
            assert!(
                conn.is_autocommit(),
                "the snapshot releases its transaction"
            );
        }
        #[test]
        fn broker_snapshots_are_read_only_oldest_first_and_ignore_preview_metadata() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            let later = broker::send_message(&mut conn, worker, director, "later").unwrap();
            let earlier = broker::send_message(&mut conn, director, director, "self").unwrap();
            let tied = broker::send_message(&mut conn, worker, director, "notified").unwrap();
            conn.execute("UPDATE messages SET created_at='1900-01-01T00:00:00+00:00', notified_at=status_timestamp WHERE message_id IN (?1, ?2)", [earlier.message_id, tied.message_id]).unwrap();
            let changes = conn.total_changes();
            let messages = snapshot_messages(
                listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
                fleet,
            );
            assert_eq!(
                messages.iter().map(|m| m.message_id).collect::<Vec<_>>(),
                [earlier.message_id, tied.message_id, later.message_id]
            );
            assert_eq!(conn.total_changes(), changes);
            assert_handler_cleared(&conn);
        }

        #[test]
        fn broker_snapshots_exclude_acks_committed_before_the_snapshot() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            let acknowledged =
                broker::send_message(&mut conn, worker, director, "consumed").unwrap();
            broker::ack_message(&mut conn, acknowledged.message_id).unwrap();
            assert!(
                snapshot_messages(
                    listen_snapshot(&mut conn, director, None, Deadline::new()).unwrap(),
                    fleet
                )
                .is_empty()
            );
        }

        #[test]
        fn broker_snapshots_validate_pinned_identity_and_distinguish_normal_closure() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            assert!(listen_snapshot(&mut conn, worker, Some(fleet + 1), Deadline::new()).is_err());
            conn.execute(
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                [worker],
            )
            .unwrap();
            assert!(listen_snapshot(&mut conn, worker, None, Deadline::new()).is_err());
            assert!(matches!(
                listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap(),
                InboxSnapshot::Closed {
                    reason: InboxClosure::MemberInactive,
                    ..
                }
            ));
            conn.execute(
                "UPDATE fleets SET deleted_at=created_at WHERE fleet_id=?1",
                [fleet],
            )
            .unwrap();
            assert!(matches!(
                listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap(),
                InboxSnapshot::Closed {
                    reason: InboxClosure::FleetDeleted,
                    ..
                }
            ));
            assert!(matches!(
                listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
                InboxSnapshot::Closed {
                    reason: InboxClosure::FleetDeleted,
                    ..
                }
            ));
        }

        #[test]
        fn broker_snapshot_requires_a_concrete_root_placement_but_allows_no_subject_placement() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            conn.execute("DELETE FROM member_placements WHERE member_id=?1", [worker])
                .unwrap();
            listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap();
            conn.execute(
                "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
                [director],
            )
            .unwrap();
            assert!(listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).is_err());
            assert_handler_cleared(&conn);
        }

        #[test]
        fn a_long_running_select_without_lock_contention_is_interrupted_and_cleans_its_handler() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            broker::send_message(&mut conn, worker, director, "expensive selection").unwrap();
            conn.execute_batch(
        "ALTER TABLE messages RENAME TO stored_messages;
         CREATE VIEW messages AS
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000)
         SELECT m.message_id+n.x*1000000 AS message_id, m.from_member_id, m.to_member_id,
                printf('%09d', 1000000-n.x) AS created_at, m.status_state, m.status_timestamp, m.text,
                m.notified_at
         FROM stored_messages m CROSS JOIN n;"
    ).unwrap();
            let started = Instant::now();
            let result = listen_snapshot(
                &mut conn,
                director,
                Some(fleet),
                Deadline::until(started + Duration::from_millis(25)),
            );
            assert!(
                matches!(result, Err(CafleetError::DeadlineExceeded)),
                "{result:?}"
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_handler_cleared(&conn);
        }

        #[test]
        fn row_collection_discards_an_expired_partial_snapshot_using_clock_boundary_checks() {
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            for index in 0..100 {
                broker::send_message(&mut conn, worker, director, &format!("row {index}")).unwrap();
            }
            let origin = Instant::now() + Duration::from_secs(10_000);
            let reads = Cell::new(0_u64);
            let deadline = Deadline::until(origin + Duration::from_millis(40));
            let result =
                listen_snapshot_with_clock(&mut conn, director, Some(fleet), deadline, || {
                    let read = reads.get();
                    reads.set(read + 1);
                    origin + Duration::from_millis(read)
                });
            assert!(
                matches!(result, Err(CafleetError::DeadlineExceeded)),
                "{result:?}"
            );
            assert!(reads.get() >= 40);
            assert_handler_cleared(&conn);
            assert_eq!(
                snapshot_messages(
                    listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
                    fleet
                )
                .len(),
                100
            );
        }

        #[test]
        fn success_and_query_failure_both_remove_the_owned_handler_and_restore_busy_timeout() {
            for corrupt in [false, true] {
                let (_dir, mut conn, fleet, director, _) = broker_fixture();
                conn.busy_timeout(Duration::from_millis(137)).unwrap();
                if corrupt {
                    conn.execute_batch("DROP TABLE messages").unwrap();
                }
                let deadline = Deadline::until(Instant::now() + Duration::from_millis(50));
                let result = listen_snapshot(&mut conn, director, Some(fleet), deadline);
                assert_eq!(result.is_err(), corrupt);
                std::thread::sleep(Duration::from_millis(60));
                assert_handler_cleared(&conn);
                let timeout: u32 = conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(timeout, 137);
            }
        }

        #[test]
        fn a_failed_snapshot_commit_preserves_rollback_failure_and_removes_the_progress_handler() {
            use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
            use std::sync::{Arc, Mutex};

            let (_dir, mut conn, fleet, director, _) = broker_fixture();
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&attempts);
            conn.authorizer(Some(move |context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    // rusqlite 0.39 maps SQLite's COMMIT authorizer operation to Unknown.
                    operation: TransactionOperation::Unknown,
                } => {
                    observed.lock().unwrap().push("commit");
                    Authorization::Deny
                }
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => {
                    observed.lock().unwrap().push("rollback");
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }))
            .unwrap();
            let expiry = Instant::now() + Duration::from_millis(200);
            let result = listen_snapshot(&mut conn, director, Some(fleet), Deadline::until(expiry));
            let transaction_open = !conn.is_autocommit();
            conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            if transaction_open {
                conn.execute_batch("ROLLBACK").unwrap();
            }
            std::thread::sleep(
                expiry.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            assert_handler_cleared(&conn);

            let attempts = attempts.lock().unwrap();
            assert_eq!(attempts.first(), Some(&"commit"));
            assert!(attempts[1..].contains(&"rollback"), "{attempts:?}");
            assert!(
                transaction_open,
                "the denied rollback leaves its transaction open for explicit cleanup"
            );
            let error = result.unwrap_err();
            let diagnostic = error.message();
            assert!(
                diagnostic.contains("rolling back listener snapshot failed"),
                "{diagnostic}"
            );
            assert!(
                diagnostic.matches("not authorized").count() >= 2,
                "commit and rollback causes both remain visible: {diagnostic}"
            );
        }

        #[test]
        fn an_unrelated_sqlite_interrupt_preserves_its_cause_before_the_deadline() {
            use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
            let (_dir, mut conn, fleet, director, worker) = broker_fixture();
            broker::send_message(
                &mut conn,
                worker,
                director,
                "externally interrupted selection",
            )
            .unwrap();
            conn.execute_batch(
        "ALTER TABLE messages RENAME TO stored_messages;
         CREATE VIEW messages AS
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000)
         SELECT m.message_id+n.x*1000000 AS message_id, m.from_member_id, m.to_member_id,
                printf('%09d', 1000000-n.x) AS created_at, m.status_state, m.status_timestamp, m.text,
                m.notified_at
         FROM stored_messages m CROSS JOIN n;"
    ).unwrap();
            let interrupt = conn.get_interrupt_handle();
            let (prepared, preparation) = std::sync::mpsc::sync_channel(1);
            let interrupted = std::thread::spawn(move || {
                preparation.recv_timeout(Duration::from_secs(2)).unwrap();
                std::thread::sleep(Duration::from_millis(10));
                interrupt.interrupt();
            });
            conn.authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "messages",
                        ..
                    }
                ) {
                    match prepared.try_send(()) {
                        Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => {}
                        Err(error) => panic!("interruption observer disappeared: {error}"),
                    }
                }
                Authorization::Allow
            }))
            .unwrap();
            let result = listen_snapshot(&mut conn, director, Some(fleet), Deadline::new());
            interrupted.join().unwrap();
            assert!(result.is_err());
            assert!(!matches!(result, Err(CafleetError::DeadlineExceeded)));
            conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
            assert_handler_cleared(&conn);
        }
    }
}
