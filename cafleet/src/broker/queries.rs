//! Read-only message queries — inbox / sent / timeline / `get_message_record`
//! visibility rule (SPEC §6.2 *Queries*). The colocated tests pin the
//! contract; see [`super::test_support`] for the API.

use rusqlite::{Connection, params};

use super::members::db_err;
use super::messaging::{map_message_row, message_row};
use super::records::MessageRecord;
use crate::error::CafleetError;

fn message_list(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<MessageRecord>, CafleetError> {
    message_list_observed(conn, sql, params, &|_| {})
}

fn message_list_observed(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError> {
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params, map_message_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    observe(&stmt);
    Ok(rows)
}

const MESSAGE_COLUMNS: &str = "message_id, owner_member_id, from_member_id, to_member_id, \
     type, created_at, status_state, status_timestamp, origin_message_id, text";

pub(crate) const HISTORY_LIMIT_ERROR: &str = "limit must be an integer between 1 and 1000";

/// An optional SQL row bound; the default preserves complete member history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryOptions {
    pub limit: Option<usize>,
}

impl HistoryOptions {
    pub(crate) fn validate(self) -> Result<Self, CafleetError> {
        if self.limit.is_some_and(|limit| !(1..=1000).contains(&limit)) {
            return Err(CafleetError::Value(HISTORY_LIMIT_ERROR.to_string()));
        }
        Ok(self)
    }
}

/// Every delivery the member received, acked rows included; summaries are the
/// sender's bookkeeping, never a delivery. This entry point stays unbounded.
pub fn list_inbox_records(
    conn: &Connection,
    member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError> {
    list_inbox_records_with_options(conn, member_id, HistoryOptions::default())
}

/// Every sent delivery, including broadcast fan-out but excluding summaries.
/// This entry point stays unbounded.
pub fn list_sent_records(
    conn: &Connection,
    member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError> {
    list_sent_records_with_options(conn, member_id, HistoryOptions::default())
}

pub fn list_inbox_records_with_options(
    conn: &Connection,
    member_id: i64,
    options: HistoryOptions,
) -> Result<Vec<MessageRecord>, CafleetError> {
    list_inbox_records_observed(conn, member_id, options, &|_| {})
}

pub fn list_sent_records_with_options(
    conn: &Connection,
    member_id: i64,
    options: HistoryOptions,
) -> Result<Vec<MessageRecord>, CafleetError> {
    list_sent_records_observed(conn, member_id, options, &|_| {})
}

pub(crate) fn list_inbox_records_observed(
    conn: &Connection,
    member_id: i64,
    options: HistoryOptions,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError> {
    history_records(conn, "owner_member_id", member_id, options, observe)
}

pub(crate) fn list_sent_records_observed(
    conn: &Connection,
    member_id: i64,
    options: HistoryOptions,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError> {
    history_records(conn, "from_member_id", member_id, options, observe)
}

fn history_records(
    conn: &Connection,
    member_column: &str,
    member_id: i64,
    options: HistoryOptions,
    observe: &dyn Fn(&rusqlite::Statement<'_>),
) -> Result<Vec<MessageRecord>, CafleetError> {
    let options = options.validate()?;
    // The column is selected only by the two fixed entry points above.
    let mut sql = format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages \
         WHERE {member_column}=?1 AND type='unicast' \
         ORDER BY status_timestamp DESC, message_id DESC"
    );
    match options.limit {
        Some(limit) => {
            sql.push_str(" LIMIT ?2");
            message_list_observed(conn, &sql, params![member_id, limit as i64], observe)
        }
        None => message_list_observed(conn, &sql, [member_id], observe),
    }
}

/// The fleet's deliveries (scoped via the owning member's fleet), newest first,
/// hard-capped at `limit`.
pub fn list_timeline_records(
    conn: &Connection,
    fleet_id: i64,
    limit: usize,
) -> Result<Vec<MessageRecord>, CafleetError> {
    message_list(
        conn,
        "SELECT g.message_id, g.owner_member_id, g.from_member_id, g.to_member_id, \
                g.type, g.created_at, g.status_state, g.status_timestamp, \
                g.origin_message_id, g.text \
         FROM messages g JOIN members m ON m.member_id=g.owner_member_id \
         WHERE m.fleet_id=?1 AND g.type='unicast' \
         ORDER BY g.status_timestamp DESC, g.message_id DESC LIMIT ?2",
        params![fleet_id, limit as i64],
    )
}

/// Fetch one message by id — the fleet is derived from the message row;
/// existence is the only guard (SPEC §6.2).
pub fn get_message_record(
    conn: &Connection,
    message_id: i64,
) -> Result<MessageRecord, CafleetError> {
    let message = message_row(conn, message_id)?
        .ok_or_else(|| CafleetError::Value(format!("Message {message_id} not found")))?;
    Ok(message)
}

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

        let first = common::send(&mut conn, &notifier, director_id, member_id, "one");
        let second = common::send(&mut conn, &notifier, director_id, member_id, "two");
        let first_id = first["message"]["message_id"].as_i64().unwrap();
        let second_id = second["message"]["message_id"].as_i64().unwrap();
        broker::ack_message_record(&mut conn, first_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        broker::broadcast_message_record(&mut conn, &notifier, MAX_TEXT_LEN, member_id, "x")
            .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
            .unwrap();

        let inbox = broker::list_inbox_records(&conn, member_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
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
        broker::broadcast_message_record(&mut conn, &notifier, MAX_TEXT_LEN, director_id, "fanout")
            .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
            .unwrap();

        let sent = broker::list_sent_records(&conn, director_id)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            sent.len(),
            3,
            "one delivery per peer (monitor included); summary excluded"
        );
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

        common::send(&mut conn, &notifier, director_a, member_a, "one");
        let second = common::send(&mut conn, &notifier, director_a, member_a, "two");
        let third = common::send(&mut conn, &notifier, member_a, director_a, "three");
        let foreign = common::send(&mut conn, &notifier, director_b, member_b, "other");

        let timeline = broker::list_timeline_records(&conn, fleet_a, 2)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(timeline.len(), 2, "capped at the supplied limit");
        assert_eq!(
            timeline[0]["message_id"],
            third["message"]["message_id"].as_i64().unwrap()
        );
        assert_eq!(
            timeline[1]["message_id"],
            second["message"]["message_id"].as_i64().unwrap()
        );

        let full = broker::list_timeline_records(&conn, fleet_a, 200)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
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
        let sent = common::send(&mut conn, &notifier, director_id, member_id, "hi");
        let message_id = sent["message"]["message_id"].as_i64().unwrap();

        let result = broker::get_message_record(&conn, message_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        assert_eq!(result["message"]["message_id"], message_id);
        assert_eq!(result["message"]["text"], "hi");
    }

    #[test]
    fn get_message_missing_is_the_existence_error() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let _ = create_fleet(&mut conn, "alpha");

        let err = broker::get_message_record(&conn, 999)
            .map(|record| crate::presentation::message_envelope(&record))
            .expect_err("missing message");
        assert!(matches!(err, CafleetError::Value(_)));
        assert_eq!(err.message(), "Message 999 not found");
    }

    #[test]
    fn get_message_resolves_a_broadcast_summary_via_its_sender() {
        let dir = TempDir::new().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet_id, director_id) = create_fleet(&mut conn, "alpha");
        register(&mut conn, fleet_id, "worker", Some("%2"));
        let notifier = FakeNotifier::succeeding();
        let result = broker::broadcast_message_record(
            &mut conn,
            &notifier,
            MAX_TEXT_LEN,
            director_id,
            "fanout",
        )
        .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
        .unwrap();
        let summary_id = result[0]["message"]["message_id"].as_i64().unwrap();

        let fetched = broker::get_message_record(&conn, summary_id)
            .map(|record| crate::presentation::message_envelope(&record))
            .unwrap();
        assert_eq!(fetched["message"]["type"], "broadcast_summary");
        assert_eq!(
            fetched["message"]["to_member_id"],
            serde_json::Value::Null,
            "the NULL recipient endpoint is dropped, not dereferenced"
        );
    }
}

#[cfg(test)]
mod timeline_regressions {
    use super::*;
    use crate::broker::{self, test_support as common};
    use common::{FakeNotifier, MAX_TEXT_LEN};
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Connection, i64, i64, i64) {
        let dir = tempfile::Builder::new()
            .prefix(".timeline-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "timeline");
        let worker = common::register(&mut conn, fleet, "worker", None);
        (dir, conn, fleet, director, worker)
    }

    #[test]
    fn timeline_broadcast_counts_only_two_deliveries_through_ack_zero_one_two() {
        let (_dir, mut conn, fleet, director, _) = fixture();
        let result = broker::broadcast_message_record(
            &mut conn,
            &FakeNotifier::succeeding(),
            MAX_TEXT_LEN,
            director,
            "work",
        )
        .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["recipients"], 2);
        let summary = &result[0]["message"];
        let summary_id = summary["message_id"].as_i64().unwrap();
        assert_eq!(summary["type"], "broadcast_summary");
        assert_eq!(summary["status_state"], "completed");
        assert!(summary["to_member_id"].is_null());
        for acked in 0..=2 {
            let rows = list_timeline_records(&conn, fleet, 200)
                .map(|records| {
                    records
                        .iter()
                        .map(crate::presentation::message)
                        .collect::<Vec<_>>()
                })
                .unwrap();
            assert_eq!(rows.len(), 2, "summary is not a recipient or ACK");
            assert!(rows.iter().all(|r| r["type"] == "unicast"
                && r["origin_message_id"] == summary_id
                && r["to_member_id"].is_i64()));
            assert_eq!(
                rows.iter()
                    .filter(|r| r["status_state"] == "completed")
                    .count(),
                acked
            );
            assert_eq!(
                get_message_record(&conn, summary_id)
                    .map(|record| crate::presentation::message_envelope(&record))
                    .unwrap()["message"],
                *summary,
                "timeline/ACK never removes or rewrites the summary"
            );
            if let Some(row) = rows.iter().find(|r| r["status_state"] == "input_required") {
                broker::ack_message_record(&mut conn, row["message_id"].as_i64().unwrap())
                    .map(|record| crate::presentation::message_envelope(&record))
                    .unwrap();
            }
        }
        let count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn timeline_empty_fleet_returns_no_rows() {
        let (_dir, conn, fleet, _, _) = fixture();
        assert!(
            list_timeline_records(&conn, fleet, 200)
                .map(|records| records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn timeline_summary_only_fleet_returns_no_deliveries_but_keeps_show_result() {
        let (_dir, mut conn, fleet, director, worker) = fixture();
        let monitor = broker::active_monitor_member_id(&conn, fleet)
            .unwrap()
            .unwrap();
        broker::deregister_member(&mut conn, monitor).unwrap();
        broker::deregister_member(&mut conn, worker).unwrap();
        let result = broker::broadcast_message_record(
            &mut conn,
            &FakeNotifier::succeeding(),
            MAX_TEXT_LEN,
            director,
            "nobody",
        )
        .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
        .unwrap();
        assert_eq!(result[0]["recipients"], 0);
        let id = result[0]["message"]["message_id"].as_i64().unwrap();
        assert!(
            list_timeline_records(&conn, fleet, 200)
                .map(|records| records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            get_message_record(&conn, id)
                .map(|record| crate::presentation::message_envelope(&record))
                .unwrap()["message"],
            result[0]["message"]
        );
    }

    #[test]
    fn timeline_scope_uses_owner_fleet_even_when_sender_and_recipient_disagree() {
        let (_dir, mut conn, fleet_a, director_a, worker_a) = fixture();
        let (fleet_b, director_b) = common::create_fleet(&mut conn, "foreign");
        let worker_b = common::register(&mut conn, fleet_b, "foreign worker", None);
        let notifier = FakeNotifier::succeeding();
        let local = common::send(
            &mut conn,
            &notifier,
            director_a,
            worker_a,
            "local endpoints",
        )["message"]["message_id"]
            .as_i64()
            .unwrap();
        let foreign = common::send(
            &mut conn,
            &notifier,
            director_b,
            worker_b,
            "foreign endpoints",
        )["message"]["message_id"]
            .as_i64()
            .unwrap();
        // Deliberately distinguish ownership from either endpoint. This is a
        // read-scope fixture, not a cross-fleet send API contract.
        conn.execute(
            "UPDATE messages SET owner_member_id=?1 WHERE message_id=?2",
            params![worker_b, local],
        )
        .unwrap();
        conn.execute(
            "UPDATE messages SET owner_member_id=?1 WHERE message_id=?2",
            params![worker_a, foreign],
        )
        .unwrap();
        broker::deregister_member(&mut conn, worker_a).unwrap();
        let a = list_timeline_records(&conn, fleet_a, 200)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        let b = list_timeline_records(&conn, fleet_b, 200)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            a.iter()
                .map(|r| r["message_id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [foreign]
        );
        assert_eq!(
            b.iter()
                .map(|r| r["message_id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [local]
        );
    }

    #[test]
    fn timeline_uses_status_timestamp_then_descending_id_not_created_at() {
        let (_dir, mut conn, fleet, director, worker) = fixture();
        for text in ["first", "second", "third"] {
            common::send(
                &mut conn,
                &FakeNotifier::succeeding(),
                director,
                worker,
                text,
            );
        }
        conn.execute_batch("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00', created_at='2099-01-01T00:00:00+00:00' WHERE message_id=3;
            UPDATE messages SET status_timestamp='2026-02-01T00:00:00+00:00', created_at='2020-01-01T00:00:00+00:00' WHERE message_id IN (1,2);").unwrap();
        let rows = list_timeline_records(&conn, fleet, 200)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["message_id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [2, 1, 3]
        );
        assert!(rows.iter().all(|r| r["origin_message_id"].is_null()));
    }

    #[test]
    fn timeline_filters_summaries_before_cap_and_keeps_partial_broadcast_as_rows() {
        let (_dir, mut conn, fleet, director, worker) = fixture();
        let notifier = FakeNotifier::succeeding();
        broker::broadcast_message_record(&mut conn, &notifier, MAX_TEXT_LEN, director, "broadcast")
            .map(|record| vec![crate::presentation::broadcast_outcome(&record)])
            .unwrap();
        for _ in 0..199 {
            common::send(&mut conn, &notifier, director, worker, "single");
        }
        conn.execute_batch("UPDATE messages SET status_timestamp='2026-01-01T00:00:00+00:00';
            UPDATE messages SET status_timestamp='2099-01-01T00:00:00+00:00' WHERE type='broadcast_summary';").unwrap();
        let rows = list_timeline_records(&conn, fleet, 200)
            .map(|records| {
                records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(
            rows.len(),
            200,
            "filter before limit, not after fetching 200 mixed rows"
        );
        assert_eq!(
            rows.iter()
                .map(|r| r["message_id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            (3..=202).rev().collect::<Vec<_>>()
        );
        let partial: Vec<_> = rows
            .iter()
            .filter(|r| !r["origin_message_id"].is_null())
            .collect();
        assert_eq!(
            partial.len(),
            1,
            "row cap must not be widened to complete a group"
        );
        assert_eq!(partial[0]["status_state"], "input_required");
        assert_eq!(
            list_timeline_records(&conn, fleet, 201)
                .map(|records| records
                    .iter()
                    .map(crate::presentation::message)
                    .collect::<Vec<_>>())
                .unwrap()
                .len(),
            201
        );
        assert_eq!(
            get_message_record(&conn, 1)
                .map(|record| crate::presentation::message_envelope(&record))
                .unwrap()["message"]["type"],
            "broadcast_summary"
        );
    }
}

#[cfg(test)]
mod integrity_regressions {
    use crate::broker::{self, test_support as common};

    #[test]
    fn invalid_stored_message_kind_is_an_error_without_unwinding_or_fabricating_a_row() {
        invalid_message_field("type");
    }

    #[test]
    fn invalid_stored_message_status_is_an_error_without_unwinding_or_fabricating_a_row() {
        invalid_message_field("status_state");
    }

    fn invalid_message_field(field: &str) {
        let dir = tempfile::Builder::new()
            .prefix(".invalid-message-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (_, director) = common::create_fleet(&mut conn, "integrity");
        let id = common::send(
            &mut conn,
            &common::FakeNotifier::succeeding(),
            director,
            director,
            "work",
        )["message"]["message_id"]
            .as_i64()
            .unwrap();
        // Only the isolated fixture relaxes CHECK constraints; production
        // schema and ordinary writes remain untouched.
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute(
            &format!("UPDATE messages SET {field}='corrupt-enum' WHERE message_id=?1"),
            [id],
        )
        .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            broker::get_message_record(&conn, id)
                .map(|record| crate::presentation::message_envelope(&record))
        }));
        let error = result
            .expect("invalid stored enum must not panic")
            .expect_err("unknown enum must not become a successful wire row");
        assert_eq!(error.exit_code(), 1);
        assert!(
            matches!(error, crate::error::CafleetError::InvalidStoredValue {
            field: actual_field, value
        } if actual_field == format!("messages.{field}") && value == "corrupt-enum")
        );
    }
}

#[cfg(test)]
mod step7_unbounded_compatibility {
    use super::*;
    use crate::broker::test_support as common;

    #[test]
    fn history_existing_two_argument_apis_remain_unbounded_beyond_1000_deliveries() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let (fleet, sender) = common::create_fleet(&mut conn, "history");
        let owner = common::register(&mut conn, fleet, "owner", None);
        let tx = conn.transaction().unwrap();
        for id in 1..=1205 {
            tx.execute("INSERT INTO messages(message_id,owner_member_id,from_member_id,to_member_id,type,created_at,status_state,status_timestamp,origin_message_id,text) VALUES (?1,?2,?3,?2,'unicast','raw-created','input_required','same-status',NULL,'body')",params![id,owner,sender]).unwrap();
        }
        tx.commit().unwrap();
        for rows in [
            list_inbox_records(&conn, owner).unwrap(),
            list_sent_records(&conn, sender).unwrap(),
        ] {
            assert_eq!(rows.len(), 1205);
            assert_eq!(
                rows.iter().map(|row| row.message_id).collect::<Vec<_>>(),
                (1..=1205).rev().collect::<Vec<_>>()
            );
        }
    }
}
