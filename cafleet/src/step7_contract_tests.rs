//! Phase A: connect from lib.rs with `#[cfg(test)] mod step7_contract_tests;`
//! after the approved HistoryOptions and observed query APIs are implemented.
//! These tests inspect executed statements and real rows; no query stubs.

use crate::broker::{self, HistoryOptions, records::MessageRecord, test_support as common};
use crate::error::CafleetError;
use rusqlite::{Connection, params};
use std::cell::{Cell, RefCell};

type Query = fn(&Connection, i64, HistoryOptions) -> Result<Vec<MessageRecord>, CafleetError>;
type ObservedQuery = fn(
    &Connection,
    i64,
    HistoryOptions,
    &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError>;

fn endpoints(sender: i64, owner: i64) -> [(i64, &'static str, Query, ObservedQuery); 2] {
    [
        (
            owner,
            "idx_messages_owner_member_status_ts",
            broker::list_inbox_records_with_options,
            broker::queries::list_inbox_records_observed,
        ),
        (
            sender,
            "idx_messages_from_member_status_ts",
            broker::list_sent_records_with_options,
            broker::queries::list_sent_records_observed,
        ),
    ]
}

fn fixture(count: usize) -> (Connection, i64, i64) {
    let mut conn = Connection::open_in_memory().unwrap();
    crate::db::migrate_to_head(&mut conn).unwrap();
    let (fleet, sender) = common::create_fleet(&mut conn, "history");
    let owner = common::register(&mut conn, fleet, "owner", None);
    let tx = conn.transaction().unwrap();
    for id in 1..=count as i64 {
        tx.execute("INSERT INTO messages(message_id,owner_member_id,from_member_id,to_member_id,type,created_at,status_state,status_timestamp,origin_message_id,text) VALUES (?1,?2,?3,?2,'unicast','2026-01-01T00:00:00Z','input_required','2026-01-02T00:00:00Z',NULL,'body')", params![id,owner,sender]).unwrap();
    }
    tx.execute("INSERT INTO messages(message_id,owner_member_id,from_member_id,to_member_id,type,created_at,status_state,status_timestamp,origin_message_id,text) VALUES (20000,?1,?2,NULL,'broadcast_summary','2026-02-01T00:00:00Z','completed','2026-02-01T00:00:00Z',20000,'summary')", params![owner,sender]).unwrap();
    tx.commit().unwrap();
    (conn, sender, owner)
}

fn ids(rows: &[MessageRecord]) -> Vec<i64> {
    rows.iter().map(|row| row.message_id).collect()
}

#[test]
fn history_options_preserve_unbounded_compatibility_and_all_boundary_result_counts() {
    assert_eq!(HistoryOptions::default(), HistoryOptions { limit: None });
    for count in [0, 200, 201, 1205] {
        let (conn, sender, owner) = fixture(count);
        for (subject, _, query, _) in endpoints(sender, owner) {
            let unbounded = query(&conn, subject, HistoryOptions::default()).unwrap();
            assert_eq!(
                ids(&unbounded),
                (1..=count as i64).rev().collect::<Vec<_>>()
            );
            for limit in [1, 200, 201, 1000] {
                let rows = query(&conn, subject, HistoryOptions { limit: Some(limit) }).unwrap();
                assert_eq!(
                    rows,
                    unbounded.iter().take(limit).cloned().collect::<Vec<_>>()
                );
            }
        }
        assert_eq!(
            broker::list_inbox_records(&conn, owner).unwrap(),
            broker::list_inbox_records_with_options(&conn, owner, HistoryOptions::default())
                .unwrap()
        );
        assert_eq!(
            broker::list_sent_records(&conn, sender).unwrap(),
            broker::list_sent_records_with_options(&conn, sender, HistoryOptions::default())
                .unwrap()
        );
    }
}

#[test]
fn history_observers_expose_bound_sql_limit_and_existing_index_plans() {
    let (conn, sender, owner) = fixture(1205);
    for (subject, index, query, observed) in endpoints(sender, owner) {
        for limit in [None, Some(1), Some(200), Some(201), Some(1000)] {
            let observations = RefCell::new(Vec::new());
            let observe = |stmt: &rusqlite::Statement<'_>| {
                observations
                    .borrow_mut()
                    .push((stmt.parameter_count(), stmt.expanded_sql().unwrap()))
            };
            let rows = observed(&conn, subject, HistoryOptions { limit }, &observe).unwrap();
            assert_eq!(
                rows,
                query(&conn, subject, HistoryOptions { limit }).unwrap()
            );
            assert_eq!(
                ids(&rows),
                (1..=1205)
                    .rev()
                    .take(limit.unwrap_or(1205))
                    .collect::<Vec<_>>()
            );
            let observations = observations.borrow();
            assert_eq!(observations.len(), 1);
            let (binds, sql) = &observations[0];
            assert_eq!(*binds, if limit.is_some() { 2 } else { 1 });
            let tokens = sql
                .split_whitespace()
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>();
            match limit {
                Some(n) => {
                    let at = tokens
                        .iter()
                        .position(|token| token == "limit")
                        .expect("the executed SELECT itself must have LIMIT");
                    assert_eq!(tokens[at + 1].trim_end_matches(';'), n.to_string());
                }
                None => assert!(!tokens.iter().any(|token| token == "limit")),
            }
            let mut explain = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan = explain
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(plan.iter().any(|detail| detail.contains(index)), "{plan:?}");
        }
    }
}

#[test]
fn history_invalid_direct_options_fail_before_sql_or_completion_callbacks() {
    let conn = Connection::open_in_memory().unwrap(); // no messages table
    for (subject, _, query, observed) in endpoints(1, 2) {
        for limit in [0, 1001, usize::MAX] {
            let calls = Cell::new(0);
            let observe = |_: &rusqlite::Statement<'_>| calls.set(calls.get() + 1);
            for result in [
                query(&conn, subject, HistoryOptions { limit: Some(limit) }),
                observed(
                    &conn,
                    subject,
                    HistoryOptions { limit: Some(limit) },
                    &observe,
                ),
            ] {
                assert!(
                    matches!(result,Err(CafleetError::Value(message)) if message == "limit must be an integer between 1 and 1000")
                );
            }
            assert_eq!(calls.get(), 0);
        }
    }
}

#[test]
fn history_zero_rows_are_observed_once_and_sql_failures_are_not_observed() {
    let (conn, sender, owner) = fixture(0);
    for (subject, _, _, observed) in endpoints(sender, owner) {
        let calls = Cell::new(0);
        let observe = |_: &rusqlite::Statement<'_>| calls.set(calls.get() + 1);
        assert!(
            observed(&conn, subject, HistoryOptions { limit: Some(1) }, &observe)
                .unwrap()
                .is_empty()
        );
        assert_eq!(calls.get(), 1);
        let missing = Connection::open_in_memory().unwrap();
        assert!(
            observed(
                &missing,
                subject,
                HistoryOptions { limit: Some(1) },
                &observe
            )
            .is_err()
        );
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn history_limit_excludes_corrupt_rows_before_decode_not_after_materialization() {
    let (conn, sender, owner) = fixture(3);
    conn.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE messages SET status_state='corrupt-status' WHERE message_id=1").unwrap();
    for (subject, _, _, observed) in endpoints(sender, owner) {
        let calls = Cell::new(0);
        let observe = |_: &rusqlite::Statement<'_>| calls.set(calls.get() + 1);
        let rows = observed(&conn, subject, HistoryOptions { limit: Some(2) }, &observe).unwrap();
        assert_eq!(ids(&rows), vec![3, 2]);
        assert_eq!(calls.get(), 1);
        let error = observed(&conn, subject, HistoryOptions::default(), &observe).unwrap_err();
        assert!(
            matches!(error,CafleetError::InvalidStoredValue { field,value } if field=="messages.status_state" && value=="corrupt-status")
        );
        assert_eq!(
            calls.get(),
            1,
            "a decode failure does not emit query completion"
        );
    }
}

#[test]
fn history_scope_order_and_ack_fields_survive_options_and_deregistration() {
    let (mut conn, sender, owner) = fixture(3);
    // Recipient deliberately differs from owner: inbox membership uses owner.
    conn.execute(
        "UPDATE messages SET to_member_id=?1 WHERE type='unicast'",
        [sender],
    )
    .unwrap();
    conn.execute("UPDATE messages SET created_at='2020-01-01T00:00:00Z',status_timestamp='2026-01-03T00:00:00Z',status_state='completed' WHERE message_id=1",[]).unwrap();
    broker::deregister_member(&mut conn, owner).unwrap();
    for (subject, _, query, _) in endpoints(sender, owner) {
        let rows = query(&conn, subject, HistoryOptions { limit: Some(3) }).unwrap();
        assert_eq!(ids(&rows), vec![1, 3, 2]);
        assert_eq!(rows[0].created_at, "2020-01-01T00:00:00Z");
        assert_eq!(rows[0].status, broker::records::MessageStatus::Completed);
        assert_eq!(rows[0].origin_message_id, None);
        assert_eq!(rows[0].to_member_id, Some(sender));
        assert!(
            query(&conn, 999999, HistoryOptions { limit: Some(1) })
                .unwrap()
                .is_empty()
        );
    }
    assert!(
        broker::list_inbox_records_with_options(&conn, sender, HistoryOptions { limit: Some(1) })
            .unwrap()
            .is_empty()
    );
    assert!(
        broker::list_sent_records_with_options(&conn, owner, HistoryOptions { limit: Some(1) })
            .unwrap()
            .is_empty()
    );
}
