//! Ownership and explicit compensation for CLI creation operations.

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
