//! Read-only message queries — inbox / sent / timeline / `get_message`
//! visibility rule (SPEC §6.2 *Queries*). The colocated tests pin the
//! contract; see [`super::test_support`] for the API.

use rusqlite::{Connection, params};

use super::members::db_err;
use super::messaging::{MESSAGE_COLUMNS, map_message_row, message_row};
use super::records::MessageRecord;
use crate::error::CafleetError;

fn message_list(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<MessageRecord>, CafleetError> {
    let mut stmt = conn.prepare(sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params, map_message_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn list_inbox(conn: &Connection, member_id: i64) -> Result<Vec<MessageRecord>, CafleetError> {
    history_records(conn, "owner_member_id", member_id)
}

pub fn list_sent(conn: &Connection, member_id: i64) -> Result<Vec<MessageRecord>, CafleetError> {
    history_records(conn, "from_member_id", member_id)
}

fn history_records(
    conn: &Connection,
    member_column: &str,
    member_id: i64,
) -> Result<Vec<MessageRecord>, CafleetError> {
    let sql = format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE {member_column}=?1 AND type='unicast' ORDER BY status_timestamp DESC, message_id DESC"
    );
    message_list(conn, &sql, [member_id])
}

/// The fleet's deliveries (scoped via the owning member's fleet), newest first,
/// hard-capped at `limit`.
pub fn list_timeline(
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
pub fn get_message(conn: &Connection, message_id: i64) -> Result<MessageRecord, CafleetError> {
    let message = message_row(conn, message_id)?
        .ok_or_else(|| CafleetError::Value(format!("Message {message_id} not found")))?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use crate::broker::{self, test_support as common};
    use crate::error::CafleetError;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Connection, i64, i64, i64) {
        let dir = TempDir::new().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let (fleet, director) = common::create_fleet(&mut conn, "queries");
        let worker = common::register(&mut conn, fleet, "worker", None);
        (dir, conn, fleet, director, worker)
    }

    fn send(conn: &mut Connection, sender: i64, recipient: i64, text: &str) -> i64 {
        broker::send_message(conn, sender, &recipient.to_string(), text)
            .unwrap()
            .message_id
    }

    #[test]
    fn get_message_returns_the_complete_ordered_envelope() {
        let (_dir, mut conn, _fleet, director, worker) = fixture();
        let id = send(&mut conn, director, worker, "hello");
        let row = broker::get_message(&conn, id).unwrap();
        let envelope = crate::presentation::message_envelope(&row);
        let message = &envelope["message"];
        assert_eq!(message["message_id"], id);
        assert_eq!(message["to_member_id"], worker);
        assert_eq!(message["text"], "hello");
        assert_eq!(
            message
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "message_id",
                "from_member_id",
                "to_member_id",
                "created_at",
                "status_state",
                "status_timestamp",
                "text"
            ]
        );
    }

    #[test]
    fn get_message_missing_is_the_existence_error() {
        let (_dir, conn, _fleet, _director, _worker) = fixture();
        let error = broker::get_message(&conn, 999).unwrap_err();
        assert!(matches!(error, CafleetError::Value(_)));
        assert_eq!(error.message(), "Message 999 not found");
    }

    #[test]
    fn invalid_stored_message_status_returns_the_domain_error() {
        let (_dir, mut conn, _fleet, director, worker) = fixture();
        let id = send(&mut conn, director, worker, "work");
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute(
            "UPDATE messages SET status_state='corrupt-enum' WHERE message_id=?1",
            [id],
        )
        .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            broker::get_message(&conn, id)
        }));
        let error = result
            .expect("invalid stored status must return an error")
            .unwrap_err();
        assert!(
            matches!(error, CafleetError::InvalidStoredValue { field, value }
            if field == "messages.status_state" && value == "corrupt-enum")
        );
    }
}
