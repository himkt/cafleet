use super::{
    InputFailure, ProcessIdentity, Result, RunLock, SubmissionStage, Transport, check_deadline,
    ensure, error, stable_run_identity, with_database_deadline,
};
use crate::broker::members::db_err;
use crate::broker::runtime::{self as storage, OwnerKind, RunRecord, RunStatus};
use crate::coding_agent::ProfileCatalog;
use crate::error::CafleetError;
use rusqlite::{Connection, TransactionBehavior};
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
mod contract_tests;

pub(crate) trait RequestTokenSource: Send {
    #[expect(
        dead_code,
        reason = "the reviewed permission integration requests random tokens"
    )]
    fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()>;
}

#[expect(
    dead_code,
    reason = "the reviewed input phases publish hook boundaries"
)]
pub(crate) enum InputBoundary {
    Reserved,
    BeforeWrite,
    AfterWrite,
    TerminalState,
    Settled,
    Retired,
}

#[expect(
    dead_code,
    reason = "the reviewed input phases publish actual stage observations"
)]
pub(crate) struct InputCheckpoint {
    pub input_id: Option<i64>,
    pub stage: SubmissionStage,
    pub written_bytes: u64,
    pub output_seq: u64,
}

pub(crate) trait InputHooks: Send + Sync {
    #[expect(
        dead_code,
        reason = "the reviewed input phases call passive checkpoints"
    )]
    fn checkpoint(
        &self,
        boundary: InputBoundary,
        state: &InputCheckpoint,
        deadline: Instant,
    ) -> Result<()>;
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reviewed service integration selects inert production hooks"
    )
)]
pub(crate) struct InertInputHooks;

impl InputHooks for InertInputHooks {
    fn checkpoint(&self, _: InputBoundary, _: &InputCheckpoint, _: Instant) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct InputServices {
    #[expect(
        dead_code,
        reason = "the reviewed request integration selects validated plans"
    )]
    pub catalog: ProfileCatalog,
    #[expect(
        dead_code,
        reason = "the reviewed permission integration requests random tokens"
    )]
    pub token_source: Box<dyn RequestTokenSource>,
    #[expect(
        dead_code,
        reason = "the reviewed input phases call passive checkpoints"
    )]
    pub hooks: Arc<dyn InputHooks>,
}

pub(crate) struct InputCoordinator {
    run: RunRecord,
    #[expect(
        dead_code,
        reason = "the reviewed input validation rechecks the retained original lock"
    )]
    lock: Arc<RunLock>,
    #[expect(
        dead_code,
        reason = "the reviewed request integration consumes retained input services"
    )]
    services: InputServices,
    manual_generation: u64,
    manual_accounting_failed: bool,
}

impl InputCoordinator {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed service integration constructs the coordinator"
        )
    )]
    pub(crate) fn new(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        services: InputServices,
        deadline: Instant,
    ) -> Result<Self> {
        let run = with_database_deadline(conn, deadline, |conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(db_err)?;
            let current = storage::validate_owner(&tx, &run.owner())?;
            ensure(
                stable_run_identity(&current) == stable_run_identity(run),
                "input coordinator run identity changed",
            )?;
            ensure(
                current.owner_kind == OwnerKind::Runner && current.status == RunStatus::Running,
                "input coordinator requires a running runner owner",
            )?;
            let process = ProcessIdentity::current()?;
            ensure(
                current.owner_pid == process.pid
                    && current.owner_start_identity == process.start_identity
                    && current.runner_pid == Some(process.pid)
                    && current.runner_start_identity.as_deref()
                        == Some(process.start_identity.as_str()),
                "input coordinator runner generation mismatch",
            )?;
            lock.validate(&tx, &current.owner())?;
            check_deadline(Instant::now(), deadline)?;
            tx.commit().map_err(db_err)?;
            Ok(current)
        })?;
        Ok(Self {
            run,
            lock,
            services,
            manual_generation: 0,
            manual_accounting_failed: false,
        })
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed service integration accounts manual progress"
        )
    )]
    pub(super) fn progress_with_manual<T>(
        &mut self,
        transport: &mut Transport,
        step: impl FnOnce(&mut Transport) -> Result<T>,
    ) -> std::result::Result<T, InputFailure> {
        let before = transport.human_generation();
        let progress = step(transport);
        let accounting = transport
            .human_generation()
            .checked_sub(before)
            .ok_or_else(|| error("manual input generation decreased"))
            .and_then(|count| {
                usize::try_from(count).map_err(|_| error("manual input delta exceeds count range"))
            })
            .and_then(|count| {
                if count == 0 {
                    Ok(())
                } else {
                    self.observe_manual(count)
                }
            });
        if accounting.is_err() {
            self.manual_accounting_failed = true;
        }
        combine_manual_progress(progress, accounting).map_err(|source| InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: None,
            input_id: None,
            operation_key: None,
        })
    }

    pub(crate) fn observe_manual(&mut self, count: usize) -> Result<()> {
        let next = u64::try_from(count)
            .map_err(|_| error("manual input count exceeds generation range"))
            .and_then(|count| {
                self.manual_generation
                    .checked_add(count)
                    .ok_or_else(|| error("manual input generation exhausted"))
            });
        match next {
            Ok(next) => {
                self.manual_generation = next;
                Ok(())
            }
            Err(cause) => {
                self.manual_accounting_failed = true;
                Err(cause)
            }
        }
    }
}

fn combine_manual_progress<T>(progress: Result<T>, accounting: Result<()>) -> Result<T> {
    match (progress, accounting) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(cause), Ok(())) | (Ok(_), Err(cause)) => Err(cause),
        (Err(primary), Err(accounting)) => Err(CafleetError::WithDiagnostics {
            cause: Box::new(primary),
            diagnostics: vec![accounting.message().into_owned()],
        }),
    }
}
