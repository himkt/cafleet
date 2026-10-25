use rusqlite::Connection;

use crate::broker;
use crate::error::CafleetError;

pub(crate) struct RegistrationGuard<'a> {
    conn: &'a mut Connection,
    member_id: Option<i64>,
}

impl<'a> RegistrationGuard<'a> {
    pub(crate) fn new(conn: &'a mut Connection, member_id: i64) -> Self {
        Self {
            conn,
            member_id: Some(member_id),
        }
    }

    pub(crate) fn connection(&mut self) -> &mut Connection {
        self.conn
    }

    pub(crate) fn finish(&mut self) {
        self.member_id = None;
    }

    fn cleanup(&mut self) -> Option<String> {
        let member_id = self.member_id.take()?;
        let result =
            broker::deregister_member(self.conn, member_id).map_err(|error| error.to_string());
        result
            .err()
            .map(|error| format!("cleanup failed for member {member_id}: {error}"))
    }

    pub(crate) fn rollback(&mut self, primary: CafleetError) -> CafleetError {
        match self.cleanup() {
            Some(diagnostic) => primary.with_cleanup(diagnostic),
            None => primary,
        }
    }
}

impl Drop for RegistrationGuard<'_> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_guard_drop_obeys_ownership() {
        use crate::broker::tests as common;
        for finish in [false, true] {
            let mut conn = Connection::open_in_memory().unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            let (fleet, _) = common::create_fleet(&mut conn, "guard");
            let member = common::register(&mut conn, fleet, "worker", None);
            let mut guard = RegistrationGuard::new(&mut conn, member);
            if finish {
                guard.finish();
            }
            drop(guard);
            let status: String = conn
                .query_row(
                    "SELECT status FROM members WHERE member_id=?1",
                    [member],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, if finish { "active" } else { "deregistered" });
        }
    }

    #[test]
    fn rollback_keeps_primary_exit_and_reports_cleanup_failure_once() {
        use crate::broker::tests as common;
        for cleanup_fails in [false, true] {
            let mut conn = Connection::open_in_memory().unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            let (fleet, _) = common::create_fleet(&mut conn, "rollback");
            let worker = common::register(&mut conn, fleet, "worker", None);
            if cleanup_fails {
                conn.execute_batch(
                    "CREATE TRIGGER refuse_cleanup BEFORE UPDATE OF status ON members
                     BEGIN SELECT RAISE(ABORT, 'cleanup blocked'); END;",
                )
                .unwrap();
            }
            let mut guard = RegistrationGuard::new(&mut conn, worker);
            let error = guard.rollback(CafleetError::Usage("invalid placeholder".into()));
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().starts_with("invalid placeholder"));
            assert_eq!(error.to_string().contains("cleanup blocked"), cleanup_fails);
            assert_eq!(
                guard
                    .rollback(CafleetError::App("second".into()))
                    .to_string(),
                "second"
            );
            drop(guard);
            assert_eq!(
                broker::get_member(&conn, worker, fleet).unwrap().is_some(),
                cleanup_fails
            );
        }
    }
}
