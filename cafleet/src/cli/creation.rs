//! Ownership and explicit compensation for CLI creation operations.

#[cfg(test)]
pub(crate) mod test_support;

use rusqlite::Connection;

use crate::broker::{self, fleets::BootstrapHooks};
use crate::error::CafleetError;
use crate::multiplexer::Multiplexer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuardResource {
    Pane { pane_id: String },
    Registration { member_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanupEvent {
    PaneKillFinished {
        pane_id: String,
        error: Option<String>,
    },
    DeregisterFinished {
        member_id: i64,
        result: Result<bool, String>,
    },
    GuardDisarmed {
        resource: GuardResource,
    },
}

pub(crate) trait CreationHooks: BootstrapHooks {
    fn observe_cleanup(&self, _event: CleanupEvent) {}
}

pub(crate) struct NoopCreationHooks;
impl BootstrapHooks for NoopCreationHooks {}
impl CreationHooks for NoopCreationHooks {}

pub(crate) struct PaneGuard<'a> {
    mux: &'a dyn Multiplexer,
    pane_id: Option<String>,
    hooks: &'a dyn CreationHooks,
}

impl<'a> PaneGuard<'a> {
    pub(crate) fn new(
        mux: &'a dyn Multiplexer,
        pane_id: String,
        hooks: &'a dyn CreationHooks,
    ) -> Self {
        Self {
            mux,
            pane_id: Some(pane_id),
            hooks,
        }
    }

    pub(crate) fn finish(&mut self) {
        if let Some(pane_id) = self.pane_id.take() {
            self.hooks.observe_cleanup(CleanupEvent::GuardDisarmed {
                resource: GuardResource::Pane { pane_id },
            });
        }
    }

    fn cleanup(&mut self) -> Option<String> {
        let pane_id = self.pane_id.take()?;
        let error = self
            .mux
            .kill_pane(&pane_id, true)
            .err()
            .map(|error| error.to_string());
        self.hooks.observe_cleanup(CleanupEvent::PaneKillFinished {
            pane_id: pane_id.clone(),
            error: error.clone(),
        });
        self.hooks.observe_cleanup(CleanupEvent::GuardDisarmed {
            resource: GuardResource::Pane {
                pane_id: pane_id.clone(),
            },
        });
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
    hooks: &'a dyn CreationHooks,
}

impl<'a> RegistrationGuard<'a> {
    pub(crate) fn new(
        conn: &'a mut Connection,
        member_id: i64,
        hooks: &'a dyn CreationHooks,
    ) -> Self {
        Self {
            conn,
            member_id: Some(member_id),
            hooks,
        }
    }

    pub(crate) fn connection(&mut self) -> &mut Connection {
        self.conn
    }

    pub(crate) fn finish(&mut self) {
        if let Some(member_id) = self.member_id.take() {
            self.hooks.observe_cleanup(CleanupEvent::GuardDisarmed {
                resource: GuardResource::Registration { member_id },
            });
        }
    }

    fn cleanup(&mut self) -> Option<String> {
        let member_id = self.member_id.take()?;
        let result =
            broker::deregister_member(self.conn, member_id).map_err(|error| error.to_string());
        self.hooks
            .observe_cleanup(CleanupEvent::DeregisterFinished {
                member_id,
                result: result.clone(),
            });
        self.hooks.observe_cleanup(CleanupEvent::GuardDisarmed {
            resource: GuardResource::Registration { member_id },
        });
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
mod guard_regressions {
    use super::test_support::{Event, Fixture};
    use super::*;

    #[test]
    fn pane_finish_disarms_once_and_later_rollback_or_drop_cannot_kill() {
        let f = Fixture::new(false);
        let mux = f.mux(None, false, false);
        {
            let mut guard = PaneGuard::new(&mux, "w1:p9".into(), &f);
            guard.finish();
            guard.finish();
            assert_eq!(
                guard
                    .rollback(CafleetError::Usage("primary".into()))
                    .to_string(),
                "primary"
            );
        }
        assert_eq!(
            *f.events.borrow(),
            [Event::Cleanup(CleanupEvent::GuardDisarmed {
                resource: GuardResource::Pane {
                    pane_id: "w1:p9".into()
                },
            })]
        );
    }

    #[test]
    fn pane_rollback_disarms_after_one_attempt_even_when_kill_fails() {
        for fail in [false, true] {
            let f = Fixture::new(false);
            let mux = f.mux(None, fail, false);
            {
                let mut guard = PaneGuard::new(&mux, "w1:p9".into(), &f);
                let error = guard.rollback(CafleetError::Usage("primary".into()));
                assert!(matches!(error, CafleetError::Usage(_)));
                assert_eq!(error.exit_code(), 2);
                assert_eq!(
                    error.to_string().contains("cleanup failed for pane w1:p9:"),
                    fail
                );
                assert_eq!(
                    guard
                        .rollback(CafleetError::App("second".into()))
                        .to_string(),
                    "second"
                );
                guard.finish();
            }
            assert_eq!(
                f.timeline(),
                [
                    "get:error",
                    "write-lock:true",
                    if fail { "close:error" } else { "close:ok" },
                    if fail {
                        "cli-kill:error"
                    } else {
                        "cli-kill:ok"
                    },
                    "disarm:pane"
                ]
            );
            assert!(
                matches!(f.events.borrow().last(), Some(Event::Cleanup(CleanupEvent::GuardDisarmed { resource: GuardResource::Pane { pane_id } })) if pane_id == "w1:p9")
            );
        }
    }

    #[test]
    fn armed_pane_drop_is_a_single_last_resort_cleanup_attempt() {
        let f = Fixture::new(false);
        let mux = f.mux(None, true, false);
        drop(PaneGuard::new(&mux, "w1:p9".into(), &f));
        assert_eq!(
            f.timeline(),
            [
                "get:error",
                "write-lock:true",
                "close:error",
                "cli-kill:error",
                "disarm:pane"
            ]
        );
    }

    #[test]
    fn registration_finish_disarms_once_without_deregister_on_later_rollback_or_drop() {
        let f = Fixture::new(true);
        let mut conn = f.conn();
        crate::broker::test_support::register(&mut conn, 1, "worker", None);
        {
            let mut guard = RegistrationGuard::new(&mut conn, 3, &f);
            guard.finish();
            guard.finish();
            assert_eq!(
                guard
                    .rollback(CafleetError::App("primary".into()))
                    .to_string(),
                "primary"
            );
        }
        assert_eq!(
            *f.events.borrow(),
            [Event::Cleanup(CleanupEvent::GuardDisarmed {
                resource: GuardResource::Registration { member_id: 3 },
            })]
        );
        assert_eq!(f.count("member_placements"), 3);
        let status: String = conn
            .query_row("SELECT status FROM members WHERE member_id=3", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn registration_rollback_disarms_even_when_deregister_fails_and_drop_does_not_retry() {
        for fail in [false, true] {
            let f = Fixture::new(true);
            let mut conn = f.conn();
            crate::broker::test_support::register(&mut conn, 1, "worker", None);
            if fail {
                f.sql("CREATE TRIGGER fail_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'secondary deregister failure'); END;");
            }
            {
                let mut guard = RegistrationGuard::new(&mut conn, 3, &f);
                let error = guard.rollback(CafleetError::Usage("primary".into()));
                assert!(matches!(error, CafleetError::Usage(_)));
                assert_eq!(
                    error.to_string().contains("cleanup failed for member 3:"),
                    fail
                );
                assert_eq!(
                    guard
                        .rollback(CafleetError::App("second".into()))
                        .to_string(),
                    "second"
                );
                guard.finish();
            }
            assert_eq!(
                f.timeline(),
                [
                    if fail {
                        "deregister:error"
                    } else {
                        "deregister:ok"
                    },
                    "disarm:registration"
                ]
            );
            let events = f.events.borrow();
            assert!(
                matches!(&events[0], Event::Cleanup(CleanupEvent::DeregisterFinished { member_id: 3, result }) if result.is_err() == fail)
            );
            assert_eq!(
                events[1],
                Event::Cleanup(CleanupEvent::GuardDisarmed {
                    resource: GuardResource::Registration { member_id: 3 }
                })
            );
            if !fail {
                f.assert_deregistered();
            }
        }
    }

    #[test]
    fn armed_registration_drop_performs_real_deregister_once() {
        let f = Fixture::new(true);
        let mut conn = f.conn();
        crate::broker::test_support::register(&mut conn, 1, "worker", None);
        drop(RegistrationGuard::new(&mut conn, 3, &f));
        assert_eq!(f.timeline(), ["deregister:ok", "disarm:registration"]);
        f.assert_deregistered();
    }
}
