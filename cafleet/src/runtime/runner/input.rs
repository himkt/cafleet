use super::{
    ControlIdentity, DrainedInput, InputControlOperation, InputControlReply, InputControlRequest,
    InputDrain, InputEligibility, ObservationController, ProgrammaticWrite, PtyIo,
    RealInputDeadlineClock, SubmissionResult, SubmissionStatus, Visibility, canonical_json,
    receive_input_expiration, validate_operation_key,
};
use super::{
    InputFailure, ProcessIdentity, Result, RunLock, SubmissionStage, Transport, check_deadline,
    ensure, error, stable_run_identity, with_database_deadline,
};
use crate::broker::members::db_err;
use crate::broker::records::{MemberKind, Placement};
use crate::broker::runtime::{self as storage, OwnerKind, RunRecord, RunStatus};
use crate::coding_agent::ProfileCatalog;
use crate::coding_agent::{ComposerOperation, coding_agent};
use crate::error::CafleetError;
use crate::input::{Deadline, inherited_database_deadline, with_database_budget};
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
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

pub(crate) struct InputCheckpoint {
    pub input_id: Option<i64>,
    pub stage: SubmissionStage,
    pub written_bytes: u64,
    pub output_seq: u64,
}

pub(crate) trait InputHooks: Send + Sync {
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
    pub catalog: ProfileCatalog,
    #[expect(
        dead_code,
        reason = "the reviewed permission integration requests random tokens"
    )]
    pub token_source: Box<dyn RequestTokenSource>,
    pub hooks: Arc<dyn InputHooks>,
}

pub(crate) struct InputCoordinator {
    run: RunRecord,
    lock: Arc<RunLock>,
    services: InputServices,
    manual_generation: u64,
    manual_accounting_failed: bool,
    placement: Placement,
    operations: BTreeMap<i64, RetainedInput>,
    pending: Option<PendingInput>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    AwaitReservation,
    AwaitBeforeWrite,
    Armed,
    Blocked,
}

struct PendingInput {
    request: storage::NewInput,
    bytes: Arc<[u8]>,
    deadline: Instant,
    phase: InputPhase,
    input_id: Option<i64>,
}

struct RetainedInput {
    operation: storage::InputOperation,
    bytes: Arc<[u8]>,
    stage: SubmissionStage,
    written_bytes: u64,
    output_seq: u64,
    attempt_available: bool,
    leased: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardPermit {
    Fresh,
    Rejected,
    ConsumedPreIo,
    IoPossible,
    ResultRecorded,
}

pub(super) struct InputGuard<'a> {
    retained: &'a mut RetainedInput,
    proof: DrainedInput,
    deadline: Instant,
    permit: GuardPermit,
}

impl InputGuard<'_> {
    pub(super) fn begin_write(
        &mut self,
        transport: &Transport,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<Instant> {
        ensure(
            self.permit == GuardPermit::Fresh,
            "input write permit is consumed",
        )?;
        self.permit = GuardPermit::Rejected;
        self.deadline = self.deadline.min(deadline);
        check_deadline(Instant::now(), self.deadline)?;
        ensure(
            bytes == self.retained.bytes.as_ref(),
            "input bytes do not match the selected stage",
        )?;
        ensure(
            self.retained.operation.state == storage::InputState::InProgress
                && self.retained.leased,
            "input write requires its retained reservation",
        )?;
        transport.validate_input_drain(&self.proof, self.deadline)?;
        self.permit = GuardPermit::ConsumedPreIo;
        Ok(self.deadline)
    }

    pub(super) fn mark_io_possible(&mut self) {
        assert!(
            self.permit == GuardPermit::ConsumedPreIo,
            "possible I/O requires a validated consumed permit"
        );
        self.permit = GuardPermit::IoPossible;
    }

    pub(super) fn record_write_result(
        &mut self,
        result: &io::Result<usize>,
        offered: usize,
    ) -> Result<()> {
        ensure(
            self.permit == GuardPermit::IoPossible,
            "input result requires its possible-I/O attempt",
        )?;
        ensure(
            offered == self.retained.bytes.len(),
            "input offered count does not match its selected stage",
        )?;
        match result {
            Ok(count) => {
                ensure(*count <= offered, "native input count exceeds its offer")?;
                self.retained.written_bytes = self
                    .retained
                    .written_bytes
                    .checked_add(
                        u64::try_from(*count)
                            .map_err(|_| error("native input count exceeds range"))?,
                    )
                    .ok_or_else(|| error("native input count exhausted"))?;
                if *count == offered {
                    self.retained.stage = SubmissionStage::Submitted;
                } else if *count > 0 {
                    self.retained.stage = SubmissionStage::Partial;
                }
            }
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => {
                if self.retained.stage == SubmissionStage::ZeroBytes {
                    self.retained.stage = SubmissionStage::Unknown;
                }
            }
        }
        self.permit = GuardPermit::ResultRecorded;
        Ok(())
    }
}

