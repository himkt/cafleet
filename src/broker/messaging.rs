use crate::broker::MemberStatus;
use crate::broker::MessageRecord;
use crate::error::CafleetError;
use crate::time::now_stamp;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;

pub(crate) const MESSAGE_COLUMNS: &str =
    "message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text";

pub(crate) const PENDING_PREVIEW: &str = "status_state='input_required' AND notified_at IS NULL";

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

pub(crate) fn get_message(
    conn: &Connection,
    message_id: i64,
) -> Result<MessageRecord, CafleetError> {
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE message_id=?1"),
        [message_id],
        map_message_row,
    )
    .optional()?
    .ok_or_else(|| CafleetError::App(format!("Message {message_id} not found")))
}

fn sender_fleet(conn: &Connection, from_member_id: i64) -> Result<i64, CafleetError> {
    crate::broker::members::active_member_fleet(conn, from_member_id)?.ok_or_else(|| {
        CafleetError::App(format!(
            "Sender member not found or not active: {from_member_id}"
        ))
    })
}

pub(crate) fn send_message(
    conn: &mut Connection,
    from_member_id: i64,
    to_member_id: i64,
    text: &str,
) -> Result<MessageRecord, CafleetError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let message = insert_message_validated(&tx, from_member_id, to_member_id, text)?;
    tx.commit()?;
    Ok(message)
}

