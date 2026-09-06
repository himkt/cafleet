//! Ownership and explicit compensation for CLI creation operations.

use rusqlite::Connection;

use crate::broker;
use crate::error::CafleetError;
use crate::multiplexer::MultiplexerError;

type PaneKill<'a> = dyn Fn(&str) -> Result<(), MultiplexerError> + 'a;

pub(crate) struct PaneGuard<'a> {
    kill: Box<PaneKill<'a>>,
    pane_id: Option<String>,
}

impl<'a> PaneGuard<'a> {
    pub(crate) fn with_kill(pane_id: String, kill: Box<PaneKill<'a>>) -> Self {
        Self {
            kill,
            pane_id: Some(pane_id),
        }
    }

    pub(crate) fn finish(&mut self) {
        self.pane_id = None;
    }

    fn cleanup(&mut self) -> Option<String> {
        let pane_id = self.pane_id.take()?;
        let error = (self.kill)(&pane_id).err().map(|error| error.to_string());
        error.map(|error| format!("cleanup failed for pane {pane_id}: {error}"))
    }

    pub(crate) fn rollback(&mut self, primary: CafleetError) -> CafleetError {
        match self.cleanup() {
            Some(diagnostic) => primary.with_cleanup(diagnostic),
            None => primary,
        }
    }
}

impl Drop for PaneGuard<'_> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

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
    use std::cell::Cell;

    #[test]
    fn pane_guard_cleans_once_unless_finished() {
        for finish in [false, true] {
            for fail in [false, true] {
                let calls = Cell::new(0);
                let mut guard = PaneGuard::with_kill(
                    "%9".into(),
                    Box::new(|pane| {
                        assert_eq!(pane, "%9");
                        calls.set(calls.get() + 1);
                        if fail {
                            Err(MultiplexerError::new("close failed"))
                        } else {
                            Ok(())
                        }
                    }),
                );
                if finish {
                    guard.finish();
                }
                let error = guard.rollback(CafleetError::Usage("primary".into()));
                assert_eq!(error.exit_code(), 2);
                assert_eq!(error.to_string().contains("close failed"), !finish && fail);
                guard.rollback(CafleetError::App("second".into()));
                drop(guard);
                assert_eq!(calls.get(), usize::from(!finish));
            }
        }
    }

    #[test]
    fn registration_guard_drop_obeys_ownership() {
        use crate::broker::test_support as common;
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
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, if finish { "active" } else { "deregistered" });
        }
    }
}
