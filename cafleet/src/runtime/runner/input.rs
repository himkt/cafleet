use super::{
    ControlIdentity, DrainedInput, InputControlOperation, InputControlReply, InputControlRequest,
    InputDrain, InputEligibility, ObservationController, ProgrammaticWrite, PtyIo,
    SubmissionResult, SubmissionStatus, Visibility, canonical_json, validate_operation_key,
};
use super::{
    InputFailure, ProcessIdentity, Result, RunLock, SubmissionStage, Transport, check_deadline,
    ensure, error, stable_run_identity, with_database_deadline,
};
#[cfg(test)]
use super::{RealInputDeadlineClock, receive_input_expiration};
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

    fn validate_receipt_owner(&self, conn: &Connection, run: &RunRecord) -> Result<()> {
        ensure(
            stable_run_identity(run) == stable_run_identity(&self.run),
            "receipt caller run identity changed",
        )?;
        let current = storage::validate_owner(conn, &self.run.owner())?;
        ensure(
            stable_run_identity(&current) == stable_run_identity(&self.run),
            "receipt current run identity changed",
        )?;
        self.lock.validate(conn, &current.owner())
    }

    fn validate_receipt_cache(&self, input: &storage::InputOperation) -> Result<()> {
        ensure(
            input.run_id == self.run.run_id && input.kind != storage::InputKind::PermissionResponse,
            "ordinary receipt for this run required",
        )?;
        if let Some(retained) = self.operations.get(&input.input_id) {
            ensure(
                retained.operation.run_id == input.run_id
                    && retained.operation.operation_key == input.operation_key
                    && retained.operation.kind == input.kind
                    && retained.operation.payload_digest == input.payload_digest
                    && retained.operation.message_id == input.message_id,
                "receipt cache identity changed",
            )?;
        }
        Ok(())
    }

    fn receipt_projection(&self, operation: &storage::InputOperation) -> Result<SubmissionResult> {
        self.validate_receipt_cache(operation)?;
        let mut result = self.result(operation);
        if self.operations.contains_key(&operation.input_id) {
            let actual = self.retained_checkpoint(operation.input_id)?;
            result.stage = actual.stage;
            result.output_seq = Some(actual.output_seq);
        } else if operation.state == storage::InputState::Settled
            && operation.disposition != Some(storage::InputDisposition::Submitted)
        {
            result.stage = SubmissionStage::Unknown;
        }
        Ok(result)
    }

    fn receipt_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        action: &super::ReceiptAction,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        use super::{ReceiptAction, ReceiptReply};
        if let ReceiptAction::Retire { input_id } = action {
            return self.retire_received(conn, run, *input_id, deadline);
        }
        let deadline = inherited_database_deadline(conn)
            .map_or(deadline, |inherited| deadline.min(inherited.instant()));
        check_deadline(Instant::now(), deadline)?;
        let mut input_id = None;
        let mut operation_key = None;
        let result = (|| match action {
            ReceiptAction::Query { operation_key: key } => {
                operation_key = Some(key.clone());
                validate_operation_key(key)?;
                with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction().map_err(db_err)?;
                    self.validate_receipt_owner(&tx, run)?;
                    let reply = if let Some(input) = storage::find_input(&tx, self.run.run_id, key)?
                    {
                        input_id = Some(input.input_id);
                        InputControlReply::Receipt(ReceiptReply::Present {
                            input_id: input.input_id,
                            submission: self.receipt_projection(&input)?,
                        })
                    } else {
                        ensure(
                            !self
                                .operations
                                .values()
                                .any(|retained| retained.operation.operation_key == *key),
                            "retained input has no durable reservation",
                        )?;
                        InputControlReply::Receipt(ReceiptReply::Missing)
                    };
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit().map_err(db_err)?;
                    Ok(reply)
                })
            }
            ReceiptAction::Reconcile {
                input_id: id,
                expected,
                disposition,
                evidence_ref,
            } => {
                input_id = Some(*id);
                ensure(*id > 0, "positive reconciliation input ID required")?;
                ensure(
                    !evidence_ref.trim().is_empty(),
                    "nonempty reconciliation evidence required",
                )?;
                let settled = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .map_err(db_err)?;
                    self.validate_receipt_owner(&tx, run)?;
                    let input = storage::get_input(&tx, *id)?;
                    operation_key = Some(input.operation_key.clone());
                    self.validate_receipt_cache(&input)?;
                    ensure(
                        self.pending.is_none(),
                        "reconciliation requires no active input writer",
                    )?;
                    if let Some(retained) = self.operations.get(id) {
                        ensure(
                            !retained.leased && !retained.attempt_available,
                            "reconciliation requires an unleased consumed attempt",
                        )?;
                    }
                    ensure(input.state == *expected, "input state changed")?;
                    let settled = if input.state == storage::InputState::Settled {
                        ensure(
                            input.disposition == Some(*disposition),
                            "settled input disposition changed",
                        )?;
                        input
                    } else {
                        storage::settle_input_in_transaction(
                            &tx,
                            &self.run.owner(),
                            *id,
                            *expected,
                            *disposition,
                            evidence_ref,
                        )?
                    };
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit().map_err(db_err)?;
                    Ok(settled)
                })?;
                if let Some(retained) = self.operations.get_mut(id) {
                    retained.operation = settled;
                }
                check_deadline(Instant::now(), deadline)?;
                Ok(InputControlReply::Receipt(ReceiptReply::Settled {
                    input_id: *id,
                }))
            }
            ReceiptAction::Retire { .. } => {
                unreachable!("retirement handled before receipt transaction")
            }
        })();
        result.map_err(|source| {
            CafleetError::from(InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id,
                operation_key,
            })
        })
    }

    fn retire_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        input_id: i64,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        let deadline = inherited_database_deadline(conn)
            .map_or(deadline, |inherited| deadline.min(inherited.instant()));
        check_deadline(Instant::now(), deadline)?;
        let mut operation_key = None;
        let result = (|| {
            ensure(input_id > 0, "positive retirement input ID required")?;
            ensure(
                stable_run_identity(run) == stable_run_identity(&self.run),
                "retirement caller run identity changed",
            )?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction().map_err(db_err)?;
                let current = storage::validate_owner(&tx, &self.run.owner())?;
                ensure(
                    stable_run_identity(&current) == stable_run_identity(&self.run),
                    "retirement current run identity changed",
                )?;
                self.lock.validate(&tx, &current.owner())?;
                let input = storage::get_input(&tx, input_id)?;
                operation_key = Some(input.operation_key.clone());
                ensure(
                    input.run_id == self.run.run_id
                        && input.kind != storage::InputKind::PermissionResponse
                        && input.state == storage::InputState::Settled,
                    "retirement requires this run's settled ordinary input",
                )?;
                ensure(
                    self.pending.is_none(),
                    "retirement requires no active input writer",
                )?;
                if let Some(retained) = self.operations.get(&input_id) {
                    ensure(
                        !retained.leased && !retained.attempt_available,
                        "retirement requires an unleased consumed attempt",
                    )?;
                    ensure(
                        retained.operation.run_id == input.run_id
                            && retained.operation.operation_key == input.operation_key
                            && retained.operation.kind == input.kind
                            && retained.operation.payload_digest == input.payload_digest
                            && retained.operation.message_id == input.message_id,
                        "retirement cache identity changed",
                    )?;
                }
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(db_err)
            })?;
            check_deadline(Instant::now(), deadline)?;
            self.operations.remove(&input_id);
            Ok(InputControlReply::Receipt(super::ReceiptReply::Retired {
                input_id,
            }))
        })();
        result.map_err(|source| {
            CafleetError::from(InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id: Some(input_id),
                operation_key,
            })
        })
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

    #[cfg(test)]
    pub(crate) fn handle(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let deadline = check_deadline(Instant::now(), deadline)
            .and_then(|()| {
                receive_input_expiration(
                    request.expires_at_mono_ns,
                    Instant::now(),
                    Some(deadline),
                    &mut RealInputDeadlineClock,
                )
            })
            .map_err(|source| InputFailure {
                source,
                run_id: Some(self.run.run_id),
                interaction_id: None,
                input_id: None,
                operation_key: match &request.operation {
                    InputControlOperation::SubmitPrompt(input)
                    | InputControlOperation::SubmitExec(input) => Some(input.operation_key.clone()),
                    _ => None,
                },
            })?;
        self.handle_received(conn, run, request, observation, deadline)
    }

    pub(super) fn handle_received(
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
            if let InputControlOperation::Receipt(action) = &request.operation {
                match action {
                    super::ReceiptAction::Query { operation_key } => {
                        known_key = Some(operation_key.clone())
                    }
                    super::ReceiptAction::Retire { input_id }
                    | super::ReceiptAction::Reconcile { input_id, .. } => {
                        known_input_id = Some(*input_id)
                    }
                }
                request.identity.validate(&self.run)?;
                return self.receipt_received(conn, run, action, deadline).map(Some);
            }
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
            self.pending = Some(PendingInput {
                request: storage::NewInput {
                    operation_key: input.operation_key,
                    kind: input.input_kind,
                    payload_digest: digest,
                    message_id: input.message_id,
                },
                bytes,
                deadline,
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

    pub(super) fn finish_pending_attempt(&mut self) {
        if let Some(id) = self.pending.as_ref().and_then(|pending| pending.input_id) {
            let retained = self
                .operations
                .get_mut(&id)
                .expect("pending input reservation remains retained");
            assert!(
                !retained.leased,
                "completed input has no active guard lease"
            );
            retained.attempt_available = false;
        }
        self.pending = None;
    }

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
        combine_manual_progress(progress, accounting).map_err(|source| self.failure(source))
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

#[cfg(test)]
mod retirement_contracts {
    use super::contract_tests::ordinary_prompt::support::{Host, with_host};
    use super::*;
    use crate::runtime::runner::{ReceiptAction, ReceiptReply, SubmitInput};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn assert_retired(reply: InputControlReply, expected: i64) {
        assert!(
            matches!(reply, InputControlReply::Receipt(ReceiptReply::Retired { input_id }) if input_id == expected)
        );
    }

    fn receipt(
        h: &mut Host,
        action: ReceiptAction,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let mut request = h.request("unused-receipt-key");
        request.operation = InputControlOperation::Receipt(action);
        h.coordinator
            .handle(
                &mut h.fixture.conn,
                &h.fixture.run,
                request,
                &h.controller,
                h.until,
            )
            .map(|reply| reply.expect("receipt actions reply immediately"))
    }

    fn pending(h: &Host) -> (String, String, Vec<u8>, Instant, InputPhase, Option<i64>) {
        let pending = h
            .coordinator
            .pending
            .as_ref()
            .expect("actual pending writer");
        (
            pending.request.operation_key.clone(),
            pending.request.payload_digest.clone(),
            pending.bytes.to_vec(),
            pending.deadline,
            pending.phase,
            pending.input_id,
        )
    }

    #[test]
    fn live_receipt_query_preserves_the_real_zero_byte_checkpoint_and_pending_writer() {
        with_host(|h| {
            let key = "prompt:live-receipt-query";
            let id = h.arm(key);
            let point = cached(h, id);
            assert!(matches!(point.1, SubmissionStage::ZeroBytes));
            assert!(point.4 && !point.5);
            h.pty
                .child
                .bytes
                .extend(b"\x1b[2J\x1b[HLater unclassified output");
            h.drain();
            assert!(h.transport.snapshot().screen_seq > point.3);
            let writer = pending(h);
            let changes = h.fixture.conn.total_changes();
            let hooks = h.hooks.points.lock().unwrap().len();
            let reply = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .expect("Query projects actual retained facts even while a writer is Armed");
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = reply
            else {
                panic!("actual retained input is Present")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.input_id, Some(id));
            assert_eq!(submission.status, SubmissionStatus::Uncertain);
            assert_eq!(submission.stage, SubmissionStage::ZeroBytes);
            assert_eq!(submission.output_seq, Some(point.3));
            assert!(matches!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: "prompt:absent-receipt".into()
                    }
                )
                .unwrap(),
                InputControlReply::Receipt(ReceiptReply::Missing)
            ));
            let mut stale = h.request("unused-receipt-key");
            stale.operation = InputControlOperation::Receipt(ReceiptAction::Query {
                operation_key: key.into(),
            });
            stale.identity.owner_epoch += 1;
            assert!(
                h.coordinator
                    .handle(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        stale,
                        &h.controller,
                        h.until
                    )
                    .is_err()
            );
            assert!(
                receipt(
                    h,
                    ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::InProgress,
                        disposition: storage::InputDisposition::NotSubmitted,
                        evidence_ref: "fixture:active-writer-is-not-zero-proof".into(),
                    }
                )
                .is_err(),
                "reconciliation preserves the actual Armed writer and its available permit"
            );
            assert!(
                pending(h) == writer,
                "Query preserves exact request, bytes, phase and original deadline"
            );
            assert_eq!(cached(h, id), point);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), point.0);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
            assert!(h.pty.offers.is_empty());
            h.fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2",
                    rusqlite::params!["f".repeat(64), id],
                )
                .unwrap();
            assert!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: key.into()
                    }
                )
                .is_err(),
                "cached immutable identity must match its actual durable reservation"
            );
            h.fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2",
                    rusqlite::params![point.0.payload_digest, id],
                )
                .unwrap();
            assert_eq!(
                h.fixture
                    .conn
                    .execute(
                        "DELETE FROM member_input_operations WHERE input_id=?1",
                        [id]
                    )
                    .unwrap(),
                1
            );
            let changes = h.fixture.conn.total_changes();
            assert!(
                receipt(
                    h,
                    ReceiptAction::Query {
                        operation_key: key.into()
                    }
                )
                .is_err(),
                "a retained attempt without its actual reservation is an invariant error"
            );
            assert!(pending(h) == writer);
            assert_eq!(cached(h, id), point);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert!(h.pty.offers.is_empty());
        });
    }

    #[test]
    fn live_receipt_reconciliation_uses_cas_rolls_back_and_keeps_real_stage_until_retirement() {
        with_host(|h| {
            let key = "prompt:live-receipt-partial";
            let id = h.arm(key);
            h.pty.short = Some(3);
            h.drain();
            let InputControlReply::Submission(partial) = h.advance().expect("actual partial write")
            else {
                panic!("actual partial result")
            };
            assert_eq!(partial.status, SubmissionStatus::Uncertain);
            assert_eq!(partial.stage, SubmissionStage::Partial);
            let original = storage::get_input(&h.fixture.conn, id).unwrap();
            let point = cached(h, id);
            assert_eq!(point.2, 3);
            let offers = h.pty.offers.clone();
            let action = |expected| ReceiptAction::Reconcile {
                input_id: id,
                expected,
                disposition: storage::InputDisposition::AbandonedUncertain,
                evidence_ref: "fixture:observed-partial".into(),
            };
            let Err(stale) = receipt(h, action(storage::InputState::Accepted)) else {
                panic!("actual state CAS rejects stale expected state")
            };
            assert!(
                stale.source.to_string().contains("state"),
                "{}",
                stale.source
            );
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), original);
            assert_eq!(cached(h, id), point);
            h.fixture.conn.execute_batch("CREATE TRIGGER receipt_settlement_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.state='settled' BEGIN SELECT RAISE(ABORT,'owned receipt settlement failure'); END").unwrap();
            let failed = receipt(h, action(storage::InputState::Uncertain));
            let removed = h
                .fixture
                .conn
                .execute_batch("DROP TRIGGER receipt_settlement_failure");
            assert!(
                failed.as_ref().is_err_and(|failure| failure
                    .source
                    .to_string()
                    .contains("owned receipt settlement failure")),
                "settlement_error={:?}; trigger_cleanup={removed:?}",
                failed.as_ref().err()
            );
            removed.unwrap();
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), original);
            assert_eq!(cached(h, id), point);
            assert!(
                matches!(receipt(h, action(storage::InputState::Uncertain)).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
            );
            let settled = storage::get_input(&h.fixture.conn, id).unwrap();
            assert_eq!(settled.state, storage::InputState::Settled);
            assert_eq!(
                settled.disposition,
                Some(storage::InputDisposition::AbandonedUncertain)
            );
            assert_eq!(
                settled.evidence_ref.as_deref(),
                Some("fixture:observed-partial")
            );
            let changes = h.fixture.conn.total_changes();
            assert!(
                matches!(receipt(h, ReceiptAction::Reconcile { input_id: id, expected: storage::InputState::Settled, disposition: storage::InputDisposition::AbandonedUncertain, evidence_ref: "fixture:preserve-first-evidence".into() }).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
            );
            assert!(
                receipt(
                    h,
                    ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::Settled,
                        disposition: storage::InputDisposition::Submitted,
                        evidence_ref: "fixture:conflicting-wire-disposition".into()
                    }
                )
                .is_err()
            );
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            let retained = cached(h, id);
            assert_eq!(
                (retained.1, retained.2, retained.3),
                (point.1, point.2, point.3)
            );
            let hooks = h.hooks.points.lock().unwrap().len();
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .unwrap()
            else {
                panic!("settled cache retains its actual prior facts")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.status, SubmissionStatus::Rejected);
            assert_eq!(submission.stage, SubmissionStage::Partial);
            assert_eq!(submission.output_seq, Some(point.3));
            assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
            assert_retired(
                receipt(h, ReceiptAction::Retire { input_id: id }).unwrap(),
                id,
            );
            assert!(!h.coordinator.operations.contains_key(&id));
            let InputControlReply::Receipt(ReceiptReply::Present {
                input_id,
                submission,
            }) = receipt(
                h,
                ReceiptAction::Query {
                    operation_key: key.into(),
                },
            )
            .unwrap()
            else {
                panic!("retired cache preserves durable Present history")
            };
            assert_eq!(input_id, id);
            assert_eq!(submission.input_id, Some(id));
            assert_eq!(submission.status, SubmissionStatus::Rejected);
            assert_eq!(submission.stage, SubmissionStage::Unknown);
            assert_eq!(submission.output_seq, None);
            let repeat = h.request(key);
            let InputControlReply::Submission(repeat) = h
                .coordinator
                .handle(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    repeat,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .unwrap()
            else {
                panic!("consumed key returns its submission retry projection")
            };
            assert_eq!(repeat.status, SubmissionStatus::Rejected);
            assert_eq!(repeat.stage, SubmissionStage::ZeroBytes);
            assert!(h.coordinator.pending.is_none());
            assert_eq!(h.pty.offers, offers);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            assert_eq!(h.hooks.count("reserved"), 1);
            assert_eq!(h.hooks.count("before"), 1);
            assert_eq!(h.hooks.count("after"), 1);
        });
    }

    fn cached(
        h: &Host,
        id: i64,
    ) -> (
        storage::InputOperation,
        SubmissionStage,
        u64,
        u64,
        bool,
        bool,
    ) {
        let retained = h
            .coordinator
            .operations
            .get(&id)
            .expect("actual retained reservation");
        (
            retained.operation.clone(),
            retained.stage,
            retained.written_bytes,
            retained.output_seq,
            retained.attempt_available,
            retained.leased,
        )
    }

    #[test]
    fn retirement_requires_settlement_and_no_active_writer_then_removes_only_its_cache() {
        with_host(|h| {
            let run = h.fixture.run.clone();
            let message = crate::broker::send_message(
                &mut h.fixture.conn,
                run.director_member_id,
                run.member_id,
                "retirement preview",
            )
            .unwrap();
            let key = format!("preview:{}:{}", run.run_id, message.message_id);
            let mut request = h.request(&key);
            request.operation = InputControlOperation::SubmitPrompt(SubmitInput {
                operation_key: key.clone(),
                input_kind: storage::InputKind::Preview,
                message_id: Some(message.message_id),
                literal: "ordinary fixture λ".into(),
            });
            assert!(
                h.coordinator
                    .handle(&mut h.fixture.conn, &run, request, &h.controller, h.until)
                    .unwrap()
                    .is_none()
            );
            let accepted = (0..8)
                .find_map(|_| {
                    h.drain();
                    h.advance()
                })
                .expect("actual Preview completes within finite drained turns");
            let InputControlReply::Submission(accepted) = accepted else {
                panic!("actual Preview submission reply")
            };
            assert!(matches!(accepted.status, SubmissionStatus::Accepted));
            let id = accepted.input_id.expect("durable Preview ID");
            let before = cached(h, id);
            assert!(matches!(before.1, SubmissionStage::Submitted));
            assert!(!before.4 && !before.5);
            assert!(h.coordinator.pending.is_none());
            let offers = h.pty.offers.clone();
            assert_eq!(
                h.pty.program,
                vec!["ordinary fixture λ~".as_bytes().to_vec()]
            );

            let Err(uncommitted) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, h.until)
            else {
                panic!("Accepted Preview requires committed settlement before retirement")
            };
            assert!(uncommitted.to_string().contains(&id.to_string()));
            assert_eq!(cached(h, id), before);
            assert_eq!(
                storage::get_input(&h.fixture.conn, id).unwrap().state,
                storage::InputState::Accepted
            );
            let settled = storage::settle_preview(&mut h.fixture.conn, &run.owner(), id).unwrap();
            let stamp: Option<String> = h
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(stamp.is_some());

            let Err(expired) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, Instant::now())
            else {
                panic!("retirement requires a live original deadline")
            };
            assert!(matches!(expired, CafleetError::DeadlineExceeded));
            let mut foreign_owner = run.clone();
            foreign_owner.owner_epoch += 1;
            assert!(
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &foreign_owner, id, h.until)
                    .is_err()
            );
            let mut foreign_request = h.request("unused-retirement-key");
            foreign_request.operation =
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
            foreign_request.identity.member_id += 1;
            assert!(
                h.coordinator
                    .handle(
                        &mut h.fixture.conn,
                        &run,
                        foreign_request,
                        &h.controller,
                        h.until
                    )
                    .is_err()
            );
            assert_eq!(cached(h, id), before);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);

            let unrelated_key = "prompt:retirement-active-writer";
            let unrelated = h.request(unrelated_key);
            assert!(
                h.coordinator
                    .handle(&mut h.fixture.conn, &run, unrelated, &h.controller, h.until)
                    .unwrap()
                    .is_none()
            );
            for _ in 0..8 {
                h.drain();
                assert!(h.advance().is_none());
                if h.coordinator.pending.as_ref().unwrap().phase == InputPhase::Armed {
                    break;
                }
            }
            let pending = h.coordinator.pending.as_ref().unwrap();
            assert!(pending.phase == InputPhase::Armed);
            let unrelated_id = pending.input_id.expect("actual second reservation");
            let original_deadline = pending.deadline;
            let original_bytes = pending.bytes.clone();
            let unrelated_before = cached(h, unrelated_id);
            assert!(unrelated_before.4 && !unrelated_before.5);
            let Err(failure) =
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, id, h.until)
            else {
                panic!("retirement excludes the actual Armed writer")
            };
            assert!(failure.to_string().contains(&id.to_string()));
            let pending = h
                .coordinator
                .pending
                .as_ref()
                .expect("refusal preserves unrelated writer");
            assert_eq!(pending.request.operation_key, unrelated_key);
            assert_eq!(pending.input_id, Some(unrelated_id));
            assert_eq!(pending.deadline, original_deadline);
            assert_eq!(pending.bytes, original_bytes);
            assert!(pending.phase == InputPhase::Armed);
            assert_eq!(cached(h, unrelated_id), unrelated_before);
            assert_eq!(cached(h, id), before);
            assert_eq!(h.pty.offers, offers);

            h.coordinator.finish_pending_attempt();
            let other_cache = cached(h, unrelated_id);
            let other_row = storage::get_input(&h.fixture.conn, unrelated_id).unwrap();
            assert_eq!(other_row.state, storage::InputState::InProgress);
            h.pty
                .child
                .bytes
                .extend(b"\x1b[2J\x1b[HUnclassified screen");
            h.drain();
            assert!(!matches!(
                h.controller.health(&h.writer).eligibility,
                InputEligibility::ComposerEmpty
            ));
            let changes = h.fixture.conn.total_changes();
            let mut retirement = h.request("unused-retirement-key");
            retirement.operation =
                InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
            let reply = h
                .coordinator
                .handle(
                    &mut h.fixture.conn,
                    &run,
                    retirement,
                    &h.controller,
                    h.until,
                )
                .unwrap()
                .expect("typed retirement dispatch");
            assert_retired(reply, id);
            assert!(!h.coordinator.operations.contains_key(&id));
            assert_eq!(cached(h, unrelated_id), other_cache);
            assert_eq!(h.fixture.conn.total_changes(), changes);
            assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
            assert_eq!(
                storage::get_input(&h.fixture.conn, unrelated_id).unwrap(),
                other_row
            );
            let after_stamp: Option<String> = h
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(after_stamp, stamp);
            assert_eq!(h.pty.offers, offers);

            let reads = Arc::new(AtomicUsize::new(0));
            let observed = reads.clone();
            h.fixture
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "member_input_operations",
                            ..
                        }
                    ) {
                        observed.fetch_add(1, Ordering::SeqCst);
                    }
                    Authorization::Allow
                }))
                .unwrap();
            let again = h
                .coordinator
                .retire_received(&mut h.fixture.conn, &run, id, h.until);
            let removed = h
                .fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            assert!(
                again.is_ok(),
                "repeat_error={:?}; authorizer_cleanup={removed:?}",
                again.as_ref().err()
            );
            removed.unwrap();
            assert_retired(again.unwrap(), id);
            assert!(
                reads.load(Ordering::SeqCst) > 0,
                "cache absence still checks the durable row on the supplied connection"
            );
            assert!(
                h.coordinator
                    .retire_received(&mut h.fixture.conn, &run, i64::MAX, h.until)
                    .is_err()
            );
            assert_eq!(cached(h, unrelated_id), other_cache);
            assert_eq!(h.pty.offers, offers);
        });
    }

    #[test]
    fn live_receipt_retirement_preserves_settled_ordinary_history_without_a_cached_attempt() {
        with_host(|h| {
            let run = h.fixture.run.clone();
            let prompt = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: "prompt:retirement-history".into(),
                    kind: storage::InputKind::Prompt,
                    payload_digest: "a".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            storage::record_input_state(
                &mut h.fixture.conn,
                &run.owner(),
                prompt.input_id,
                storage::InputState::InProgress,
                storage::InputState::Accepted,
            )
            .unwrap();
            let prompt = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                prompt.input_id,
                storage::InputState::Accepted,
                storage::InputDisposition::Submitted,
                "owned historical Prompt",
            )
            .unwrap();
            let message = crate::broker::send_message(
                &mut h.fixture.conn,
                run.director_member_id,
                run.member_id,
                "abandoned preview",
            )
            .unwrap();
            let preview = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: format!("preview:{}:{}", run.run_id, message.message_id),
                    kind: storage::InputKind::Preview,
                    payload_digest: "b".repeat(64),
                    message_id: Some(message.message_id),
                },
            )
            .unwrap();
            storage::record_input_state(
                &mut h.fixture.conn,
                &run.owner(),
                preview.input_id,
                storage::InputState::InProgress,
                storage::InputState::Uncertain,
            )
            .unwrap();
            let preview = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                preview.input_id,
                storage::InputState::Uncertain,
                storage::InputDisposition::AbandonedUncertain,
                "owned abandoned Preview",
            )
            .unwrap();
            let exec = storage::reserve_input(
                &mut h.fixture.conn,
                &run.owner(),
                &storage::NewInput {
                    operation_key: "exec:retirement-history".into(),
                    kind: storage::InputKind::Exec,
                    payload_digest: "c".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            let exec = storage::settle_input(
                &mut h.fixture.conn,
                &run.owner(),
                exec.input_id,
                storage::InputState::InProgress,
                storage::InputDisposition::NotSubmitted,
                "owned no-write Exec history",
            )
            .unwrap();
            assert!(h.coordinator.operations.is_empty());
            let changes = h.fixture.conn.total_changes();
            for row in [prompt, preview, exec] {
                assert_retired(
                    h.coordinator
                        .retire_received(&mut h.fixture.conn, &run, row.input_id, h.until)
                        .unwrap(),
                    row.input_id,
                );
                assert_eq!(
                    storage::get_input(&h.fixture.conn, row.input_id).unwrap(),
                    row
                );
                assert_eq!(h.fixture.conn.total_changes(), changes);
            }
            assert!(h.coordinator.operations.is_empty());
            assert!(h.pty.offers.is_empty());
        });
    }
}