impl Drop for InputGuard<'_> {
    fn drop(&mut self) {
        if self.permit == GuardPermit::IoPossible
            && self.retained.stage == SubmissionStage::ZeroBytes
        {
            self.retained.stage = SubmissionStage::Unknown;
        }
        self.retained.leased = false;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteValidation {
    Ready,
    Contended,
}

impl InputCoordinator {
    fn failure(&self, source: CafleetError) -> InputFailure {
        InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: None,
            input_id: self.pending.as_ref().and_then(|pending| pending.input_id),
            operation_key: self
                .pending
                .as_ref()
                .map(|pending| pending.request.operation_key.clone()),
        }
    }

    fn effective_deadline(&self, conn: &Connection, deadline: Instant) -> Result<Instant> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| error("input writer is absent"))?;
        let mut effective = deadline.min(pending.deadline);
        if let Some(inherited) = inherited_database_deadline(conn) {
            effective = effective.min(inherited.instant());
        }
        check_deadline(Instant::now(), effective)?;
        Ok(effective)
    }

    fn validate_live(&self, conn: &Connection, expected: &RunRecord) -> Result<()> {
        ensure(
            !self.manual_accounting_failed,
            "manual accounting failure fenced input",
        )?;
        ensure(
            stable_run_identity(expected) == stable_run_identity(&self.run),
            "input coordinator run identity changed",
        )?;
        let current = storage::validate_owner(conn, &self.run.owner())?;
        ensure(
            stable_run_identity(&current) == stable_run_identity(&self.run)
                && current.status == RunStatus::Running
                && current.owner_kind == OwnerKind::Runner
                && current.native_version == self.run.native_version
                && current.profile_version == self.run.profile_version,
            "input coordinator current run changed",
        )?;
        ensure(
            crate::broker::active_member_fleet(conn, self.run.member_id)?
                == Some(self.run.fleet_id),
            "input member is inactive",
        )?;
        let member = crate::broker::get_member(conn, self.run.member_id, self.run.fleet_id)?
            .ok_or_else(|| error("input member is absent"))?;
        ensure(
            member.kind == MemberKind::Member && member.placement.as_ref() == Some(&self.placement),
            "input member placement changed",
        )?;
        self.lock.validate(conn, &current.owner())
    }

    fn composer_eligible(observation: &ObservationController) -> bool {
        let status = observation.status();
        status.eligibility == InputEligibility::ComposerEmpty
            && status.visibility == Visibility::NotVisible
            && !status.retained_transition
            && !status.retained_draft
            && !status.retained_evidence
            && status.gap_count == 0
            && status.diagnostic.is_none()
    }

    fn result(&self, operation: &storage::InputOperation) -> SubmissionResult {
        use storage::{InputDisposition, InputState};
        let (status, mut stage, reason) = match (operation.state, operation.disposition) {
            (InputState::Accepted, _)
            | (InputState::Settled, Some(InputDisposition::Submitted)) => {
                (SubmissionStatus::Accepted, SubmissionStage::Submitted, None)
            }
            (InputState::InProgress | InputState::Uncertain, _) => {
                (SubmissionStatus::Uncertain, SubmissionStage::Unknown, None)
            }
            (InputState::Ready, _) => (
                SubmissionStatus::Held,
                SubmissionStage::ZeroBytes,
                Some("input requires live proven-zero revalidation".into()),
            ),
            (InputState::Settled, _) => (
                SubmissionStatus::Rejected,
                SubmissionStage::ZeroBytes,
                Some("input operation key is consumed".into()),
            ),
        };
        let retained = self.operations.get(&operation.input_id).map(|_| {
            self.retained_checkpoint(operation.input_id)
                .expect("present input cache supplies its checkpoint")
        });
        if matches!(
            operation.state,
            InputState::InProgress | InputState::Uncertain
        ) && let Some(retained) = retained.as_ref()
            && retained.stage != SubmissionStage::ZeroBytes
        {
            stage = retained.stage;
        }
        SubmissionResult {
            status,
            input_id: retained
                .as_ref()
                .map_or(Some(operation.input_id), |point| point.input_id),
            stage,
            output_seq: retained.map(|point| point.output_seq),
            reason,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed external service integration invokes the ordinary input coordinator"
        )
    )]
    pub(crate) fn handle(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let mut known_input_id = None;
        let mut known_key = None;
        let result = (|| {
            check_deadline(Instant::now(), deadline)?;
            ensure(
                request.protocol_version == super::CONTROL_PROTOCOL_VERSION,
                "input protocol version mismatch",
            )?;
            let mut expected = ControlIdentity::from_run(&self.run);
            ensure(
                request.identity.owner_epoch > 0,
                "positive input owner epoch required",
            )?;
            expected.owner_epoch = request.identity.owner_epoch;
            ensure(
                request.identity == expected && run.run_id == self.run.run_id,
                "input request fixed identity mismatch",
            )?;
            let (input, composer_operation) = match request.operation {
                InputControlOperation::SubmitPrompt(input) => (input, ComposerOperation::Prompt),
                InputControlOperation::SubmitExec(input) => (input, ComposerOperation::Exec),
                _ => return Err(error("composer submission operation required")),
            };
            known_key = Some(input.operation_key.clone());
            ensure(
                matches!(
                    (composer_operation, input.input_kind),
                    (
                        ComposerOperation::Prompt,
                        storage::InputKind::Prompt | storage::InputKind::Preview
                    ) | (ComposerOperation::Exec, storage::InputKind::Exec)
                ),
                "composer submission operation and kind mismatch",
            )?;
            if input.input_kind == storage::InputKind::Preview {
                let message_id = input
                    .message_id
                    .ok_or_else(|| error("preview message required"))?;
                ensure(
                    message_id > 0
                        && input.operation_key
                            == format!("preview:{}:{message_id}", self.run.run_id),
                    "preview operation key must bind its run and message",
                )?;
            } else {
                ensure(
                    input.message_id.is_none(),
                    "explicit input has no preview message",
                )?;
            }
            validate_operation_key(&input.operation_key)?;
            ensure(
                crate::input::valid_input(&input.literal),
                "composer submission requires one nonempty line without control characters",
            )?;
            let digest = Sha256::digest(canonical_json(&serde_json::json!({"run_id":self.run.run_id,"kind":input.input_kind.as_str(),"message_id":input.message_id,"literal":input.literal})).as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
            let retained = with_database_deadline(conn, deadline, |conn| {
                ensure(
                    storage::database_identity(conn)? == self.run.database_identity_json,
                    "input database identity changed",
                )?;
                storage::find_input(conn, self.run.run_id, &input.operation_key)
            })?;
            if let Some(retained) = retained {
                known_input_id = Some(retained.input_id);
                ensure(
                    retained.kind == input.input_kind
                        && retained.payload_digest == digest
                        && retained.message_id == input.message_id,
                    "conflicting input operation key reuse",
                )?;
                return Ok(Some(InputControlReply::Submission(self.result(&retained))));
            }
            request.identity.validate(&self.run)?;
            if self.pending.is_some() || !Self::composer_eligible(observation) {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("ordinary composer input is held".into()),
                })));
            }
            ensure(
                self.operations.len() < 4096,
                "input receipt cache is at capacity",
            )?;
            with_database_deadline(conn, deadline, |conn| self.validate_live(conn, run))?;
            let Some(native) = self.run.native_version.as_deref() else {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("matching executable composer plan is unavailable".into()),
                })));
            };
            let backend =
                coding_agent(&self.run.backend).ok_or_else(|| error("unknown input backend"))?;
            let Some(plan) =
                self.services
                    .catalog
                    .composer_plan(backend, native, composer_operation)?
            else {
                return Ok(Some(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Held,
                    input_id: None,
                    stage: SubmissionStage::ZeroBytes,
                    output_seq: None,
                    reason: Some("matching executable composer plan is unavailable".into()),
                })));
            };
            ensure(
                plan.backend() == backend
                    && plan.native_version() == native
                    && Some(plan.profile_version()) == self.run.profile_version.as_deref()
                    && !plan.rule_ids().is_empty()
                    && !plan.fixture_ids().is_empty(),
                "composer plan provenance changed",
            )?;
            let bytes = Arc::from(plan.encode(&input.literal)?);
            let expiration = receive_input_expiration(
                request.expires_at_mono_ns,
                Instant::now(),
                Some(deadline),
                &mut RealInputDeadlineClock,
            )?;
            self.pending = Some(PendingInput {
                request: storage::NewInput {
                    operation_key: input.operation_key,
                    kind: input.input_kind,
                    payload_digest: digest,
                    message_id: input.message_id,
                },
                bytes,
                deadline: expiration.min(deadline),
                phase: InputPhase::AwaitReservation,
                input_id: None,
            });
            Ok(None)
        })();
        result.map_err(|source| InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: None,
            input_id: known_input_id,
            operation_key: known_key,
        })
    }

    fn retained_checkpoint(&self, input_id: i64) -> Result<InputCheckpoint> {
        let retained = self
            .operations
            .get(&input_id)
            .ok_or_else(|| error("retained input operation is absent"))?;
        Ok(InputCheckpoint {
            input_id: Some(input_id),
            stage: retained.stage,
            written_bytes: retained.written_bytes,
            output_seq: retained.output_seq,
        })
    }

    fn checkpoint(
        &mut self,
        input_id: i64,
        boundary: InputBoundary,
        deadline: Instant,
    ) -> Result<()> {
        let checkpoint = self.retained_checkpoint(input_id)?;
        let result = self
            .services
            .hooks
            .checkpoint(boundary, &checkpoint, deadline)
            .and_then(|()| check_deadline(Instant::now(), deadline));
        if result.is_err()
            && let Some(pending) = &mut self.pending
        {
            pending.phase = InputPhase::Blocked;
        }
        result
    }

    fn fresh_drain(
        &mut self,
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<Option<u64>> {
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        transport.validate_input_drain(&proof, deadline)?;
        if !Self::composer_eligible(observation) {
            return Ok(None);
        }
        let InputDrain::Quiescent {
            human_generation,
            output_seq,
        } = proof.evidence()
        else {
            unreachable!("opaque drain is quiescent");
        };
        ensure(
            human_generation == self.manual_generation && !self.manual_accounting_failed,
            "input manual generation changed",
        )?;
        Ok(Some(output_seq))
    }

    fn validate_write_now(
        &self,
        conn: &mut Connection,
        run: &RunRecord,
        operation: &PendingInput,
        deadline: Instant,
    ) -> std::result::Result<WriteValidation, InputFailure> {
        let result = (|| {
            let effective = self.effective_deadline(conn, deadline)?;
            ensure(
                conn.is_autocommit(),
                "final input validation requires autocommit",
            )?;
            let result = with_database_budget(conn, Deadline::until(effective), |conn| {
                conn.busy_handler(Some(|_| false)).map_err(db_err)?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_err)?;
                self.validate_live(&tx, run)?;
                let id = operation
                    .input_id
                    .ok_or_else(|| error("input reservation is absent"))?;
                let retained = self
                    .operations
                    .get(&id)
                    .ok_or_else(|| error("retained input reservation is absent"))?;
                let current = storage::get_input(&tx, id)?;
                ensure(
                    current == retained.operation
                        && current.state == storage::InputState::InProgress
                        && current.run_id == self.run.run_id
                        && current.operation_key == operation.request.operation_key
                        && current.payload_digest == operation.request.payload_digest
                        && current.kind == operation.request.kind
                        && current.message_id == operation.request.message_id,
                    "input reservation changed before write",
                )?;
                check_deadline(Instant::now(), effective)?;
                tx.rollback().map_err(db_err)
            });
            let effective = inherited_database_deadline(conn)
                .map_or(effective, |bound| effective.min(bound.instant()));
            match result {
                Ok(()) => {
                    check_deadline(Instant::now(), effective)?;
                    Ok(WriteValidation::Ready)
                }
                Err(CafleetError::DatabaseBusy(_)) => {
                    check_deadline(Instant::now(), effective)?;
                    Ok(WriteValidation::Contended)
                }
                Err(cause) => Err(cause),
            }
        })();
        result.map_err(|source| self.failure(source))
    }

    fn acquire_write_guard<'a>(
        &'a mut self,
        conn: &mut Connection,
        run: &RunRecord,
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputGuard<'a>>, InputFailure> {
        let result = (|| {
            let Some(pending) = self.pending.as_ref() else {
                return Ok(None);
            };
            if pending.phase != InputPhase::Armed {
                return Ok(None);
            }
            let effective = self.effective_deadline(conn, deadline)?;
            let Some(proof) = transport.take_input_drain() else {
                return Ok(None);
            };
            transport.validate_input_drain(&proof, effective)?;
            if !Self::composer_eligible(observation) {
                return Ok(None);
            }
            let InputDrain::Quiescent {
                human_generation,
                output_seq,
            } = proof.evidence()
            else {
                unreachable!("opaque drain is quiescent");
            };
            ensure(
                human_generation == self.manual_generation && !self.manual_accounting_failed,
                "input manual generation changed",
            )?;
            let id = pending
                .input_id
                .ok_or_else(|| error("armed input has no reservation"))?;
            let retained = self
                .operations
                .get(&id)
                .ok_or_else(|| error("armed input reservation is absent"))?;
            ensure(
                retained.attempt_available && !retained.leased,
                "input attempt permit is consumed",
            )?;
            if self
                .validate_write_now(conn, run, pending, effective)
                .map_err(|failure| failure.source)?
                == WriteValidation::Contended
            {
                return Ok(None);
            }
            let effective = inherited_database_deadline(conn)
                .map_or(effective, |bound| effective.min(bound.instant()));
            check_deadline(Instant::now(), effective)?;
            Ok(Some((id, proof, effective, output_seq)))
        })()
        .map_err(|source| self.failure(source))?;
        let Some((id, proof, deadline, output_seq)) = result else {
            return Ok(None);
        };
        let retained = self
            .operations
            .get_mut(&id)
            .expect("validated input remains retained");
        retained.output_seq = output_seq;
        retained.attempt_available = false;
        retained.leased = true;
        Ok(Some(InputGuard {
            retained,
            proof,
            deadline,
            permit: GuardPermit::Fresh,
        }))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the reviewed external service integration invokes the ordinary input coordinator"
        )
    )]
    pub(crate) fn advance(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        transport: &mut Transport,
        pty: &mut impl PtyIo,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let result = (|| {
            let Some(pending) = self.pending.as_ref() else {
                return Ok(None);
            };
            let effective = self.effective_deadline(conn, deadline)?;
            let phase = pending.phase;
            if phase == InputPhase::Blocked {
                return Ok(None);
            }
            if phase == InputPhase::AwaitReservation {
                let Some(output_seq) = self.fresh_drain(transport, observation, effective)? else {
                    return Ok(None);
                };
                let pending = self.pending.as_ref().expect("retained pending input");
                let reservation = with_database_deadline(conn, effective, |conn| {
                    self.validate_live(conn, run)?;
                    storage::reserve_input_with_provenance(
                        conn,
                        &self.run.owner(),
                        &pending.request,
                    )
                })?;
                let operation = match reservation {
                    storage::InputReservation::Inserted(operation) => operation,
                    storage::InputReservation::Existing(operation) => {
                        let reply = InputControlReply::Submission(self.result(&operation));
                        self.pending = None;
                        return Ok(Some(reply));
                    }
                };
                ensure(
                    operation.state == storage::InputState::InProgress,
                    "ordinary input reservation is not writable",
                )?;
                let id = operation.input_id;
                self.operations.insert(
                    id,
                    RetainedInput {
                        operation,
                        bytes: pending.bytes.clone(),
                        stage: SubmissionStage::ZeroBytes,
                        written_bytes: 0,
                        output_seq,
                        attempt_available: false,
                        leased: false,
                    },
                );
                let pending = self.pending.as_mut().expect("reserved pending input");
                pending.input_id = Some(id);
                pending.phase = InputPhase::AwaitBeforeWrite;
                self.checkpoint(id, InputBoundary::Reserved, effective)?;
                return Ok(None);
            }
            let id = self
                .pending
                .as_ref()
                .and_then(|pending| pending.input_id)
                .ok_or_else(|| error("input writer has no reservation"))?;
            if phase == InputPhase::AwaitBeforeWrite {
                let Some(output_seq) = self.fresh_drain(transport, observation, effective)? else {
                    return Ok(None);
                };
                let retained = self
                    .operations
                    .get_mut(&id)
                    .expect("reserved input remains retained");
                retained.output_seq = output_seq;
                retained.attempt_available = true;
                self.pending.as_mut().expect("pending input").phase = InputPhase::Armed;
                self.checkpoint(id, InputBoundary::BeforeWrite, effective)?;
                return Ok(None);
            }
            let bytes = self
                .pending
                .as_ref()
                .expect("armed pending input")
                .bytes
                .clone();
            let Some(mut guard) = self
                .acquire_write_guard(conn, run, transport, observation, effective)
                .map_err(|failure| failure.source)?
            else {
                return Ok(None);
            };
            let write = transport.write_programmatic(pty, &bytes, &mut guard, effective);
            let attempted = matches!(
                guard.permit,
                GuardPermit::IoPossible | GuardPermit::ResultRecorded
            );
            drop(guard);
            let after = if attempted {
                self.checkpoint(id, InputBoundary::AfterWrite, effective)
            } else {
                Ok(())
            };
            let write = combine_manual_progress(write, after).and_then(|outcome| {
                let point = self.retained_checkpoint(id)?;
                match &outcome {
                    ProgrammaticWrite::Complete { bytes: count } => ensure(
                        *count == bytes.len()
                            && point.stage == SubmissionStage::Submitted
                            && point.written_bytes
                                == u64::try_from(*count)
                                    .map_err(|_| error("native input count exceeds range"))?,
                        "complete native result conflicts with retained evidence",
                    )?,
                    ProgrammaticWrite::Partial { bytes: count } => ensure(
                        *count > 0
                            && *count < bytes.len()
                            && point.stage == SubmissionStage::Partial
                            && point.written_bytes
                                == u64::try_from(*count)
                                    .map_err(|_| error("native input count exceeds range"))?,
                        "partial native result conflicts with retained evidence",
                    )?,
                    ProgrammaticWrite::Pending => ensure(
                        point.stage == SubmissionStage::ZeroBytes && point.written_bytes == 0,
                        "pending native result conflicts with retained evidence",
                    )?,
                }
                Ok(outcome)
            });
            if matches!(&write, Ok(ProgrammaticWrite::Pending)) {
                self.pending
                    .as_mut()
                    .expect("pending zero-byte attempt")
                    .phase = InputPhase::AwaitBeforeWrite;
                return Ok(None);
            }
            let retained = self
                .operations
                .get(&id)
                .expect("actual input outcome retained");
            let next = match retained.stage {
                SubmissionStage::Submitted => storage::InputState::Accepted,
                SubmissionStage::Partial | SubmissionStage::Unknown => {
                    storage::InputState::Uncertain
                }
                SubmissionStage::ZeroBytes => storage::InputState::Ready,
            };
            let persisted = with_database_deadline(conn, effective, |conn| {
                self.validate_live(conn, run)?;
                storage::record_input_state(
                    conn,
                    &self.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    next,
                )
            });
            if let Ok(operation) = &persisted {
                self.operations
                    .get_mut(&id)
                    .expect("input retained")
                    .operation = operation.clone();
            }
            let completed = match (write, persisted) {
                (Ok(_), Ok(operation)) => Ok(operation),
                (Err(primary), Ok(_)) | (Ok(_), Err(primary)) => Err(primary),
                (Err(primary), Err(bookkeeping)) => Err(CafleetError::WithDiagnostics {
                    cause: Box::new(primary),
                    diagnostics: vec![bookkeeping.message().into_owned()],
                }),
            };
            self.pending.as_mut().expect("completed input").phase = InputPhase::Blocked;
            let operation = completed?;
            self.checkpoint(id, InputBoundary::TerminalState, effective)?;
            let reply = InputControlReply::Submission(self.result(&operation));
            self.pending = None;
            Ok(Some(reply))
        })();
        result.map_err(|source| self.failure(source))
    }

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
        let placement = with_database_deadline(conn, deadline, |conn| {
            let member = crate::broker::get_member(conn, run.member_id, run.fleet_id)?
                .ok_or_else(|| error("input coordinator member is absent"))?;
            ensure(
                member.kind == MemberKind::Member,
                "input coordinator requires an ordinary member",
            )?;
            member
                .placement
                .ok_or_else(|| error("input coordinator placement is absent"))
        })?;
        Ok(Self {
            placement,
            operations: BTreeMap::new(),
            pending: None,
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