pub(crate) fn insert_message_validated(
    conn: &Connection,
    from_member_id: i64,
    to_member_id: i64,
    text: &str,
) -> Result<MessageRecord, CafleetError> {
    let fleet_id = sender_fleet(conn, from_member_id)?;
    crate::broker::fleets::require_live_fleet(conn, fleet_id)?;
    let Some((recipient_fleet, MemberStatus::Active)) =
        crate::broker::members::member_fleet_status(conn, to_member_id)?
    else {
        return Err(CafleetError::App(format!(
            "Destination member not found: {to_member_id}"
        )));
    };
    if recipient_fleet != fleet_id {
        return Err(CafleetError::App(format!(
            "members {from_member_id} and {to_member_id} are not in the same fleet."
        )));
    }

    let now = now_stamp();
    // A self-send owes no keystroke, so it is settled at insert.
    let notified_at = (to_member_id == from_member_id).then_some(&now);
    let message = conn.query_row(
        &format!("INSERT INTO messages (from_member_id, to_member_id, created_at, status_state, status_timestamp, text, notified_at)
         VALUES (?1, ?2, ?3, 'input_required', ?3, ?4, ?5) RETURNING {MESSAGE_COLUMNS}"),
        params![from_member_id, to_member_id, now, text, notified_at],
        map_message_row,
    )?;
    Ok(message)
}

pub(crate) fn oldest_pending_preview(
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
    .map_err(CafleetError::from)
}

pub(crate) fn poll_messages(
    conn: &Connection,
    member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    if crate::broker::members::active_member_fleet(&tx, member_id)?.is_none() {
        return Err(CafleetError::App(format!("Member {member_id} not found")));
    }
    let pending_ids = tx
        .prepare(
            "SELECT message_id FROM messages \
             WHERE to_member_id=?1 AND status_state='input_required' \
             ORDER BY status_timestamp DESC, message_id DESC",
        )?
        .query_map([member_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let consumed_at = now_stamp();
    let consumed = {
        let mut consume = tx.prepare(&format!(
            "UPDATE messages SET status_state='completed', status_timestamp=?1 \
             WHERE message_id=?2 AND status_state='input_required' RETURNING {MESSAGE_COLUMNS}"
        ))?;
        pending_ids
            .into_iter()
            .map(|message_id| consume.query_row(params![consumed_at, message_id], map_message_row))
            .collect::<Result<Vec<_>, _>>()?
    };
    tx.commit()?;
    Ok(consumed)
}

#[cfg(test)]
mod tests {
    use crate::broker;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::messaging::*;
    use crate::db::test_support::initialized_file_conn;
    use std::sync::atomic::Ordering;
    use tempfile::TempDir;

    fn status_state(conn: &rusqlite::Connection, message_id: i64) -> String {
        conn.query_row(
            "SELECT status_state FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn second_connection(conn: &rusqlite::Connection) -> rusqlite::Connection {
        crate::db::connect(&format!("sqlite:///{}", conn.path().unwrap())).unwrap()
    }

    fn consumed(record: &MessageRecord, status_timestamp: &str) -> MessageRecord {
        MessageRecord {
            status: crate::broker::MessageStatus::Completed,
            status_timestamp: status_timestamp.to_owned(),
            ..record.clone()
        }
    }

    #[test]
    fn send_poll_and_show_preserve_the_complete_body_and_poll_consumes_what_it_returns() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = initialized_file_conn(&dir);
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
        assert_eq!(first.status, crate::broker::MessageStatus::InputRequired);
        assert_eq!(first.created_at, first.status_timestamp);
        assert_eq!(get_message(&conn, first.message_id).unwrap(), first);

        let polled = poll_messages(&conn, worker).unwrap();
        assert_eq!(polled.len(), 2);
        let consumed_at = polled[0].status_timestamp.clone();
        assert!(consumed_at >= second.status_timestamp);
        assert_eq!(
            polled,
            [
                consumed(&second, &consumed_at),
                consumed(&first, &consumed_at)
            ],
            "poll returns the post-consumption records newest first with one consumption time"
        );
        assert_eq!(polled[1].text, text);
        assert_eq!(notified_at(&conn, first.message_id), None);
        for record in &polled {
            assert_eq!(get_message(&conn, record.message_id).unwrap(), *record);
        }
        assert!(poll_messages(&conn, worker).unwrap().is_empty());

        let later = send_message(&mut conn, director, worker, "after the poll").unwrap();
        let next = poll_messages(&conn, worker).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].message_id, later.message_id);
        assert_eq!(next[0].status, crate::broker::MessageStatus::Completed);
        assert!(get_message(&conn, 999).is_err());
    }

    #[test]
    fn polling_an_unknown_or_inactive_member_fails_and_consumes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "unknown");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = send_message(&mut conn, director, worker, "held").unwrap();
        let error = poll_messages(&conn, 999).unwrap_err();
        assert_eq!(error.message(), "Member 999 not found");
        assert_eq!(status_state(&conn, message.message_id), "input_required");
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [worker],
        )
        .unwrap();
        let error = poll_messages(&conn, worker).unwrap_err();
        assert_eq!(error.message(), format!("Member {worker} not found"));
        assert_eq!(status_state(&conn, message.message_id), "input_required");
        assert!(conn.is_autocommit());
    }

    #[test]
    fn a_poll_that_fails_before_commit_rolls_back_and_consumes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "rollback");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let first = send_message(&mut conn, director, worker, "first").unwrap();
        let second = send_message(&mut conn, director, worker, "second").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_second_consumption BEFORE UPDATE OF status_state ON messages
             WHEN NEW.message_id=2
             BEGIN SELECT RAISE(ABORT, 'injected consumption failure'); END;",
        )
        .unwrap();
        assert_eq!(second.message_id, 2);
        let error = poll_messages(&conn, worker).unwrap_err();
        assert!(
            error.message().contains("injected consumption failure"),
            "{error}"
        );
        assert!(
            conn.is_autocommit(),
            "the failed poll releases its transaction"
        );
        for message in [&first, &second] {
            assert_eq!(status_state(&conn, message.message_id), "input_required");
        }
        conn.execute_batch("DROP TRIGGER reject_second_consumption")
            .unwrap();
        assert_eq!(poll_messages(&conn, worker).unwrap().len(), 2);
    }

    #[test]
    fn a_poll_holds_the_write_lock_from_selection_through_consumption() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use std::sync::Arc;
        use std::sync::Mutex;

        let dir = TempDir::new().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "poll-race");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let message = send_message(&mut conn, director, worker, "only once").unwrap();
        let other = second_connection(&conn);
        other.busy_timeout(std::time::Duration::ZERO).unwrap();
        let rival = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&rival);
        let mut intercepted = false;
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if !intercepted
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "messages",
                        ..
                    }
                )
            {
                intercepted = true;
                *observed.lock().unwrap() = Some(poll_messages(&other, worker));
            }
            Authorization::Allow
        }))
        .unwrap();
        let polled = poll_messages(&conn, worker).unwrap();
        conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        let rival = rival.lock().unwrap().take().expect("the rival poll ran");
        assert!(
            rival.is_err(),
            "a rival poll during the selection cannot take the write lock: {rival:?}"
        );
        assert_eq!(
            polled.iter().map(|m| m.message_id).collect::<Vec<_>>(),
            [message.message_id]
        );
        assert_eq!(status_state(&conn, message.message_id), "completed");
    }

    #[test]
    fn concurrent_polls_on_separate_connections_return_disjoint_sets_covering_the_pending_set() {
        let dir = TempDir::new().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "concurrent");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        for round in 0..20 {
            let mut expected = std::collections::BTreeSet::new();
            for index in 0..10 {
                expected.insert(
                    send_message(&mut conn, director, worker, &format!("{round}-{index}"))
                        .unwrap()
                        .message_id,
                );
            }
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let pollers = (0..2)
                .map(|_| {
                    let reader = second_connection(&conn);
                    let barrier = std::sync::Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        poll_messages(&reader, worker)
                            .unwrap()
                            .into_iter()
                            .map(|message| {
                                assert_eq!(message.status, crate::broker::MessageStatus::Completed);
                                message.message_id
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            let results = pollers
                .into_iter()
                .map(|poller| poller.join().unwrap())
                .collect::<Vec<_>>();
            let mut union = std::collections::BTreeSet::new();
            for id in results.iter().flatten() {
                assert!(union.insert(*id), "message {id} returned by both polls");
            }
            assert_eq!(union, expected, "round {round}");
            assert!(poll_messages(&conn, worker).unwrap().is_empty());
        }
    }

    #[test]
    fn message_ids_from_interleaved_inserts_increase_in_commit_order() {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::AtomicU64;

        let dir = TempDir::new().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "commit-order");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let commits = Arc::new(AtomicU64::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let senders = (0..4)
            .map(|sender| {
                let mut writer = second_connection(&conn);
                let commits = Arc::clone(&commits);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let committed = Arc::new(Mutex::new(None));
                    let hook = Arc::clone(&committed);
                    writer
                        .commit_hook(Some(move || {
                            *hook.lock().unwrap() = Some(commits.fetch_add(1, Ordering::SeqCst));
                            false
                        }))
                        .unwrap();
                    barrier.wait();
                    (0..25)
                        .map(|index| {
                            let id = send_message(
                                &mut writer,
                                director,
                                worker,
                                &format!("{sender}-{index}"),
                            )
                            .unwrap()
                            .message_id;
                            let sequence = committed.lock().unwrap().take().unwrap();
                            (sequence, id)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut observed = senders
            .into_iter()
            .flat_map(|sender| sender.join().unwrap())
            .collect::<Vec<_>>();
        observed.sort();
        assert_eq!(observed.len(), 100);
        assert!(
            observed.windows(2).all(|pair| pair[0].1 < pair[1].1),
            "IDs ordered by commit sequence must strictly increase: {observed:?}"
        );
    }

    #[test]
    fn self_and_pending_placement_messages_have_distinct_preview_states() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = initialized_file_conn(&dir);
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
        assert_eq!(
            poll_messages(&conn, pending)
                .unwrap()
                .iter()
                .map(|m| m.message_id)
                .collect::<Vec<_>>(),
            [assignment.message_id]
        );
    }

    #[test]
    fn sending_requires_active_same_fleet_members_and_a_live_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
        let worker = register(&mut conn, fleet, "worker", None);
        let (_, other) = create_fleet_fixture(&mut conn, "other");
        for (sender, recipient) in [(999, worker), (director, 999), (director, other)] {
            assert!(matches!(
                send_message(&mut conn, sender, recipient, "invalid"),
                Err(CafleetError::App(_))
            ));
        }
        crate::broker::members::test_support::deregister_member(&mut conn, worker).unwrap();
        assert!(send_message(&mut conn, director, worker, "inactive recipient").is_err());
        assert!(send_message(&mut conn, worker, director, "inactive sender").is_err());
        conn.execute(
            "UPDATE members SET status='deregistered' WHERE member_id=?1",
            [director],
        )
        .unwrap();
        assert!(send_message(&mut conn, director, director, "inactive root").is_err());
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
        let mut conn = initialized_file_conn(&dir);
        let (_, director) = create_fleet_fixture(&mut conn, "corrupt");
        let message = send_message(&mut conn, director, director, "body").unwrap();
        conn.execute(
            "UPDATE messages SET status_state='corrupt' WHERE message_id=?1",
            [message.message_id],
        )
        .unwrap();
        assert!(
            matches!(get_message(&conn, message.message_id), Err(CafleetError::InvalidStoredValue { field, value }) if field == "messages.status_state" && value == "corrupt")
        );
        assert!(poll_messages(&conn, director).unwrap().is_empty());
        conn.execute(
            "UPDATE messages SET status_state='input_required', text=X'00' WHERE message_id=?1",
            [message.message_id],
        )
        .unwrap();
        assert!(get_message(&conn, message.message_id).is_err());
        assert!(poll_messages(&conn, director).is_err());
        assert_eq!(status_state(&conn, message.message_id), "input_required");
        assert!(conn.is_autocommit());
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
        let mut conn = initialized_file_conn(&dir);
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
            [ids[1], ids[0], ids[2]],
            "the order is evaluated on the pre-consumption timestamps"
        );
        assert!(rows.iter().all(|row| row.to_member_id == worker));
        assert!(
            rows.iter()
                .all(|row| row.status == crate::broker::MessageStatus::Completed
                    && row.status_timestamp == rows[0].status_timestamp)
        );
        assert!(rows[0].status_timestamp.as_str() > "2026-02-01T00:00:00+00:00");
        for (member, text) in [(peer, "other inbox"), (director, "outgoing")] {
            let untouched = broker::poll_messages(&conn, member).unwrap();
            assert_eq!(untouched.len(), 1, "{text}");
            assert_eq!(untouched[0].text, text);
        }
    }

    #[test]
    fn send_failure_rolls_back_the_message_without_leaving_an_identifier_gap() {
        let dir = TempDir::new().unwrap();
        let mut conn = initialized_file_conn(&dir);
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
    fn oldest_pending_preview_uses_creation_order_and_skips_consumed_and_notified_rows() {
        let dir = TempDir::new().unwrap();
        let mut conn = initialized_file_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "preview");
        let worker = register(&mut conn, fleet, "worker", None);
        let mut ids = vec![
            broker::send_message(&mut conn, director, worker, "consumed")
                .unwrap()
                .message_id,
        ];
        assert_eq!(broker::poll_messages(&conn, worker).unwrap().len(), 1);
        for text in ["notified", "oldest", "newest"] {
            ids.push(
                broker::send_message(&mut conn, director, worker, text)
                    .unwrap()
                    .message_id,
            );
        }
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
