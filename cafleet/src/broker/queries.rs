use rusqlite::Connection;

use super::messaging::message_row;
use super::records::MessageRecord;
use crate::error::CafleetError;

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
