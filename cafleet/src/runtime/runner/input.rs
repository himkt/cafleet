#[cfg(test)]
use super::InputDrain;
use super::{
    ControlIdentity, DrainedInput, InputControlOperation, InputControlReply, InputControlRequest,
    InputEligibility, ObservationController, ObservedContinuation, ObservedRequest,
    ProgrammaticWrite, PtyIo, SubmissionResult, SubmissionStatus, Visibility,
    validate_operation_key,
};
use super::{
    InputFailure, ProcessIdentity, Result, RunLock, SubmissionStage, Transport, check_deadline,
    ensure, error, stable_run_identity, with_database_deadline,
};
use super::{
    MissingEvidence, PermissionInspectInput, PermissionInspection, PermissionResponseResult,
};
use crate::broker::records::{MemberKind, Placement};
use crate::broker::runtime::{self as storage, OwnerKind, RunRecord, RunStatus};
use crate::coding_agent::{AnswerMatch, AnswerPlan, AnswerStage, ProfileCatalog, StageGate};
use crate::coding_agent::{ComposerOperation, coding_agent};
use crate::error::CafleetError;
use crate::input::{Deadline, PaneLock, bounded_by_inherited, with_database_budget};
use rusqlite::{Connection, TransactionBehavior};
use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) trait RequestTokenSource: Send {
    fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()>;
}

const RECEIPT_CACHE_CAPACITY: usize = 4096;
const STAGE_SETTLE: Duration = Duration::from_millis(500);
const DEFERRED_ATTEMPT_BUDGET: Duration = Duration::from_secs(5);
const DEFERRED_FINALIZATION: Duration = Duration::from_secs(1);
const FALLBACK_ESCAPE: u8 = 0x1b;
const FALLBACK_ENTER: u8 = 0x0d;

pub(crate) enum InputBoundary {
    Reserved,
    BeforeWrite,
    AfterWrite,
    TerminalState,
}

pub(crate) struct InputCheckpoint {
    pub(crate) input_id: i64,
    pub(crate) stage: SubmissionStage,
    pub(crate) written_bytes: u64,
    pub(crate) output_seq: u64,
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
    pub(crate) catalog: ProfileCatalog,
    pub(crate) token_source: Box<dyn RequestTokenSource>,
    pub(crate) hooks: Arc<dyn InputHooks>,
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
    current_request: Option<(CurrentRequest, String)>,
    input_fenced: bool,
    observation_blocker: Option<String>,
    database_url: String,
    queue: std::collections::VecDeque<QueuedInput>,
}

/// Runner-owned ordinary input accepted with a durable ready receipt; its literal stays in
/// memory only.
#[derive(Clone)]
struct QueuedInput {
    input_id: i64,
    request: storage::NewInput,
    literal: String,
    operation: ComposerOperation,
    payload_digest_version: i64,
    attempt_deadline: Option<Instant>,
}

type DeliverySelection = (
    storage::DeliveryMethod,
    Option<&'static str>,
    Arc<[Arc<[u8]>]>,
);

struct DeferredAttempt {
    queued: QueuedInput,
    obstacle: &'static str,
}

#[derive(Clone)]
pub(crate) struct CurrentRequest {
    observed: ObservedRequest,
    identity: ControlIdentity,
    interaction_id: i64,
    event_id: i64,
    message_id: i64,
    manual_generation: u64,
}
impl CurrentRequest {
    fn same_request(&self, other: &Self) -> bool {
        self.observed.request_generation == other.observed.request_generation
            && self.identity == other.identity
            && self.interaction_id == other.interaction_id
            && self.event_id == other.event_id
            && self.message_id == other.message_id
            && self.manual_generation == other.manual_generation
            && self.observed.same_request(&other.observed)
    }
}

pub(super) fn inspection_association(
    conn: &Connection,
    run: &RunRecord,
    input: &PermissionInspectInput,
) -> Result<storage::RuntimeEventView> {
    ensure(
        input.event_id > 0
            && input.interaction_id > 0
            && input.director_member_id == run.director_member_id,
        "permission inspection identity mismatch",
    )?;
    let view = storage::get_event(conn, input.event_id)?;
    ensure(
        view.event.run_id == run.run_id
            && view.event.interaction_id == Some(input.interaction_id)
            && view.event.kind == storage::RuntimeEventKind::InteractionOpened
            && view.event.notification_state == storage::NotificationState::Inbox
            && view.event.message_id.is_some_and(|id| id > 0)
            && view.interaction.as_ref().is_some_and(|i| {
                i.run_id == run.run_id
                    && i.classification == storage::InteractionClass::PermissionRequest
                    && i.closed_at.is_none()
            }),
        "permission inspection current event/interaction/message mismatch",
    )?;
    let director = crate::broker::get_member(conn, run.director_member_id, run.fleet_id)?
        .ok_or_else(|| error("permission Director is absent"))?;
    ensure(
        director.kind == MemberKind::Director
            && crate::broker::active_member_fleet(conn, run.director_member_id)?
                == Some(run.fleet_id),
        "permission Director is inactive",
    )?;
    let live: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM fleets WHERE fleet_id=?1 AND director_member_id=?2 AND deleted_at IS NULL)", rusqlite::params![run.fleet_id, run.director_member_id], |r| r.get(0))?;
    ensure(live, "permission fleet/Director changed")?;
    Ok(view)
}

pub(super) fn inspection_missing_evidence(
    evidence: &serde_json::Value,
) -> Result<Vec<MissingEvidence>> {
    storage::evidence::validate(evidence)?;
    let mut missing = Vec::new();
    let known = evidence["truncated"] == false
        && evidence["operation"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty() && text != "[redacted]");
    if !known {
        missing.push(MissingEvidence::Operation);
    }
    missing.extend([
        MissingEvidence::Arguments,
        MissingEvidence::Cwd,
        MissingEvidence::Environment,
        MissingEvidence::Targets,
        MissingEvidence::Effects,
        MissingEvidence::ChoiceScope,
    ]);
    Ok(missing)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    AwaitReservation,
    AwaitBeforeWrite,
    Armed,
    Blocked,
}

struct PermissionSelection {
    request: storage::NewPermissionResponse,
    binding: CurrentRequest,
    continuation: Option<ObservedContinuation>,
    gap_count: u64,
    plan: AnswerPlan,
}
enum StageGateProgress {
    Ready,
    Waiting,
}
// A gate still unmet this close to the response deadline fails now, so the written stages'
// uncertainty is persisted within the original budget.
const GATE_FINALIZATION_RESERVE: std::time::Duration = std::time::Duration::from_millis(100);

enum RetainedSelection {
    Composer {
        method: storage::DeliveryMethod,
        stages: Arc<[Arc<[u8]>]>,
        next_stage: usize,
    },
    Permission {
        selection: Arc<PermissionSelection>,
        next_stage: usize,
    },
}

#[derive(Clone, Copy)]
enum PendingReservation {
    Unreserved,
    HistoricalReady,
    Installed,
}

struct PendingInput {
    reservation: PendingReservation,
    ready: Option<storage::PermissionReceipt>,
    request: storage::NewInput,
    bytes: Arc<[u8]>,
    permission: Option<Arc<PermissionSelection>>,
    composer: Option<ComposerDelivery>,
    deadline: Instant,
    phase: InputPhase,
    input_id: Option<i64>,
}

struct ComposerDelivery {
    method: storage::DeliveryMethod,
    reason: Option<&'static str>,
    stages: Arc<[Arc<[u8]>]>,
    written_stages: usize,
    settle_until: Option<Instant>,
    manual_generation: Option<u64>,
    pane: Option<PaneLock>,
    payload_digest_version: i64,
    deferred: Option<DeferredAttempt>,
}

impl ComposerDelivery {
    fn requires_empty_composer(&self) -> bool {
        self.method == storage::DeliveryMethod::Profile && self.written_stages == 0
    }
}

enum OrdinaryBlocker {
    Interaction,
    ObserverUnavailable,
}

fn ordinary_blocker(status: &super::ObservationStatus) -> Option<OrdinaryBlocker> {
    use super::GateReason;
    if matches!(
        status.visibility,
        Visibility::Visible | Visibility::Obscured
    ) || matches!(
        status.eligibility,
        InputEligibility::Hold(GateReason::Candidate | GateReason::Interaction)
    ) || status.retained_transition
    {
        return Some(OrdinaryBlocker::Interaction);
    }
    if matches!(
        status.eligibility,
        InputEligibility::Hold(GateReason::ObserverUnavailable | GateReason::Gap)
    ) || status.diagnostic.is_some()
    {
        return Some(OrdinaryBlocker::ObserverUnavailable);
    }
    None
}

fn fallback_stages(prefix: &[u8], literal: &str) -> Result<Arc<[Arc<[u8]>]>> {
    let size = prefix
        .len()
        .checked_add(literal.len())
        .and_then(|size| size.checked_add(1))
        .filter(|size| *size <= 65536)
        .ok_or_else(|| error("native composer payload exceeds 65536 bytes"))?;
    let mut text = Vec::with_capacity(size);
    text.extend_from_slice(prefix);
    text.extend_from_slice(literal.as_bytes());
    Ok(Arc::from(vec![
        Arc::from(&[FALLBACK_ESCAPE][..]),
        Arc::from(text),
        Arc::from(&[FALLBACK_ENTER][..]),
    ]))
}

impl PendingInput {
    fn installed_input_id(&self) -> Option<i64> {
        match self.reservation {
            PendingReservation::Unreserved => {
                assert!(
                    self.input_id.is_none(),
                    "unreserved input has no installed ID"
                );
                assert!(
                    self.ready.is_none(),
                    "unreserved input has no Ready history"
                );
                None
            }
            PendingReservation::HistoricalReady => {
                let ready = self
                    .ready
                    .as_ref()
                    .expect("historical Ready snapshot retained");
                assert_eq!(self.input_id, Some(ready.input.input_id));
                assert_eq!(ready.input.state, storage::InputState::Ready);
                None
            }
            PendingReservation::Installed => Some(
                self.input_id
                    .expect("installed input reservation has an ID"),
            ),
        }
    }
}

struct RetainedInput {
    operation: storage::InputOperation,
    bytes: Arc<[u8]>,
    selection: RetainedSelection,
    stage: SubmissionStage,
    written_bytes: u64,
    output_seq: u64,
    attempt_available: bool,
    leased: bool,
}

fn selected_answer_stage(plan: &AnswerPlan, index: usize) -> Result<&AnswerStage> {
    plan.stages()
        .get(index)
        .ok_or_else(|| error("selected answer stage is absent"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GuardPermit {
    Fresh,
    Rejected,
    ConsumedPreIo,
    IoPossible,
    ResultRecorded,
}

struct ReceiptGuardBinding {
    owner: storage::RunOwner,
    receipt: storage::PermissionReceipt,
    zero_known: bool,
    rearm: Option<Arc<PermissionSelection>>,
}

pub(crate) struct InputGuard<'a> {
    retained: Option<&'a mut RetainedInput>,
    proof: Option<DrainedInput>,
    binding: Option<ReceiptGuardBinding>,
    deadline: Instant,
    permit: GuardPermit,
    exclusion: std::marker::PhantomData<(&'a mut InputCoordinator, std::rc::Rc<()>)>,
}

impl InputGuard<'_> {
    pub(crate) fn zero_byte_retirement(
        &self,
        owner: &storage::RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<storage::ZeroByteRetirement<'_>> {
        storage::ZeroByteRetirement::from_live_guard(self, owner, input_id, interaction_id)
    }

    pub(crate) fn retirement_binding(
        &self,
        owner: &storage::RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<(
        &storage::PermissionReceipt,
        Option<&crate::coding_agent::AnswerCapability>,
    )> {
        check_deadline(Instant::now(), self.deadline)?;
        let binding = self
            .binding
            .as_ref()
            .ok_or_else(|| error("native writer has no receipt-exclusion proof"))?;
        ensure(
            binding.owner == *owner
                && binding.receipt.input.input_id == input_id
                && binding.receipt.response.metadata.interaction_id == interaction_id
                && binding.zero_known,
            "matching actual zero-byte exclusion required",
        )?;
        Ok((
            &binding.receipt,
            binding
                .rearm
                .as_ref()
                .map(|selection| selection.plan.capability()),
        ))
    }

    pub(super) fn begin_write(
        &mut self,
        transport: &Transport,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<Instant> {
        ensure(
            self.binding.is_none(),
            "receipt exclusion grants no native-write permit",
        )?;
        let retained = self
            .retained
            .as_mut()
            .ok_or_else(|| error("native writer is absent"))?;
        ensure(
            self.permit == GuardPermit::Fresh,
            "input write permit is consumed",
        )?;
        self.permit = GuardPermit::Rejected;
        self.deadline = self.deadline.min(deadline);
        check_deadline(Instant::now(), self.deadline)?;
        ensure(
            bytes == retained.bytes.as_ref(),
            "input bytes do not match the selected stage",
        )?;
        ensure(
            retained.operation.state == storage::InputState::InProgress && retained.leased,
            "input write requires its retained reservation",
        )?;
        match &retained.selection {
            RetainedSelection::Composer {
                method,
                stages,
                next_stage,
            } => {
                let proof = self
                    .proof
                    .as_ref()
                    .ok_or_else(|| error("native drain is absent"))?;
                ensure(
                    stages
                        .get(*next_stage)
                        .is_some_and(|stage| stage.as_ref() == bytes),
                    "composer stage selection changed",
                )?;
                if *method == storage::DeliveryMethod::Profile && *next_stage == 0 {
                    transport.validate_input_drain(proof, self.deadline)?
                } else {
                    transport.validate_observation_drain(proof, self.deadline)?
                }
            }
            RetainedSelection::Permission {
                selection,
                next_stage,
            } => {
                let stage = selected_answer_stage(&selection.plan, *next_stage)?;
                ensure(stage.bytes() == bytes, "permission stage selection changed")?;
                transport.validate_observation_drain(
                    self.proof
                        .as_ref()
                        .ok_or_else(|| error("native drain is absent"))?,
                    self.deadline,
                )?;
            }
        }
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
        let retained = self
            .retained
            .as_mut()
            .ok_or_else(|| error("native writer is absent"))?;
        ensure(
            self.permit == GuardPermit::IoPossible,
            "input result requires its possible-I/O attempt",
        )?;
        ensure(
            offered == retained.bytes.len(),
            "input offered count does not match its selected stage",
        )?;
        match result {
            Ok(count) => {
                ensure(*count <= offered, "native input count exceeds its offer")?;
                retained.written_bytes = retained
                    .written_bytes
                    .checked_add(
                        u64::try_from(*count)
                            .map_err(|_| error("native input count exceeds range"))?,
                    )
                    .ok_or_else(|| error("native input count exhausted"))?;
                if *count == offered {
                    retained.stage = match &mut retained.selection {
                        RetainedSelection::Composer {
                            stages, next_stage, ..
                        } => {
                            *next_stage = next_stage
                                .checked_add(1)
                                .ok_or_else(|| error("composer stage index exhausted"))?;
                            match stages.get(*next_stage) {
                                Some(stage) => {
                                    retained.bytes = stage.clone();
                                    SubmissionStage::Partial
                                }
                                None => SubmissionStage::Submitted,
                            }
                        }
                        RetainedSelection::Permission {
                            selection,
                            next_stage,
                        } => {
                            let _ = selected_answer_stage(&selection.plan, *next_stage)?;
                            *next_stage = next_stage
                                .checked_add(1)
                                .ok_or_else(|| error("answer stage index exhausted"))?;
                            if *next_stage == selection.plan.stages().len() {
                                SubmissionStage::Submitted
                            } else {
                                retained.bytes = Arc::from(
                                    selected_answer_stage(&selection.plan, *next_stage)?.bytes(),
                                );
                                SubmissionStage::Partial
                            }
                        }
                    };
                } else if *count > 0 {
                    retained.stage = SubmissionStage::Partial;
                }
            }
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => {
                if retained.stage == SubmissionStage::ZeroBytes {
                    retained.stage = SubmissionStage::Unknown;
                }
            }
        }
        self.permit = GuardPermit::ResultRecorded;
        Ok(())
    }
}

impl Drop for InputGuard<'_> {
    fn drop(&mut self) {
        if let Some(retained) = self.retained.as_mut() {
            if self.permit == GuardPermit::IoPossible
                && retained.stage == SubmissionStage::ZeroBytes
            {
                retained.stage = SubmissionStage::Unknown;
            }
            retained.leased = false;
        }
    }
}

#[derive(Clone, Copy)]
enum DrainRequirement {
    Observation,
    EmptyComposer,
    NoBlockingInteraction,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteValidation {
    Ready,
    Contended,
}

impl InputCoordinator {
    pub(super) fn inspect_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        live: (&ObservationController, &mut Transport),
        deadline: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let (observation, transport) = live;
        let interaction_id = match &request.operation {
            InputControlOperation::PermissionInspect(input) => Some(input.interaction_id),
            _ => None,
        };
        let result =
            (|| {
                let deadline = bounded_by_inherited(conn, deadline);
                check_deadline(Instant::now(), deadline)?;
                request.identity.validate(&self.run)?;
                let InputControlOperation::PermissionInspect(input) = request.operation else {
                    return Err(error("permission inspection operation required"));
                };
                let view = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction()?;
                    self.validate_live_run(&tx, run)?;
                    let view = inspection_association(&tx, &self.run, &input)?;
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit()?;
                    Ok(view)
                })?;
                let interaction = view
                    .interaction
                    .expect("validated current permission interaction");
                let message_id = view.event.message_id.expect("validated inbox message");
                let evidence = observation
                    .current_evidence()
                    .ok_or_else(|| error("current inspection observation unavailable"))?
                    .clone();
                let missing_evidence = inspection_missing_evidence(&evidence)?;
                let fresh = match transport.take_input_drain() {
                    Some(proof) => {
                        transport.validate_observation_drain(&proof, deadline)?;
                        true
                    }
                    None => false,
                };
                let observed = observation.current_request();
                if let Some((current, _)) = &self.current_request
                    && (current.observed.request_generation != observation.request_generation()
                        || current.manual_generation != self.manual_generation
                        || observed.is_none())
                {
                    self.current_request = None;
                }
                let mut supported_decisions = Vec::new();
                let mut answer_profile_version = None;
                let mut answer_rule_id = None;
                let mut request_token = None;
                let blocker = self.permission_blocker(observation);
                let mut reason =
                    Some(blocker.clone().unwrap_or_else(|| {
                        "current executable permission request unavailable".into()
                    }));
                if fresh
                    && blocker.is_none()
                    && let Some(observed) = observed
                {
                    ensure(
                        observed.episode_seq == interaction.episode_seq
                            && observed.classification == interaction.classification,
                        "permission inspection durable episode changed",
                    )?;
                    let next = CurrentRequest {
                        observed: observed.clone(),
                        identity: ControlIdentity::from_run(&self.run),
                        interaction_id: interaction.interaction_id,
                        event_id: view.event.event_id,
                        message_id,
                        manual_generation: self.manual_generation,
                    };
                    if !next.observed.answer_matches.is_empty() {
                        if let Some((current, _)) = &mut self.current_request
                            && current.same_request(&next)
                        {
                            current.observed.output_seq = next.observed.output_seq;
                        } else {
                            self.current_request = None;
                            let mut bytes = [0; 32];
                            self.services.token_source.fill(&mut bytes)?;
                            let token = crate::digest::hex(&bytes);
                            self.current_request = Some((next, token));
                        }
                        let (current, token) = self
                            .current_request
                            .as_ref()
                            .expect("matched current request retained");
                        let first = current.observed.answer_matches[0].plan().capability();
                        answer_profile_version = Some(first.profile_version().to_owned());
                        if current.observed.answer_matches.iter().all(|m| {
                            m.plan().capability().answer_rule_id() == first.answer_rule_id()
                        }) {
                            answer_rule_id = Some(first.answer_rule_id().to_owned());
                        }
                        supported_decisions = current
                            .observed
                            .answer_matches
                            .iter()
                            .map(|m| m.plan().capability().decision())
                            .collect();
                        request_token = Some(token.clone());
                        reason = None;
                    }
                }
                check_deadline(Instant::now(), deadline)?;
                Ok(InputControlReply::Inspection(Box::new(
                    PermissionInspection {
                        event_id: view.event.event_id,
                        message_id,
                        member_id: self.run.member_id,
                        fleet_id: self.run.fleet_id,
                        director_member_id: self.run.director_member_id,
                        run_id: self.run.run_id,
                        owner_epoch: self.run.owner_epoch,
                        interaction_id: interaction.interaction_id,
                        episode_seq: interaction.episode_seq,
                        classification: observation.current_classification(),
                        visibility: observation.status().visibility,
                        backend: self.run.backend.clone(),
                        native_version: self.run.native_version.clone(),
                        profile_version: self.run.profile_version.clone(),
                        answer_profile_version,
                        answer_rule_id,
                        answer_capability: if request_token.is_some() {
                            storage::Coverage::Tested
                        } else {
                            storage::Coverage::Unknown
                        },
                        capability_reason: reason,
                        supported_decisions,
                        request_token,
                        output_seq: transport.snapshot().screen_seq,
                        evidence,
                        missing_evidence,
                    },
                )))
            })();
        result.map_err(|source| {
            self.current_request = None;
            self.run_failure(source, interaction_id, None, None)
        })
    }

    pub(crate) fn fence(&mut self, deadline: Instant) -> Result<()> {
        self.current_request = None;
        self.input_fenced = true;
        check_deadline(Instant::now(), deadline)
    }
    fn permission_blocker(&self, observation: &ObservationController) -> Option<String> {
        let status = observation.status();
        if self.input_fenced {
            Some("permission input is fenced".into())
        } else if self.manual_accounting_failed {
            Some("manual input accounting failed".into())
        } else if let Some(reason) = &self.observation_blocker {
            Some(reason.clone())
        } else if let Some(code) = &status.diagnostic {
            Some(format!("observation degraded: {code}"))
        } else if status.retained_transition || status.retained_draft || status.retained_evidence {
            Some("observation update pending".into())
        } else if status.visibility != Visibility::Visible {
            Some("permission request is not visible".into())
        } else if status.eligibility != InputEligibility::Hold(super::GateReason::Interaction) {
            Some("permission request is not the current interaction".into())
        } else {
            None
        }
    }
    fn validate_permission_binding(
        &self,
        selection: &PermissionSelection,
        observation: &ObservationController,
    ) -> Result<()> {
        if let Some(reason) = self.permission_blocker(observation) {
            return Err(error(format!("permission request is blocked: {reason}")));
        }
        let (current, token) = self
            .current_request
            .as_ref()
            .ok_or_else(|| error("current permission token unavailable"))?;
        let bound = &selection.binding;
        if let Some(continuation) = &selection.continuation {
            return ensure(
                token == &selection.request.metadata.request_token
                    && self.manual_generation == bound.manual_generation
                    && observation.gap_count() == selection.gap_count
                    && observation.request_generation() == continuation.request_generation
                    && observation.current_continuation() == Some(continuation),
                "permission continuation binding changed",
            );
        }
        let observed = observation
            .current_request()
            .ok_or_else(|| error("current permission request unavailable"))?;
        ensure(
            token == &selection.request.metadata.request_token
                && current.same_request(bound)
                && self.manual_generation == bound.manual_generation
                && observation.request_generation() == bound.observed.request_generation
                && observed.request_generation == bound.observed.request_generation
                && observed.classification == storage::InteractionClass::PermissionRequest
                && observed.same_request(&bound.observed),
            "permission request token or current plan binding changed",
        )?;
        ensure(
            bound
                .observed
                .answer_matches
                .iter()
                .any(|m| m.plan() == &selection.plan),
            "selected permission plan changed",
        )?;
        Ok(())
    }

    /// Before each gated stage, holds the plan until the observation shows that stage's gate.
    /// After a complete earlier stage it accepts exactly the transition the gate names and
    /// rebinds the in-flight selection to it without minting a token.
    fn advance_stage_gate(
        &mut self,
        observation: &ObservationController,
    ) -> Result<StageGateProgress> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(StageGateProgress::Ready);
        };
        let Some(selection) = pending.permission.clone() else {
            return Ok(StageGateProgress::Ready);
        };
        let id = pending
            .input_id
            .ok_or_else(|| error("permission reservation absent"))?;
        let RetainedSelection::Permission { next_stage, .. } = &self
            .operations
            .get(&id)
            .ok_or_else(|| error("retained permission selection absent"))?
            .selection
        else {
            return Err(error("permission stage target changed"));
        };
        let next_stage = *next_stage;
        let Some(gate) = selected_answer_stage(&selection.plan, next_stage)?
            .gate()
            .cloned()
        else {
            return Ok(StageGateProgress::Ready);
        };
        let bound = &selection.binding;
        let bound_focus = || {
            bound
                .observed
                .answer_matches
                .iter()
                .find(|matched| matched.plan() == &selection.plan)
                .and_then(AnswerMatch::focused_choice_id)
        };
        let holds = match (&gate, &selection.continuation) {
            (StageGate::Focus(choice), None) => bound_focus() == Some(choice.as_str()),
            (StageGate::Screen(rule), Some(continuation)) => &continuation.rule_id == rule,
            _ => false,
        };
        if holds {
            return Ok(StageGateProgress::Ready);
        }
        ensure(next_stage > 0, "the first stage's gate does not hold")?;
        ensure(
            self.manual_generation == bound.manual_generation
                && !self.manual_accounting_failed
                && !self.input_fenced
                && self.observation_blocker.is_none()
                && observation.status().diagnostic.is_none()
                && observation.gap_count() == selection.gap_count,
            "input or observation changed between answer stages",
        )?;
        ensure(
            observation.active_episode_seq() == Some(bound.observed.episode_seq),
            "the permission episode changed between answer stages",
        )?;
        let deadline = self
            .pending
            .as_ref()
            .expect("validated pending permission")
            .deadline;
        let waiting = || {
            ensure(
                Instant::now() + GATE_FINALIZATION_RESERVE < deadline,
                "the next stage's gate did not hold before the response deadline",
            )
            .map(|()| StageGateProgress::Waiting)
        };
        let observed = observation.current_request();
        let unchanged = observed.is_some_and(|observed| {
            selection.continuation.is_none()
                && observed.request_generation == bound.observed.request_generation
                && observed.focus == bound.observed.focus
        });
        let rebound = match &gate {
            StageGate::Focus(choice) => {
                ensure(
                    observation.current_continuation().is_none(),
                    "an unexpected continuation screen appeared between answer stages",
                )?;
                let Some(observed) = observed.filter(|_| !unchanged) else {
                    return waiting();
                };
                let focused = observed
                    .answer_matches
                    .iter()
                    .find(|matched| matched.plan() == &selection.plan)
                    .and_then(AnswerMatch::focused_choice_id);
                ensure(
                    observed.episode_seq == bound.observed.episode_seq
                        && observed.request_rule_id == bound.observed.request_rule_id
                        && observed.semantic == bound.observed.semantic
                        && observed.choices == bound.observed.choices
                        && observed.native_correlation == bound.observed.native_correlation
                        && focused == Some(choice.as_str()),
                    "the request changed other than to the next stage's focus",
                )?;
                PermissionSelection {
                    request: selection.request.clone(),
                    binding: CurrentRequest {
                        observed: ObservedRequest {
                            request_generation: observed.request_generation,
                            focus: observed.focus.clone(),
                            output_seq: observed.output_seq,
                            answer_matches: observed.answer_matches.clone(),
                            ..bound.observed.clone()
                        },
                        ..bound.clone()
                    },
                    continuation: None,
                    gap_count: selection.gap_count,
                    plan: selection.plan.clone(),
                }
            }
            StageGate::Screen(rule) => {
                let Some(continuation) = observation.current_continuation() else {
                    ensure(
                        observed.is_none() || unchanged,
                        "the request changed instead of opening its continuation",
                    )?;
                    return waiting();
                };
                ensure(
                    &continuation.rule_id == rule
                        && continuation.episode_seq == bound.observed.episode_seq,
                    "a different continuation screen appeared",
                )?;
                PermissionSelection {
                    request: selection.request.clone(),
                    binding: bound.clone(),
                    continuation: Some(continuation.clone()),
                    gap_count: selection.gap_count,
                    plan: selection.plan.clone(),
                }
            }
        };
        let rebound = Arc::new(rebound);
        if rebound.continuation.is_none() {
            let (_, token) = self
                .current_request
                .take()
                .ok_or_else(|| error("current permission token unavailable"))?;
            self.current_request = Some((rebound.binding.clone(), token));
        }
        let retained = self
            .operations
            .get_mut(&id)
            .expect("validated retained permission selection");
        let RetainedSelection::Permission { selection, .. } = &mut retained.selection else {
            unreachable!("validated permission selection");
        };
        *selection = rebound.clone();
        self.pending
            .as_mut()
            .expect("validated pending permission")
            .permission = Some(rebound);
        Ok(StageGateProgress::Ready)
    }

    fn validate_permission_association(
        &self,
        conn: &Connection,
        selection: &PermissionSelection,
    ) -> Result<()> {
        let metadata = &selection.request.metadata;
        let view = inspection_association(
            conn,
            &self.run,
            &PermissionInspectInput {
                interaction_id: metadata.interaction_id,
                event_id: metadata.event_id,
                director_member_id: metadata.director_member_id,
            },
        )?;
        let interaction = view.interaction.expect("validated permission interaction");
        ensure(
            interaction.episode_seq == selection.binding.observed.episode_seq
                && view.event.message_id == Some(selection.binding.message_id)
                && metadata.expected_owner_epoch == self.run.owner_epoch,
            "permission response current association changed",
        )
    }

    fn permission_history(
        &self,
        conn: &mut Connection,
        run: &RunRecord,
        request: &InputControlRequest,
        input: &super::PermissionRespondInput,
        deadline: Instant,
        known_id: &mut Option<i64>,
    ) -> Result<Option<storage::PermissionReceipt>> {
        check_deadline(Instant::now(), deadline)?;
        super::input_client::validate_respond(input)?;
        let mut expected = ControlIdentity::from_run(&self.run);
        expected.owner_epoch = input.expected_owner_epoch;
        ensure(
            request.identity == expected
                && input.expected_run_id == self.run.run_id
                && run.run_id == self.run.run_id,
            "permission response fixed identity mismatch",
        )?;
        with_database_deadline(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            ensure(
                storage::database_identity(&tx)? == self.run.database_identity_json,
                "input database identity changed",
            )?;
            let retained = super::input_client::response_lookup(&tx, &self.run, input, known_id)?;
            if let Some(receipt) = &retained {
                *known_id = Some(receipt.input.input_id);
            }
            check_deadline(Instant::now(), deadline)?;
            tx.commit()?;
            Ok(retained)
        })
    }

    fn retained_projection(
        &self,
        history: &Option<storage::PermissionReceipt>,
    ) -> Result<Option<InputControlReply>> {
        match history {
            Some(receipt) if receipt.input.state != storage::InputState::Ready => {
                super::input_client::projection(&self.run, receipt.clone())
                    .map(|result| Some(InputControlReply::PermissionResponse(result)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn retained_permission_response(
        &self,
        conn: &mut Connection,
        run: &RunRecord,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let InputControlOperation::PermissionRespond(input) = &request.operation else {
            unreachable!("retained permission lookup receives a response operation");
        };
        let mut known_id = None;
        let result = (|| {
            let deadline = bounded_by_inherited(conn, deadline);
            let history =
                self.permission_history(conn, run, request, input, deadline, &mut known_id)?;
            self.retained_projection(&history)
        })();
        result.map_err(|source| self.response_failure(source, input, known_id))
    }

    fn permission_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let InputControlOperation::PermissionRespond(input) = &request.operation else {
            unreachable!("permission response route selects its semantic operation");
        };
        let mut known_id = None;
        let result = (|| {
            let deadline = bounded_by_inherited(conn, deadline);
            let history =
                self.permission_history(conn, run, &request, input, deadline, &mut known_id)?;
            if let Some(reply) = self.retained_projection(&history)? {
                return Ok(Some(reply));
            }
            request.identity.validate(&self.run)?;
            if let Some(reason) = self.permission_blocker(observation) {
                return Err(error(format!("permission request is blocked: {reason}")));
            }
            let (binding, token) = self
                .current_request
                .as_ref()
                .ok_or_else(|| error("current permission token unavailable"))?;
            ensure(
                token == &input.request_token
                    && binding.interaction_id == input.interaction_id
                    && binding.event_id == input.event_id
                    && binding.identity == request.identity,
                "permission response token identity changed",
            )?;
            let plan = binding
                .observed
                .answer_matches
                .iter()
                .find(|matched| matched.plan().capability().decision() == input.decision)
                .ok_or_else(|| error("selected permission decision is unsupported"))?
                .plan()
                .clone();
            let cap = plan.capability();
            let selection = Arc::new(PermissionSelection {
                request: storage::NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: storage::PermissionResponseMetadata {
                        event_id: input.event_id,
                        interaction_id: input.interaction_id,
                        expected_owner_epoch: input.expected_owner_epoch,
                        request_token: input.request_token.clone(),
                        decision: input.decision,
                        director_member_id: input.director_member_id,
                        director_mode: input.director_mode,
                        decision_source: input.decision_source,
                        mode_evidence_ref: input.mode_evidence_ref.clone(),
                        decision_evidence_ref: input.decision_evidence_ref.clone(),
                        answer_profile_version: cap.profile_version().into(),
                        answer_rule_id: cap.answer_rule_id().into(),
                    },
                },
                binding: binding.clone(),
                continuation: None,
                gap_count: observation.gap_count(),
                plan,
            });
            if let Some(receipt) = &history {
                ensure(
                    receipt.response.metadata == selection.request.metadata,
                    "ready permission metadata changed",
                )?;
            }
            self.validate_permission_binding(&selection, observation)?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                self.validate_live(&tx, run)?;
                self.validate_permission_association(&tx, &selection)?;
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })?;
            if self.pending.is_some() {
                return Ok(Some(InputControlReply::PermissionResponse(
                    self.permission_result(
                        &selection.request.metadata,
                        SubmissionResult::held("another programmatic input is pending"),
                    ),
                )));
            }
            ensure(
                history
                    .as_ref()
                    .is_some_and(|r| self.operations.contains_key(&r.input.input_id))
                    || self.operations.len() < RECEIPT_CACHE_CAPACITY,
                "input receipt cache is at capacity",
            )?;
            let bytes = Arc::from(selected_answer_stage(&selection.plan, 0)?.bytes());
            let digest =
                storage::permission_response_digest(self.run.run_id, &selection.request.metadata)?;
            self.pending = Some(PendingInput {
                reservation: if history.is_some() {
                    PendingReservation::HistoricalReady
                } else {
                    PendingReservation::Unreserved
                },
                request: storage::NewInput {
                    operation_key: input.operation_key.clone(),
                    kind: storage::InputKind::PermissionResponse,
                    payload_digest: digest,
                    message_id: None,
                },
                bytes,
                permission: Some(selection),
                composer: None,
                ready: history,
                deadline,
                phase: InputPhase::AwaitReservation,
                input_id: known_id,
            });
            Ok(None)
        })();
        result.map_err(|source| self.response_failure(source, input, known_id))
    }

    fn permission_result(
        &self,
        metadata: &storage::PermissionResponseMetadata,
        submission: SubmissionResult,
    ) -> PermissionResponseResult {
        PermissionResponseResult {
            submission,
            member_id: self.run.member_id,
            run_id: self.run.run_id,
            owner_epoch: metadata.expected_owner_epoch,
            interaction_id: metadata.interaction_id,
            event_id: metadata.event_id,
            decision: metadata.decision,
            decision_source: metadata.decision_source,
        }
    }

    fn run_failure(
        &self,
        source: CafleetError,
        interaction_id: Option<i64>,
        input_id: Option<i64>,
        operation_key: Option<String>,
    ) -> InputFailure {
        InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id,
            input_id,
            operation_key,
        }
    }

    fn response_failure(
        &self,
        source: CafleetError,
        input: &super::PermissionRespondInput,
        input_id: Option<i64>,
    ) -> InputFailure {
        self.run_failure(
            source,
            Some(input.interaction_id),
            input_id,
            Some(input.operation_key.clone()),
        )
    }

    fn failure(&self, source: CafleetError) -> InputFailure {
        InputFailure {
            source,
            run_id: Some(self.run.run_id),
            interaction_id: self.pending.as_ref().and_then(|pending| {
                pending
                    .permission
                    .as_ref()
                    .map(|selection| selection.request.metadata.interaction_id)
            }),
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
        let effective = bounded_by_inherited(conn, deadline.min(pending.deadline));
        check_deadline(Instant::now(), effective)?;
        Ok(effective)
    }

    fn validate_live(&self, conn: &Connection, expected: &RunRecord) -> Result<()> {
        ensure(
            !self.manual_accounting_failed && !self.input_fenced,
            "manual accounting failure fenced input",
        )?;
        self.validate_live_run(conn, expected)
    }

    fn validate_live_run(&self, conn: &Connection, expected: &RunRecord) -> Result<()> {
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

    fn receipt_guard_snapshot(
        &self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        deadline: Instant,
    ) -> Result<ReceiptGuardBinding> {
        check_deadline(Instant::now(), deadline)?;
        self.validate_receipt_owner(conn, run)?;
        let receipt = storage::get_permission_response(conn, expected.input.input_id)?;
        ensure(receipt == *expected, "permission receipt snapshot changed")?;
        self.validate_receipt_cache(&receipt.input)?;
        let zero_known = if let Some(retained) = self.operations.get(&receipt.input.input_id) {
            ensure(
                !retained.leased && !retained.attempt_available,
                "receipt requires an unleased consumed attempt",
            )?;
            retained.stage == SubmissionStage::ZeroBytes && retained.written_bytes == 0
        } else {
            receipt.input.state == storage::InputState::Ready
        };
        Ok(ReceiptGuardBinding {
            owner: self.run.owner(),
            receipt,
            zero_known,
            rearm: None,
        })
    }

    fn acquire_receipt_guard<'a>(
        &'a mut self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        deadline: Instant,
    ) -> Result<InputGuard<'a>> {
        ensure(
            !conn.is_autocommit(),
            "receipt exclusion requires its ownership transaction",
        )?;
        ensure(
            self.pending.is_none(),
            "receipt exclusion requires no active writer",
        )?;
        let deadline = bounded_by_inherited(conn, deadline);
        let binding = self.receipt_guard_snapshot(conn, run, expected, deadline)?;
        Ok(InputGuard {
            retained: None,
            proof: None,
            binding: Some(binding),
            deadline,
            permit: GuardPermit::Rejected,
            exclusion: std::marker::PhantomData,
        })
    }

    fn acquire_rearm_guard<'a>(
        &'a mut self,
        conn: &Connection,
        run: &RunRecord,
        expected: &storage::PermissionReceipt,
        selection: &Arc<PermissionSelection>,
        live: (&mut Transport, &ObservationController),
        deadline: Instant,
    ) -> Result<Option<InputGuard<'a>>> {
        let (transport, observation) = live;
        self.validate_permission_binding(selection, observation)?;
        self.validate_live(conn, run)?;
        self.validate_permission_association(conn, selection)?;
        ensure(
            self.pending.as_ref().is_some_and(|p| {
                p.phase == InputPhase::AwaitReservation
                    && p.input_id == Some(expected.input.input_id)
            }) && expected.input.state == storage::InputState::Ready
                && expected.response.metadata == selection.request.metadata,
            "exact pending ready rearm required",
        )?;
        let mut binding = self.receipt_guard_snapshot(conn, run, expected, deadline)?;
        ensure(binding.zero_known, "ready rearm requires actual zero bytes")?;
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        transport.validate_observation_drain(&proof, deadline)?;
        self.current_drain_seq(&proof, "permission manual generation changed")?;
        binding.rearm = Some(selection.clone());
        Ok(Some(InputGuard {
            retained: None,
            proof: Some(proof),
            binding: Some(binding),
            deadline,
            permit: GuardPermit::Rejected,
            exclusion: std::marker::PhantomData,
        }))
    }

    fn validate_receipt_cache(&self, input: &storage::InputOperation) -> Result<()> {
        ensure(
            input.run_id == self.run.run_id,
            "receipt for this run required",
        )?;
        if let Some(retained) = self.operations.get(&input.input_id) {
            ensure(
                retained.operation.run_id == input.run_id
                    && retained.operation.operation_key == input.operation_key
                    && retained.operation.kind == input.kind
                    && retained.operation.payload_digest == input.payload_digest
                    && retained.operation.message_id == input.message_id
                    && retained.operation.permission_response == input.permission_response,
                "receipt cache identity changed",
            )?;
        }
        Ok(())
    }

    fn receipt_projection(&self, operation: &storage::InputOperation) -> Result<SubmissionResult> {
        self.validate_receipt_cache(operation)?;
        let mut result = self.result(operation);
        if self.operations.contains_key(&operation.input_id) {
            result.stage = self.retained_checkpoint(operation.input_id)?.stage;
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
        let deadline = bounded_by_inherited(conn, deadline);
        check_deadline(Instant::now(), deadline)?;
        let mut input_id = None;
        let mut operation_key = None;
        let result = (|| match action {
            ReceiptAction::Query { operation_key: key } => {
                operation_key = Some(key.clone());
                validate_operation_key(key)?;
                with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction()?;
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
                    tx.commit()?;
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
                    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
                        let owner = self.run.owner();
                        let permission = if input.kind == storage::InputKind::PermissionResponse {
                            let receipt = storage::get_permission_response(&tx, *id)?;
                            let guard = self.acquire_receipt_guard(&tx, run, &receipt, deadline)?;
                            Some((guard, receipt.response.metadata.interaction_id))
                        } else {
                            None
                        };
                        let proof = match &permission {
                            Some((guard, interaction_id))
                                if *disposition == storage::InputDisposition::NotSubmitted
                                    && guard
                                        .binding
                                        .as_ref()
                                        .expect("receipt guard has its binding")
                                        .zero_known =>
                            {
                                Some(guard.zero_byte_retirement(&owner, *id, *interaction_id)?)
                            }
                            _ => None,
                        };
                        storage::settle_any_input_in_transaction(
                            &tx,
                            &owner,
                            &input,
                            *disposition,
                            evidence_ref,
                            proof.as_ref(),
                            deadline,
                        )?
                    };
                    check_deadline(Instant::now(), deadline)?;
                    tx.commit()?;
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
            CafleetError::from(self.run_failure(source, None, input_id, operation_key))
        })
    }

    fn retire_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        input_id: i64,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        let deadline = bounded_by_inherited(conn, deadline);
        check_deadline(Instant::now(), deadline)?;
        let mut operation_key = None;
        let result = (|| {
            ensure(input_id > 0, "positive retirement input ID required")?;
            ensure(
                stable_run_identity(run) == stable_run_identity(&self.run),
                "retirement caller run identity changed",
            )?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                let current = storage::validate_owner(&tx, &self.run.owner())?;
                ensure(
                    stable_run_identity(&current) == stable_run_identity(&self.run),
                    "retirement current run identity changed",
                )?;
                self.lock.validate(&tx, &current.owner())?;
                let input = storage::get_input(&tx, input_id)?;
                operation_key = Some(input.operation_key.clone());
                self.validate_receipt_cache(&input)?;
                ensure(
                    input.state == storage::InputState::Settled,
                    "retirement requires this run's settled input",
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
                }
                check_deadline(Instant::now(), deadline)?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })?;
            check_deadline(Instant::now(), deadline)?;
            self.operations.remove(&input_id);
            Ok(InputControlReply::Receipt(super::ReceiptReply::Retired {
                input_id,
            }))
        })();
        result.map_err(|source| {
            CafleetError::from(self.run_failure(source, None, Some(input_id), operation_key))
        })
    }

    fn composer_eligible(observation: &ObservationController) -> bool {
        let status = observation.status();
        status.eligibility == InputEligibility::ComposerEmpty
            && status.visibility == Visibility::NotVisible
            && !status.retained_transition
            && !status.retained_draft
            && !status.retained_evidence
            && status.diagnostic.is_none()
    }

    fn result(&self, operation: &storage::InputOperation) -> SubmissionResult {
        use storage::InputState;
        let (status, mut stage, reason) = super::input_client::durable_outcome(operation)
            .unwrap_or_else(|| {
                (
                    SubmissionStatus::Held,
                    SubmissionStage::ZeroBytes,
                    Some("input requires live proven-zero revalidation".into()),
                )
            });
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
                .map_or(operation.input_id, |point| point.input_id)
                .into(),
            stage,
            output_seq: retained.map(|point| point.output_seq),
            reason,
            delivery: operation.delivery(),
        }
    }

    pub(super) fn handle_received(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        if matches!(
            &request.operation,
            InputControlOperation::PermissionRespond(_)
        ) {
            return self.permission_received(conn, run, request, observation, deadline);
        }
        let mut known_input_id = None;
        let mut known_key = None;
        let result = (|| {
            check_deadline(Instant::now(), deadline)?;
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
            if input.input_kind == storage::InputKind::Preview {
                let message_id = input.message_id.expect("validated preview message");
                let message = with_database_deadline(conn, deadline, |conn| {
                    crate::broker::get_message(conn, message_id)
                })?;
                ensure(
                    message.to_member_id == self.run.member_id
                        && crate::delivery::canonical_preview_line(&message) == input.literal,
                    "preview literal must be its canonical preview",
                )?;
            }
            let retained = with_database_deadline(conn, deadline, |conn| {
                ensure(
                    storage::database_identity(conn)? == self.run.database_identity_json,
                    "input database identity changed",
                )?;
                storage::find_input(conn, self.run.run_id, &input.operation_key)
            })?;
            if let Some(retained) = retained {
                known_input_id = Some(retained.input_id);
                let digest = storage::versioned_payload_digest(
                    retained.payload_digest_version,
                    self.run.run_id,
                    &input,
                )?;
                ensure(
                    retained.kind == input.input_kind
                        && retained.payload_digest == digest
                        && retained.message_id == input.message_id,
                    "conflicting input operation key reuse",
                )?;
                return Ok(Some(InputControlReply::Submission(self.result(&retained))));
            }
            let payload_digest_version = if input.input_kind == storage::InputKind::Preview {
                2
            } else {
                1
            };
            let digest =
                storage::versioned_payload_digest(payload_digest_version, self.run.run_id, &input)?;
            request.identity.validate(&self.run)?;
            let status = observation.status();
            let defer_reason = match ordinary_blocker(&status) {
                Some(OrdinaryBlocker::ObserverUnavailable) => {
                    return Ok(Some(InputControlReply::Submission(SubmissionResult::held(
                        "observer_unavailable",
                    ))));
                }
                Some(OrdinaryBlocker::Interaction) => Some(
                    if status.eligibility == InputEligibility::Hold(super::GateReason::Candidate) {
                        "interaction_candidate"
                    } else {
                        "interaction_open"
                    },
                ),
                None if self.pending.is_some() || !self.queue.is_empty() => Some("earlier_input"),
                None => None,
            };
            let new_input = storage::NewInput {
                operation_key: input.operation_key.clone(),
                kind: input.input_kind,
                payload_digest: digest,
                message_id: input.message_id,
            };
            if let Some(defer_reason) = defer_reason {
                let reservation = with_database_deadline(conn, deadline, |conn| {
                    self.validate_live(conn, run)?;
                    storage::reserve_input_with_provenance(
                        conn,
                        &self.run.owner(),
                        &new_input,
                        storage::NewReservation {
                            selection: None,
                            defer_reason: Some(defer_reason),
                            payload_digest_version,
                        },
                    )
                })?;
                let operation = match reservation {
                    storage::InputReservation::Inserted(operation) => {
                        self.queue.push_back(QueuedInput {
                            input_id: operation.input_id,
                            request: new_input,
                            literal: input.literal,
                            operation: composer_operation,
                            payload_digest_version,
                            attempt_deadline: None,
                        });
                        operation
                    }
                    storage::InputReservation::Existing(operation) => operation,
                };
                known_input_id = Some(operation.input_id);
                return Ok(Some(InputControlReply::Submission(self.result(&operation))));
            }
            ensure(
                self.operations.len() < RECEIPT_CACHE_CAPACITY,
                "input receipt cache is at capacity",
            )?;
            with_database_deadline(conn, deadline, |conn| self.validate_live(conn, run))?;
            let (method, reason, stages) = self.select_delivery(
                observation,
                composer_operation,
                input.input_kind,
                &input.literal,
            )?;
            self.pending = Some(PendingInput {
                reservation: PendingReservation::Unreserved,
                request: new_input,
                bytes: stages[0].clone(),
                permission: None,
                composer: Some(ComposerDelivery {
                    method,
                    reason,
                    stages,
                    written_stages: 0,
                    settle_until: None,
                    manual_generation: None,
                    pane: None,
                    payload_digest_version,
                    deferred: None,
                }),
                ready: None,
                deadline,
                phase: InputPhase::AwaitReservation,
                input_id: None,
            });
            Ok(None)
        })();
        result.map_err(|source| self.run_failure(source, None, known_input_id, known_key))
    }

    /// Chooses profile delivery for a verified empty composer with an executable plan, and
    /// delivery fallback otherwise, rendering a fallback preview with its dismissal clause.
    fn select_delivery(
        &self,
        observation: &ObservationController,
        operation: ComposerOperation,
        kind: storage::InputKind,
        literal: &str,
    ) -> Result<DeliverySelection> {
        let composer = match self.run.native_version.as_deref() {
            Some(native) => {
                let backend = coding_agent(&self.run.backend)
                    .ok_or_else(|| error("unknown input backend"))?;
                self.services
                    .catalog
                    .composer_plan(backend, native, operation)
                    .map(|plan| (native, backend, plan))
            }
            None => None,
        };
        let prefix: &[u8] = match operation {
            ComposerOperation::Prompt => b"",
            ComposerOperation::Exec => b"! ",
        };
        let fallback_literal = if kind == storage::InputKind::Preview {
            crate::delivery::fallback_preview_line(literal)
        } else {
            literal.to_owned()
        };
        Ok(match composer {
            Some((native, backend, plan)) => {
                ensure(
                    plan.backend() == backend
                        && plan.native_version() == native
                        && Some(plan.profile_version()) == self.run.profile_version.as_deref()
                        && !plan.rule_ids().is_empty()
                        && !plan.fixture_ids().is_empty(),
                    "composer plan provenance changed",
                )?;
                if Self::composer_eligible(observation) {
                    let [literal, submit] = plan.stages(literal)?;
                    let stages: Arc<[Arc<[u8]>]> =
                        Arc::from(vec![Arc::from(literal), Arc::from(submit)]);
                    (storage::DeliveryMethod::Profile, None, stages)
                } else {
                    (
                        storage::DeliveryMethod::Fallback,
                        Some("composer_unverified"),
                        fallback_stages(prefix, &fallback_literal)?,
                    )
                }
            }
            None => (
                storage::DeliveryMethod::Fallback,
                Some("submission_plan_unavailable"),
                fallback_stages(prefix, &fallback_literal)?,
            ),
        })
    }

    pub(super) fn has_deferred_work(&self) -> bool {
        !self.queue.is_empty()
            || self
                .pending
                .as_ref()
                .and_then(|pending| pending.composer.as_ref())
                .is_some_and(|composer| composer.deferred.is_some())
    }

    fn publish_result(observation: &ObservationController, input_id: i64, stage: SubmissionStage) {
        observation.publish_delivery_result(storage::input_delivery_result_cause(
            input_id,
            stage.as_str(),
            None,
        ));
    }

    /// Settles queued or attempted zero-byte work as not submitted and publishes its result.
    fn cancel_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        input_id: i64,
        expected: storage::InputState,
        reason: &str,
    ) -> Result<()> {
        let finalization = Instant::now() + DEFERRED_FINALIZATION;
        let operation = with_database_deadline(conn, finalization, |conn| {
            storage::settle_zero_byte_deferred(conn, &self.run.owner(), input_id, expected, reason)
        })?;
        if let Some(retained) = self.operations.get_mut(&input_id) {
            retained.operation = operation;
            retained.attempt_available = false;
        }
        Self::publish_result(observation, input_id, SubmissionStage::ZeroBytes);
        Ok(())
    }

    /// Starts the queued head once no interaction blocks it, or settles it when its attempt
    /// budget expires or its preview was consumed.
    fn start_deferred(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        observation: &ObservationController,
    ) -> Result<()> {
        if self.input_fenced {
            return Ok(());
        }
        let Some(head) = self.queue.front_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        let blocker = ordinary_blocker(&observation.status());
        if matches!(blocker, Some(OrdinaryBlocker::Interaction)) {
            head.attempt_deadline = None;
            return Ok(());
        }
        let attempt_deadline = *head
            .attempt_deadline
            .get_or_insert(now + DEFERRED_ATTEMPT_BUDGET);
        let input_id = head.input_id;
        let message_id = head.request.message_id;
        if matches!(blocker, Some(OrdinaryBlocker::ObserverUnavailable)) {
            if now >= attempt_deadline {
                self.queue.pop_front();
                self.cancel_deferred(
                    conn,
                    observation,
                    input_id,
                    storage::InputState::Ready,
                    "observer_unavailable",
                )?;
            }
            return Ok(());
        }
        let target = with_database_deadline(conn, attempt_deadline, |conn| {
            if let Some(message_id) = message_id {
                let message = crate::broker::get_message(conn, message_id)?;
                if message.status != crate::broker::records::MessageStatus::InputRequired {
                    return Ok(Some("message_consumed"));
                }
            }
            Ok(self
                .validate_live(conn, run)
                .err()
                .map(|_| "target_inactive"))
        })?;
        if let Some(reason) = target {
            self.queue.pop_front();
            return self.cancel_deferred(
                conn,
                observation,
                input_id,
                storage::InputState::Ready,
                reason,
            );
        }
        let head = self.queue.front().expect("queued head remains");
        let (method, reason, stages) = self.select_delivery(
            observation,
            head.operation,
            head.request.kind,
            &head.literal,
        )?;
        let operation = with_database_deadline(conn, attempt_deadline, |conn| {
            storage::begin_deferred_attempt(conn, &self.run.owner(), input_id, method, reason)
        })?;
        let head = self.queue.pop_front().expect("queued head remains");
        self.operations.insert(
            input_id,
            RetainedInput {
                operation,
                bytes: stages[0].clone(),
                selection: RetainedSelection::Composer {
                    method,
                    stages: stages.clone(),
                    next_stage: 0,
                },
                stage: SubmissionStage::ZeroBytes,
                written_bytes: 0,
                output_seq: 0,
                attempt_available: false,
                leased: false,
            },
        );
        self.pending = Some(PendingInput {
            reservation: PendingReservation::Installed,
            request: head.request.clone(),
            bytes: stages[0].clone(),
            permission: None,
            composer: Some(ComposerDelivery {
                method,
                reason,
                stages,
                written_stages: 0,
                settle_until: None,
                manual_generation: None,
                pane: None,
                payload_digest_version: head.payload_digest_version,
                deferred: Some(DeferredAttempt {
                    queued: head,
                    obstacle: "delivery_budget_exhausted",
                }),
            }),
            ready: None,
            deadline: attempt_deadline,
            phase: InputPhase::AwaitBeforeWrite,
            input_id: Some(input_id),
        });
        self.checkpoint(input_id, InputBoundary::Reserved, attempt_deadline)
    }

    /// Handles a deferred attempt's interaction, expiry and obstacles before any write; `true`
    /// means the attempt ended or returned to the queue.
    fn deferred_preflight(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
    ) -> Result<bool> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(false);
        };
        let Some(composer) = pending.composer.as_ref() else {
            return Ok(false);
        };
        if composer.deferred.is_none() {
            return Ok(false);
        }
        let id = pending
            .input_id
            .expect("a deferred attempt has its receipt");
        let zero_bytes = self
            .operations
            .get(&id)
            .is_some_and(|retained| retained.stage == SubmissionStage::ZeroBytes)
            && composer.written_stages == 0;
        if !zero_bytes {
            return Ok(false);
        }
        let interaction = matches!(
            ordinary_blocker(&observation.status()),
            Some(OrdinaryBlocker::Interaction)
        );
        let expired = Instant::now() >= pending.deadline;
        if !interaction && !expired {
            return Ok(false);
        }
        let mut pending = self.pending.take().expect("deferred pending input");
        let composer = pending.composer.take().expect("deferred composer delivery");
        let deferred = composer.deferred.expect("deferred attempt");
        if interaction {
            let operation =
                with_database_deadline(conn, Instant::now() + DEFERRED_FINALIZATION, |conn| {
                    storage::record_input_state(
                        conn,
                        &self.run.owner(),
                        id,
                        storage::InputState::InProgress,
                        storage::InputState::Ready,
                    )
                })?;
            self.operations.remove(&id);
            debug_assert_eq!(operation.state, storage::InputState::Ready);
            let mut queued = deferred.queued;
            queued.attempt_deadline = None;
            self.queue.push_front(queued);
        } else {
            self.cancel_deferred(
                conn,
                observation,
                id,
                storage::InputState::InProgress,
                deferred.obstacle,
            )?;
            self.operations.remove(&id);
        }
        Ok(true)
    }

    fn finish_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        operation: &storage::InputOperation,
    ) -> Result<()> {
        let stage = self
            .operations
            .get(&operation.input_id)
            .map_or(SubmissionStage::Unknown, |retained| retained.stage);
        if operation.state == storage::InputState::Accepted
            && operation.kind == storage::InputKind::Preview
        {
            let settled =
                with_database_deadline(conn, Instant::now() + DEFERRED_FINALIZATION, |conn| {
                    storage::settle_preview(conn, &self.run.owner(), operation.input_id)
                })?;
            self.operations.remove(&settled.input_id);
        }
        Self::publish_result(observation, operation.input_id, stage);
        Ok(())
    }

    fn retained_checkpoint(&self, input_id: i64) -> Result<InputCheckpoint> {
        let retained = self
            .operations
            .get(&input_id)
            .ok_or_else(|| error("retained input operation is absent"))?;
        Ok(InputCheckpoint {
            input_id,
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

    fn drain_requirement(&self) -> DrainRequirement {
        let pending = self.pending.as_ref();
        if pending.and_then(|p| p.permission.as_ref()).is_some() {
            DrainRequirement::Observation
        } else if pending
            .and_then(|p| p.composer.as_ref())
            .is_some_and(|composer| !composer.requires_empty_composer())
        {
            DrainRequirement::NoBlockingInteraction
        } else {
            DrainRequirement::EmptyComposer
        }
    }

    fn fresh_drain(
        &mut self,
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<Option<u64>> {
        let permission = self.pending.as_ref().and_then(|p| p.permission.as_ref());
        if let Some(permission) = permission {
            self.validate_permission_binding(permission, observation)?;
        }
        let requirement = self.drain_requirement();
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        if !Self::validate_taken_drain(transport, &proof, observation, requirement, deadline)? {
            return Ok(None);
        }
        self.current_drain_seq(&proof, "input manual generation changed")
            .map(Some)
    }

    /// Reports `false` when the observed state does not yet permit the drain's requirement.
    fn validate_taken_drain(
        transport: &Transport,
        proof: &DrainedInput,
        observation: &ObservationController,
        requirement: DrainRequirement,
        deadline: Instant,
    ) -> Result<bool> {
        match requirement {
            DrainRequirement::Observation => {
                transport.validate_observation_drain(proof, deadline)?;
                Ok(true)
            }
            DrainRequirement::EmptyComposer => {
                transport.validate_input_drain(proof, deadline)?;
                Ok(Self::composer_eligible(observation))
            }
            DrainRequirement::NoBlockingInteraction => {
                transport.validate_observation_drain(proof, deadline)?;
                Ok(ordinary_blocker(&observation.status()).is_none())
            }
        }
    }

    fn current_drain_seq(&self, proof: &DrainedInput, message: &str) -> Result<u64> {
        ensure(
            proof.human_generation() == self.manual_generation && !self.manual_accounting_failed,
            message,
        )?;
        Ok(proof.output_seq())
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
                conn.busy_handler(Some(|_| false))?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
                if let Some(selection) = &operation.permission {
                    self.validate_permission_association(&tx, selection)?;
                    let receipt = storage::get_permission_response(&tx, id)?;
                    ensure(
                        receipt.input == current
                            && receipt.response.metadata == selection.request.metadata
                            && receipt.claim.as_ref().is_some_and(|claim| {
                                claim.input_id == id
                                    && claim.interaction_id == selection.binding.interaction_id
                            }),
                        "permission audit or claim changed before write",
                    )?;
                }
                check_deadline(Instant::now(), effective)?;
                tx.rollback().map_err(crate::error::CafleetError::from)
            });
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

    /// Holds the concrete pane-input lock for a composer write; `false` while another writer
    /// holds it, so the attempt writes nothing and retries.
    fn hold_composer_pane(&mut self) -> Result<bool> {
        let Some(composer) = self.pending.as_mut().and_then(|p| p.composer.as_mut()) else {
            return Ok(true);
        };
        if let Some(pane) = &composer.pane {
            pane.validate_binding(&self.database_url, &self.placement)?;
            return Ok(true);
        }
        composer.pane = PaneLock::try_acquire(&self.database_url, &self.placement)?;
        if composer.pane.is_none()
            && let Some(deferred) = composer.deferred.as_mut()
        {
            deferred.obstacle = "pane_input_busy";
        }
        Ok(composer.pane.is_some())
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
            if self
                .pending
                .as_ref()
                .is_none_or(|p| p.phase != InputPhase::Armed)
            {
                return Ok(None);
            }
            let effective = self.effective_deadline(conn, deadline)?;
            if !self.hold_composer_pane()? {
                return Ok(None);
            }
            let requirement = self.drain_requirement();
            let pending = self.pending.as_ref().expect("armed pending input");
            let Some(proof) = transport.take_input_drain() else {
                return Ok(None);
            };
            if let Some(permission) = &pending.permission {
                self.validate_permission_binding(permission, observation)?;
            }
            if !Self::validate_taken_drain(transport, &proof, observation, requirement, effective)?
            {
                return Ok(None);
            }
            if let Some(composer) = &pending.composer {
                ensure(
                    composer
                        .manual_generation
                        .is_none_or(|generation| generation == self.manual_generation),
                    "human input arrived between composer stages",
                )?;
            }
            if let Some(permission) = &pending.permission {
                let retained = self
                    .operations
                    .get(
                        &pending
                            .input_id
                            .ok_or_else(|| error("permission reservation absent"))?,
                    )
                    .ok_or_else(|| error("retained permission selection absent"))?;
                let RetainedSelection::Permission {
                    selection,
                    next_stage,
                } = &retained.selection
                else {
                    return Err(error("permission stage target changed"));
                };
                ensure(
                    Arc::ptr_eq(selection, permission),
                    "permission retained selection changed",
                )?;
                let stage = selected_answer_stage(&selection.plan, *next_stage)?;
                ensure(
                    stage.required_rule_id() == selection.binding.observed.request_rule_id
                        && stage.required_choice_id() == selection.plan.choice_id(),
                    "permission stage rule or choice changed",
                )?;
            }
            let output_seq = self.current_drain_seq(&proof, "input manual generation changed")?;
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
            check_deadline(Instant::now(), effective)?;
            Ok(Some((id, proof, effective, output_seq)))
        })()
        .map_err(|source| self.failure(source))?;
        let Some((id, proof, deadline, output_seq)) = result else {
            return Ok(None);
        };
        if let Some(composer) = self.pending.as_mut().and_then(|p| p.composer.as_mut()) {
            composer
                .manual_generation
                .get_or_insert(self.manual_generation);
        }
        let retained = self
            .operations
            .get_mut(&id)
            .expect("validated input remains retained");
        retained.output_seq = output_seq;
        retained.attempt_available = false;
        retained.leased = true;
        Ok(Some(InputGuard {
            retained: Some(retained),
            proof: Some(proof),
            binding: None,
            deadline,
            permit: GuardPermit::Fresh,
            exclusion: std::marker::PhantomData,
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
        let mut terminal_bookkeeping = false;
        let result = (|| {
            if self.pending.is_none() {
                self.start_deferred(conn, run, observation)?;
                return Ok(None);
            }
            if self.deferred_preflight(conn, observation)? {
                return Ok(None);
            }
            let deferred = self
                .pending
                .as_ref()
                .and_then(|pending| pending.composer.as_ref())
                .is_some_and(|composer| composer.deferred.is_some());
            let deadline = if deferred {
                deadline.min(self.pending.as_ref().expect("pending input").deadline)
            } else {
                deadline
            };
            let Some(pending) = self.pending.as_ref() else {
                return Ok(None);
            };
            let effective = self.effective_deadline(conn, deadline)?;
            let phase = pending.phase;
            if phase == InputPhase::Blocked {
                return Ok(None);
            }
            if phase == InputPhase::AwaitReservation {
                let ready = self
                    .pending
                    .as_ref()
                    .and_then(|p| p.ready.clone().zip(p.permission.clone()));
                let (operation, output_seq) = if let Some((ready, selection)) = ready {
                    let owner = self.run.owner();
                    let Some(guard) = with_database_deadline(conn, effective, |conn| {
                        self.acquire_rearm_guard(
                            conn,
                            run,
                            &ready,
                            &selection,
                            (transport, observation),
                            effective,
                        )
                    })?
                    else {
                        return Ok(None);
                    };
                    let output_seq = guard
                        .proof
                        .as_ref()
                        .expect("rearm guard retains its drain")
                        .output_seq();
                    let proof = guard.zero_byte_retirement(
                        &owner,
                        ready.input.input_id,
                        ready.response.metadata.interaction_id,
                    )?;
                    let rearmed = storage::rearm_permission_response(
                        conn,
                        &owner,
                        &selection.request,
                        &proof,
                        effective,
                    )?;
                    (rearmed.input, output_seq)
                } else {
                    let Some(output_seq) = self.fresh_drain(transport, observation, effective)?
                    else {
                        return Ok(None);
                    };
                    let pending = self.pending.as_ref().expect("retained pending input");
                    let operation = if let Some(selection) = &pending.permission {
                        let reservation = with_database_deadline(conn, effective, |conn| {
                            self.validate_live(conn, run)?;
                            self.validate_permission_association(conn, selection)?;
                            storage::reserve_permission_response(
                                conn,
                                &self.run.owner(),
                                &selection.request,
                                Some(selection.plan.capability()),
                                effective,
                            )
                        })?;
                        match reservation {
                            storage::PermissionReservation::Reserved(receipt) => receipt.input,
                            storage::PermissionReservation::Retained(receipt) => {
                                let reply = InputControlReply::PermissionResponse(
                                    super::input_client::projection(&self.run, receipt)?,
                                );
                                self.pending = None;
                                return Ok(Some(reply));
                            }
                        }
                    } else {
                        let composer = pending
                            .composer
                            .as_ref()
                            .ok_or_else(|| error("ordinary input has no composer delivery"))?;
                        let reservation = storage::NewReservation {
                            selection: Some((composer.method, composer.reason)),
                            defer_reason: None,
                            payload_digest_version: composer.payload_digest_version,
                        };
                        let reservation = with_database_deadline(conn, effective, |conn| {
                            self.validate_live(conn, run)?;
                            storage::reserve_input_with_provenance(
                                conn,
                                &self.run.owner(),
                                &pending.request,
                                reservation,
                            )
                        })?;
                        match reservation {
                            storage::InputReservation::Inserted(operation) => operation,
                            storage::InputReservation::Existing(operation) => {
                                let reply = InputControlReply::Submission(self.result(&operation));
                                self.pending = None;
                                return Ok(Some(reply));
                            }
                        }
                    };
                    (operation, output_seq)
                };
                ensure(
                    operation.state == storage::InputState::InProgress,
                    "ordinary input reservation is not writable",
                )?;
                let id = operation.input_id;
                let pending = self.pending.as_ref().expect("reserved pending input");
                self.operations.insert(
                    id,
                    RetainedInput {
                        operation,
                        bytes: pending.bytes.clone(),
                        selection: match (&pending.permission, &pending.composer) {
                            (Some(selection), _) => RetainedSelection::Permission {
                                selection: selection.clone(),
                                next_stage: 0,
                            },
                            (None, Some(composer)) => RetainedSelection::Composer {
                                method: composer.method,
                                stages: composer.stages.clone(),
                                next_stage: 0,
                            },
                            (None, None) => {
                                return Err(error("ordinary input has no composer delivery"));
                            }
                        },
                        stage: SubmissionStage::ZeroBytes,
                        written_bytes: 0,
                        output_seq,
                        attempt_available: false,
                        leased: false,
                    },
                );
                let pending = self.pending.as_mut().expect("reserved pending input");
                pending.input_id = Some(id);
                pending.reservation = PendingReservation::Installed;
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
                if let StageGateProgress::Waiting = self.advance_stage_gate(observation)? {
                    return Ok(None);
                }
                if let Some(composer) = self.pending.as_ref().and_then(|p| p.composer.as_ref())
                    && let Some(settle_until) = composer.settle_until
                {
                    let interrupted = composer.manual_generation != Some(self.manual_generation)
                        || self.manual_accounting_failed
                        || ordinary_blocker(&observation.status()).is_some();
                    if interrupted {
                        terminal_bookkeeping = true;
                        let deferred = composer.deferred.is_some();
                        let reply = self.stop_composer_stages(conn, run, id, effective)?;
                        if deferred {
                            Self::publish_result(observation, id, SubmissionStage::Partial);
                            return Ok(None);
                        }
                        return Ok(Some(reply));
                    }
                    if Instant::now() < settle_until {
                        return Ok(None);
                    }
                }
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
            let before_stage = guard
                .retained
                .as_ref()
                .expect("write guard retains its input")
                .stage;
            let before_count = guard
                .retained
                .as_ref()
                .expect("write guard retains its input")
                .written_bytes;
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
            let write =
                combine_manual_progress(write, after).and_then(|outcome| {
                    let point = self.retained_checkpoint(id)?;
                    match &outcome {
                        ProgrammaticWrite::Complete { bytes: count } => ensure(
                            *count == bytes.len()
                                && matches!(
                                    point.stage,
                                    SubmissionStage::Partial | SubmissionStage::Submitted
                                )
                                && point.written_bytes
                                    == before_count
                                        .checked_add(u64::try_from(*count).map_err(|_| {
                                            error("native input count exceeds range")
                                        })?)
                                        .ok_or_else(|| error("native input count exhausted"))?,
                            "complete native result conflicts with retained evidence",
                        )?,
                        ProgrammaticWrite::Partial { bytes: count } => ensure(
                            *count > 0
                                && *count < bytes.len()
                                && point.stage == SubmissionStage::Partial
                                && point.written_bytes
                                    == before_count
                                        .checked_add(u64::try_from(*count).map_err(|_| {
                                            error("native input count exceeds range")
                                        })?)
                                        .ok_or_else(|| error("native input count exhausted"))?,
                            "partial native result conflicts with retained evidence",
                        )?,
                        ProgrammaticWrite::Pending => ensure(
                            point.stage == before_stage && point.written_bytes == before_count,
                            "pending native result conflicts with retained evidence",
                        )?,
                    }
                    Ok(outcome)
                });
            if matches!(&write, Ok(ProgrammaticWrite::Complete { .. }))
                && self
                    .operations
                    .get(&id)
                    .expect("actual stage remains retained")
                    .stage
                    == SubmissionStage::Partial
            {
                let next_bytes = self
                    .operations
                    .get(&id)
                    .expect("next stage retained")
                    .bytes
                    .clone();
                let pending = self.pending.as_mut().expect("pending multistage response");
                ensure(
                    pending.permission.is_some() || pending.composer.is_some(),
                    "nonfinal stage requires a staged plan",
                )?;
                if let Some(composer) = pending.composer.as_mut() {
                    composer.written_stages += 1;
                    composer.settle_until = Some(Instant::now() + STAGE_SETTLE);
                }
                pending.bytes = next_bytes;
                pending.phase = InputPhase::AwaitBeforeWrite;
                return Ok(None);
            }
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
            terminal_bookkeeping = true;
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
                (Err(primary), Err(bookkeeping)) => Err(with_diagnostic(primary, &bookkeeping)),
            };
            self.pending.as_mut().expect("completed input").phase = InputPhase::Blocked;
            let operation = completed?;
            self.checkpoint(id, InputBoundary::TerminalState, effective)?;
            let pending = self.pending.as_ref().expect("completed input retained");
            if pending
                .composer
                .as_ref()
                .is_some_and(|composer| composer.deferred.is_some())
            {
                self.finish_deferred(conn, observation, &operation)?;
                self.pending = None;
                return Ok(None);
            }
            let reply = match &pending.permission {
                Some(selection) => InputControlReply::PermissionResponse(
                    self.permission_result(&selection.request.metadata, self.result(&operation)),
                ),
                None => InputControlReply::Submission(self.result(&operation)),
            };
            self.pending = None;
            Ok(Some(reply))
        })();
        result.map_err(|source| {
            if let Some(composer) = self.pending.as_mut().and_then(|p| p.composer.as_mut()) {
                composer.pane = None;
            }
            let source = if terminal_bookkeeping {
                source
            } else {
                self.retain_permission_failure(conn, run, deadline, source)
            };
            self.failure(source)
        })
    }

    fn stop_composer_stages(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        id: i64,
        effective: Instant,
    ) -> Result<InputControlReply> {
        let pending = self.pending.as_mut().expect("staged composer input");
        pending.phase = InputPhase::Blocked;
        if let Some(composer) = pending.composer.as_mut() {
            composer.pane = None;
        }
        let operation = with_database_deadline(conn, effective, |conn| {
            self.validate_live(conn, run)?;
            storage::record_input_state(
                conn,
                &self.run.owner(),
                id,
                storage::InputState::InProgress,
                storage::InputState::Uncertain,
            )
        })?;
        let retained = self.operations.get_mut(&id).expect("staged input retained");
        retained.operation = operation.clone();
        retained.attempt_available = false;
        self.checkpoint(id, InputBoundary::TerminalState, effective)?;
        let reply = InputControlReply::Submission(self.result(&operation));
        self.pending = None;
        Ok(reply)
    }

    fn retain_permission_failure(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        deadline: Instant,
        source: CafleetError,
    ) -> CafleetError {
        let Some(pending) = self.pending.as_mut() else {
            return source;
        };
        if pending.permission.is_none() {
            return source;
        }
        pending.phase = InputPhase::Blocked;
        let Some(id) = pending.installed_input_id() else {
            return source;
        };
        let retained = self
            .operations
            .get_mut(&id)
            .expect("pending permission receipt retained");
        assert!(!retained.leased, "failed attempt has released its guard");
        retained.attempt_available = false;
        if retained.operation.state != storage::InputState::InProgress {
            return source;
        }
        let next = if retained.stage == SubmissionStage::ZeroBytes {
            storage::InputState::Ready
        } else {
            storage::InputState::Uncertain
        };
        let Ok(effective) = self.effective_deadline(conn, deadline) else {
            return source;
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
        match persisted {
            Ok(operation) => {
                self.operations
                    .get_mut(&id)
                    .expect("failed receipt retained")
                    .operation = operation;
                source
            }
            Err(bookkeeping) => with_diagnostic(source, &bookkeeping),
        }
    }

    pub(super) fn finish_pending_attempt(&mut self) {
        if let Some(id) = self
            .pending
            .as_ref()
            .and_then(PendingInput::installed_input_id)
        {
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
            let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
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
            tx.commit()?;
            Ok(current)
        })?;
        let database_url = format!(
            "sqlite:///{}",
            conn.path()
                .ok_or_else(|| error("input coordinator database has no file path"))?
        );
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
            current_request: None,
            input_fenced: false,
            observation_blocker: None,
            database_url,
            queue: std::collections::VecDeque::new(),
        })
    }

    pub(super) fn progress_with_manual<T>(
        &mut self,
        transport: &mut Transport,
        step: impl FnOnce(&mut Transport) -> Result<T>,
    ) -> std::result::Result<T, InputFailure> {
        let before = transport.human_generation();
        let progress = step(transport);
        self.observation_blocker = transport.drain_blocker().map(str::to_owned);
        let accounting = transport
            .human_generation()
            .checked_sub(before)
            .ok_or_else(|| error("manual input generation decreased"))
            .and_then(|count| {
                usize::try_from(count).map_err(|_| error("manual input delta exceeds count range"))
            })
            .and_then(|count| self.observe_manual(count));
        if accounting.is_err() {
            self.manual_accounting_failed = true;
        }
        combine_manual_progress(progress, accounting).map_err(|source| self.failure(source))
    }

    fn observe_manual(&mut self, count: usize) -> Result<()> {
        if count > 0 {
            self.current_request = None;
        }
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
        (Err(primary), Err(accounting)) => Err(with_diagnostic(primary, &accounting)),
    }
}

fn with_diagnostic(cause: CafleetError, diagnostic: &CafleetError) -> CafleetError {
    CafleetError::WithDiagnostics {
        cause: Box::new(cause),
        diagnostics: vec![diagnostic.message().into_owned()],
    }
}

#[cfg(test)]
mod tests {
    mod manual_progress {
        use super::super::*;
        use super::ordinary_prompt::support::RunnerFixture;
        use crate::broker::runtime as storage;
        use crate::coding_agent::ProfileCatalog;
        use crate::error::CafleetError;
        use crate::runtime::runner::*;
        use std::collections::VecDeque;
        use std::io::{self, ErrorKind};
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        #[derive(Default)]
        struct Human {
            queued: VecDeque<u8>,
            consumed: Vec<u8>,
        }

        impl ByteInput for Human {
            fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                assert!(!bytes.is_empty());
                if self.queued.is_empty() {
                    return Err(ErrorKind::WouldBlock.into());
                }
                let count = bytes.len().min(self.queued.len());
                for byte in &mut bytes[..count] {
                    *byte = self.queued.pop_front().expect("queued human byte");
                }
                self.consumed.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
        }

        #[derive(Default)]
        struct Pty {
            blocked: bool,
            failure: Option<ErrorKind>,
            offered: Vec<Vec<u8>>,
            written: Vec<u8>,
            reads: usize,
        }

        impl ByteInput for Pty {
            fn try_read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                Err(ErrorKind::WouldBlock.into())
            }
        }

        impl PtyIo for Pty {
            fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                assert!(!bytes.is_empty());
                self.offered.push(bytes.to_vec());
                if let Some(kind) = self.failure {
                    return Err(io::Error::new(kind, "manual progress PTY failure"));
                }
                if self.blocked {
                    return Err(ErrorKind::WouldBlock.into());
                }
                self.written.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn resize(&mut self, _: TerminalSize) -> Result<()> {
                Ok(())
            }

            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        struct Output;

        impl Display for Output {
            fn try_write(&mut self, _: &[u8]) -> io::Result<usize> {
                panic!("the quiet child supplies no display bytes")
            }

            fn try_flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        impl RunnerFixture {
            fn coordinator(&mut self) -> InputCoordinator {
                InputCoordinator::new(
                    &mut self.fixture.conn,
                    &self.fixture.run,
                    self.lock.clone(),
                    InputServices {
                        catalog: ProfileCatalog::bundled().unwrap(),
                        token_source: Box::new(super::ordinary_prompt::support::PanicTokens(
                            "manual progress",
                        )),
                        hooks: Arc::new(InertInputHooks),
                    },
                    self.until,
                )
                .unwrap()
            }

            fn quiet_control(&mut self) {
                loop {
                    self.controller
                        .poll(
                            &self.transport.snapshot(),
                            &self.writer,
                            &RealObservationClock,
                        )
                        .unwrap();
                    let health = self.controller.health(&self.writer);
                    if !health.degraded && health.pending_count == 0 && !health.pending_evidence {
                        break;
                    }
                    assert!(Instant::now() < self.until, "writer health: {health:?}");
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(
                    self.transport
                        .quiesce_for_input(
                            &mut Human::default(),
                            &mut Pty::default(),
                            &mut Output,
                            &mut self
                                .controller
                                .observer(&self.writer, &RealObservationClock),
                            self.until,
                        )
                        .unwrap(),
                    InputDrain::Quiescent {
                        human_generation: 0,
                        output_seq: 0,
                    }
                );
            }
        }

        fn with_harness(body: impl FnOnce(&mut RunnerFixture)) {
            let mut harness = RunnerFixture::new(
                &ProfileCatalog::bundled().unwrap(),
                storage::evidence::EvidencePolicy::metadata(),
                None,
                TerminalSize { rows: 24, cols: 80 },
                TransportLimits {
                    input_bytes: 64,
                    display_bytes: 64,
                    bytes_per_tick: 64,
                },
                Instant::now() + Duration::from_secs(4),
            );
            let primary = catch_unwind(AssertUnwindSafe(|| body(&mut harness)));
            let cleanup = catch_unwind(AssertUnwindSafe(|| harness.finish()));
            if let Err(panic) = primary.and(cleanup) {
                resume_unwind(panic);
            }
        }

        #[test]
        fn manual_progress_counts_only_new_reads_across_pending_zero_and_successive_steps() {
            with_harness(|h| {
                h.quiet_control();
                let mut coordinator = h.coordinator();
                assert_eq!(coordinator.manual_generation, 0);
                let mut human = Human::default();
                human.queued.extend(b"first");
                let mut pty = Pty {
                    blocked: true,
                    ..Pty::default()
                };
                let mut calls = 0;
                let result = coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        calls += 1;
                        transport.quiesce_for_input(
                            &mut human,
                            &mut pty,
                            &mut Output,
                            &mut h.controller.observer(&h.writer, &RealObservationClock),
                            h.until,
                        )
                    })
                    .unwrap();
                assert_eq!(result, InputDrain::Pending);
                assert_eq!(calls, 1);
                assert_eq!(human.consumed, b"first");
                assert_eq!(h.transport.human_generation(), 5);
                assert_eq!(coordinator.manual_generation, 5);
                assert_eq!(h.transport.pending_input_bytes(), 5);
                assert!(pty.written.is_empty());

                let zero = coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        calls += 1;
                        transport.tick(
                            &mut human,
                            &mut pty,
                            &mut Output,
                            &mut h.controller.observer(&h.writer, &RealObservationClock),
                        )
                    })
                    .unwrap();
                assert_eq!(zero.human_bytes, 0);
                assert_eq!(calls, 2);
                assert_eq!(h.transport.human_generation(), 5);
                assert_eq!(coordinator.manual_generation, 5);
                assert_eq!(h.transport.pending_input_bytes(), 5);

                human.queued.extend(b"+next");
                pty.blocked = false;
                let next = coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        calls += 1;
                        transport.tick(
                            &mut human,
                            &mut pty,
                            &mut Output,
                            &mut h.controller.observer(&h.writer, &RealObservationClock),
                        )
                    })
                    .unwrap();
                assert_eq!(next.human_bytes, 5);
                assert_eq!(calls, 3);
                assert_eq!(human.consumed, b"first+next");
                assert_eq!(h.transport.human_generation(), 10);
                assert_eq!(coordinator.manual_generation, 10);
                assert_eq!(h.transport.pending_input_bytes(), 0);
                assert_eq!(pty.written, b"first+next");
            });
        }

        #[test]
        fn manual_progress_accounts_real_human_read_before_returning_the_pty_failure() {
            with_harness(|h| {
                h.quiet_control();
                let mut coordinator = h.coordinator();
                let mut human = Human::default();
                human.queued.extend(b"typed");
                let mut pty = Pty {
                    failure: Some(ErrorKind::BrokenPipe),
                    ..Pty::default()
                };
                let mut calls = 0;
                let result = coordinator.progress_with_manual(&mut h.transport, |transport| {
                    calls += 1;
                    transport.tick(
                        &mut human,
                        &mut pty,
                        &mut Output,
                        &mut h.controller.observer(&h.writer, &RealObservationClock),
                    )
                });
                let failure = match result {
                    Ok(_) => panic!("actual PTY failure remains the progress result"),
                    Err(failure) => failure,
                };
                assert_eq!(calls, 1);
                assert_eq!(human.consumed, b"typed");
                assert_eq!(h.transport.human_generation(), 5);
                assert_eq!(coordinator.manual_generation, 5);
                assert_eq!(h.transport.pending_input_bytes(), 5);
                assert_eq!(pty.offered, vec![b"typed".to_vec()]);
                assert!(pty.written.is_empty());
                assert_eq!(pty.reads, 0);
                assert!(matches!(&failure.source, CafleetError::App(message)
                    if message == "member runtime: manual progress PTY failure"));
                assert_eq!(failure.source.exit_code(), 1);
                assert_eq!(failure.run_id, Some(h.fixture.run.run_id));
                assert_eq!(failure.interaction_id, None);
                assert_eq!(failure.input_id, None);
                assert_eq!(failure.operation_key, None);
            });
        }

        #[test]
        fn manual_progress_composes_only_the_constructed_accounting_error_branches() {
            let only = combine_manual_progress(
                Ok(17),
                Err(CafleetError::DatabaseBusy("accounting only".into())),
            )
            .expect_err("accounting error remains visible after successful progress");
            assert!(
                matches!(&only, CafleetError::DatabaseBusy(message) if message == "accounting only")
            );
            assert_eq!(only.exit_code(), 1);
            let both = combine_manual_progress::<()>(
                Err(CafleetError::Usage("primary progress".into())),
                Err(CafleetError::DatabaseBusy("secondary accounting".into())),
            )
            .expect_err("both errors remain visible");
            assert_eq!(both.exit_code(), 2);
            match both {
                CafleetError::WithDiagnostics { cause, diagnostics } => {
                    assert!(
                        matches!(*cause, CafleetError::Usage(ref message) if message == "primary progress")
                    );
                    assert_eq!(diagnostics, vec!["secondary accounting"]);
                }
                other => {
                    panic!(
                        "dual errors retain the primary cause and accounting diagnostic: {other:?}"
                    )
                }
            }
        }
    }

    pub(in crate::runtime::runner::input) mod ordinary_prompt {
        use super::super::*;
        use crate::broker::runtime as storage;
        use crate::coding_agent::ProfileCatalog;
        use crate::error::CafleetError;
        use crate::runtime::runner::test_support::Fixture;
        use crate::runtime::runner::*;
        use std::collections::VecDeque;
        use std::io;
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use support::*;

        fn submitted(reply: InputControlReply) -> SubmissionResult {
            match reply {
                InputControlReply::Submission(value) => value,
                _ => panic!("ordinary Prompt returns its actual submission"),
            }
        }

        #[test]
        fn prompt_reservation_and_each_hook_require_a_fresh_drain_before_one_real_write() {
            with_host(|h| {
                let id = h.arm("prompt:fixture-complete");
                assert!(h.advance().is_none());
                assert!(
                    h.pty.offers.is_empty(),
                    "no stage is written without a fresh drain"
                );
                h.drain();
                let first_seq = h.transport.snapshot().screen_seq;
                let reply = loop {
                    assert!(
                        Instant::now() < h.until,
                        "the staged write completes in time"
                    );
                    if let Some(reply) = h.advance() {
                        break submitted(reply);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    h.drain();
                };
                assert!(matches!(reply.status, SubmissionStatus::Accepted));
                assert!(matches!(reply.stage, SubmissionStage::Submitted));
                assert_eq!(reply.input_id, Some(id));
                assert_eq!(
                    h.pty.stage_bytes(),
                    vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                );
                assert_eq!(h.pty.program, vec![encoded()]);
                assert!(matches!(
                    storage::get_input(&h.fixture.conn, id).unwrap().state,
                    storage::InputState::Accepted
                ));
                let retained = h.coordinator.retained_checkpoint(id).unwrap();
                assert!(matches!(retained.stage, SubmissionStage::Submitted));
                assert_eq!(
                    retained.written_bytes,
                    u64::try_from(encoded().len()).unwrap()
                );
                assert!(retained.output_seq >= first_seq);
                assert_eq!(reply.output_seq, Some(retained.output_seq));
                let repeat = h.request("prompt:fixture-complete");
                let replay = submitted(h.handle(repeat).unwrap().expect("retained key lookup"));
                assert!(matches!(replay.status, SubmissionStatus::Accepted));
                assert_eq!(replay.input_id, Some(id));
                assert_eq!(h.pty.program.len(), 1);
                assert_eq!(h.hooks.count("reserved"), 1);
                assert!(h.hooks.count("before") >= 1);
                assert!(h.hooks.count("after") >= 1);
            });
        }

        #[test]
        fn prompt_positive_short_write_retains_partial_uncertainty_without_a_second_stage() {
            with_host(|h| {
                let id = h.arm("prompt:fixture-short");
                h.pty.short = Some(3);
                h.drain();
                let reply = submitted(h.advance().expect("actual short-write outcome"));
                assert!(matches!(reply.status, SubmissionStatus::Uncertain));
                assert!(matches!(reply.stage, SubmissionStage::Partial));
                assert_eq!(reply.input_id, Some(id));
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert!(matches!(point.stage, SubmissionStage::Partial));
                assert_eq!(point.written_bytes, 3);
                assert!(matches!(
                    storage::get_input(&h.fixture.conn, id).unwrap().state,
                    storage::InputState::Uncertain
                ));
                h.drain();
                assert!(h.advance().is_none());
                assert!(h.pty.program.is_empty());
                assert_eq!(
                    h.pty.stage_bytes(),
                    vec![LITERAL.as_bytes().to_vec()],
                    "a short literal stage is never followed by its submit stage"
                );
                assert_eq!(h.hooks.count("after"), 1);
                let after = h
                    .hooks
                    .points
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.boundary == "after")
                    .unwrap()
                    .clone();
                assert_eq!(after.count, 3);
                assert_eq!(after.seq, point.output_seq);
            });
        }

        #[test]
        fn prompt_reservation_race_returns_existing_uncertainty_without_write_authority() {
            with_host(|h| {
                let key = "prompt:interleaved-reservation";
                let request = h.request(key);
                assert!(h.handle(request).unwrap().is_none());
                assert!(
                    storage::find_input(&h.fixture.conn, h.fixture.run.run_id, key)
                        .unwrap()
                        .is_none()
                );
                let new = storage::NewInput {
                    operation_key: key.into(),
                    kind: storage::InputKind::Prompt,
                    payload_digest: storage::input_payload_digest(
                        h.fixture.run.run_id,
                        &SubmitInput {
                            operation_key: key.into(),
                            input_kind: storage::InputKind::Prompt,
                            message_id: None,
                            literal: LITERAL.into(),
                        },
                    ),
                    message_id: None,
                };
                let mut independent =
                    crate::db::connect(&format!("sqlite:///{}", h.fixture.conn.path().unwrap()))
                        .unwrap();
                let original =
                    storage::reserve_input(&mut independent, &h.fixture.run.owner(), &new).unwrap();
                assert!(matches!(original.state, storage::InputState::InProgress));
                assert_eq!(
                    storage::find_input(&h.fixture.conn, h.fixture.run.run_id, key).unwrap(),
                    Some(original.clone())
                );
                let reply = submitted(
                    (0..3)
                        .find_map(|_| {
                            h.drain();
                            h.advance()
                        })
                        .expect("retained reservation resolves within three real drained turns"),
                );
                let after = storage::get_input(&independent, original.input_id).unwrap();
                assert!(
                    reply.input_id == Some(original.input_id)
                        && matches!(reply.status, SubmissionStatus::Uncertain)
                        && matches!(reply.stage, SubmissionStage::Unknown)
                        && h.pty.offers.is_empty()
                        && after == original,
                    "existing reservation retains its history without fresh write authority: status={}; input_id={:?}; stage={:?}; offers={:?}; before={original:?}; after={after:?}",
                    reply.status.as_str(),
                    reply.input_id,
                    reply.stage,
                    h.pty.offers,
                );
            });
        }

        mod guards {
            use super::*;

            fn rejected(result: Result<ProgrammaticWrite>) {
                assert!(
                    result.is_err(),
                    "genuine guarded write is refused before native I/O"
                );
            }

            fn binding(
                h: &mut Host,
                other: Option<&mut Transport>,
                wrong_bytes: bool,
                stale: bool,
            ) {
                let id = h.arm("prompt:guard-binding");
                h.drain();
                let before = storage::get_input(&h.fixture.conn, id).unwrap();
                let mut guard = h
                    .coordinator
                    .acquire_write_guard(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        &mut h.transport,
                        &h.controller,
                        h.until,
                    )
                    .unwrap()
                    .expect("actual Armed acquisition");
                if stale {
                    h.transport
                        .tick(
                            &mut h.human,
                            &mut h.pty,
                            &mut h.output,
                            &mut h.controller.observer(&h.writer, &RealObservationClock),
                        )
                        .unwrap();
                }
                let bytes = if wrong_bytes {
                    b"unselected bytes".to_vec()
                } else {
                    LITERAL.as_bytes().to_vec()
                };
                match other {
                    Some(other) => {
                        rejected(other.write_programmatic(&mut h.pty, &bytes, &mut guard, h.until))
                    }
                    None => rejected(
                        h.transport
                            .write_programmatic(&mut h.pty, &bytes, &mut guard, h.until),
                    ),
                }
                rejected(h.transport.write_programmatic(
                    &mut h.pty,
                    LITERAL.as_bytes(),
                    &mut guard,
                    h.until,
                ));
                assert!(h.pty.offers.is_empty());
                drop(guard);
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
                assert_eq!(point.written_bytes, 0);
                assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), before);
            }

            #[test]
            fn prompt_genuine_guard_binds_transport_turn_and_selected_bytes_and_consumes_rejection()
            {
                with_host(|h| {
                    with_host(|other| binding(h, Some(&mut other.transport), false, false))
                });
                with_host(|h| binding(h, None, true, false));
                with_host(|h| binding(h, None, false, true));
            }

            #[test]
            fn prompt_guard_drop_distinguishes_consumed_pre_io_from_unrecorded_possible_io() {
                for possible in [false, true] {
                    with_host(|h| {
                        let id = h.arm("prompt:guard-drop");
                        h.drain();
                        let before = storage::get_input(&h.fixture.conn, id).unwrap();
                        let hooks = h.hooks.points.lock().unwrap().len();
                        let mut guard = h
                            .coordinator
                            .acquire_write_guard(
                                &mut h.fixture.conn,
                                &h.fixture.run,
                                &mut h.transport,
                                &h.controller,
                                h.until,
                            )
                            .unwrap()
                            .expect("real reservation and fresh drain");
                        let bound = if possible {
                            h.until
                        } else {
                            Instant::now() + Duration::from_millis(30)
                        };
                        let effective = guard
                            .begin_write(&h.transport, LITERAL.as_bytes(), bound)
                            .unwrap();
                        assert!(effective <= bound);
                        if possible {
                            guard.mark_io_possible();
                        } else {
                            assert!(
                                guard
                                    .begin_write(&h.transport, LITERAL.as_bytes(), h.until)
                                    .is_err(),
                                "successful pre-I/O validation consumes the permit too"
                            );
                            std::thread::sleep(
                                effective.saturating_duration_since(Instant::now())
                                    + Duration::from_millis(1),
                            );
                        }
                        drop(guard);
                        let point = h.coordinator.retained_checkpoint(id).unwrap();
                        if possible {
                            assert!(matches!(point.stage, SubmissionStage::Unknown));
                        } else {
                            assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
                        }
                        assert_eq!(point.written_bytes, 0);
                        assert_eq!(point.output_seq, h.transport.snapshot().screen_seq);
                        assert!(h.pty.offers.is_empty());
                        assert_eq!(
                            h.hooks.points.lock().unwrap().len(),
                            hooks,
                            "drop creates no checkpoint hook"
                        );
                        assert_eq!(
                            storage::get_input(&h.fixture.conn, id).unwrap(),
                            before,
                            "durable InProgress stays unchanged"
                        );
                    });
                }
            }
        }

        mod preview_exec {
            use super::*;
            use crate::coding_agent::CodingAgent;

            fn request(
                host: &Host,
                key: String,
                kind: storage::InputKind,
                message_id: Option<i64>,
            ) -> InputControlRequest {
                let mut request = host.request(&key);
                let input = SubmitInput {
                    operation_key: key,
                    input_kind: kind,
                    message_id,
                    literal: LITERAL.into(),
                };
                request.operation = if kind == storage::InputKind::Exec {
                    InputControlOperation::SubmitExec(input)
                } else {
                    InputControlOperation::SubmitPrompt(input)
                };
                request
            }

            fn complete(host: &mut Host, request: InputControlRequest) -> SubmissionResult {
                let reply = settle(host, request);
                assert_eq!(reply.status, SubmissionStatus::Accepted);
                assert_eq!(reply.stage, SubmissionStage::Submitted);
                reply
            }

            fn refuse(host: &mut Host, request: InputControlRequest) {
                let result = host.handle(request);
                let result = match result {
                    Ok(None) => (0..3)
                        .find_map(|_| {
                            host.drain();
                            match host.coordinator.advance(
                                &mut host.fixture.conn,
                                &host.fixture.run,
                                &mut host.transport,
                                &mut host.pty,
                                &host.controller,
                                host.until,
                            ) {
                                Ok(None) => None,
                                result => Some(result),
                            }
                        })
                        .expect("ineligible operation resolves within bounded drained turns"),
                    result => result,
                };
                match result {
                    Err(_) => {}
                    Ok(Some(reply)) => assert!(matches!(
                        submitted(reply).status,
                        SubmissionStatus::Held | SubmissionStatus::Rejected
                    )),
                    Ok(None) => panic!("ineligible operation returns a refusal"),
                }
                assert!(host.pty.offers.is_empty());
                assert_eq!(
                    host.fixture
                        .conn
                        .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap(),
                    0
                );
            }

            #[test]
            fn operations_preview_binds_the_exact_run_message_key_and_durable_recipient() {
                with_host(|host| {
                    let message = crate::broker::send_message(
                        &mut host.fixture.conn,
                        host.fixture.run.director_member_id,
                        host.fixture.run.member_id,
                        LITERAL,
                    )
                    .unwrap();
                    let key = format!("preview:{}:{}", host.fixture.run.run_id, message.message_id);
                    let canonical = canonical_preview(&message);
                    let mut program = canonical.as_bytes().to_vec();
                    program.push(b'~');
                    host.pty.expect_program(program);
                    let input = preview_request(host, &message, &canonical);
                    let reply = complete(host, input);
                    let row =
                        storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
                    assert_eq!(row.kind, storage::InputKind::Preview);
                    assert_eq!(row.operation_key, key);
                    assert_eq!(row.message_id, Some(message.message_id));
                    assert_eq!(row.state, storage::InputState::Accepted);
                    assert_eq!(
                        host.pty.offers,
                        vec![canonical.as_bytes().to_vec(), b"~".to_vec()]
                    );
                    assert_eq!(
                        crate::broker::get_message(&host.fixture.conn, message.message_id).unwrap(),
                        message
                    );
                    let notified: Option<String> = host
                        .fixture
                        .conn
                        .query_row(
                            "SELECT notified_at FROM messages WHERE message_id=?1",
                            [message.message_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert!(notified.is_none());
                });

                for wrong_recipient in [false, true] {
                    with_host(|host| {
                        let run = &host.fixture.run;
                        let (sender, recipient) = if wrong_recipient {
                            (run.member_id, run.director_member_id)
                        } else {
                            (run.director_member_id, run.member_id)
                        };
                        let message = crate::broker::send_message(
                            &mut host.fixture.conn,
                            sender,
                            recipient,
                            LITERAL,
                        )
                        .unwrap();
                        let key_run = run.run_id + i64::from(!wrong_recipient);
                        let input = request(
                            host,
                            format!("preview:{key_run}:{}", message.message_id),
                            storage::InputKind::Preview,
                            Some(message.message_id),
                        );
                        refuse(host, input);
                        assert_eq!(
                            crate::broker::get_message(&host.fixture.conn, message.message_id)
                                .unwrap(),
                            message
                        );
                    });
                }
            }

            #[test]
            fn operations_exec_selects_its_native_shell_plan_and_requires_its_own_coverage() {
                let mut definition = catalog_definition();
                let mut fixture = definition["fixtures"][0].clone();
                fixture["id"] = serde_json::json!("synthetic-exec-plan");
                definition["fixtures"].as_array_mut().unwrap().push(fixture);
                definition["profiles"][0]["capabilities"]["composer"]["submission"] = serde_json::json!({
                    "prompt":null,
                    "exec":{"prefix_hex":"2120","submit_hex":"6b6c","fixture_ids":["synthetic-exec-plan"]}
                });
                let exec_catalog = ProfileCatalog::from_definitions(&definition).unwrap();
                let coverage = exec_catalog.capabilities(CodingAgent::Codex, Some(VERSION));
                assert_eq!(coverage.composer, storage::Coverage::Tested);
                assert_eq!(coverage.prompt_submission, storage::Coverage::Unknown);
                assert_eq!(coverage.native_exec_submission, storage::Coverage::Tested);
                let mut expected = b"! ".to_vec();
                expected.extend_from_slice(LITERAL.as_bytes());
                expected.extend_from_slice(b"kl");
                with_catalog_host(exec_catalog, expected.clone(), |host| {
                    let key = format!("exec:{}:{}", host.fixture.run.run_id, "b".repeat(64));
                    let input = request(host, key.clone(), storage::InputKind::Exec, None);
                    let reply = complete(host, input);
                    let row =
                        storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
                    assert_eq!(row.kind, storage::InputKind::Exec);
                    assert_eq!(row.operation_key, key);
                    assert_eq!(row.message_id, None);
                    assert_eq!(row.state, storage::InputState::Accepted);
                    let mut literal = b"! ".to_vec();
                    literal.extend_from_slice(LITERAL.as_bytes());
                    assert_eq!(host.pty.offers, vec![literal, b"kl".to_vec()]);
                    assert_eq!(
                        delivery_selection(host, row.input_id),
                        (Some("profile".into()), None)
                    );
                });
            }
        }

        mod staged_delivery {
            use super::*;
            use crate::coding_agent::CodingAgent;

            const SETTLE: Duration = Duration::from_millis(500);
            const ESCAPE: &[u8] = b"\x1b";

            fn prompt_request(host: &Host, key: &str) -> InputControlRequest {
                host.request(key)
            }

            fn exec_request(host: &Host, key: &str) -> InputControlRequest {
                let mut request = host.request(key);
                request.operation = InputControlOperation::SubmitExec(SubmitInput {
                    operation_key: key.into(),
                    input_kind: storage::InputKind::Exec,
                    message_id: None,
                    literal: LITERAL.into(),
                });
                request
            }

            fn fallback_program(prefix: &[u8]) -> Vec<u8> {
                let mut bytes = ESCAPE.to_vec();
                bytes.extend_from_slice(prefix);
                bytes.extend_from_slice(LITERAL.as_bytes());
                bytes.push(b'\r');
                bytes
            }

            fn literal_with(prefix: &[u8]) -> Vec<u8> {
                let mut bytes = prefix.to_vec();
                bytes.extend_from_slice(LITERAL.as_bytes());
                bytes
            }

            fn assert_settled_gaps(host: &Host, count: usize) {
                let gaps = host.pty.stage_gaps();
                assert_eq!(gaps.len(), count, "{gaps:?}");
                for gap in gaps {
                    assert!(gap >= SETTLE, "each stage follows a 500 ms settle: {gap:?}");
                }
            }

            pub(super) fn permission_catalog() -> ProfileCatalog {
                let mut definition =
                    crate::coding_agent::profiles::catalog::test_support::inspection_fixture_definition();
                let composer = catalog_definition();
                definition["fixtures"]
                    .as_array_mut()
                    .unwrap()
                    .push(composer["fixtures"][1].clone());
                definition["profiles"][0]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(composer["profiles"][0]["rules"][0].clone());
                definition["profiles"][0]["capabilities"]["composer"] =
                    composer["profiles"][0]["capabilities"]["composer"].clone();
                ProfileCatalog::from_definitions(&definition).unwrap()
            }

            /// Drives turns until the literal stage has been written, then returns.
            fn until_literal_written(host: &mut Host, request: InputControlRequest) {
                assert!(host.handle(request).unwrap().is_none());
                while host.pty.stages.is_empty() {
                    assert!(Instant::now() < host.until, "the literal stage is written");
                    host.drain();
                    assert!(host.advance().is_none(), "no reply before the submit stage");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }

            fn finish_after_interruption(host: &mut Host) -> SubmissionResult {
                loop {
                    assert!(Instant::now() < host.until, "the interrupted input replies");
                    host.drain();
                    if let Some(reply) = host.advance() {
                        return submitted(reply);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }

            #[test]
            fn staged_profile_delivery_writes_the_literal_then_its_submit_key_after_a_500ms_settle()
            {
                with_host(|host| {
                    let reply = settle(host, prompt_request(host, "prompt:staged"));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(reply.stage, SubmissionStage::Submitted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                    );
                    assert_settled_gaps(host, 1);
                    assert!(!host.pty.offers.iter().any(|offer| offer.contains(&0x1b)));
                    assert_eq!(
                        delivery_selection(host, reply.input_id.unwrap()),
                        (Some("profile".into()), None)
                    );
                });
            }

            #[test]
            fn staged_fallback_without_an_operation_plan_escapes_settles_types_settles_and_enters()
            {
                with_catalog_host(catalog(), fallback_program(b"! "), |host| {
                    let reply = settle(host, exec_request(host, "exec:fallback"));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![ESCAPE.to_vec(), literal_with(b"! "), b"\r".to_vec()]
                    );
                    assert_settled_gaps(host, 2);
                    assert_eq!(
                        delivery_selection(host, reply.input_id.unwrap()),
                        (
                            Some("fallback".into()),
                            Some("submission_plan_unavailable".into())
                        )
                    );
                });
            }

            #[test]
            fn staged_fallback_for_an_unverified_composer_reports_composer_unverified() {
                with_catalog_host(catalog(), fallback_program(b""), |host| {
                    host.show(b"\x1b[2J\x1b[1;1Hworking on something".to_vec());
                    let reply = settle(host, prompt_request(host, "prompt:unverified"));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![ESCAPE.to_vec(), LITERAL.as_bytes().to_vec(), b"\r".to_vec()]
                    );
                    assert_settled_gaps(host, 2);
                    assert_eq!(
                        delivery_selection(host, reply.input_id.unwrap()),
                        (Some("fallback".into()), Some("composer_unverified".into()))
                    );
                });
            }

            #[test]
            fn staged_fallback_for_unknown_coverage_uses_the_missing_plan_reason() {
                let profile = HostProfile {
                    backend: CodingAgent::Claude,
                    profile_version: None,
                    ..HostProfile::metadata()
                };
                with_profile_host(catalog(), fallback_program(b""), profile, |host| {
                    let reply = settle(host, prompt_request(host, "prompt:unknown-coverage"));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![ESCAPE.to_vec(), LITERAL.as_bytes().to_vec(), b"\r".to_vec()]
                    );
                    assert_eq!(
                        delivery_selection(host, reply.input_id.unwrap()),
                        (
                            Some("fallback".into()),
                            Some("submission_plan_unavailable".into())
                        )
                    );
                });
            }

            #[test]
            fn staged_delivery_writes_zero_bytes_including_escape_while_an_interaction_is_open() {
                with_profile_host(
                    permission_catalog(),
                    encoded(),
                    HostProfile::metadata(),
                    |host| {
                        super::super::permission_inspection_contracts::stable_form(host);
                        let deadline = Instant::now() + Duration::from_millis(1200);
                        let mut reply =
                            host.handle(prompt_request(host, "prompt:blocked")).unwrap();
                        while reply.is_none() && Instant::now() < deadline {
                            host.drain();
                            reply = host.advance();
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                        if let Some(reply) = reply {
                            let reply = submitted(reply);
                            assert_eq!(reply.stage, SubmissionStage::ZeroBytes);
                            assert!(!matches!(
                                reply.status,
                                SubmissionStatus::Accepted | SubmissionStatus::Uncertain
                            ));
                        }
                    },
                );
            }

            #[test]
            fn staged_human_input_during_the_settle_stops_the_submit_stage_with_uncertainty() {
                with_host(|host| {
                    until_literal_written(host, prompt_request(host, "prompt:human-race"));
                    host.human.bytes.extend(b"h");
                    let reply = finish_after_interruption(host);
                    assert_eq!(reply.status, SubmissionStatus::Uncertain);
                    assert_eq!(reply.stage, SubmissionStage::Partial);
                    assert_eq!(host.pty.stage_bytes(), vec![LITERAL.as_bytes().to_vec()]);
                    let id = reply.input_id.unwrap();
                    assert_eq!(
                        storage::get_input(&host.fixture.conn, id).unwrap().state,
                        storage::InputState::Uncertain
                    );
                    assert_eq!(
                        delivery_selection(host, id),
                        (Some("profile".into()), None),
                        "the selected method survives the uncertain outcome"
                    );
                });
            }

            #[test]
            fn staged_new_interaction_during_the_settle_stops_the_submit_stage_with_uncertainty() {
                with_profile_host(
                    permission_catalog(),
                    encoded(),
                    HostProfile::metadata(),
                    |host| {
                        until_literal_written(
                            host,
                            prompt_request(host, "prompt:interaction-race"),
                        );
                        super::super::permission_inspection_contracts::form(
                            host,
                            "write raced.txt",
                            "synthetic-only",
                        );
                        let reply = finish_after_interruption(host);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        assert_eq!(reply.stage, SubmissionStage::Partial);
                        assert_eq!(host.pty.stage_bytes(), vec![LITERAL.as_bytes().to_vec()]);
                        assert!(!host.pty.offers.iter().any(|offer| offer.contains(&0x1b)));
                    },
                );
            }

            fn placement(host: &Host) -> crate::broker::records::Placement {
                crate::broker::get_member(
                    &host.fixture.conn,
                    host.fixture.run.member_id,
                    host.fixture.run.fleet_id,
                )
                .unwrap()
                .unwrap()
                .placement
                .expect("wrapped member placement")
            }

            #[test]
            fn staged_runner_holds_the_pane_lock_across_the_settle_and_releases_it_after_submission()
             {
                with_host(|host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    let placement = placement(host);
                    until_literal_written(host, prompt_request(host, "prompt:locked"));
                    assert!(
                        crate::input::PaneLock::try_acquire(&url, &placement)
                            .unwrap()
                            .is_none(),
                        "the runner retains the pane lock between its stages"
                    );
                    let reply = finish_after_interruption(host);
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert!(
                        crate::input::PaneLock::acquire_released(&url, &placement)
                            .unwrap()
                            .is_some(),
                        "the runner releases the pane lock after the submit stage"
                    );
                });
            }

            #[test]
            fn staged_runner_writes_zero_bytes_while_another_writer_holds_the_pane_lock() {
                with_host(|host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    let placement = placement(host);
                    let held = crate::input::PaneLock::try_acquire(&url, &placement)
                        .unwrap()
                        .expect("the pane lock is free before submission");
                    assert!(
                        host.handle(prompt_request(host, "prompt:busy-lock"))
                            .unwrap()
                            .is_none()
                    );
                    let busy_until = Instant::now() + Duration::from_millis(300);
                    while Instant::now() < busy_until {
                        host.drain();
                        assert!(host.advance().is_none());
                        assert!(host.pty.offers.is_empty(), "no byte while the lock is busy");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    drop(held);
                    let reply = finish_after_interruption(host);
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(host.pty.program, vec![encoded()]);
                });
            }
        }

        mod deferred_queue {
            use super::staged_delivery::permission_catalog;
            use super::*;
            use crate::broker::runtime::{DeliveryMethod, DeliveryProjection, DeliveryStatus};
            use crate::input::PaneLock;

            pub(super) fn form(host: &mut Host) {
                super::super::permission_inspection_contracts::stable_form(host);
            }

            pub(super) fn close(host: &mut Host) {
                host.show(SCREEN.to_vec());
            }

            pub(super) fn with_permission_host(until: Duration, body: impl FnOnce(&mut Host)) {
                with_profile_host_until(
                    permission_catalog(),
                    encoded(),
                    HostProfile::metadata(),
                    Instant::now() + until,
                    body,
                );
            }

            pub(super) fn queued(
                host: &mut Host,
                request: InputControlRequest,
                reason: &str,
            ) -> i64 {
                let reply = submitted(
                    host.handle(request)
                        .unwrap()
                        .expect("queue acceptance replies immediately"),
                );
                assert_eq!(reply.status, SubmissionStatus::Queued);
                assert_eq!(reply.stage, SubmissionStage::ZeroBytes);
                assert_eq!(
                    reply.delivery,
                    Some(DeliveryProjection {
                        status: DeliveryStatus::Queued,
                        method: None,
                        reason: Some(reason.into()),
                    })
                );
                reply.input_id.expect("queued work has a durable input ID")
            }

            /// Drives runner turns with no further request until `done` holds.
            pub(super) fn turns_until(host: &mut Host, mut done: impl FnMut(&Host) -> bool) {
                while !done(host) {
                    assert!(Instant::now() < host.until, "queued work resolves in time");
                    host.drain();
                    host.advance();
                    std::thread::sleep(Duration::from_millis(5));
                }
            }

            pub(super) fn row(host: &Host, input_id: i64) -> storage::InputOperation {
                storage::get_input(&host.fixture.conn, input_id).unwrap()
            }

            fn settled(host: &Host, input_id: i64) -> bool {
                matches!(
                    row(host, input_id).state,
                    storage::InputState::Accepted | storage::InputState::Settled
                )
            }

            fn result_events(host: &Host, input_id: i64) -> i64 {
                host.fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2",
                        rusqlite::params![host.fixture.run.run_id, input_id],
                        |row| row.get(0),
                    )
                    .unwrap()
            }

            fn assert_not_submitted(host: &Host, input_id: i64, reason: &str) {
                let input = row(host, input_id);
                assert_eq!(input.state, storage::InputState::Settled);
                assert_eq!(
                    input.disposition,
                    Some(storage::InputDisposition::NotSubmitted)
                );
                assert_eq!(input.delivery_method, None);
                assert_eq!(input.delivery_reason.as_deref(), Some(reason));
            }

            fn no_literal_persisted(host: &Host) {
                let rows = crate::db::test_support::table_snapshot(&host.fixture.conn, |_| true);
                for table in rows {
                    for row in table.rows {
                        for value in row {
                            if let rusqlite::types::Value::Text(text) = value {
                                assert!(!text.contains(LITERAL), "{}: {text}", table.name);
                            }
                        }
                    }
                }
            }

            #[test]
            fn queue_admission_commits_a_durable_ready_receipt_and_keeps_the_literal_in_memory() {
                with_permission_host(Duration::from_secs(5), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:queued"), "interaction_open");
                    let input = row(host, id);
                    assert_eq!(input.state, storage::InputState::Ready);
                    assert!(input.deferred_at.is_some());
                    assert_eq!(input.defer_reason.as_deref(), Some("interaction_open"));
                    assert_eq!(input.delivery_method, None);
                    assert!(host.pty.offers.is_empty());
                    no_literal_persisted(host);
                });
            }

            #[test]
            fn queue_repeated_key_returns_its_queued_result_and_a_conflicting_digest_fails() {
                with_permission_host(Duration::from_secs(5), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:repeat"), "interaction_open");
                    assert_eq!(
                        queued(host, host.request("prompt:repeat"), "interaction_open"),
                        id
                    );
                    let mut conflicting = host.request("prompt:repeat");
                    let InputControlOperation::SubmitPrompt(input) = &mut conflicting.operation
                    else {
                        panic!("prompt request")
                    };
                    input.literal = "different literal".into();
                    assert!(host.handle(conflicting).is_err());
                    let count: i64 = host
                        .fixture
                        .conn
                        .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    assert_eq!(count, 1);
                });
            }

            #[test]
            fn queue_capacity_bounds_queued_plus_active_work_at_4096_with_a_zero_byte_error() {
                with_permission_host(Duration::from_secs(120), |host| {
                    form(host);
                    for index in 0..RECEIPT_CACHE_CAPACITY {
                        let reply = host.handle(host.request(&format!("prompt:capacity:{index}")));
                        assert!(
                            matches!(&reply, Ok(Some(_))),
                            "request {index} is queued within the bound"
                        );
                    }
                    assert!(
                        host.handle(host.request("prompt:capacity:overflow"))
                            .is_err(),
                        "the 4097th unresolved input fails explicitly"
                    );
                    assert!(
                        storage::find_input(
                            &host.fixture.conn,
                            host.fixture.run.run_id,
                            "prompt:capacity:overflow"
                        )
                        .unwrap()
                        .is_none()
                    );
                    assert!(host.pty.offers.is_empty());
                });
            }

            #[test]
            fn queue_drains_once_after_the_interaction_closes_without_another_request() {
                with_permission_host(Duration::from_secs(8), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:after-close"), "interaction_open");
                    close(host);
                    turns_until(host, |host| {
                        settled(host, id) && result_events(host, id) == 1
                    });
                    assert_eq!(host.pty.program, vec![encoded()]);
                    let input = row(host, id);
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Profile));
                    assert_eq!(input.defer_reason.as_deref(), Some("interaction_open"));
                    for _ in 0..20 {
                        host.drain();
                        host.advance();
                    }
                    assert_eq!(result_events(host, id), 1, "the result is published once");
                    assert_eq!(host.pty.program.len(), 1, "no replay");
                });
            }

            #[test]
            fn queue_drains_after_a_candidate_clears() {
                with_permission_host(Duration::from_secs(8), |host| {
                    host.show(
                        crate::coding_agent::profiles::catalog::test_support::inspection_fixture_form(
                            "write candidate.txt",
                            "synthetic-only",
                        ),
                    );
                    assert_eq!(
                        host.controller.health(&host.writer).eligibility,
                        InputEligibility::Hold(GateReason::Candidate)
                    );
                    let id = queued(
                        host,
                        host.request("prompt:candidate"),
                        "interaction_candidate",
                    );
                    close(host);
                    turns_until(host, |host| settled(host, id));
                    assert_eq!(host.pty.program, vec![encoded()]);
                });
            }

            #[test]
            fn queue_drains_behind_an_earlier_input_when_it_completes() {
                with_host(|host| {
                    assert!(host.handle(host.request("prompt:first")).unwrap().is_none());
                    while host.pty.stages.is_empty() {
                        assert!(Instant::now() < host.until);
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let second = queued(host, host.request("prompt:second"), "earlier_input");
                    turns_until(host, |host| settled(host, second));
                    assert_eq!(host.pty.program, vec![encoded(), encoded()]);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![
                            LITERAL.as_bytes().to_vec(),
                            b"~".to_vec(),
                            LITERAL.as_bytes().to_vec(),
                            b"~".to_vec()
                        ],
                        "the queued input never interleaves with the active one"
                    );
                });
            }

            fn placement(host: &Host) -> crate::broker::records::Placement {
                crate::broker::get_member(
                    &host.fixture.conn,
                    host.fixture.run.member_id,
                    host.fixture.run.fleet_id,
                )
                .unwrap()
                .unwrap()
                .placement
                .expect("wrapped member placement")
            }

            #[test]
            fn queue_timer_retries_a_busy_pane_lock_without_child_output_and_delivers_once_free() {
                with_permission_host(Duration::from_secs(10), |host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    form(host);
                    let id = queued(host, host.request("prompt:busy"), "interaction_open");
                    let held = PaneLock::try_acquire(&url, &placement(host))
                        .unwrap()
                        .unwrap();
                    close(host);
                    let busy_until = Instant::now() + Duration::from_secs(1);
                    while Instant::now() < busy_until {
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    assert!(host.pty.offers.is_empty());
                    assert_eq!(row(host, id).state, storage::InputState::Ready);
                    drop(held);
                    turns_until(host, |host| settled(host, id));
                    assert_eq!(host.pty.program, vec![encoded()]);
                });
            }

            #[test]
            fn queue_attempt_deadline_with_a_busy_pane_lock_settles_not_submitted_and_publishes_once()
             {
                with_permission_host(Duration::from_secs(12), |host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    form(host);
                    let id = queued(host, host.request("prompt:deadline"), "interaction_open");
                    let _held = PaneLock::try_acquire(&url, &placement(host))
                        .unwrap()
                        .unwrap();
                    close(host);
                    let started = Instant::now();
                    turns_until(host, |host| {
                        row(host, id).state == storage::InputState::Settled
                    });
                    assert!(
                        started.elapsed() >= Duration::from_secs(5),
                        "the attempt keeps its five-second budget: {:?}",
                        started.elapsed()
                    );
                    assert_not_submitted(host, id, "pane_input_busy");
                    assert_eq!(
                        row(host, id).defer_reason.as_deref(),
                        Some("interaction_open")
                    );
                    assert!(host.pty.offers.is_empty());
                    turns_until(host, |host| result_events(host, id) == 1);
                });
            }

            #[test]
            fn queued_preview_acknowledged_before_its_drain_settles_not_submitted_without_typing() {
                with_permission_host(Duration::from_secs(8), |host| {
                    let message = crate::broker::send_message(
                        &mut host.fixture.conn,
                        host.fixture.run.director_member_id,
                        host.fixture.run.member_id,
                        "queued preview body",
                    )
                    .unwrap();
                    form(host);
                    let request = preview_request(host, &message, &canonical_preview(&message));
                    let id = queued(host, request, "interaction_open");
                    crate::broker::ack_message(&mut host.fixture.conn, message.message_id).unwrap();
                    close(host);
                    turns_until(host, |host| {
                        row(host, id).state == storage::InputState::Settled
                    });
                    assert_not_submitted(host, id, "message_consumed");
                    assert!(host.pty.offers.is_empty());
                    let notified: Option<String> = host
                        .fixture
                        .conn
                        .query_row(
                            "SELECT notified_at FROM messages WHERE message_id=?1",
                            [message.message_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert!(notified.is_none());
                });
            }

            #[test]
            fn preview_receipts_use_the_versioned_canonical_digest_and_reject_another_literal() {
                with_permission_host(Duration::from_secs(5), |host| {
                    let message = crate::broker::send_message(
                        &mut host.fixture.conn,
                        host.fixture.run.director_member_id,
                        host.fixture.run.member_id,
                        "digest preview body",
                    )
                    .unwrap();
                    form(host);
                    let other = preview_request(host, &message, "not the canonical preview");
                    assert!(host.handle(other).is_err());
                    let canonical = canonical_preview(&message);
                    let id = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "interaction_open",
                    );
                    let input = row(host, id);
                    assert_eq!(input.payload_digest_version, 2);
                    let expected = crate::digest::sha256_hex(
                        crate::digest::canonical_json(&serde_json::json!({
                            "payload_digest_version":2,"run_id":host.fixture.run.run_id,
                            "kind":"preview","message_id":message.message_id,"literal":canonical
                        }))
                        .as_bytes(),
                    );
                    assert_eq!(input.payload_digest, expected);
                });
            }

            #[test]
            fn queue_orderly_shutdown_settles_queued_work_not_submitted_without_bytes() {
                with_permission_host(Duration::from_secs(5), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:shutdown"), "interaction_open");
                    host.coordinator.finish_pending_attempt();
                    host.coordinator.fence(host.until).unwrap();
                    assert_not_submitted(host, id, "runner_stopped");
                    assert!(host.pty.offers.is_empty());
                    close(host);
                    for _ in 0..10 {
                        host.drain();
                        host.advance();
                    }
                    assert!(
                        host.pty.offers.is_empty(),
                        "a stopped queue is never replayed"
                    );
                });
            }
        }

        mod method_rendering {
            use super::deferred_queue::{
                close, form, queued, row, turns_until, with_permission_host,
            };
            use super::*;
            use crate::broker::runtime::DeliveryMethod;

            const DISMISSAL: &str = "[cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.";

            fn message(host: &mut Host, body: &str) -> crate::broker::records::MessageRecord {
                crate::broker::send_message(
                    &mut host.fixture.conn,
                    host.fixture.run.director_member_id,
                    host.fixture.run.member_id,
                    body,
                )
                .unwrap()
            }

            fn fallback_preview(canonical: &str) -> Vec<u8> {
                let mut bytes = b"\x1b".to_vec();
                bytes.extend_from_slice(format!("{canonical} {DISMISSAL}").as_bytes());
                bytes.push(b'\r');
                bytes
            }

            fn preview_digest(host: &Host, message: i64, canonical: &str) -> String {
                crate::digest::sha256_hex(
                    crate::digest::canonical_json(&serde_json::json!({
                        "payload_digest_version":2,"run_id":host.fixture.run.run_id,
                        "kind":"preview","message_id":message,"literal":canonical
                    }))
                    .as_bytes(),
                )
            }

            #[test]
            fn method_rendering_fallback_preview_appends_the_dismissal_clause_outside_its_digest() {
                with_catalog_host(catalog(), Vec::new(), |host| {
                    let message = message(host, "fallback preview body");
                    let canonical = canonical_preview(&message);
                    host.pty.expect_program(fallback_preview(&canonical));
                    host.show(b"\x1b[2J\x1b[1;1Hworking on something".to_vec());
                    let reply = settle(host, preview_request(host, &message, &canonical));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![
                            b"\x1b".to_vec(),
                            format!("{canonical} {DISMISSAL}").into_bytes(),
                            b"\r".to_vec()
                        ]
                    );
                    let input =
                        storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Fallback));
                    assert_eq!(input.payload_digest_version, 2);
                    assert_eq!(
                        input.payload_digest,
                        preview_digest(host, message.message_id, &canonical)
                    );
                });
            }

            #[test]
            fn method_rendering_queued_fallback_keeps_one_input_id_across_composer_changes_and_retries()
             {
                with_permission_host(Duration::from_secs(8), |host| {
                    let message = message(host, "queued fallback body");
                    let canonical = canonical_preview(&message);
                    host.pty.expect_program(fallback_preview(&canonical));
                    form(host);
                    let id = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "interaction_open",
                    );
                    let digest = row(host, id).payload_digest;
                    close(host);
                    while host
                        .fixture
                        .conn
                        .query_row(
                            "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1 AND closed_at IS NULL",
                            [host.fixture.run.run_id],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap()
                        > 0
                    {
                        assert!(Instant::now() < host.until, "the interaction closes");
                        host.drain();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    host.show(b"\x1b[2J\x1b[1;1Hworking on something".to_vec());
                    let repeated = submitted(
                        host.handle(preview_request(host, &message, &canonical))
                            .unwrap()
                            .expect("a repeated preview key replies from its receipt"),
                    );
                    assert_eq!(repeated.input_id, Some(id));
                    turns_until(host, |host| {
                        matches!(
                            row(host, id).state,
                            storage::InputState::Accepted | storage::InputState::Settled
                        )
                    });
                    close(host);
                    let after = submitted(
                        host.handle(preview_request(host, &message, &canonical))
                            .unwrap()
                            .expect("a submitted preview key replies without typing"),
                    );
                    assert_eq!(after.input_id, Some(id));
                    assert_eq!(after.status, SubmissionStatus::Accepted);
                    for _ in 0..20 {
                        host.drain();
                        host.advance();
                    }
                    assert_eq!(host.pty.program, vec![fallback_preview(&canonical)]);
                    let input = row(host, id);
                    assert_eq!(
                        input.payload_digest, digest,
                        "the digest ignores the method"
                    );
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Fallback));
                    assert_eq!(
                        input.delivery_reason.as_deref(),
                        Some("composer_unverified")
                    );
                    let rows: i64 = host
                        .fixture
                        .conn
                        .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    assert_eq!(rows, 1);
                });
            }
        }

        mod sql {
            use super::*;
            use crate::input::{Deadline, with_database_budget};
            use rusqlite::Connection;
            use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

            struct SqlObservation {
                begins: AtomicUsize,
                run_read: AtomicBool,
                delay: Option<Instant>,
                entered: AtomicBool,
            }
            fn observed_sql<T>(
                conn: &mut Connection,
                state: Arc<SqlObservation>,
                body: impl FnOnce(&mut Connection) -> T,
            ) -> T {
                let authorization = state.clone();
                conn.authorizer(Some(move |context: AuthContext<'_>| {
                    match context.action {
                        AuthAction::Transaction {
                            operation: TransactionOperation::Begin,
                        } => {
                            authorization.begins.fetch_add(1, Ordering::SeqCst);
                        }
                        AuthAction::Read {
                            table_name: "member_runs",
                            ..
                        } => {
                            authorization.run_read.store(true, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                    Authorization::Allow
                }))
                .unwrap();
                conn.progress_handler(
                    1,
                    Some(move || {
                        if state.run_read.load(Ordering::SeqCst)
                            && !state.entered.swap(true, Ordering::SeqCst)
                            && let Some(until) = state.delay
                        {
                            std::thread::sleep(
                                until.saturating_duration_since(Instant::now())
                                    + Duration::from_millis(1),
                            );
                        }
                        false
                    }),
                )
                .expect("install supplied-connection SQL progress observer");
                let result = catch_unwind(AssertUnwindSafe(|| body(conn)));
                let progress = conn.progress_handler(0, None::<fn() -> bool>);
                let authorization = conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
                match result {
                    Ok(result) => {
                        assert!(
                            progress.is_ok() && authorization.is_ok(),
                            "SQL observer cleanup failed: progress={progress:?}; authorizer={authorization:?}"
                        );
                        result
                    }
                    Err(primary) => resume_unwind(primary),
                }
            }
            fn sql_state(delay: Option<Instant>) -> Arc<SqlObservation> {
                Arc::new(SqlObservation {
                    begins: AtomicUsize::new(0),
                    run_read: AtomicBool::new(false),
                    delay,
                    entered: AtomicBool::new(false),
                })
            }
            fn timeout(conn: &Connection) -> i64 {
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                    .unwrap()
            }

            #[test]
            fn prompt_final_sql_is_nonwaiting_on_the_supplied_connection_and_fresh_drain_progresses()
             {
                with_host(|h| {
                    let id = h.arm("prompt:contended");
                    h.drain();
                    h.fixture
                        .conn
                        .busy_timeout(Duration::from_millis(600))
                        .unwrap();
                    let blocker = crate::db::connect(&format!(
                        "sqlite:///{}",
                        h.fixture.conn.path().unwrap()
                    ))
                    .unwrap();
                    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
                    let state = sql_state(None);
                    let run = h.fixture.run.clone();
                    let until = h.until;
                    let original_address = &h.fixture.conn as *const Connection as usize;
                    let coordinator = &mut h.coordinator;
                    let transport = &mut h.transport;
                    let pty = &mut h.pty;
                    let observation = &h.controller;
                    let started = Instant::now();
                    let pending = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
                        with_database_budget(conn, Deadline::until(until), |conn| {
                            assert_eq!(conn as *const Connection as usize, original_address);
                            let result = coordinator
                                .advance(conn, &run, transport, pty, observation, until)
                                .map_err(|f| f.source)?;
                            assert!(conn.is_autocommit());
                            Ok(result)
                        })
                    })
                    .unwrap();
                    let elapsed = started.elapsed();
                    assert!(pending.is_none());
                    assert!(
                        elapsed < Duration::from_millis(200),
                        "final validation waits neither its600ms connection timeout nor the original request budget: {elapsed:?}"
                    );
                    assert!(
                        state.begins.load(Ordering::SeqCst) > 0,
                        "the supplied connection attempts its actual final transaction"
                    );
                    assert!(h.fixture.conn.is_autocommit());
                    assert_eq!(timeout(&h.fixture.conn), 600);
                    assert!(h.pty.program.is_empty());
                    assert!(matches!(
                        storage::get_input(&h.fixture.conn, id).unwrap().state,
                        storage::InputState::InProgress
                    ));
                    assert_eq!(h.hooks.count("before"), 1);

                    h.human.bytes.extend(b"human");
                    h.pty.child.bytes.extend(SCREEN);
                    let previous_seq = h.transport.snapshot().screen_seq;
                    h.drain();
                    assert_eq!(h.human.consumed, 5);
                    assert_eq!(h.coordinator.manual_generation, 5);
                    assert_eq!(h.pty.manual, b"human");
                    assert!(h.transport.snapshot().screen_seq > previous_seq);
                    assert!(h.advance().is_none());
                    assert!(h.pty.program.is_empty());
                    blocker.execute_batch("ROLLBACK").unwrap();
                    assert!(
                        h.advance().is_none(),
                        "contention consumes the preceding drain"
                    );
                    let result = loop {
                        assert!(
                            Instant::now() < h.until,
                            "the released write completes in time"
                        );
                        h.drain();
                        if let Some(reply) = h.advance() {
                            break submitted(reply);
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    };
                    assert!(matches!(result.status, SubmissionStatus::Accepted));
                    assert_eq!(result.input_id, Some(id));
                    assert_eq!(
                        h.pty.stage_bytes(),
                        vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                    );
                    assert_eq!(h.pty.program, vec![encoded()]);
                    assert!(h.hooks.count("before") >= 1);
                    assert_eq!(timeout(&h.fixture.conn), 600);
                });
            }

            #[test]
            fn prompt_inherited_expiry_refuses_before_sql_and_after_uncontended_sql_on_the_actual_connection()
             {
                for during in [false, true] {
                    with_host(|h| {
                        let id = h.arm("prompt:sql-expiry");
                        h.drain();
                        h.fixture
                            .conn
                            .busy_timeout(Duration::from_millis(123))
                            .unwrap();
                        let outer = Instant::now() + Duration::from_millis(50);
                        let state = sql_state(during.then_some(outer));
                        let run = h.fixture.run.clone();
                        let until = h.until;
                        let coordinator = &mut h.coordinator;
                        let transport = &mut h.transport;
                        let observation = &h.controller;
                        let original_address = &h.fixture.conn as *const Connection as usize;
                        let result = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
                            with_database_budget(conn, Deadline::until(outer), |conn| {
                                assert_eq!(conn as *const Connection as usize, original_address);
                                if !during {
                                    std::thread::sleep(
                                        outer.saturating_duration_since(Instant::now())
                                            + Duration::from_millis(1),
                                    );
                                }
                                match coordinator.acquire_write_guard(
                                    conn,
                                    &run,
                                    transport,
                                    observation,
                                    until,
                                ) {
                                    Err(failure) => {
                                        assert_eq!(failure.run_id, Some(run.run_id));
                                        assert_eq!(failure.input_id, Some(id));
                                        Err(failure.source)
                                    }
                                    Ok(Some(guard)) => {
                                        drop(guard);
                                        Ok(true)
                                    }
                                    Ok(None) => Ok(false),
                                }
                            })
                        });
                        assert!(matches!(result, Err(CafleetError::DeadlineExceeded)));
                        assert_eq!(
                            state.entered.load(Ordering::SeqCst),
                            during,
                            "actual member-run SQL entry is control-separated"
                        );
                        assert!(h.fixture.conn.is_autocommit());
                        assert_eq!(timeout(&h.fixture.conn), 123);
                        assert!(h.pty.offers.is_empty());
                        assert!(matches!(
                            storage::get_input(&h.fixture.conn, id).unwrap().state,
                            storage::InputState::InProgress
                        ));
                    });
                }
            }

            #[test]
            fn prompt_guard_keeps_the_earliest_inherited_expiration_after_sql_scope_restoration() {
                with_host(|h| {
                    let id = h.arm("prompt:guard-sql-bound");
                    h.drain();
                    let before = storage::get_input(&h.fixture.conn, id).unwrap();
                    h.fixture
                        .conn
                        .busy_timeout(Duration::from_millis(321))
                        .unwrap();
                    let outer = Instant::now() + Duration::from_millis(80);
                    let run = h.fixture.run.clone();
                    let until = h.until;
                    let coordinator = &mut h.coordinator;
                    let transport = &mut h.transport;
                    let observation = &h.controller;
                    let mut guard =
                        with_database_budget(&mut h.fixture.conn, Deadline::until(outer), |conn| {
                            coordinator
                                .acquire_write_guard(conn, &run, transport, observation, until)
                                .map_err(|failure| failure.source)?
                                .ok_or_else(|| {
                                    CafleetError::App(
                                        "healthy Armed operation yields a real guard".into(),
                                    )
                                })
                        })
                        .unwrap();
                    assert_eq!(timeout(&h.fixture.conn), 321);
                    std::thread::sleep(
                        outer.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                    );
                    assert!(Instant::now() < until);
                    let result =
                        h.transport
                            .write_programmatic(&mut h.pty, &encoded(), &mut guard, until);
                    assert!(matches!(result, Err(CafleetError::DeadlineExceeded)));
                    assert!(h.pty.offers.is_empty());
                    drop(guard);
                    let point = h.coordinator.retained_checkpoint(id).unwrap();
                    assert!(matches!(point.stage, SubmissionStage::ZeroBytes));
                    assert_eq!(point.written_bytes, 0);
                    assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), before);
                    assert!(h.fixture.conn.is_autocommit());
                    assert_eq!(timeout(&h.fixture.conn), 321);
                });
            }
        }

        pub(in crate::runtime::runner::input) mod support {
            use super::*;
            use crate::runtime::system::test_support::TestChild;
            use std::process::{Command, Stdio};
            use std::sync::Mutex;

            pub(super) const LITERAL: &str = "ordinary fixture λ";
            pub(super) const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
            pub(super) const VERSION: &str = "0.0.1-synthetic";
            const PROFILE: &str = "synthetic-profile-1";

            pub(in crate::runtime::runner::input) fn catalog_definition() -> serde_json::Value {
                let mut value =
                    crate::coding_agent::profiles::catalog::test_support::synthetic_definitions();
                let fixture = value["fixtures"][0].clone();
                let mut executable = fixture.clone();
                executable["id"] = serde_json::json!("synthetic-prompt-plan");
                value["fixtures"].as_array_mut().unwrap().push(executable);
                let rule = &mut value["profiles"][0]["rules"][0];
                rule["id"] = serde_json::json!("synthetic-composer-empty");
                rule["observation"] = serde_json::json!("composer_empty");
                rule["region"]["start_pattern"] = serde_json::json!("^Synthetic state$");
                rule["region"]["end_pattern"] = serde_json::json!("^State end$");
                rule["matchers"] = serde_json::json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
                rule["extract"] =
                    serde_json::json!({"operation":null,"choices":[],"native_request_id":null});
                rule["verified_noninteraction"] = serde_json::json!(true);
                value["profiles"][0]["capabilities"] = serde_json::json!({
                    "detection":null,"annotations":null,"answers":[],"composer":{
                        "rule_ids":["synthetic-composer-empty"],"fixture_ids":[fixture["id"]],
                        "submission":{"prompt":{"prefix_hex":"","submit_hex":"7e","fixture_ids":["synthetic-prompt-plan"]},"exec":null}
                    }
                });
                value
            }

            pub(super) fn catalog() -> ProfileCatalog {
                ProfileCatalog::from_definitions(&catalog_definition()).unwrap()
            }

            pub(super) fn encoded() -> Vec<u8> {
                let mut bytes = LITERAL.as_bytes().to_vec();
                bytes.push(b'~');
                bytes
            }

            /// Submits `request`, then drives drained turns until its first reply.
            pub(in crate::runtime::runner::input) fn settle(
                host: &mut Host,
                request: InputControlRequest,
            ) -> SubmissionResult {
                if let Some(reply) = host.handle(request).unwrap() {
                    return submitted(reply);
                }
                loop {
                    assert!(
                        Instant::now() < host.until,
                        "the submission replies in time"
                    );
                    host.drain();
                    if let Some(reply) = host.advance() {
                        return submitted(reply);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }

            /// The method-independent wrapped preview the runner reconstructs from the broker
            /// message: the visible line ending at the resume clause.
            pub(in crate::runtime::runner::input) fn canonical_preview(
                message: &crate::broker::records::MessageRecord,
            ) -> String {
                format!(
                    "[cafleet msg {} from {} {}] {} [cafleet] Resume your work if something was still running.",
                    message.message_id, message.from_member_id, message.created_at, message.text
                )
            }

            pub(in crate::runtime::runner::input) fn preview_request(
                host: &Host,
                message: &crate::broker::records::MessageRecord,
                literal: &str,
            ) -> InputControlRequest {
                let key = format!("preview:{}:{}", host.fixture.run.run_id, message.message_id);
                let mut request = host.request(&key);
                request.operation = InputControlOperation::SubmitPrompt(SubmitInput {
                    operation_key: key,
                    input_kind: storage::InputKind::Preview,
                    message_id: Some(message.message_id),
                    literal: literal.into(),
                });
                request
            }

            pub(in crate::runtime::runner::input) fn delivery_selection(
                host: &Host,
                input_id: i64,
            ) -> (Option<String>, Option<String>) {
                host.fixture
                    .conn
                    .query_row(
                        "SELECT delivery_method,delivery_reason FROM member_input_operations WHERE input_id=?1",
                        [input_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap()
            }

            pub(in crate::runtime::runner::input) struct HostProfile {
                pub(crate) policy: storage::evidence::EvidencePolicy,
                pub(crate) event_evidence: &'static str,
                pub(crate) backend: crate::coding_agent::CodingAgent,
                pub(crate) native_version: &'static str,
                pub(crate) profile_version: Option<&'static str>,
                pub(crate) size: TerminalSize,
                pub(crate) screen: Vec<u8>,
            }
            impl HostProfile {
                pub(crate) fn synthetic(
                    policy: storage::evidence::EvidencePolicy,
                    event_evidence: &'static str,
                ) -> Self {
                    Self {
                        policy,
                        event_evidence,
                        backend: crate::coding_agent::CodingAgent::Codex,
                        native_version: VERSION,
                        profile_version: Some(PROFILE),
                        size: TerminalSize { rows: 24, cols: 80 },
                        screen: SCREEN.to_vec(),
                    }
                }

                pub(crate) fn metadata() -> Self {
                    Self::synthetic(storage::evidence::EvidencePolicy::metadata(), "metadata")
                }
            }

            pub(in crate::runtime::runner::input) struct PanicTokens(pub(crate) &'static str);
            impl RequestTokenSource for PanicTokens {
                fn fill(&mut self, _: &mut [u8; 32]) -> Result<()> {
                    panic!("{} creates no request token", self.0)
                }
            }

            #[derive(Clone, Debug)]
            pub(in crate::runtime::runner::input) struct Point {
                pub(crate) boundary: &'static str,
                pub(crate) id: Option<i64>,
                pub(crate) count: u64,
                pub(crate) seq: u64,
            }

            pub(in crate::runtime::runner::input) struct Hooks {
                pub(crate) points: Mutex<Vec<Point>>,
                pub(crate) until: Instant,
            }
            impl Hooks {
                pub(crate) fn count(&self, boundary: &str) -> usize {
                    self.points
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|p| p.boundary == boundary)
                        .count()
                }
                pub(crate) fn input_id(&self) -> i64 {
                    self.points
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|p| p.boundary == "reserved")
                        .expect("real Reserved checkpoint")
                        .id
                        .expect("actual durable input ID")
                }
            }
            impl InputHooks for Hooks {
                fn checkpoint(
                    &self,
                    boundary: InputBoundary,
                    state: &InputCheckpoint,
                    deadline: Instant,
                ) -> Result<()> {
                    assert!(deadline <= self.until);
                    let boundary = match boundary {
                        InputBoundary::Reserved => "reserved",
                        InputBoundary::BeforeWrite => "before",
                        InputBoundary::AfterWrite => "after",
                        InputBoundary::TerminalState => "terminal",
                    };
                    self.points.lock().unwrap().push(Point {
                        boundary,
                        id: Some(state.input_id),
                        count: state.written_bytes,
                        seq: state.output_seq,
                    });
                    Ok(())
                }
            }

            #[derive(Default)]
            pub(in crate::runtime::runner::input) struct Reader {
                pub(crate) bytes: VecDeque<u8>,
                pub(crate) consumed: usize,
                pub(crate) calls: usize,
            }
            impl ByteInput for Reader {
                fn try_read(&mut self, target: &mut [u8]) -> io::Result<usize> {
                    self.calls += 1;
                    if self.bytes.is_empty() {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    let count = target.len().min(self.bytes.len());
                    assert!(count > 0);
                    for byte in &mut target[..count] {
                        *byte = self.bytes.pop_front().unwrap();
                    }
                    self.consumed += count;
                    Ok(count)
                }
            }

            /// `program` holds each completed programmatic input; `stages` holds every
            /// separately offered programmatic stage with its offer time, so settle intervals
            /// between a literal and its submit key are observable.
            pub(in crate::runtime::runner::input) struct Pty {
                pub(crate) child: Reader,
                pub(crate) offers: Vec<Vec<u8>>,
                pub(crate) program: Vec<Vec<u8>>,
                pub(crate) stages: Vec<(Instant, Vec<u8>)>,
                pub(crate) manual: Vec<u8>,
                pub(crate) short: Option<usize>,
                expected_program: Vec<u8>,
                program_offset: usize,
                database_url: String,
                hooks: Arc<Hooks>,
            }
            impl Pty {
                pub(crate) fn expect_program(&mut self, bytes: Vec<u8>) {
                    assert_eq!(self.program_offset, 0, "no partially written program");
                    self.expected_program = bytes;
                }
                pub(crate) fn stage_bytes(&self) -> Vec<Vec<u8>> {
                    self.stages.iter().map(|(_, bytes)| bytes.clone()).collect()
                }
                pub(crate) fn stage_gaps(&self) -> Vec<Duration> {
                    self.stages
                        .windows(2)
                        .map(|pair| pair[1].0.duration_since(pair[0].0))
                        .collect()
                }
            }
            impl ByteInput for Pty {
                fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                    self.child.try_read(bytes)
                }
            }
            impl PtyIo for Pty {
                fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    assert!(!bytes.is_empty());
                    self.offers.push(bytes.to_vec());
                    if self.expected_program[self.program_offset..].starts_with(bytes) {
                        let conn = crate::db::connect(&self.database_url).unwrap();
                        let row = storage::get_input(&conn, self.hooks.input_id()).unwrap();
                        assert!(matches!(row.state, storage::InputState::InProgress));
                        assert!(self.hooks.count("before") > 0);
                        self.stages.push((Instant::now(), bytes.to_vec()));
                        let accepted = self.short.unwrap_or(bytes.len());
                        self.program_offset += accepted;
                        if self.program_offset == self.expected_program.len() {
                            self.program.push(self.expected_program.clone());
                            self.program_offset = 0;
                        }
                        Ok(accepted)
                    } else {
                        self.manual.extend_from_slice(bytes);
                        Ok(bytes.len())
                    }
                }
                fn resize(&mut self, _: TerminalSize) -> Result<()> {
                    Ok(())
                }
                fn close(&mut self) -> Result<()> {
                    Ok(())
                }
            }
            #[derive(Default)]
            pub(in crate::runtime::runner::input) struct Output(pub(crate) Vec<u8>);
            impl Display for Output {
                fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                    self.0.extend_from_slice(bytes);
                    Ok(bytes.len())
                }
                fn try_flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }

            pub(in crate::runtime::runner::input) struct Child {
                native: TestChild,
                until: Instant,
            }
            impl Drop for Child {
                fn drop(&mut self) {
                    self.native.terminate_until(self.until);
                }
            }

            pub(in crate::runtime::runner::input) struct RunSelection {
                pub(crate) backend: crate::coding_agent::CodingAgent,
                pub(crate) native_version: &'static str,
                pub(crate) profile_version: Option<&'static str>,
                pub(crate) event_evidence: &'static str,
            }

            /// A run owned by this process as its runner, with a started writer, observation
            /// controller and transport over a live `/bin/cat` native child.
            pub(in crate::runtime::runner::input) struct RunnerFixture {
                pub(crate) fixture: Fixture,
                pub(crate) artifacts: RunArtifacts,
                pub(crate) lock: Arc<RunLock>,
                pub(crate) writer: EventWriter,
                pub(crate) controller: ObservationController,
                pub(crate) transport: Transport,
                pub(crate) child: Child,
                pub(crate) until: Instant,
            }

            impl RunnerFixture {
                pub(in crate::runtime::runner::input) fn new(
                    catalog: &ProfileCatalog,
                    policy: storage::evidence::EvidencePolicy,
                    selection: Option<RunSelection>,
                    size: TerminalSize,
                    limits: TransportLimits,
                    until: Instant,
                ) -> Self {
                    let mut fixture = Fixture::new();
                    let artifacts = fixture.artifacts();
                    let lock = Arc::new(
                        RunLock::acquire(&fixture.conn, &fixture.run.owner())
                            .unwrap()
                            .expect("original run lock"),
                    );
                    lock.transfer_to_runner(
                        &mut fixture.conn,
                        &fixture.run.owner(),
                        ProcessIdentity::current().unwrap(),
                    )
                    .unwrap();
                    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                    storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner())
                        .unwrap();
                    if let Some(selection) = selection {
                        let selected =
                            catalog.capabilities(selection.backend, Some(selection.native_version));
                        let coverage = selected.detection;
                        fixture.conn.execute("UPDATE member_runs SET backend=?1,native_version=?2,profile_version=?3,coverage=?4,event_evidence=?5 WHERE run_id=?6",rusqlite::params![selection.backend.name(),selection.native_version,selection.profile_version,coverage.as_str(),selection.event_evidence,fixture.run.run_id]).unwrap();
                        fixture
                            .conn
                            .execute(
                                "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                                rusqlite::params![selection.backend.name(), fixture.run.member_id],
                            )
                            .unwrap();
                    }
                    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                    let mut controller =
                        ObservationController::new(&fixture.run, catalog, policy, 1..=256).unwrap();
                    let mut writer = EventWriter::start(
                        format!("sqlite:///{}", fixture.conn.path().unwrap()),
                        fixture.run.clone(),
                        lock.clone(),
                        artifacts.paths.outbox.clone(),
                    )
                    .unwrap();
                    writer.set_deadline(until);
                    let child = Child {
                        native: TestChild::new(
                            Command::new("/bin/cat")
                                .stdin(Stdio::piped())
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .spawn()
                                .unwrap(),
                        ),
                        until,
                    };
                    let identity = ProcessIdentity::read(i64::from(child.native.id()))
                        .unwrap()
                        .expect("owned live child");
                    assert!(identity.matches().unwrap());
                    fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![identity.pid,identity.start_identity,crate::time::now_stamp(),fixture.run.run_id]).unwrap();
                    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                    assert_eq!(fixture.run.status, storage::RunStatus::Running);
                    let transport = Transport::new(size, limits).unwrap();
                    controller
                        .started(
                            &fixture.run,
                            &transport.snapshot(),
                            &writer,
                            &RealObservationClock,
                        )
                        .unwrap();
                    Self {
                        fixture,
                        artifacts,
                        lock,
                        writer,
                        controller,
                        transport,
                        child,
                        until,
                    }
                }

                pub(crate) fn finish(&mut self) {
                    finish_runner(&mut self.writer, &mut self.child, self.until);
                }
            }

            fn finish_runner(writer: &mut EventWriter, child: &mut Child, until: Instant) {
                let counts = writer.finish(until);
                let status = writer.status();
                child.native.terminate_until(until);
                let reaped = child.native.try_wait();
                assert_eq!(counts, (0, 0), "writer cleanup: {status:?}");
                assert!(
                    !status.degraded && status.diagnostic.is_none(),
                    "writer finish confirms join or reports degradation: {status:?}"
                );
                assert!(reaped.unwrap().is_some(), "owned native child was reaped");
            }

            pub(in crate::runtime::runner::input) struct Host {
                pub(crate) fixture: Fixture,
                _artifacts: RunArtifacts,
                pub(crate) coordinator: InputCoordinator,
                pub(crate) writer: EventWriter,
                pub(crate) controller: ObservationController,
                pub(crate) transport: Transport,
                pub(crate) human: Reader,
                pub(crate) pty: Pty,
                pub(crate) output: Output,
                pub(crate) hooks: Arc<Hooks>,
                pub(crate) until: Instant,
                expires: u64,
                child: Child,
            }

            impl Host {
                pub(in crate::runtime::runner::input) fn pending_snapshot(
                    &self,
                ) -> (String, String, Vec<u8>, Instant, InputPhase, Option<i64>) {
                    let pending = self
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

                pub(crate) fn new() -> Self {
                    Self::until(Instant::now() + Duration::from_secs(5))
                }
                fn until(until: Instant) -> Self {
                    Self::build(catalog, encoded(), HostProfile::metadata(), until)
                }
                fn with_catalog(
                    catalog: ProfileCatalog,
                    expected_program: Vec<u8>,
                    profile: HostProfile,
                ) -> Self {
                    Self::build(
                        || catalog,
                        expected_program,
                        profile,
                        Instant::now() + Duration::from_secs(5),
                    )
                }
                fn build(
                    catalog: impl FnOnce() -> ProfileCatalog,
                    expected_program: Vec<u8>,
                    profile: HostProfile,
                    until: Instant,
                ) -> Self {
                    assert!(
                        Instant::now() < until,
                        "original fixture construction deadline"
                    );
                    let expires = InputExpiration::capture(until, &mut RealInputDeadlineClock)
                        .unwrap()
                        .monotonic_ns();
                    let HostProfile {
                        policy,
                        event_evidence,
                        backend,
                        native_version,
                        profile_version,
                        size,
                        screen,
                    } = profile;
                    let catalog = catalog();
                    let RunnerFixture {
                        mut fixture,
                        artifacts,
                        lock,
                        writer,
                        controller,
                        transport,
                        child,
                        until,
                    } = RunnerFixture::new(
                        &catalog,
                        policy,
                        Some(RunSelection {
                            backend,
                            native_version,
                            profile_version,
                            event_evidence,
                        }),
                        size,
                        TransportLimits {
                            input_bytes: 4096,
                            display_bytes: 4096,
                            bytes_per_tick: 4096,
                        },
                        until,
                    );
                    let url = format!("sqlite:///{}", fixture.conn.path().unwrap());
                    let hooks = Arc::new(Hooks {
                        points: Mutex::new(Vec::new()),
                        until,
                    });
                    let coordinator = InputCoordinator::new(
                        &mut fixture.conn,
                        &fixture.run,
                        lock,
                        InputServices {
                            catalog,
                            token_source: Box::new(PanicTokens("ordinary Prompt")),
                            hooks: hooks.clone(),
                        },
                        until,
                    )
                    .unwrap();
                    let pty = Pty {
                        child: Reader {
                            bytes: screen.into_iter().collect(),
                            ..Reader::default()
                        },
                        offers: Vec::new(),
                        program: Vec::new(),
                        stages: Vec::new(),
                        manual: Vec::new(),
                        short: None,
                        expected_program,
                        program_offset: 0,
                        database_url: url,
                        hooks: hooks.clone(),
                    };
                    let mut host = Self {
                        fixture,
                        _artifacts: artifacts,
                        coordinator,
                        writer,
                        controller,
                        transport,
                        human: Reader::default(),
                        pty,
                        output: Output::default(),
                        hooks,
                        until,
                        expires,
                        child,
                    };
                    host.drain();
                    if profile_version.is_some() {
                        assert!(matches!(
                            host.controller.health(&host.writer).eligibility,
                            InputEligibility::ComposerEmpty
                        ));
                    }
                    host
                }
                pub(crate) fn request(&self, key: &str) -> InputControlRequest {
                    let run = &self.fixture.run;
                    InputControlRequest {
                        identity: ControlIdentity {
                            database_identity_json: run.database_identity_json.clone(),
                            fleet_id: run.fleet_id,
                            member_id: run.member_id,
                            director_member_id: run.director_member_id,
                            run_id: run.run_id,
                            owner_epoch: run.owner_epoch,
                            launch_nonce: run.launch_nonce.clone(),
                        },
                        expires_at_mono_ns: self.expires,
                        operation: InputControlOperation::SubmitPrompt(SubmitInput {
                            operation_key: key.into(),
                            input_kind: storage::InputKind::Prompt,
                            message_id: None,
                            literal: LITERAL.into(),
                        }),
                    }
                }
                pub(crate) fn show(&mut self, bytes: Vec<u8>) {
                    self.pty.child.bytes.extend(bytes);
                    self.drain();
                }
                pub(crate) fn drain(&mut self) {
                    loop {
                        assert!(
                            Instant::now() < self.until,
                            "actual fixture becomes healthy and quiescent within its original deadline"
                        );
                        self.controller
                            .poll(
                                &self.transport.snapshot(),
                                &self.writer,
                                &RealObservationClock,
                            )
                            .unwrap();
                        let result = self
                            .coordinator
                            .progress_with_manual(&mut self.transport, |transport| {
                                transport.quiesce_for_input(
                                    &mut self.human,
                                    &mut self.pty,
                                    &mut self.output,
                                    &mut self
                                        .controller
                                        .observer(&self.writer, &RealObservationClock),
                                    self.until,
                                )
                            })
                            .unwrap();
                        if matches!(result, InputDrain::Quiescent { .. }) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                pub(crate) fn advance(&mut self) -> Option<InputControlReply> {
                    self.try_advance().unwrap()
                }
                pub(crate) fn try_advance(
                    &mut self,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    self.coordinator.advance(
                        &mut self.fixture.conn,
                        &self.fixture.run,
                        &mut self.transport,
                        &mut self.pty,
                        &self.controller,
                        self.until,
                    )
                }
                pub(crate) fn handle(
                    &mut self,
                    request: InputControlRequest,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    self.handle_until(request, self.until)
                }
                /// Applies the request's own expiration first, as the control service does before
                /// it hands a received request to the coordinator.
                pub(crate) fn handle_until(
                    &mut self,
                    request: InputControlRequest,
                    deadline: Instant,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    let deadline = check_deadline(Instant::now(), deadline)
                        .and_then(|()| {
                            crate::runtime::runner::receive_input_expiration(
                                request.expires_at_mono_ns,
                                Instant::now(),
                                Some(deadline),
                                &mut crate::runtime::runner::RealInputDeadlineClock,
                            )
                        })
                        .map_err(|source| {
                            let operation_key = match &request.operation {
                                InputControlOperation::SubmitPrompt(input)
                                | InputControlOperation::SubmitExec(input) => {
                                    Some(input.operation_key.clone())
                                }
                                _ => None,
                            };
                            self.coordinator
                                .run_failure(source, None, None, operation_key)
                        })?;
                    self.coordinator.handle_received(
                        &mut self.fixture.conn,
                        &self.fixture.run,
                        request,
                        &self.controller,
                        deadline,
                    )
                }
                pub(crate) fn receipt(
                    &mut self,
                    action: crate::runtime::runner::ReceiptAction,
                ) -> std::result::Result<InputControlReply, InputFailure> {
                    let mut request = self.request("unused-receipt-key");
                    request.operation = InputControlOperation::Receipt(action);
                    self.handle(request)
                        .map(|reply| reply.expect("receipt actions reply immediately"))
                }
                pub(crate) fn respond(
                    &mut self,
                    input: &crate::runtime::runner::PermissionRespondInput,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    self.respond_until(input, self.until)
                }
                pub(crate) fn respond_until(
                    &mut self,
                    input: &crate::runtime::runner::PermissionRespondInput,
                    deadline: Instant,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    let mut request = self.request("unused-response-key");
                    request.operation = InputControlOperation::PermissionRespond(input.clone());
                    self.coordinator.handle_received(
                        &mut self.fixture.conn,
                        &self.fixture.run,
                        request,
                        &self.controller,
                        deadline,
                    )
                }
                pub(crate) fn arm(&mut self, key: &str) -> i64 {
                    let request = self.request(key);
                    assert!(self.handle(request).unwrap().is_none());
                    for _ in 0..8 {
                        self.drain();
                        assert!(self.advance().is_none());
                        assert!(self.pty.program.is_empty());
                        if self.hooks.count("before") == 1 {
                            let id = self.hooks.input_id();
                            assert!(matches!(
                                storage::get_input(&self.fixture.conn, id).unwrap().state,
                                storage::InputState::InProgress
                            ));
                            return id;
                        }
                        if self.hooks.count("reserved") == 1 {
                            assert!(self.advance().is_none());
                            assert_eq!(
                                self.hooks.count("before"),
                                0,
                                "reservation consumes its old drain proof"
                            );
                        }
                    }
                    panic!("real reservation and BeforeWrite reach Armed within finite turns");
                }
                pub(crate) fn finish(&mut self) {
                    finish_runner(&mut self.writer, &mut self.child, self.until);
                }
            }
            pub(in crate::runtime::runner::input) fn with_host(body: impl FnOnce(&mut Host)) {
                with_owned_host(Host::new(), body);
            }
            pub(in crate::runtime::runner::input) fn with_host_until(
                until: Instant,
                body: impl FnOnce(&mut Host),
            ) {
                with_owned_host(Host::until(until), body);
            }
            pub(in crate::runtime::runner::input) fn with_catalog_host(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                body: impl FnOnce(&mut Host),
            ) {
                with_profile_host(catalog, expected_program, HostProfile::metadata(), body);
            }
            pub(in crate::runtime::runner::input) fn with_profile_host(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                profile: HostProfile,
                body: impl FnOnce(&mut Host),
            ) {
                with_owned_host(Host::with_catalog(catalog, expected_program, profile), body);
            }
            pub(in crate::runtime::runner::input) fn with_profile_host_until(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                profile: HostProfile,
                until: Instant,
                body: impl FnOnce(&mut Host),
            ) {
                with_owned_host(
                    Host::build(|| catalog, expected_program, profile, until),
                    body,
                );
            }
            fn with_owned_host(mut host: Host, body: impl FnOnce(&mut Host)) {
                let primary = catch_unwind(AssertUnwindSafe(|| body(&mut host)));
                let cleanup = catch_unwind(AssertUnwindSafe(|| host.finish()));
                if let Err(panic) = primary.and(cleanup) {
                    resume_unwind(panic);
                }
            }
        }
    }
    mod permission_inspection_contracts {
        use super::super::*;
        use super::ordinary_prompt::support::{
            Host, HostProfile, catalog_definition, with_profile_host,
        };
        use crate::broker::runtime::{Coverage, InteractionClass, PermissionDecision};
        use crate::coding_agent::profiles::catalog::test_support::{
            inspection_fixture_definition, inspection_fixture_form,
        };
        pub(super) use crate::coding_agent::profiles::detector::test_support::native_fixture_bytes;
        use crate::runtime::runner::{
            MissingEvidence, PermissionInspectInput, PermissionInspection, RuntimePaths,
        };
        use rusqlite::OptionalExtension;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::time::Duration;

        struct Tokens {
            calls: Arc<AtomicUsize>,
            fail: Arc<AtomicBool>,
        }
        impl RequestTokenSource for Tokens {
            fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if self.fail.load(Ordering::SeqCst) {
                    return Err(error("owned inspection entropy failure"));
                }
                bytes.fill(u8::try_from(call).expect("bounded fixture token calls"));
                Ok(())
            }
        }

        pub(super) fn with_inspection(
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_definition(
                inspection_fixture_definition(),
                b"unused-program~".to_vec(),
                body,
            );
        }
        pub(super) fn with_redacted_inspection(
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_definition_profile(
            inspection_fixture_definition(),
            b"unused-program~".to_vec(),
            HostProfile::synthetic(
                storage::evidence::EvidencePolicy::redacted(
                    &serde_json::json!({"schema_version":1,"patterns":["synthetic-redaction-canary"],"sensitive_rules":[]}),
                    &["synthetic-permission"],
                )
                .unwrap(),
                "redacted",
            ),
            body,
        );
        }
        pub(super) fn with_definition(
            definition: serde_json::Value,
            expected_program: Vec<u8>,
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_definition_profile(definition, expected_program, HostProfile::metadata(), body);
        }

        pub(super) fn with_claude_inspection(
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_profile_host(
                ProfileCatalog::bundled().unwrap(),
                b"unused-program~".to_vec(),
                HostProfile {
                    policy: storage::evidence::EvidencePolicy::metadata(),
                    event_evidence: "metadata",
                    backend: crate::coding_agent::CodingAgent::Claude,
                    native_version: "2.1.302",
                    profile_version: Some("claude-2.1.302-auto-1"),
                    size: crate::runtime::runner::TerminalSize {
                        rows: 36,
                        cols: 120,
                    },
                    screen: native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "claude-2.1.302-idle-composer-1",
                        ),
                    ),
                },
                |h| with_counted_tokens(h, body),
            );
        }

        pub(super) fn with_opencode_inspection(
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_profile_host(
                ProfileCatalog::bundled().unwrap(),
                b"\r".to_vec(),
                HostProfile {
                    policy: storage::evidence::EvidencePolicy::metadata(),
                    event_evidence: "metadata",
                    backend: crate::coding_agent::CodingAgent::Opencode,
                    native_version: "2.1.4",
                    profile_version: Some("opencode-2.1.4-cafleet-ask-1"),
                    size: crate::runtime::runner::TerminalSize {
                        rows: 36,
                        cols: 120,
                    },
                    screen: native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-idle-composer-1",
                        ),
                    ),
                },
                |h| with_counted_tokens(h, body),
            );
        }

        fn with_counted_tokens(
            h: &mut Host,
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            let calls = Arc::new(AtomicUsize::new(0));
            let fail = Arc::new(AtomicBool::new(false));
            h.coordinator.services.token_source = Box::new(Tokens {
                calls: calls.clone(),
                fail: fail.clone(),
            });
            body(h, calls, fail);
        }

        fn with_definition_profile(
            mut definition: serde_json::Value,
            expected_program: Vec<u8>,
            profile: HostProfile,
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            let composer = catalog_definition();
            definition["fixtures"]
                .as_array_mut()
                .unwrap()
                .push(composer["fixtures"][1].clone());
            definition["profiles"][0]["rules"]
                .as_array_mut()
                .unwrap()
                .push(composer["profiles"][0]["rules"][0].clone());
            definition["profiles"][0]["capabilities"]["composer"] =
                composer["profiles"][0]["capabilities"]["composer"].clone();
            with_profile_host(
                ProfileCatalog::from_definitions(&definition).unwrap(),
                expected_program,
                profile,
                |h| with_counted_tokens(h, body),
            );
        }

        pub(super) fn form(h: &mut Host, operation: &str, focus: &str) {
            h.show(inspection_fixture_form(operation, focus));
        }

        fn current_ids(h: &Host) -> Option<(i64, i64, i64, i64)> {
            h.fixture.conn.query_row(
            "SELECT e.event_id,e.interaction_id,e.message_id,i.episode_seq FROM member_runtime_events e JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 AND e.kind='interaction_opened' AND i.closed_at IS NULL ORDER BY e.event_id DESC LIMIT 1",
            [h.fixture.run.run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).optional().unwrap()
        }

        pub(super) fn stable_form(h: &mut Host) -> (i64, i64, i64, i64) {
            form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
            settled_ids(h)
        }
        // The detail row is inside the request region but outside its fingerprint and answer focus.
        pub(super) fn stable_detailed_form(h: &mut Host, detail: &str) -> (i64, i64, i64, i64) {
            h.pty.child.bytes.extend(
            format!("\x1b[2J\x1b[18;1HSynthetic permission request\r\nOperation: PRIVATE_INSPECTION_OPERATION\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nDetail: {detail}\r\nSelection: ").into_bytes(),
        );
            h.drain();
            settled_ids(h)
        }
        pub(super) fn detail_rewrite(detail: &str) -> Vec<u8> {
            format!("\x1b[23;1H\x1b[2KDetail: {detail}\x1b[24;12H").into_bytes()
        }
        pub(super) fn settled_ids(h: &mut Host) -> (i64, i64, i64, i64) {
            loop {
                h.drain();
                if h.controller.current_request().is_some()
                    && let Some(ids) = current_ids(h)
                {
                    return ids;
                }
                assert!(
                    Instant::now() < h.until,
                    "stable actual request within original fixture deadline"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        pub(super) fn request(h: &Host, ids: (i64, i64, i64, i64)) -> InputControlRequest {
            let mut request = h.request("unused-inspection-key");
            request.operation = InputControlOperation::PermissionInspect(PermissionInspectInput {
                event_id: ids.0,
                interaction_id: ids.1,
                director_member_id: h.fixture.run.director_member_id,
            });
            request
        }

        pub(super) fn inspect_request(
            h: &mut Host,
            request: InputControlRequest,
            until: Instant,
        ) -> std::result::Result<PermissionInspection, InputFailure> {
            h.coordinator
                .inspect_received(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    request,
                    (&h.controller, &mut h.transport),
                    until,
                )
                .map(|reply| {
                    let InputControlReply::Inspection(value) = reply else {
                        panic!("actual inspection typed reply")
                    };
                    *value
                })
        }
        pub(super) fn inspect(h: &mut Host, ids: (i64, i64, i64, i64)) -> PermissionInspection {
            h.drain();
            inspect_request(h, request(h, ids), h.until).unwrap()
        }

        #[test]
        fn current_request_binding_compares_every_field_except_output_seq() {
            with_inspection(|h, _, _| {
                let first = stable_form(h);
                inspect(h, first);
                let (current, _) = h
                    .coordinator
                    .current_request
                    .clone()
                    .expect("an inspected request is current");
                form(h, "OTHER_INSPECTION_OPERATION", "synthetic-only");
                settled_ids(h);
                let other = h
                    .controller
                    .current_request()
                    .expect("the redrawn request is observed")
                    .clone();
                assert!(other.semantic != current.observed.semantic);
                assert!(current.same_request(&current.clone()));
                let mut moved = current.clone();
                moved.observed.output_seq += 1;
                assert!(current.same_request(&moved), "output_seq stays unbound");
                type Mutation<'a> = (&'a str, Box<dyn Fn(&mut CurrentRequest) + 'a>);
                let mutations: [Mutation; 16] = [
                    (
                        "request_generation",
                        Box::new(|r| r.observed.request_generation += 1),
                    ),
                    ("identity", Box::new(|r| r.identity.owner_epoch += 1)),
                    ("interaction_id", Box::new(|r| r.interaction_id += 1)),
                    ("event_id", Box::new(|r| r.event_id += 1)),
                    ("message_id", Box::new(|r| r.message_id += 1)),
                    ("manual_generation", Box::new(|r| r.manual_generation += 1)),
                    ("episode_seq", Box::new(|r| r.observed.episode_seq += 1)),
                    (
                        "classification",
                        Box::new(|r| {
                            r.observed.classification = storage::InteractionClass::Question
                        }),
                    ),
                    (
                        "native_version",
                        Box::new(|r| r.observed.native_version.push('x')),
                    ),
                    (
                        "detector_profile_version",
                        Box::new(|r| r.observed.detector_profile_version.push('x')),
                    ),
                    (
                        "request_rule_id",
                        Box::new(|r| r.observed.request_rule_id.push('x')),
                    ),
                    (
                        "semantic",
                        Box::new(|r| r.observed.semantic = other.semantic.clone()),
                    ),
                    ("choices", Box::new(|r| r.observed.choices.push("x".into()))),
                    ("focus", Box::new(|r| r.observed.focus.push('x'))),
                    (
                        "native_correlation",
                        Box::new(|r| {
                            r.observed.native_correlation =
                                Some(format!("{:?}-changed", r.observed.native_correlation));
                        }),
                    ),
                    (
                        "answer_matches",
                        Box::new(|r| {
                            r.observed.answer_matches.pop();
                        }),
                    ),
                ];
                for (field, mutate) in mutations {
                    let mut changed = current.clone();
                    mutate(&mut changed);
                    assert!(!current.same_request(&changed), "{field} binds the request");
                }
            });
        }
        fn token(value: &PermissionInspection) -> String {
            let token = value
                .request_token
                .as_ref()
                .expect("current matched request token");
            assert_eq!(token.len(), 64);
            assert!(
                token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
            token.clone()
        }
        pub(super) fn counts(h: &Host) -> [i64; 3] {
            [
                "member_input_operations",
                "member_permission_responses",
                "member_permission_response_claims",
            ]
            .map(|table| {
                h.fixture
                    .conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap()
            })
        }
        #[test]
        fn permission_inspection_matches_durable_current_request_and_preserves_the_armed_writer() {
            with_inspection(|h, calls, _| {
                let input_id = h.arm("prompt:inspection-pending");
                let ids = stable_form(h);
                let pending = h.pending_snapshot();
                let row = storage::get_input(&h.fixture.conn, input_id).unwrap();
                let retained = h.coordinator.retained_checkpoint(input_id).unwrap();
                let before = counts(h);
                let hooks = h.hooks.points.lock().unwrap().len();
                let value = inspect(h, ids);
                assert_eq!(
                    (
                        value.event_id,
                        value.interaction_id,
                        value.message_id,
                        value.episode_seq
                    ),
                    ids
                );
                let run = &h.fixture.run;
                assert_eq!(
                    (
                        value.run_id,
                        value.owner_epoch,
                        value.fleet_id,
                        value.member_id,
                        value.director_member_id
                    ),
                    (
                        run.run_id,
                        run.owner_epoch,
                        run.fleet_id,
                        run.member_id,
                        run.director_member_id
                    )
                );
                assert_eq!(
                    value.classification,
                    Some(InteractionClass::PermissionRequest)
                );
                assert_eq!(value.visibility, Visibility::Visible);
                assert_eq!(value.answer_capability, Coverage::Tested);
                assert_eq!(
                    value.answer_profile_version.as_deref(),
                    Some("synthetic-profile-1")
                );
                assert!(
                    value.answer_rule_id.is_none(),
                    "distinct Allow/Deny rules have no unique common rule"
                );
                assert_eq!(
                    value.supported_decisions,
                    vec![PermissionDecision::Allow, PermissionDecision::Deny]
                );
                assert!(value.capability_reason.is_none());
                assert_eq!(
                    value.evidence,
                    h.controller.current_evidence().unwrap().clone()
                );
                assert!(value.evidence["operation"].is_null());
                assert_eq!(
                    value
                        .missing_evidence
                        .iter()
                        .map(MissingEvidence::as_str)
                        .collect::<Vec<_>>(),
                    [
                        "operation",
                        "arguments",
                        "cwd",
                        "environment",
                        "targets",
                        "effects",
                        "choice_scope"
                    ]
                );
                let first = token(&value);
                let no_drain = inspect_request(h, request(h, ids), h.until).unwrap();
                assert!(no_drain.request_token.is_none());
                assert!(no_drain.supported_decisions.is_empty());
                assert_eq!(no_drain.answer_capability, Coverage::Unknown);
                assert_eq!(token(&inspect(h, ids)), first);
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert!(pending == h.pending_snapshot());
                assert_eq!(storage::get_input(&h.fixture.conn, input_id).unwrap(), row);
                let after = h.coordinator.retained_checkpoint(input_id).unwrap();
                assert_eq!(
                    (after.stage, after.written_bytes, after.output_seq),
                    (retained.stage, retained.written_bytes, retained.output_seq)
                );
                assert_eq!(counts(h), before);
                assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
                assert!(h.pty.offers.is_empty());
                let (evidence,body,notified):(String,String,Option<String>) = h.fixture.conn.query_row("SELECT i.evidence_json,m.text,m.notified_at FROM member_interactions i JOIN messages m ON m.message_id=?1 WHERE i.interaction_id=?2",rusqlite::params![ids.2,ids.1],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                for stored in [evidence, body] {
                    assert!(!stored.contains(&first));
                    assert!(!stored.contains("PRIVATE_INSPECTION_OPERATION"));
                }
                let paths = RuntimePaths::for_run(
                    &h.fixture.run.database_identity_json,
                    h.fixture.run.run_id,
                )
                .unwrap();
                let outbox = std::fs::read(paths.outbox).unwrap();
                for secret in [first.as_bytes(), b"PRIVATE_INSPECTION_OPERATION".as_slice()] {
                    assert!(!outbox.windows(secret.len()).any(|window| window == secret));
                }
                assert!(notified.is_none());
            });
        }

        #[test]
        fn permission_inspection_token_binds_focus_plan_set_and_intermediate_invalidations() {
            with_inspection(|h, calls, _| {
                let ids = stable_form(h);
                let original = inspect(h, ids);
                let mut previous = token(&original);
                let revision = h.controller.request_generation();
                h.pty.child.bytes.extend(b"\x1b[?25l\x1b[?25h");
                h.drain();
                let redraw = inspect(h, ids);
                assert!(redraw.output_seq > original.output_seq);
                assert_eq!(h.controller.request_generation(), revision);
                assert_eq!(token(&redraw), previous);
                h.transport
                    .resize(
                        &mut h.pty,
                        crate::runtime::runner::TerminalSize { rows: 24, cols: 81 },
                    )
                    .unwrap();
                assert_eq!(token(&inspect(h, ids)), previous);
                h.coordinator.observe_manual(0).unwrap();
                assert_eq!(token(&inspect(h, ids)), previous);
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
                let rebuilt = token(&inspect(h, ids));
                assert_ne!(
                    rebuilt, previous,
                    "clear/rebuild traverses incompatible observed screens"
                );
                previous = rebuilt;
                h.human.bytes.push_back(b'!');
                h.drain();
                assert_eq!(h.pty.manual, b"!");
                let manual = token(&inspect(h, ids));
                assert_ne!(manual, previous);
                previous = manual;
                let revision = h.controller.request_generation();
                form(h, "PRIVATE_INSPECTION_OPERATION", "missing-focus");
                assert!(h.controller.current_request().is_none());
                assert!(h.controller.request_generation() > revision);
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-only");
                let returned = token(&inspect(h, ids));
                assert_ne!(
                    returned, previous,
                    "A-invalid-A cannot resurrect a token without an intervening inspect"
                );
                previous = returned;
                assert_eq!(
                    current_ids(h),
                    Some(ids),
                    "invalidation precedes durable obscurity debounce"
                );
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other");
                let focused = token(&inspect(h, ids));
                assert_ne!(focused, previous);
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt");
                let alternate = inspect(h, ids);
                assert_ne!(token(&alternate), focused);
                assert_eq!(
                    alternate.supported_decisions,
                    vec![PermissionDecision::Allow]
                );
                assert_eq!(
                    alternate.answer_rule_id.as_deref(),
                    Some("inspection-alternate-allow")
                );
                assert_eq!(
                    current_ids(h),
                    Some(ids),
                    "plan/focus changes are independent of semantic episode identity"
                );
                form(h, "different semantic request", "synthetic-only");
                assert!(h.controller.current_request().is_none());
                let candidate = inspect_request(h, request(h, ids), h.until).unwrap();
                assert!(candidate.request_token.is_none());
                assert!(candidate.supported_decisions.is_empty());
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-alt");
                assert_ne!(token(&inspect(h, ids)), token(&alternate));
                assert_eq!(calls.load(Ordering::SeqCst), 7);
                assert!(h.pty.program.is_empty());
            });
        }

        #[test]
        fn permission_inspection_refuses_stale_associations_identity_and_expired_budget_without_effects()
         {
            with_inspection(|h, calls, _| {
                let ids = stable_form(h);
                let original = token(&inspect(h, ids));
                let before = counts(h);
                h.drain();
                let mut wrong = request(h, ids);
                wrong.identity.launch_nonce = "f".repeat(64);
                assert!(inspect_request(h, wrong, h.until).is_err());
                h.drain();
                assert!(inspect_request(h, request(h, ids), Instant::now()).is_err());
                h.fixture.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",rusqlite::params![crate::time::now_stamp(),ids.1]).unwrap();
                h.drain();
                assert!(inspect_request(h, request(h, ids), h.until).is_err());
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert_eq!(counts(h), before);
                assert!(h.pty.offers.is_empty());
                assert!(!original.is_empty());
            });
        }

        #[test]
        fn permission_inspection_entropy_failure_supplies_no_fallback_authority() {
            with_inspection(|h, calls, fail| {
                let ids = stable_form(h);
                fail.store(true, Ordering::SeqCst);
                h.drain();
                let failure = match inspect_request(h, request(h, ids), h.until) {
                    Err(e) => e,
                    Ok(_) => panic!("entropy failure is explicit"),
                };
                assert!(
                    failure
                        .source
                        .message()
                        .contains("owned inspection entropy failure")
                );
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                fail.store(false, Ordering::SeqCst);
                let value = inspect(h, ids);
                assert_eq!(token(&value), "02".repeat(32));
                assert_eq!(counts(h), [0, 0, 0]);
                assert!(h.pty.offers.is_empty());
            });
        }

        #[test]
        fn permission_inspection_owner_loss_revokes_the_existing_token() {
            with_inspection(|h, calls, _| {
                let ids = stable_form(h);
                let original = token(&inspect(h, ids));
                let paths = RuntimePaths::for_run(
                    &h.fixture.run.database_identity_json,
                    h.fixture.run.run_id,
                )
                .unwrap();
                let backup = paths.owner_lock.with_extension("inspection-owned-backup");
                h.drain();
                std::fs::rename(&paths.owner_lock, &backup).unwrap();
                let lost = inspect_request(h, request(h, ids), h.until);
                std::fs::rename(&backup, &paths.owner_lock).unwrap();
                assert!(
                    lost.is_err(),
                    "actual lock inode no longer occupies its pinned path"
                );
                assert_ne!(token(&inspect(h, ids)), original);
                assert_eq!(calls.load(Ordering::SeqCst), 2);
                assert!(h.pty.offers.is_empty());
            });
        }

        #[test]
        fn permission_inspection_observation_gap_revokes_authority_before_recovery() {
            with_inspection(|h, _, _| {
                let ids = stable_form(h);
                let original = token(&inspect(h, ids));
                let generation = h.controller.request_generation();
                h.controller
                    .observer_failed(
                        crate::runtime::runner::ObservationFailure::Parser,
                        &crate::runtime::runner::RealObservationClock,
                    )
                    .unwrap();
                assert!(h.controller.current_request().is_none());
                assert!(h.controller.request_generation() > generation);
                let unavailable = inspect_request(h, request(h, ids), h.until);
                assert!(
                    unavailable.is_err()
                        || unavailable.is_ok_and(|value| value.request_token.is_none())
                );
                let later_ids = stable_form(h);
                assert_ne!(token(&inspect(h, later_ids)), original);
                assert!(h.pty.offers.is_empty());
            });
        }

        #[test]
        fn permission_inspection_missing_facts_use_validated_post_policy_evidence_only() {
            use crate::coding_agent::{CodingAgent, Detector};
            use crate::runtime::runner::{TerminalSize, TerminalState};
            let catalog =
                ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
            let detector = Detector::new(&catalog, CodingAgent::Codex, Some("0.0.1-synthetic"));
            let mut terminal = TerminalState::new(TerminalSize { rows: 24, cols: 80 }).unwrap();
            terminal
                .feed(&inspection_fixture_form(
                    "write public.txt",
                    "synthetic-only",
                ))
                .unwrap();
            let raw = detector.observe(&terminal.snapshot()).unwrap().raw_evidence;
            let redacted = storage::evidence::EvidencePolicy::redacted(&serde_json::json!({"schema_version":1,"patterns":["OWNED_SECRET"],"sensitive_rules":[]}),&["synthetic-permission"]).unwrap();
            let missing = |value: &serde_json::Value| {
                inspection_missing_evidence(value)
                    .unwrap()
                    .iter()
                    .map(MissingEvidence::as_str)
                    .collect::<Vec<_>>()
            };
            let public = redacted.apply(&raw).unwrap();
            let unavailable = [
                "operation",
                "arguments",
                "cwd",
                "environment",
                "targets",
                "effects",
                "choice_scope",
            ];
            assert_eq!(missing(&public), unavailable[1..]);
            assert_eq!(
                missing(
                    &storage::evidence::EvidencePolicy::metadata()
                        .apply(&raw)
                        .unwrap()
                ),
                unavailable
            );
            for operation in [
                serde_json::Value::Null,
                serde_json::json!(" "),
                serde_json::json!("OWNED_SECRET"),
                serde_json::json!("[redacted]"),
            ] {
                let mut changed = raw.clone();
                if operation.is_null() {
                    changed["operation_rows"] = serde_json::Value::Null;
                }
                changed["operation"] = operation;
                assert_eq!(missing(&redacted.apply(&changed).unwrap()), unavailable);
            }
            let mut truncated = raw.clone();
            truncated["truncated"] = serde_json::json!(true);
            assert_eq!(missing(&redacted.apply(&truncated).unwrap()), unavailable);
            let mut corrupt = public;
            corrupt["evidence_digest"] = serde_json::json!("f".repeat(64));
            assert!(inspection_missing_evidence(&corrupt).is_err());
            corrupt.as_object_mut().unwrap().remove("operation");
            assert!(inspection_missing_evidence(&corrupt).is_err());
        }

        #[test]
        fn permission_inspection_current_authority_has_no_debug_or_serialized_representation() {
            assert_no_debug_or_serialize!(CurrentRequest);
        }
    }
    mod permission_response_contracts {
        use super::super::*;
        use super::ordinary_prompt::support::{Host, Pty};
        use super::permission_inspection_contracts::{
            counts, detail_rewrite, form, inspect, inspect_request, native_fixture_bytes,
            request as inspection_target, settled_ids, stable_detailed_form, stable_form,
            with_claude_inspection, with_definition, with_inspection, with_redacted_inspection,
        };
        use crate::broker::runtime::{DecisionSource, DirectorMode, PermissionDecision};
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::profiles::catalog::test_support::inspection_fixture_definition;
        use crate::runtime::runner::{
            ByteInput, PermissionInspection, PermissionRespondInput, PermissionResponseResult,
            ReceiptAction, ReceiptReply, TerminalSize,
        };
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        fn command(
            h: &Host,
            inspection: &PermissionInspection,
            key: &str,
            decision: PermissionDecision,
        ) -> PermissionRespondInput {
            PermissionRespondInput {
                event_id: inspection.event_id,
                interaction_id: inspection.interaction_id,
                director_member_id: h.fixture.run.director_member_id,
                expected_run_id: h.fixture.run.run_id,
                expected_owner_epoch: h.fixture.run.owner_epoch,
                request_token: inspection
                    .request_token
                    .as_ref()
                    .expect("actual inspected token")
                    .clone(),
                decision,
                decision_source: DecisionSource::DirectorAuto,
                director_mode: DirectorMode::Auto,
                mode_evidence_ref: "fixture:response-mode".into(),
                decision_evidence_ref: "fixture:response-decision".into(),
                operation_key: key.into(),
            }
        }
        fn response(reply: InputControlReply) -> PermissionResponseResult {
            let InputControlReply::PermissionResponse(value) = reply else {
                panic!("typed actual permission response")
            };
            value
        }
        fn arm_stage(h: &mut Host, before: usize) -> i64 {
            let offers = h.pty.offers.len();
            for _ in 0..16 {
                h.drain();
                assert!(
                    h.advance().is_none(),
                    "stage acquisition yields after its bounded hooks"
                );
                assert_eq!(
                    h.pty.offers.len(),
                    offers,
                    "fresh guard acquisition precedes its syscall"
                );
                if h.hooks.count("before") == before {
                    return h.hooks.input_id();
                }
            }
            panic!("real selected stage reaches Armed within its finite turn budget");
        }
        fn audit(
            h: &Host,
            id: i64,
            input: &PermissionRespondInput,
            rule: &str,
        ) -> storage::PermissionReceipt {
            let receipt = storage::get_permission_response(&h.fixture.conn, id).unwrap();
            assert_eq!(receipt.input.kind, storage::InputKind::PermissionResponse);
            assert_eq!(receipt.input.operation_key, input.operation_key);
            assert_eq!(receipt.input.message_id, None);
            let m = &receipt.response.metadata;
            assert_eq!(
                (
                    m.event_id,
                    m.interaction_id,
                    m.expected_owner_epoch,
                    m.director_member_id
                ),
                (
                    input.event_id,
                    input.interaction_id,
                    input.expected_owner_epoch,
                    input.director_member_id
                )
            );
            assert_eq!(
                (
                    &m.request_token,
                    m.decision,
                    m.decision_source,
                    m.director_mode
                ),
                (
                    &input.request_token,
                    input.decision,
                    input.decision_source,
                    input.director_mode
                )
            );
            assert_eq!(
                (&m.mode_evidence_ref, &m.decision_evidence_ref),
                (&input.mode_evidence_ref, &input.decision_evidence_ref)
            );
            assert_eq!(m.answer_profile_version, "synthetic-profile-1");
            assert_eq!(m.answer_rule_id, rule);
            assert_eq!(
                receipt
                    .claim
                    .as_ref()
                    .map(|c| (c.interaction_id, c.input_id)),
                Some((input.interaction_id, id))
            );
            receipt
        }

        const UNAVAILABLE: &str = "current executable permission request unavailable";

        fn blocked_reason(h: &mut Host, ids: (i64, i64, i64, i64)) -> String {
            let target = inspection_target(h, ids);
            let inspected = inspect_request(h, target, h.until)
                .expect("a blocked request is an inspection result, not an error");
            assert_eq!(inspected.request_token, None);
            assert!(inspected.supported_decisions.is_empty());
            assert_eq!(inspected.answer_capability, storage::Coverage::Unknown);
            inspected
                .capability_reason
                .expect("a blocked request names its reason")
        }

        fn refused_reason(h: &mut Host, input: &PermissionRespondInput) -> String {
            match h.respond(input) {
                Err(failure) => failure.source.message().into_owned(),
                Ok(_) => panic!("a blocked request refuses the response"),
            }
        }

        #[test]
        fn permission_inspection_and_response_name_the_same_blocking_conjunct() {
            let mut reasons = Vec::new();
            for (case, block) in [
                (
                    "fenced input",
                    (|h: &mut Host| h.coordinator.fence(h.until).unwrap()) as fn(&mut Host),
                ),
                ("failed manual accounting", |h: &mut Host| {
                    h.coordinator.manual_generation = u64::MAX;
                    assert!(h.coordinator.observe_manual(1).is_err());
                    assert!(h.coordinator.manual_accounting_failed);
                }),
            ] {
                with_inspection(|h, calls, _| {
                    let ids = stable_form(h);
                    let inspected = inspect(h, ids);
                    let input =
                        command(h, &inspected, "response:blocked", PermissionDecision::Allow);
                    let minted = calls.load(Ordering::SeqCst);
                    block(h);
                    h.drain();
                    let reason = blocked_reason(h, ids);
                    assert_ne!(reason, UNAVAILABLE, "{case} names its own conjunct");
                    let refused = refused_reason(h, &input);
                    assert!(
                        refused.contains(&reason),
                        "{case}: respond reports the inspect reason {reason:?}, got {refused:?}"
                    );
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        minted,
                        "{case}: a blocked inspection draws no request token"
                    );
                    assert_eq!(counts(h), [0, 0, 0]);
                    assert!(h.pty.program.is_empty());
                    reasons.push(reason);
                });
            }
            assert_ne!(reasons[0], reasons[1], "each conjunct has its own reason");
        }

        fn stored_evidence(h: &Host, interaction_id: i64) -> String {
            h.fixture
                .conn
                .query_row(
                    "SELECT evidence_json FROM member_interactions WHERE interaction_id=?1",
                    [interaction_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        #[test]
        fn permission_screen_seq_only_redraw_schedules_no_refresh_and_keeps_the_settled_token() {
            with_inspection(|h, _, _| {
                let ids = stable_form(h);
                let inspected = inspect(h, ids);
                let input = command(h, &inspected, "response:redraw", PermissionDecision::Allow);
                let evidence = stored_evidence(h, ids.1);
                let output_seq = h.transport.snapshot().screen_seq;
                let lock = Connection::open(h.fixture.conn.path().unwrap()).unwrap();
                lock.execute_batch("BEGIN IMMEDIATE").unwrap();
                // Every observed frame keeps identical cells with the cursor at the request end: the
                // observer runs after each control byte, so `[24;1HSelection: ` lands as one chunk.
                h.pty
                    .child
                    .bytes
                    .extend(b"\x1b[?25l\x1b[0m\x1b[24;1HSelection: \x1b[?25h");
                let drained = h
                    .coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        transport.quiesce_for_input(
                            &mut h.human,
                            &mut h.pty,
                            &mut h.output,
                            &mut h
                                .controller
                                .observer(&h.writer, &crate::runtime::runner::RealObservationClock),
                            h.until,
                        )
                    })
                    .unwrap();
                assert!(
                    h.transport.snapshot().screen_seq > output_seq,
                    "the redraw produced new output"
                );
                assert!(
                    matches!(drained, InputDrain::Quiescent { .. }),
                    "a screen_seq-only redraw needs no write, so it settles while the database is locked"
                );
                let status = h.controller.status();
                assert!(
                    !status.retained_evidence,
                    "no evidence refresh is scheduled"
                );
                assert!(!status.retained_transition);
                assert!(!h.controller.health(&h.writer).pending_evidence);
                assert!(h.controller.current_request().is_some());
                let while_locked = inspect_request(h, inspection_target(h, ids), h.until).unwrap();
                assert_eq!(
                    while_locked.request_token, inspected.request_token,
                    "the settled request keeps its token without a database write"
                );
                lock.execute_batch("COMMIT").unwrap();
                drop(lock);
                h.drain();
                assert_eq!(
                    h.controller.status().gap_count,
                    0,
                    "no writer gap occurred, so no revocation applies"
                );
                assert!(!h.writer.status().degraded);
                assert_eq!(
                    stored_evidence(h, ids.1),
                    evidence,
                    "the stored latest evidence keeps the screen_seq of its last real change"
                );
                let settled = inspect(h, ids);
                assert_eq!(settled.request_token, inspected.request_token);
                assert!(
                    h.respond(&input).unwrap().is_none(),
                    "the settled request accepts its original token"
                );
                arm_stage(h, 1);
                assert_eq!(counts(h), [1, 1, 1]);
            });
        }

        fn quiesce_once(h: &mut Host) -> InputDrain {
            h.coordinator
                .progress_with_manual(&mut h.transport, |transport| {
                    transport.quiesce_for_input(
                        &mut h.human,
                        &mut h.pty,
                        &mut h.output,
                        &mut h
                            .controller
                            .observer(&h.writer, &crate::runtime::runner::RealObservationClock),
                        h.until,
                    )
                })
                .unwrap()
        }

        // One CSI sequence longer than the drain's per-tick child budget: it changes no cell or
        // cursor and yields only two observed frames, so the drain ends with output still arriving.
        fn neutral_output_flood(h: &mut Host) {
            let mut bytes = b"\x1b[".to_vec();
            bytes.extend(b"0;".repeat(2200));
            bytes.push(b'm');
            h.pty.child.bytes.extend(bytes);
        }

        const OUTPUT_ARRIVING: &str = "terminal output still arriving";

        // The blocked Claude Code 2.1.302 form's eleven status-dot blink writes: each moves the
        // cursor home, to the bottom row and back to the form without touching form rows.
        fn blink_writes() -> Vec<Vec<u8>> {
            (0..11)
                .map(|index| {
                    let dot = if index % 2 == 0 { "●" } else { " " };
                    format!("\x1b[H\r\x1b[9B\x1b[38;5;246m{dot}\x1b[39m\x1b[80;1H\x1b[23;2H")
                        .into_bytes()
                })
                .collect()
        }

        fn claude_blocked_form(h: &mut Host) -> (i64, i64, i64, i64) {
            h.pty.child.bytes.extend(native_fixture_bytes(
                crate::coding_agent::profiles::detector::test_support::bundled_screen(
                    "claude-2.1.302-default-permission-1",
                ),
            ));
            settled_ids(h)
        }

        fn claude_boxed_single_row_form(h: &mut Host) -> (i64, i64, i64, i64) {
            h.pty.child.bytes.extend(native_fixture_bytes(
                crate::coding_agent::profiles::detector::test_support::bundled_screen(
                    "claude-2.1.302-boxed-single-row-permission-1",
                ),
            ));
            settled_ids(h)
        }

        type FormFeed = fn(&mut Host) -> (i64, i64, i64, i64);

        #[test]
        fn a_preview_during_a_visible_claude_permission_form_is_held_without_child_bytes() {
            let forms: [(&str, FormFeed); 2] = [
                ("claude-2.1.302-default-permission-1", claude_blocked_form),
                (
                    "claude-2.1.302-boxed-single-row-permission-1",
                    claude_boxed_single_row_form,
                ),
            ];
            for (fixture, feed_form) in forms {
                with_claude_inspection(|h, _, _| {
                    let ids = feed_form(h);
                    let inspected = inspect(h, ids);
                    assert!(inspected.request_token.is_some(), "{fixture}");
                    let generation = h.controller.request_generation();
                    let offers = h.pty.offers.clone();
                    let program = h.pty.program.clone();
                    assert!(
                        matches!(
                            h.controller.health(&h.writer).eligibility,
                            InputEligibility::Hold(_)
                        ),
                        "{fixture}: the visible form makes the composer ineligible"
                    );
                    let run = h.fixture.run.clone();
                    let message = crate::broker::send_message(
                        &mut h.fixture.conn,
                        run.director_member_id,
                        run.member_id,
                        "preview during a permission form",
                    )
                    .unwrap();
                    let key = format!("preview:{}:{}", run.run_id, message.message_id);
                    let mut request = h.request(&key);
                    request.operation =
                        InputControlOperation::SubmitPrompt(crate::runtime::runner::SubmitInput {
                            operation_key: key.clone(),
                            input_kind: storage::InputKind::Preview,
                            message_id: Some(message.message_id),
                            literal: "preview during a permission form".into(),
                        });
                    let reply = h
                        .coordinator
                        .handle_received(
                            &mut h.fixture.conn,
                            &h.fixture.run,
                            request,
                            &h.controller,
                            h.until,
                        )
                        .unwrap();
                    let Some(InputControlReply::Submission(held)) = reply else {
                        panic!(
                            "{fixture}: a preview during a visible form returns a Submission reply"
                        )
                    };
                    assert_eq!(
                        (held.status, held.stage, held.input_id),
                        (SubmissionStatus::Held, SubmissionStage::ZeroBytes, None),
                        "{fixture}"
                    );
                    assert_eq!(
                        held.reason.as_deref(),
                        Some("ordinary composer input is held"),
                        "{fixture}: the visible form, not a missing composer plan, holds the preview"
                    );
                    assert!(h.coordinator.pending.is_none(), "{fixture}");
                    assert_eq!(
                        h.pty.offers, offers,
                        "{fixture}: no child byte, Escape included, is offered"
                    );
                    assert_eq!(
                        h.pty.program, program,
                        "{fixture}: no program byte is written"
                    );
                    assert!(
                        storage::find_input(&h.fixture.conn, run.run_id, &key)
                            .unwrap()
                            .is_none(),
                        "{fixture}"
                    );
                    let notified: Option<String> = h
                        .fixture
                        .conn
                        .query_row(
                            "SELECT notified_at FROM messages WHERE message_id=?1",
                            [message.message_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    assert_eq!(notified, None, "{fixture}");
                    assert!(h.controller.current_request().is_some(), "{fixture}");
                    assert_eq!(h.controller.request_generation(), generation, "{fixture}");
                    assert_eq!(
                        inspect(h, ids).request_token,
                        inspected.request_token,
                        "{fixture}"
                    );
                });
            }
        }

        #[test]
        fn permission_claude_blink_cursor_transit_keeps_generation_and_token_and_admits_respond() {
            let writes = blink_writes();
            assert_eq!(writes.len(), 11);
            let captured = writes.concat();
            assert_eq!(captured.len(), 441);
            assert_eq!(
                crate::digest::sha256_hex(&captured),
                "014fe1213f56da769340abaf3cb46961c682a58d16fbccd7e16e62ca8e582e9e"
            );
            with_claude_inspection(|h, _, _| {
                let ids = claude_blocked_form(h);
                let inspected = inspect(h, ids);
                assert!(inspected.request_token.is_some());
                assert_eq!(
                    inspected.supported_decisions,
                    [PermissionDecision::Allow, PermissionDecision::Deny]
                );
                let generation = h.controller.request_generation();
                for (index, write) in writes.iter().enumerate() {
                    h.pty.child.bytes.extend(write.iter().copied());
                    h.drain();
                    assert_eq!(
                        h.controller.request_generation(),
                        generation,
                        "blink write {index} is a cursor transit"
                    );
                    assert!(
                        h.controller.current_request().is_some(),
                        "blink write {index} ends on the restored request"
                    );
                    assert_eq!(
                        inspect(h, ids).request_token,
                        inspected.request_token,
                        "blink write {index} keeps the token"
                    );
                }
                let input = command(
                    h,
                    &inspected,
                    "response:claude-blink",
                    PermissionDecision::Allow,
                );
                assert!(
                    h.respond(&input).unwrap().is_none(),
                    "respond is admitted on the quiesced frame after every blink"
                );
            });
        }

        #[test]
        fn permission_claude_transit_that_rewrites_a_form_row_revokes_the_token() {
            let blink = &blink_writes()[0];
            let landing = b"\x1b[9B";
            let at = blink
                .windows(landing.len())
                .position(|window| window == landing)
                .expect("the blink moves down to its status row");
            let mut onto_form = blink[..at].to_vec();
            onto_form.extend(b"\x1b[21B");
            onto_form.extend(&blink[at + landing.len()..]);
            with_claude_inspection(|h, _, _| {
                let ids = claude_blocked_form(h);
                let inspected = inspect(h, ids);
                let generation = h.controller.request_generation();
                h.pty.child.bytes.extend(onto_form);
                h.drain();
                assert!(
                    h.controller.request_generation() > generation,
                    "a transit frame with a changed form row invalidates the request"
                );
                assert_ne!(inspect(h, ids).request_token, inspected.request_token);
                let input = command(
                    h,
                    &inspected,
                    "response:claude-rewritten-row",
                    PermissionDecision::Allow,
                );
                assert!(
                    h.respond(&input).is_err(),
                    "the revoked token refuses respond"
                );
                assert_eq!(counts(h), [0, 0, 0]);
                assert!(h.pty.program.is_empty());
            });
        }

        #[test]
        fn permission_drain_reason_is_current_and_never_a_stale_earlier_reason() {
            with_redacted_inspection(|h, _, _| {
                let ids = stable_detailed_form(h, "opened");
                let inspected = inspect(h, ids);
                let input = command(
                    h,
                    &inspected,
                    "response:stale-reason",
                    PermissionDecision::Allow,
                );
                let lock = Connection::open(h.fixture.conn.path().unwrap()).unwrap();
                lock.execute_batch("BEGIN IMMEDIATE").unwrap();
                h.pty.child.bytes.extend(detail_rewrite("refreshed"));
                assert!(matches!(quiesce_once(h), InputDrain::Pending));
                assert_eq!(blocked_reason(h, ids), "observation evidence pending");
                neutral_output_flood(h);
                assert!(matches!(quiesce_once(h), InputDrain::Pending));
                assert_eq!(
                    blocked_reason(h, ids),
                    OUTPUT_ARRIVING,
                    "inspection reports the latest drain's reason"
                );
                let refused = refused_reason(h, &input);
                assert!(
                    refused.contains(&format!("permission request is blocked: {OUTPUT_ARRIVING}")),
                    "{refused:?}"
                );
                assert_eq!(counts(h), [0, 0, 0]);
                lock.execute_batch("COMMIT").unwrap();
                drop(lock);
                h.drain();
            });
        }

        #[test]
        fn permission_pending_admission_drain_refuses_respond_before_any_receipt() {
            with_inspection(|h, _, _| {
                let ids = stable_form(h);
                let inspected = inspect(h, ids);
                let input = command(
                    h,
                    &inspected,
                    "response:pending-admission",
                    PermissionDecision::Allow,
                );
                neutral_output_flood(h);
                assert!(matches!(quiesce_once(h), InputDrain::Pending));
                let failure = h
                    .respond(&input)
                    .err()
                    .expect("a Pending admission drain refuses the response");
                assert!(
                    failure
                        .source
                        .message()
                        .contains(&format!("permission request is blocked: {OUTPUT_ARRIVING}")),
                    "{:?}",
                    failure.source.message()
                );
                assert_eq!(
                    (failure.run_id, failure.interaction_id, failure.input_id),
                    (Some(h.fixture.run.run_id), Some(ids.1), None)
                );
                assert_eq!(counts(h), [0, 0, 0]);
                assert!(h.pty.offers.is_empty());
                let settled = inspect(h, ids);
                assert_eq!(
                    settled.request_token, inspected.request_token,
                    "the neutral output changed no request state"
                );
                assert!(
                    h.respond(&input).unwrap().is_none(),
                    "a Quiescent admission drain admits the same response"
                );
            });
        }

        #[test]
        fn permission_retained_key_replays_without_a_drain_or_typing_while_blocked() {
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let ids = stable_form(h);
                let inspected = inspect(h, ids);
                let input = command(
                    h,
                    &inspected,
                    "response:retained",
                    PermissionDecision::Allow,
                );
                assert!(h.respond(&input).unwrap().is_none());
                let id = arm_stage(h, 1);
                h.drain();
                assert!(h.advance().is_none());
                arm_stage(h, 2);
                h.drain();
                let submitted = response(h.advance().expect("actual final response stage"));
                assert_eq!(submitted.submission.input_id, Some(id));
                let offers = h.pty.offers.clone();
                h.coordinator.fence(h.until).unwrap();
                h.pty.child.bytes.extend(b"\x1b[0m");
                let lookup = |h: &mut Host, key: &str| {
                    let mut retained = input.clone();
                    retained.operation_key = key.into();
                    let mut request = h.request("unused-response-key");
                    request.operation = InputControlOperation::PermissionRespond(retained);
                    h.coordinator.retained_permission_response(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        &request,
                        h.until,
                    )
                };
                let replay = response(
                    lookup(h, "response:retained")
                        .unwrap()
                        .expect("a retained key returns its recorded result"),
                );
                assert_eq!(
                    (
                        replay.submission.status,
                        replay.submission.stage,
                        replay.submission.input_id
                    ),
                    (
                        SubmissionStatus::Accepted,
                        SubmissionStage::Submitted,
                        Some(id)
                    )
                );
                assert_eq!(h.pty.offers, offers, "the replay types nothing");
                assert_eq!(
                    h.pty.child.bytes.len(),
                    4,
                    "the replay performs no admission drain"
                );
                assert!(
                    lookup(h, "response:absent").unwrap().is_none(),
                    "an absent key proceeds to the admission drain"
                );
                assert_eq!(counts(h), [1, 1, 1]);
            });
        }

        #[test]
        fn permission_rearm_reports_the_named_blocked_reason_before_live_validation() {
            for (block, reason) in [
                (
                    (|h: &mut Host| h.coordinator.fence(h.until).unwrap()) as fn(&mut Host),
                    "permission input is fenced",
                ),
                (
                    |h: &mut Host| {
                        h.coordinator.manual_generation = u64::MAX;
                        assert!(h.coordinator.observe_manual(1).is_err());
                        assert!(h.coordinator.manual_accounting_failed);
                    },
                    "manual input accounting failed",
                ),
            ] {
                with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                    let (input, id) = terminal_zero(h, "response:rearm-blocked");
                    let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                    assert!(
                        h.respond(&input).unwrap().is_none(),
                        "exact current Ready request enters live revalidation"
                    );
                    block(h);
                    h.drain();
                    let failure = h
                        .try_advance()
                        .err()
                        .expect("a blocked rearm refuses its stage");
                    assert!(
                        failure
                            .source
                            .message()
                            .contains(&format!("permission request is blocked: {reason}")),
                        "{reason}: {:?}",
                        failure.source.message()
                    );
                    assert_eq!(failure.input_id, Some(id));
                    assert_eq!(
                        storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                        original
                    );
                    assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                });
            }
        }

        #[test]
        fn permission_response_fresh_admission_rejects_revoked_token_then_reserves_selected_deny_plan()
         {
            with_redacted_inspection(|h, _, _| {
                let ids = stable_detailed_form(h, "opened");
                let inspected = inspect(h, ids);
                let stale = command(h, &inspected, "response:stale", PermissionDecision::Allow);
                h.human.bytes.push_back(b'!');
                h.drain();
                let failure = match h.respond(&stale) {
                    Err(failure) => failure,
                    Ok(_) => panic!("revoked token refuses the fresh response"),
                };
                assert_eq!(
                    (failure.run_id, failure.interaction_id, failure.input_id),
                    (Some(h.fixture.run.run_id), Some(ids.1), None)
                );
                assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(stale.operation_key.as_str())
                );
                assert!(!failure.source.message().contains(&stale.request_token));
                assert_eq!(counts(h), [0, 0, 0]);
                assert!(h.pty.program.is_empty());
                assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
                let before_gap = inspect(h, ids);
                let before_gap = command(
                    h,
                    &before_gap,
                    "response:before-gap",
                    PermissionDecision::Allow,
                );
                h.fixture.conn.execute_batch("CREATE TRIGGER reject_response_refresh BEFORE UPDATE OF evidence_json ON member_interactions BEGIN SELECT RAISE(ABORT,'owned response refresh failure'); END").unwrap();
                let generation = h.controller.request_generation();
                h.pty.child.bytes.extend(detail_rewrite("refreshed"));
                h.coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        transport.quiesce_for_input(
                            &mut h.human,
                            &mut h.pty,
                            &mut h.output,
                            &mut h
                                .controller
                                .observer(&h.writer, &crate::runtime::runner::RealObservationClock),
                            h.until,
                        )
                    })
                    .unwrap();
                loop {
                    let status = h.writer.status();
                    if status.degraded {
                        assert!(status.pending_evidence);
                        assert!(
                            status
                                .diagnostic
                                .as_deref()
                                .unwrap()
                                .contains("owned response refresh failure")
                        );
                        break;
                    }
                    assert!(
                        Instant::now() < h.until,
                        "actual refresh failure is observed"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                h.controller
                    .poll(
                        &h.transport.snapshot(),
                        &h.writer,
                        &crate::runtime::runner::RealObservationClock,
                    )
                    .unwrap();
                assert!(h.controller.status().gap_count > 0);
                assert!(h.controller.current_request().is_none());
                assert!(h.controller.request_generation() > generation);
                let during_gap = h.controller.health(&h.writer);
                let ordinary_key = "prompt:recovered-writer";
                let ordinary = h.request(ordinary_key);
                let InputControlOperation::SubmitPrompt(ordinary_input) = &ordinary.operation
                else {
                    panic!("existing ordinary fixture supplies a Prompt request")
                };
                let ordinary_literal = ordinary_input.literal.clone();
                let held = h.coordinator.handle_received(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    ordinary,
                    &h.controller,
                    h.until,
                );
                let removed = h
                    .fixture
                    .conn
                    .execute_batch("DROP TRIGGER reject_response_refresh");
                assert!(
                    removed.is_ok(),
                    "primary={:?}; trigger_cleanup={removed:?}",
                    held.as_ref().err()
                );
                assert!(during_gap.degraded);
                assert!(matches!(during_gap.eligibility, InputEligibility::Hold(_)));
                let Some(InputControlReply::Submission(held)) = held.unwrap() else {
                    panic!("actual writer gap returns the ordinary Held reply")
                };
                assert_eq!(
                    (held.status, held.stage, held.input_id),
                    (SubmissionStatus::Held, SubmissionStage::ZeroBytes, None)
                );
                assert!(h.coordinator.pending.is_none());
                assert!(
                    storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
                while h.writer.status().degraded {
                    assert!(
                        Instant::now() < h.until,
                        "original writer recovers in budget"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                loop {
                    h.drain();
                    if h.controller.current_request().is_some() {
                        break;
                    }
                    assert!(
                        Instant::now() < h.until,
                        "complete request is observed again"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                let recovered = h.controller.health(&h.writer);
                assert!(h.controller.status().gap_count > 0);
                assert!(recovered.gap_count > 0);
                assert!(!recovered.degraded);
                assert_eq!(recovered.pending_count, 0);
                assert!(!recovered.pending_evidence);
                assert!(recovered.diagnostic.is_none());
                let proof = h
                    .transport
                    .take_input_drain()
                    .expect("actual recovered transport installs a current quiet proof");
                h.transport
                    .validate_observation_drain(&proof, h.until)
                    .unwrap();
                assert!(h.respond(&before_gap).is_err());
                assert_eq!(counts(h), [0, 0, 0]);
                assert_eq!(h.pty.offers, vec![b"!".to_vec()]);
                let fresh = inspect(h, ids);
                let input = command(h, &fresh, "response:fresh-deny", PermissionDecision::Deny);
                assert_ne!(input.request_token, stale.request_token);
                assert_ne!(input.request_token, before_gap.request_token);
                assert!(
                    h.respond(&input).unwrap().is_none(),
                    "fresh supported response enters the actual pending writer"
                );
                assert_eq!(counts(h), [0, 0, 0]);
                let id = arm_stage(h, 1);
                assert_eq!(counts(h), [1, 1, 1]);
                assert_eq!(
                    audit(h, id, &input, "inspection-deny-plan").input.state,
                    storage::InputState::InProgress
                );
                h.drain();
                assert!(
                    h.advance().is_none(),
                    "first complete answer stage is not complete submission"
                );
                let checkpoint = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (checkpoint.stage, checkpoint.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                arm_stage(h, 2);
                h.drain();
                let result = response(h.advance().expect("actual final response stage"));
                assert_eq!(
                    (
                        result.submission.status,
                        result.submission.stage,
                        result.submission.input_id
                    ),
                    (
                        SubmissionStatus::Accepted,
                        SubmissionStage::Submitted,
                        Some(id)
                    )
                );
                assert_eq!(
                    (
                        result.interaction_id,
                        result.event_id,
                        result.decision,
                        result.decision_source
                    ),
                    (
                        input.interaction_id,
                        input.event_id,
                        input.decision,
                        input.decision_source
                    )
                );
                assert_eq!(
                    h.pty.offers,
                    vec![b"!".to_vec(), b"n".to_vec(), b"\r".to_vec()]
                );
                assert_eq!(
                    audit(h, id, &input, "inspection-deny-plan").input.state,
                    storage::InputState::Accepted
                );
                let notified: Option<String> = h
                    .fixture
                    .conn
                    .query_row(
                        "SELECT notified_at FROM messages WHERE message_id=?1",
                        [ids.2],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert!(notified.is_none());
                let before_composer = h.transport.snapshot().screen_seq;
                h.pty
                    .child
                    .bytes
                    .extend(b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end");
                loop {
                    h.drain();
                    let status = h.controller.status();
                    let health = h.controller.health(&h.writer);
                    if status.eligibility == InputEligibility::ComposerEmpty
                        && status.visibility == Visibility::NotVisible
                        && !status.retained_transition
                        && !status.retained_draft
                        && !status.retained_evidence
                        && status.diagnostic.is_none()
                        && !health.degraded
                        && health.pending_count == 0
                        && !health.pending_evidence
                    {
                        break;
                    }
                    assert!(
                        Instant::now() < h.until,
                        "actual recovered writer observes complete ordinary composition"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert!(h.transport.snapshot().screen_seq > before_composer);
                assert!(h.controller.status().gap_count > 0);
                let current = h.controller.health(&h.writer);
                assert!(current.gap_count > 0);
                assert_eq!(current.eligibility, InputEligibility::ComposerEmpty);
                assert_eq!(current.visibility, Visibility::NotVisible);
                assert!(current.diagnostic.is_none());
                let proof = h
                    .transport
                    .take_input_drain()
                    .expect("real recovered composer supplies a complete quiet proof");
                h.transport
                    .validate_observation_drain(&proof, h.until)
                    .unwrap();
                assert!(
                    storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                        .unwrap()
                        .is_none()
                );
                assert!(h.coordinator.pending.is_none());
                assert_eq!(
                    h.pty.offers,
                    vec![b"!".to_vec(), b"n".to_vec(), b"\r".to_vec()]
                );
                let ordinary = h.request(ordinary_key);
                let resumed = h
                    .coordinator
                    .handle_received(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        ordinary,
                        &h.controller,
                        h.until,
                    )
                    .unwrap();
                let projection = match resumed.as_ref() {
                    None => None,
                    Some(InputControlReply::Submission(reply)) => Some((
                        reply.status,
                        reply.stage,
                        reply.input_id,
                        reply.reason.as_deref(),
                    )),
                    Some(_) => panic!("ordinary retry has its typed submission reply"),
                };
                assert!(
                    projection.is_none(),
                    "current complete composer accepts explicit retry after writer recovery: reply={projection:?}; status={:?}",
                    h.controller.status()
                );
                for _ in 0..16 {
                    h.drain();
                    assert!(h.advance().is_none());
                    assert_eq!(
                        h.pty.offers.len(),
                        3,
                        "fresh guard precedes ordinary native input"
                    );
                    if h.hooks.count("before") == 3 {
                        break;
                    }
                }
                assert_eq!(h.hooks.count("reserved"), 2);
                assert_eq!(h.hooks.count("before"), 3);
                let reserved =
                    storage::find_input(&h.fixture.conn, h.fixture.run.run_id, ordinary_key)
                        .unwrap()
                        .unwrap();
                assert_eq!(reserved.kind, storage::InputKind::Prompt);
                assert_eq!(reserved.state, storage::InputState::InProgress);
                assert_ne!(reserved.input_id, id);
                let submitted = loop {
                    assert!(
                        Instant::now() < h.until,
                        "recovered ordinary writer completes its staged submission"
                    );
                    h.drain();
                    if let Some(reply) = h.advance() {
                        let InputControlReply::Submission(submitted) = reply else {
                            panic!(
                                "recovered ordinary writer returns its real completed submission"
                            )
                        };
                        break submitted;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                };
                assert_eq!(
                    (submitted.status, submitted.stage, submitted.input_id),
                    (
                        SubmissionStatus::Accepted,
                        SubmissionStage::Submitted,
                        Some(reserved.input_id)
                    )
                );
                assert_eq!(
                    storage::get_input(&h.fixture.conn, reserved.input_id)
                        .unwrap()
                        .state,
                    storage::InputState::Accepted
                );
                let expected_bytes = [ordinary_literal.as_bytes(), b"~"].concat();
                assert_eq!(
                    h.pty.offers,
                    vec![
                        b"!".to_vec(),
                        b"n".to_vec(),
                        b"\r".to_vec(),
                        ordinary_literal.as_bytes().to_vec(),
                        b"~".to_vec()
                    ]
                );
                let point = h
                    .coordinator
                    .retained_checkpoint(reserved.input_id)
                    .unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (
                        SubmissionStage::Submitted,
                        u64::try_from(expected_bytes.len()).unwrap()
                    )
                );
                assert!(h.controller.status().gap_count > 0);
            });
        }
        fn admitted(
            h: &mut Host,
            key: &str,
        ) -> ((i64, i64, i64, i64), PermissionRespondInput, i64) {
            let ids = stable_form(h);
            let current = inspect(h, ids);
            let input = command(h, &current, key, PermissionDecision::Allow);
            assert!(h.respond(&input).unwrap().is_none());
            let id = arm_stage(h, 1);
            audit(h, id, &input, "inspection-allow-plan");
            (ids, input, id)
        }
        fn first_stage(h: &mut Host, id: i64) {
            h.drain();
            assert!(h.advance().is_none());
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::Partial, 1)
            );
            assert_eq!(
                storage::get_input(&h.fixture.conn, id).unwrap().state,
                storage::InputState::InProgress
            );
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
        }

        fn terminal_zero(h: &mut Host, key: &str) -> (PermissionRespondInput, i64) {
            let (_, input, id) = admitted(h, key);
            h.pty.short = Some(0);
            h.drain();
            let failure = h
                .try_advance()
                .err()
                .expect("actual zero-length native write terminates its attempt");
            assert!(
                failure
                    .source
                    .message()
                    .contains("native input write returned zero bytes")
            );
            assert_eq!(failure.input_id, Some(id));
            let point = h.coordinator.retained_checkpoint(id).unwrap();
            assert_eq!(
                (point.stage, point.written_bytes),
                (SubmissionStage::ZeroBytes, 0)
            );
            assert_eq!(
                audit(h, id, &input, "inspection-allow-plan").input.state,
                storage::InputState::Ready
            );
            h.coordinator.finish_pending_attempt();
            h.pty.short = None;
            assert!(h.coordinator.pending.is_none());
            assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            (input, id)
        }

        #[test]
        fn permission_ready_actual_zero_rearms_exact_binding_and_revoked_authority_preserves_ready()
        {
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:ready-rearm");
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert!(
                    h.respond(&input).unwrap().is_none(),
                    "exact current Ready request enters live revalidation"
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    original
                );
                assert_eq!(arm_stage(h, 2), id);
                let rearmed = audit(h, id, &input, "inspection-allow-plan");
                assert_eq!(rearmed.input.state, storage::InputState::InProgress);
                assert_eq!(rearmed.response, original.response);
                assert_eq!(rearmed.input.created_at, original.input.created_at);
                assert_eq!(rearmed.input.payload_digest, original.input.payload_digest);
                assert_eq!(counts(h), [1, 1, 1]);
                h.drain();
                assert!(h.advance().is_none());
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                assert_eq!(arm_stage(h, 3), id);
                h.drain();
                let completed = response(h.advance().expect("actual final rearmed stage"));
                assert_eq!(
                    (
                        completed.submission.status,
                        completed.submission.stage,
                        completed.submission.input_id
                    ),
                    (
                        SubmissionStatus::Accepted,
                        SubmissionStage::Submitted,
                        Some(id)
                    )
                );
                let accepted = audit(h, id, &input, "inspection-allow-plan");
                assert_eq!(accepted.input.state, storage::InputState::Accepted);
                assert_eq!(accepted.response, original.response);
                assert_eq!(
                    h.pty.offers,
                    vec![b"y".to_vec(), b"y".to_vec(), b"\r".to_vec()]
                );
            });
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:ready-revoked");
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                h.human.bytes.push_back(b'!');
                h.drain();
                let failure = h
                    .respond(&input)
                    .err()
                    .expect("actual manual generation revokes Ready authority");
                assert_eq!(
                    (failure.run_id, failure.interaction_id, failure.input_id),
                    (
                        Some(h.fixture.run.run_id),
                        Some(input.interaction_id),
                        Some(id)
                    )
                );
                assert!(h.coordinator.pending.is_none());
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    original
                );
                assert_eq!(counts(h), [1, 1, 1]);
                assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"!".to_vec()]);
            });
        }

        fn receipt_reply(
            h: &mut Host,
            action: ReceiptAction,
        ) -> std::result::Result<ReceiptReply, InputFailure> {
            h.receipt(action).map(|reply| {
                let InputControlReply::Receipt(reply) = reply else {
                    panic!("actual permission receipt has its typed reply")
                };
                reply
            })
        }

        fn refuses_zero_proof(h: &mut Host, row: &storage::PermissionReceipt) {
            let tx = h
                .fixture
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let guard = h
                .coordinator
                .acquire_receipt_guard(&tx, &h.fixture.run, row, h.until)
                .unwrap();
            assert!(
                guard
                    .zero_byte_retirement(
                        &h.fixture.run.owner(),
                        row.input.input_id,
                        row.response.metadata.interaction_id
                    )
                    .is_err(),
                "actual retained uncertainty supplies exclusion but no zero-byte proof"
            );
            drop(guard);
            tx.rollback().unwrap();
            assert_eq!(
                storage::get_permission_response(&h.fixture.conn, row.input.input_id).unwrap(),
                *row
            );
        }

        fn release_ready_claim(
            h: &mut Host,
            row: &storage::PermissionReceipt,
        ) -> storage::PermissionReceipt {
            let tx = h
                .fixture
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let guard = h
                .coordinator
                .acquire_receipt_guard(&tx, &h.fixture.run, row, h.until)
                .unwrap();
            let proof = guard
                .zero_byte_retirement(
                    &h.fixture.run.owner(),
                    row.input.input_id,
                    row.response.metadata.interaction_id,
                )
                .unwrap();
            tx.rollback().unwrap();
            storage::release_permission_claim(
                &mut h.fixture.conn,
                &h.fixture.run.owner(),
                row.input.input_id,
                &proof,
                h.until,
            )
            .unwrap()
        }

        #[test]
        fn permission_ready_rearm_restores_its_released_claim_and_refuses_a_competing_claim() {
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:ready-claim-restore");
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                let released = release_ready_claim(h, &original);
                assert_eq!(released.input, original.input);
                assert_eq!(released.response, original.response);
                assert!(released.claim.is_none());
                assert!(h.respond(&input).unwrap().is_none());
                assert_eq!(arm_stage(h, 2), id);
                let rearmed = audit(h, id, &input, "inspection-allow-plan");
                assert_eq!(rearmed.input.state, storage::InputState::InProgress);
                assert_eq!(rearmed.response, original.response);
                assert_eq!(rearmed.claim, original.claim);
                assert_eq!(counts(h), [1, 1, 1]);
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                h.coordinator.finish_pending_attempt();
            });
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:ready-foreign-claim");
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                release_ready_claim(h, &original);
                let request = storage::NewPermissionResponse {
                    operation_key: "response:competing-episode-owner".into(),
                    metadata: original.response.metadata.clone(),
                };
                let cap = h
                    .coordinator
                    .current_request
                    .as_ref()
                    .unwrap()
                    .0
                    .observed
                    .answer_matches
                    .iter()
                    .find(|matched| {
                        matched.plan().capability().decision() == PermissionDecision::Allow
                    })
                    .unwrap()
                    .plan()
                    .capability();
                let storage::PermissionReservation::Reserved(competing) =
                    storage::reserve_permission_response(
                        &mut h.fixture.conn,
                        &h.fixture.run.owner(),
                        &request,
                        Some(cap),
                        h.until,
                    )
                    .unwrap()
                else {
                    panic!(
                        "separate deliberate key acquires the released claim through actual storage"
                    )
                };
                assert_ne!(competing.input.input_id, id);
                let outcome = h.respond(&input);
                let refused = match outcome {
                    Err(_) => true,
                    Ok(None) => {
                        let mut refused = false;
                        for _ in 0..16 {
                            h.drain();
                            match h.try_advance() {
                                Err(_) => {
                                    refused = true;
                                    break;
                                }
                                Ok(None) => {}
                                Ok(Some(reply)) => {
                                    let reply = response(reply);
                                    assert!(matches!(
                                        reply.submission.status,
                                        SubmissionStatus::Held | SubmissionStatus::Rejected
                                    ));
                                    assert_eq!(
                                        (reply.submission.input_id, reply.submission.stage),
                                        (Some(id), SubmissionStage::ZeroBytes)
                                    );
                                    refused = true;
                                    break;
                                }
                            }
                        }
                        refused
                    }
                    Ok(Some(reply)) => {
                        let reply = response(reply);
                        assert!(matches!(
                            reply.submission.status,
                            SubmissionStatus::Held | SubmissionStatus::Rejected
                        ));
                        assert_eq!(
                            (reply.submission.input_id, reply.submission.stage),
                            (Some(id), SubmissionStage::ZeroBytes)
                        );
                        true
                    }
                };
                assert!(
                    refused,
                    "actual foreign-claim check refuses within finite turns"
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, competing.input.input_id)
                        .unwrap(),
                    competing
                );
                let unchanged = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(unchanged.input, original.input);
                assert_eq!(unchanged.response, original.response);
                assert!(unchanged.claim.is_none());
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                h.coordinator.finish_pending_attempt();
            });
        }

        #[test]
        fn permission_receipt_possible_initial_io_keeps_claim_through_attributed_settlement_and_retirement()
         {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:possible-initial-io");
                h.drain();
                let mut guard = h
                    .coordinator
                    .acquire_write_guard(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        &mut h.transport,
                        &h.controller,
                        h.until,
                    )
                    .unwrap()
                    .unwrap();
                assert!(
                    guard
                        .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                        .is_err(),
                    "an Armed writer lease is not a receipt-exclusion proof"
                );
                guard.begin_write(&h.transport, b"y", h.until).unwrap();
                guard.mark_io_possible();
                drop(guard);
                h.coordinator.finish_pending_attempt();
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Unknown, 0)
                );
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(original.input.state, storage::InputState::InProgress);
                refuses_zero_proof(h, &original);
                assert!(matches!(receipt_reply(h, ReceiptAction::Reconcile {
                input_id: id, expected: storage::InputState::InProgress,
                disposition: storage::InputDisposition::NotSubmitted,
                evidence_ref: "fixture:attributed-possible-io".into(),
            }).unwrap(), ReceiptReply::Settled { input_id } if input_id == id));
                let settled = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(
                    settled.input.disposition,
                    Some(storage::InputDisposition::NotSubmitted)
                );
                assert_eq!(settled.response, original.response);
                assert_eq!(settled.claim, original.claim);
                refuses_zero_proof(h, &settled);
                assert!(
                    matches!(receipt_reply(h, ReceiptAction::Retire { input_id: id }).unwrap(), ReceiptReply::Retired { input_id } if input_id == id)
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    settled
                );
                refuses_zero_proof(h, &settled);
                let repeated = response(h.respond(&input).unwrap().unwrap());
                assert_eq!(
                    (repeated.submission.status, repeated.submission.stage),
                    (SubmissionStatus::Rejected, SubmissionStage::ZeroBytes)
                );
                assert!(h.pty.offers.is_empty());
            });
        }

        #[test]
        fn permission_receipt_genuine_zero_exclusion_settlement_and_claim_release_are_atomic() {
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:zero-settlement");
                let original = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                let run = storage::get_run(&h.fixture.conn, h.fixture.run.run_id).unwrap();
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                let mut other = input.clone();
                other.operation_key = "response:other-pending".into();
                assert!(h.respond(&other).unwrap().is_none());
                let pending = h.coordinator.pending.as_ref().unwrap();
                let pending_before = (
                    pending.request.operation_key.clone(),
                    pending.phase,
                    pending.deadline,
                    pending.input_id,
                );
                let ReceiptReply::Present {
                    input_id,
                    submission,
                } = receipt_reply(
                    h,
                    ReceiptAction::Query {
                        operation_key: input.operation_key.clone(),
                    },
                )
                .unwrap()
                else {
                    panic!("durable Ready receipt remains present alongside another pending writer")
                };
                assert_eq!(input_id, id);
                assert_eq!(
                    (submission.status, submission.stage, submission.output_seq),
                    (
                        SubmissionStatus::Held,
                        SubmissionStage::ZeroBytes,
                        Some(point.output_seq)
                    )
                );
                let reconcile = || ReceiptAction::Reconcile {
                    input_id: id,
                    expected: storage::InputState::Ready,
                    disposition: storage::InputDisposition::NotSubmitted,
                    evidence_ref: "fixture:live-zero-byte".into(),
                };
                assert!(receipt_reply(h, reconcile()).is_err());
                let pending = h.coordinator.pending.as_ref().unwrap();
                assert!(
                    (
                        pending.request.operation_key.clone(),
                        pending.phase,
                        pending.deadline,
                        pending.input_id
                    ) == pending_before
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    original
                );
                h.coordinator.finish_pending_attempt();
                let tx = h
                    .fixture
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let mut guard = h
                    .coordinator
                    .acquire_receipt_guard(&tx, &h.fixture.run, &original, h.until)
                    .unwrap();
                assert!(
                    guard.begin_write(&h.transport, b"y", h.until).is_err(),
                    "receipt exclusion grants no selected-stage write permit"
                );
                drop(guard);
                tx.rollback().unwrap();
                {
                    let tx = h
                        .fixture
                        .conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .unwrap();
                    let guard = h
                        .coordinator
                        .acquire_receipt_guard(&tx, &h.fixture.run, &original, h.until)
                        .unwrap();
                    let proof = guard
                        .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                        .unwrap();
                    tx.rollback().unwrap();
                    let request = storage::NewPermissionResponse {
                        operation_key: input.operation_key.clone(),
                        metadata: original.response.metadata.clone(),
                    };
                    assert!(
                        storage::rearm_permission_response(
                            &mut h.fixture.conn,
                            &h.fixture.run.owner(),
                            &request,
                            &proof,
                            h.until
                        )
                        .is_err(),
                        "exclusion-only proof cannot grant current-plan rearm authority"
                    );
                }
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    original
                );
                h.fixture
                    .conn
                    .execute_batch(&format!(
                        "CREATE TRIGGER reject_live_claim_delete
                 BEFORE DELETE ON member_permission_response_claims
                 WHEN OLD.input_id={id}
                 BEGIN
                   SELECT CASE WHEN EXISTS(
                     SELECT 1 FROM member_input_operations
                     WHERE input_id=OLD.input_id AND state='settled'
                       AND disposition='not_submitted'
                       AND evidence_ref='fixture:live-zero-byte'
                   ) THEN RAISE(ABORT,'owned live claim delete failure')
                     ELSE RAISE(ABORT,'owned claim delete missing prior settlement') END;
                 END"
                    ))
                    .unwrap();
                let failed = receipt_reply(h, reconcile());
                let removed = h
                    .fixture
                    .conn
                    .execute_batch("DROP TRIGGER reject_live_claim_delete");
                assert!(
                    removed.is_ok(),
                    "primary={:?}; trigger_cleanup={removed:?}",
                    failed.as_ref().err()
                );
                let failed = failed
                    .err()
                    .expect("real claim deletion failure aborts settlement");
                assert!(
                    failed
                        .source
                        .message()
                        .contains("owned live claim delete failure")
                );
                assert_eq!(failed.input_id, Some(id));
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    original
                );
                assert_eq!(storage::get_run(&h.fixture.conn, run.run_id).unwrap(), run);
                let unchanged = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (
                        unchanged.stage,
                        unchanged.written_bytes,
                        unchanged.output_seq
                    ),
                    (point.stage, point.written_bytes, point.output_seq)
                );
                assert!(
                    matches!(receipt_reply(h, reconcile()).unwrap(), ReceiptReply::Settled { input_id } if input_id == id)
                );
                let settled = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(settled.input.state, storage::InputState::Settled);
                assert_eq!(
                    settled.input.disposition,
                    Some(storage::InputDisposition::NotSubmitted)
                );
                assert_eq!(
                    settled.input.evidence_ref.as_deref(),
                    Some("fixture:live-zero-byte")
                );
                assert_eq!(settled.response, original.response);
                assert_eq!(settled.claim, None);
                assert!(
                    matches!(receipt_reply(h, ReceiptAction::Retire { input_id: id }).unwrap(), ReceiptReply::Retired { input_id } if input_id == id)
                );
                assert!(!h.coordinator.operations.contains_key(&id));
                let ReceiptReply::Present {
                    input_id,
                    submission,
                } = receipt_reply(
                    h,
                    ReceiptAction::Query {
                        operation_key: input.operation_key.clone(),
                    },
                )
                .unwrap()
                else {
                    panic!("retired cache retains its consumed durable operation")
                };
                assert_eq!(input_id, id);
                assert_eq!(
                    (submission.status, submission.stage, submission.output_seq),
                    (SubmissionStatus::Rejected, SubmissionStage::Unknown, None)
                );
                let repeated = response(h.respond(&input).unwrap().unwrap());
                assert_eq!(
                    (
                        repeated.submission.status,
                        repeated.submission.input_id,
                        repeated.submission.stage
                    ),
                    (
                        SubmissionStatus::Rejected,
                        Some(id),
                        SubmissionStage::ZeroBytes
                    )
                );
                let ids = stable_form(h);
                let current = inspect(h, ids);
                let fresh = command(
                    h,
                    &current,
                    "response:deliberate-after-zero-settlement",
                    PermissionDecision::Allow,
                );
                assert!(h.respond(&fresh).unwrap().is_none());
                for _ in 0..16 {
                    h.drain();
                    assert!(h.try_advance().unwrap().is_none());
                    if h.hooks.count("reserved") == 2 {
                        break;
                    }
                }
                assert_eq!(h.hooks.count("reserved"), 2);
                let next = storage::find_input(
                    &h.fixture.conn,
                    h.fixture.run.run_id,
                    &fresh.operation_key,
                )
                .unwrap()
                .expect("new deliberate key reserves after proven release");
                assert_ne!(next.input_id, id);
                let next_receipt =
                    storage::get_permission_response(&h.fixture.conn, next.input_id).unwrap();
                assert_eq!(next_receipt.claim.as_ref().unwrap().input_id, next.input_id);
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    settled
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                h.coordinator.finish_pending_attempt();
            });
        }

        #[test]
        fn permission_receipt_cacheless_ready_proof_is_borrowed_and_not_an_attributed_disposition()
        {
            trait AmbiguousIfSend<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfSend<()> for T {}
            struct SendType;
            impl<T: ?Sized + Send> AmbiguousIfSend<SendType> for T {}
            trait AmbiguousIfClone<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfClone<()> for T {}
            struct CloneType;
            impl<T: Clone> AmbiguousIfClone<CloneType> for T {}
            trait AmbiguousIfSerialize<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
            struct SerializeType;
            impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<SerializeType> for T {}
            let _ = <InputGuard<'static> as AmbiguousIfSend<_>>::check;
            let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfSend<_>>::check;
            let _ = <InputGuard<'static> as AmbiguousIfClone<_>>::check;
            let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfClone<_>>::check;
            let _ = <InputGuard<'static> as AmbiguousIfSerialize<_>>::check;
            let _ = <storage::ZeroByteRetirement<'static> as AmbiguousIfSerialize<_>>::check;
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:cacheless-zero");
                let ready = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                h.coordinator.fence(h.until).unwrap();
                let mut empty = InputCoordinator::new(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    h.coordinator.lock.clone(),
                    InputServices {
                        catalog: ProfileCatalog::from_definitions(&inspection_fixture_definition())
                            .unwrap(),
                        token_source: Box::new(super::ordinary_prompt::support::PanicTokens(
                            "receipt-only proof",
                        )),
                        hooks: h.hooks.clone(),
                    },
                    h.until,
                )
                .unwrap();
                assert!(empty.operations.is_empty());
                {
                    let tx = h
                        .fixture
                        .conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .unwrap();
                    let guard = empty
                        .acquire_receipt_guard(&tx, &h.fixture.run, &ready, h.until)
                        .unwrap();
                    assert!(
                        guard
                            .zero_byte_retirement(
                                &h.fixture.run.owner(),
                                id + 1,
                                input.interaction_id
                            )
                            .is_err()
                    );
                    let _proof = guard
                        .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                        .unwrap();
                    tx.rollback().unwrap();
                }
                storage::settle_input(
                    &mut h.fixture.conn,
                    &h.fixture.run.owner(),
                    id,
                    storage::InputState::Ready,
                    storage::InputDisposition::NotSubmitted,
                    "fixture:attributed-only",
                )
                .unwrap();
                let attributed = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert!(attributed.claim.is_some());
                let tx = h
                    .fixture
                    .conn
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
                let guard = empty
                    .acquire_receipt_guard(&tx, &h.fixture.run, &attributed, h.until)
                    .unwrap();
                assert!(
                    guard
                        .zero_byte_retirement(&h.fixture.run.owner(), id, input.interaction_id)
                        .is_err(),
                    "cache-less NotSubmitted label supplies no byte proof"
                );
                drop(guard);
                tx.rollback().unwrap();
                let mut request = h.request("unused-cacheless-retire");
                request.operation =
                    InputControlOperation::Receipt(ReceiptAction::Retire { input_id: id });
                assert!(
                    matches!(empty.handle_received(&mut h.fixture.conn, &h.fixture.run, request, &h.controller, h.until).unwrap(), Some(InputControlReply::Receipt(ReceiptReply::Retired { input_id })) if input_id == id)
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    attributed
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            });
        }
        struct StageIo<'a> {
            inner: &'a mut Pty,
            fault: Option<io::ErrorKind>,
        }
        impl ByteInput for StageIo<'_> {
            fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.inner.try_read(bytes)
            }
        }
        impl PtyIo for StageIo<'_> {
            fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(kind) = self.fault.take() {
                    assert!(!bytes.is_empty());
                    self.inner.offers.push(bytes.to_vec());
                    return Err(kind.into());
                }
                self.inner.try_write(bytes)
            }
            fn resize(&mut self, size: TerminalSize) -> Result<()> {
                self.inner.resize(size)
            }
            fn close(&mut self) -> Result<()> {
                self.inner.close()
            }
        }
        fn pending_syscall(h: &mut Host, kind: io::ErrorKind) {
            h.drain();
            let mut io = StageIo {
                inner: &mut h.pty,
                fault: Some(kind),
            };
            let result = h.coordinator.advance(
                &mut h.fixture.conn,
                &h.fixture.run,
                &mut h.transport,
                &mut io,
                &h.controller,
                h.until,
            );
            assert!(
                io.fault.is_none(),
                "real selected-stage PtyIo call consumes this fault"
            );
            assert!(result.unwrap().is_none());
        }

        #[test]
        fn permission_response_raced_retained_reservation_and_claim_supply_no_write_authority() {
            with_inspection(|h, _, _| {
                let ids = stable_form(h);
                let inspected = inspect(h, ids);
                let input = command(
                    h,
                    &inspected,
                    "response:reservation-race",
                    PermissionDecision::Allow,
                );
                assert!(h.respond(&input).unwrap().is_none());
                let plan = h
                    .coordinator
                    .services
                    .catalog
                    .answer_plan(
                        CodingAgent::Codex,
                        "0.0.1-synthetic",
                        "synthetic-permission",
                        input.decision,
                    )
                    .unwrap();
                let cap = plan.capability();
                let request = storage::NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: storage::PermissionResponseMetadata {
                        event_id: input.event_id,
                        interaction_id: input.interaction_id,
                        expected_owner_epoch: input.expected_owner_epoch,
                        request_token: input.request_token.clone(),
                        decision: input.decision,
                        director_member_id: input.director_member_id,
                        director_mode: input.director_mode,
                        decision_source: input.decision_source,
                        mode_evidence_ref: input.mode_evidence_ref.clone(),
                        decision_evidence_ref: input.decision_evidence_ref.clone(),
                        answer_profile_version: cap.profile_version().into(),
                        answer_rule_id: cap.answer_rule_id().into(),
                    },
                };
                let storage::PermissionReservation::Reserved(original) =
                    storage::reserve_permission_response(
                        &mut h.fixture.conn,
                        &h.fixture.run.owner(),
                        &request,
                        Some(cap),
                        h.until,
                    )
                    .unwrap()
                else {
                    panic!("independent storage wins actual fresh reservation")
                };
                h.drain();
                let result = response(h.advance().expect("race returns existing receipt"));
                assert_eq!(
                    (
                        result.submission.input_id,
                        result.submission.status,
                        result.submission.stage
                    ),
                    (
                        Some(original.input.input_id),
                        SubmissionStatus::Uncertain,
                        SubmissionStage::Unknown
                    )
                );
                assert_eq!(h.hooks.count("reserved"), 0);
                assert!(h.coordinator.operations.is_empty());
                assert!(h.coordinator.pending.is_none());
                assert!(h.pty.offers.is_empty());
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, original.input.input_id)
                        .unwrap(),
                    original
                );
                let other = command(
                    h,
                    &inspected,
                    "response:different-claimed-key",
                    PermissionDecision::Deny,
                );
                let mut outcome = h.respond(&other);
                for _ in 0..16 {
                    if !matches!(outcome, Ok(None)) {
                        break;
                    }
                    h.drain();
                    outcome = h.try_advance();
                }
                match outcome {
                    Err(failure) => assert_eq!(
                        failure.operation_key.as_deref(),
                        Some(other.operation_key.as_str())
                    ),
                    Ok(Some(reply)) => assert_ne!(
                        response(reply).submission.status,
                        SubmissionStatus::Accepted
                    ),
                    Ok(None) => {
                        panic!("real competing claim completes refusal within finite turns")
                    }
                }
                assert_eq!(counts(h), [1, 1, 1]);
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, original.input.input_id)
                        .unwrap(),
                    original
                );
                assert!(h.pty.offers.is_empty());
                h.coordinator.finish_pending_attempt();
                let id = original.input.input_id;
                storage::record_input_state(
                    &mut h.fixture.conn,
                    &h.fixture.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    storage::InputState::Ready,
                )
                .unwrap();
                let ready = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(ready.response, original.response);
                assert_eq!(ready.claim, original.claim);
                assert!(h.coordinator.operations.is_empty());
                let expired = Instant::now() + Duration::from_millis(100);
                assert!(expired < h.until);
                let mut request = h.request("unused-cacheless-ready-expiry");
                request.operation = InputControlOperation::PermissionRespond(input.clone());
                assert!(
                    h.coordinator
                        .handle_received(
                            &mut h.fixture.conn,
                            &h.fixture.run,
                            request,
                            &h.controller,
                            expired,
                        )
                        .unwrap()
                        .is_none()
                );
                let pending = h.coordinator.pending.as_ref().unwrap();
                assert_eq!((pending.input_id, pending.deadline), (Some(id), expired));
                assert_eq!(pending.ready.as_ref().unwrap(), &ready);
                assert!(h.coordinator.operations.is_empty());
                assert!(h.pty.offers.is_empty());
                std::thread::sleep(expired.saturating_duration_since(Instant::now()));
                assert!(Instant::now() >= expired);
                let failure = h
                    .try_advance()
                    .err()
                    .expect("original pending expiration returns its primary failure");
                assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
                assert_eq!(
                    (failure.run_id, failure.interaction_id, failure.input_id),
                    (
                        Some(h.fixture.run.run_id),
                        Some(input.interaction_id),
                        Some(id)
                    )
                );
                assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(input.operation_key.as_str())
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    ready
                );
                assert!(h.coordinator.operations.is_empty());
                assert!(h.pty.offers.is_empty());
                h.coordinator.finish_pending_attempt();
                assert!(h.coordinator.pending.is_none());
                assert!(h.respond(&input).unwrap().is_none());
                assert_eq!(h.coordinator.pending.as_ref().unwrap().input_id, Some(id));
                h.coordinator.finish_pending_attempt();
                assert!(h.coordinator.pending.is_none());
                assert!(h.coordinator.operations.is_empty());
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    ready
                );
                assert!(h.pty.offers.is_empty());
                assert_eq!(h.hooks.count("reserved"), 0);
                assert!(h.respond(&input).unwrap().is_none());
                assert_eq!(arm_stage(h, 1), id);
                h.drain();
                assert!(h.advance().is_none());
                assert_eq!(arm_stage(h, 2), id);
                h.drain();
                let accepted = response(h.advance().expect("fresh explicit rearm remains usable"));
                assert_eq!(
                    (accepted.submission.input_id, accepted.submission.status),
                    (Some(id), SubmissionStatus::Accepted)
                );
                let completed = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                assert_eq!(completed.response, ready.response);
                assert_eq!(completed.claim, ready.claim);
                assert_eq!(completed.input.state, storage::InputState::Accepted);
                assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"\r".to_vec()]);
            });
        }

        #[test]
        fn permission_response_later_pending_preserves_partial_and_retries_only_untouched_stage() {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:later-pending");
                first_stage(h, id);
                let original = h.coordinator.pending.as_ref().unwrap().deadline;
                for (index, kind) in [io::ErrorKind::WouldBlock, io::ErrorKind::Interrupted]
                    .into_iter()
                    .enumerate()
                {
                    arm_stage(h, index + 2);
                    pending_syscall(h, kind);
                    let point = h.coordinator.retained_checkpoint(id).unwrap();
                    assert_eq!(
                        (point.stage, point.written_bytes),
                        (SubmissionStage::Partial, 1)
                    );
                    assert_eq!(h.coordinator.pending.as_ref().unwrap().deadline, original);
                    assert_eq!(
                        audit(h, id, &input, "inspection-allow-plan").input.state,
                        storage::InputState::InProgress
                    );
                    let offers = h.pty.offers.len();
                    assert!(h.try_advance().unwrap().is_none());
                    assert_eq!(
                        h.pty.offers.len(),
                        offers,
                        "consumed guard requires a new real drain"
                    );
                }
                arm_stage(h, 4);
                h.drain();
                let result = response(h.advance().expect("last untouched stage completes"));
                assert_eq!(
                    (result.submission.status, result.submission.stage),
                    (SubmissionStatus::Accepted, SubmissionStage::Submitted)
                );
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(point.written_bytes, 2);
                assert_eq!(
                    h.pty.offers,
                    vec![
                        b"y".to_vec(),
                        b"\r".to_vec(),
                        b"\r".to_vec(),
                        b"\r".to_vec()
                    ]
                );
                let after = h
                    .hooks
                    .points
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|p| p.boundary == "after")
                    .map(|p| p.count)
                    .collect::<Vec<_>>();
                assert_eq!(after, vec![1, 1, 1, 2]);
                assert_eq!(
                    audit(h, id, &input, "inspection-allow-plan").input.state,
                    storage::InputState::Accepted
                );
            });
        }

        #[test]
        fn permission_response_changed_focus_after_positive_stage_stops_remaining_bytes() {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:changed-focus");
                first_stage(h, id);
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other");
                let outcome = h.try_advance();
                match outcome {
                    Err(failure) => {
                        assert_eq!(failure.input_id, Some(id));
                        assert_eq!(failure.interaction_id, Some(input.interaction_id));
                    }
                    Ok(Some(reply)) => assert_eq!(
                        response(reply).submission.status,
                        SubmissionStatus::Uncertain
                    ),
                    Ok(None) => {
                        panic!("incompatible positive attempt terminates without another stage")
                    }
                }
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                assert_eq!(
                    audit(h, id, &input, "inspection-allow-plan").input.state,
                    storage::InputState::Uncertain
                );
            });
        }

        #[test]
        fn permission_response_positive_short_later_stage_keeps_cumulative_uncertainty_without_tail()
         {
            let mut definition = inspection_fixture_definition();
            definition["profiles"][0]["capabilities"]["answers"][0]["stages"][1]["bytes_hex"] =
                serde_json::json!("6f6b");
            with_definition(definition, b"ok".to_vec(), |h, _, _| {
                let (_, input, id) = admitted(h, "response:short-later-stage");
                first_stage(h, id);
                arm_stage(h, 2);
                h.pty.short = Some(1);
                h.drain();
                let result = response(h.advance().expect("positive short ends the answer plan"));
                assert_eq!(
                    (
                        result.submission.status,
                        result.submission.stage,
                        result.submission.input_id
                    ),
                    (
                        SubmissionStatus::Uncertain,
                        SubmissionStage::Partial,
                        Some(id)
                    )
                );
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 2)
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"ok".to_vec()]);
                assert_eq!(
                    audit(h, id, &input, "inspection-allow-plan").input.state,
                    storage::InputState::Uncertain
                );
                assert!(h.try_advance().unwrap().is_none());
                assert_eq!(h.pty.offers.len(), 2);
                let row = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                refuses_zero_proof(h, &row);
            });
        }

        #[test]
        fn permission_response_unrecorded_possible_later_io_and_abandonment_preserve_prior_evidence()
         {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:abandoned-later-io");
                first_stage(h, id);
                arm_stage(h, 2);
                h.drain();
                let row = audit(h, id, &input, "inspection-allow-plan");
                let mut guard = h
                    .coordinator
                    .acquire_write_guard(
                        &mut h.fixture.conn,
                        &h.fixture.run,
                        &mut h.transport,
                        &h.controller,
                        h.until,
                    )
                    .unwrap()
                    .expect("genuine later-stage guard");
                guard.begin_write(&h.transport, b"\r", h.until).unwrap();
                guard.mark_io_possible();
                drop(guard);
                h.coordinator.finish_pending_attempt();
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                assert_eq!(
                    storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                    row
                );
                refuses_zero_proof(h, &row);
                assert!(h.coordinator.pending.is_none());
                assert!(h.try_advance().unwrap().is_none());
                let repeated = response(
                    h.respond(&input)
                        .unwrap()
                        .expect("durable InProgress returns uncertainty"),
                );
                assert_eq!(
                    (repeated.submission.input_id, repeated.submission.status),
                    (Some(id), SubmissionStatus::Uncertain)
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            });
        }

        #[test]
        fn permission_response_accepted_bookkeeping_failure_keeps_known_context_claim_and_actual_counts()
         {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:accepted-failure");
                first_stage(h, id);
                arm_stage(h, 2);
                h.fixture.conn.execute_batch("CREATE TRIGGER owned_response_accept_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.kind='permission_response' AND NEW.state='accepted' BEGIN SELECT RAISE(ABORT,'owned response accepted failure'); END;").unwrap();
                h.drain();
                let failure = match h.try_advance() {
                    Err(failure) => failure,
                    Ok(_) => panic!("real Accepted trigger failure remains an error"),
                };
                h.fixture
                    .conn
                    .execute_batch("DROP TRIGGER owned_response_accept_failure;")
                    .unwrap();
                assert!(
                    failure
                        .source
                        .message()
                        .contains("owned response accepted failure")
                );
                assert_eq!(
                    (failure.run_id, failure.interaction_id, failure.input_id),
                    (
                        Some(h.fixture.run.run_id),
                        Some(input.interaction_id),
                        Some(id)
                    )
                );
                assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(input.operation_key.as_str())
                );
                assert!(!failure.source.message().contains(&input.request_token));
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Submitted, 2)
                );
                assert_eq!(
                    audit(h, id, &input, "inspection-allow-plan").input.state,
                    storage::InputState::InProgress
                );
                h.coordinator.finish_pending_attempt();
                assert_eq!(h.pty.offers, vec![b"y".to_vec(), b"\r".to_vec()]);
                let repeated = response(
                    h.respond(&input)
                        .unwrap()
                        .expect("failed commit is retained uncertainty"),
                );
                assert_eq!(repeated.submission.status, SubmissionStatus::Uncertain);
                assert_eq!(h.pty.offers.len(), 2);
            });
        }

        mod gated {
            use super::super::permission_inspection_contracts::with_opencode_inspection;
            use super::*;
            use crate::coding_agent::profiles::catalog::test_support::{
                gated_fixture_definition, gated_fixture_form, gated_reason_dialog,
            };
            use std::sync::atomic::{AtomicUsize, Ordering};

            const IDLE_COMPOSER: &[u8] = b"\x1b[0m\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
            const OPERATION: &str = "PRIVATE_GATED_OPERATION";
            type Outcome = std::result::Result<Option<PermissionResponseResult>, InputFailure>;

            fn with_gated(body: impl FnOnce(&mut Host, Arc<AtomicUsize>)) {
                with_definition(gated_fixture_definition(), b"\r".to_vec(), |h, calls, _| {
                    body(h, calls)
                });
            }

            fn gated_form(h: &mut Host, focus: &str) -> (i64, i64, i64, i64) {
                h.show(gated_fixture_form(OPERATION, focus));
                settled_ids(h)
            }

            fn admit(
                h: &mut Host,
                ids: (i64, i64, i64, i64),
                key: &str,
                decision: PermissionDecision,
                deadline: Instant,
            ) -> PermissionRespondInput {
                let inspected = inspect(h, ids);
                let input = command(h, &inspected, key, decision);
                assert!(h.respond_until(&input, deadline).unwrap().is_none());
                input
            }

            fn write_stage(h: &mut Host, before: usize) -> i64 {
                let id = arm_stage(h, before);
                h.drain();
                assert!(
                    h.advance().is_none(),
                    "a nonfinal stage keeps the response open"
                );
                assert_eq!(h.hooks.count("after"), before);
                id
            }

            // Drives the pending response until it ends or `until` passes; `None` means still open.
            fn settle(h: &mut Host, until: Instant) -> Outcome {
                loop {
                    h.drain();
                    match h.try_advance() {
                        Ok(None) if Instant::now() >= until => return Ok(None),
                        Ok(None) => std::thread::sleep(Duration::from_millis(2)),
                        Ok(Some(reply)) => return Ok(Some(response(reply))),
                        Err(failure) => return Err(failure),
                    }
                }
            }

            fn assert_uncertain(h: &Host, id: i64, outcome: Outcome) {
                match outcome {
                    Err(failure) => assert_eq!(failure.input_id, Some(id)),
                    Ok(Some(result)) => {
                        assert_eq!(result.submission.status, SubmissionStatus::Uncertain)
                    }
                    Ok(None) => panic!("the response ends instead of waiting"),
                }
                assert_eq!(
                    storage::get_input(&h.fixture.conn, id).unwrap().state,
                    storage::InputState::Uncertain
                );
            }

            fn opened(h: &Host) -> i64 {
                h.fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                    [h.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap()
            }

            fn events(h: &Host) -> i64 {
                h.fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                        [h.fixture.run.run_id],
                        |row| row.get(0),
                    )
                    .unwrap()
            }

            fn interaction_open(h: &Host, interaction_id: i64) -> bool {
                h.fixture
                    .conn
                    .query_row(
                        "SELECT closed_at IS NULL FROM member_interactions WHERE interaction_id=?1",
                        [interaction_id],
                        |row| row.get(0),
                    )
                    .unwrap()
            }

            fn offers(values: &[&[u8]]) -> Vec<Vec<u8>> {
                values.iter().map(|value| value.to_vec()).collect()
            }

            fn soon(millis: u64) -> Instant {
                Instant::now() + Duration::from_millis(millis)
            }

            // Writes deny stages one and two on the synthetic form, leaving the response open.
            fn deny_through_stage_two(
                h: &mut Host,
                key: &str,
                deadline: Instant,
            ) -> (PermissionRespondInput, i64) {
                let ids = gated_form(h, "allow");
                let input = admit(h, ids, key, PermissionDecision::Deny, deadline);
                let id = write_stage(h, 1);
                h.show(gated_fixture_form(OPERATION, "deny"));
                write_stage(h, 2);
                assert_eq!(h.pty.offers, offers(&[b"h", b"\r"]));
                (input, id)
            }

            #[test]
            fn gated_allow_writes_its_stage_on_the_matching_focus() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "allow");
                    let input = admit(
                        h,
                        ids,
                        "response:gated-allow",
                        PermissionDecision::Allow,
                        h.until,
                    );
                    let id = arm_stage(h, 1);
                    h.drain();
                    let result = response(h.advance().expect("the single gated stage completes"));
                    assert_eq!(
                        (result.submission.status, result.submission.stage),
                        (SubmissionStatus::Accepted, SubmissionStage::Submitted)
                    );
                    assert_eq!(h.pty.offers, offers(&[b"\r"]));
                    assert_eq!(
                        audit(h, id, &input, "synthetic-gated-allow").input.state,
                        storage::InputState::Accepted
                    );
                });
            }

            #[test]
            fn gated_deny_rebinds_each_expected_transition_without_a_new_token() {
                with_gated(|h, calls| {
                    let ids = gated_form(h, "allow");
                    let input = admit(
                        h,
                        ids,
                        "response:gated-deny",
                        PermissionDecision::Deny,
                        h.until,
                    );
                    let minted = calls.load(Ordering::SeqCst);
                    let id = write_stage(h, 1);
                    let point = h.coordinator.retained_checkpoint(id).unwrap();
                    assert_eq!(
                        (point.stage, point.written_bytes),
                        (SubmissionStage::Partial, 1)
                    );
                    assert_eq!(
                        storage::get_input(&h.fixture.conn, id).unwrap().state,
                        storage::InputState::InProgress
                    );
                    h.show(gated_fixture_form(OPERATION, "deny"));
                    write_stage(h, 2);
                    assert_eq!(h.pty.offers, offers(&[b"h", b"\r"]));
                    let before = events(h);
                    h.show(gated_reason_dialog(None));
                    assert_eq!(events(h), before, "the continuation emits no event");
                    assert!(interaction_open(h, ids.1));
                    arm_stage(h, 3);
                    h.drain();
                    let result = response(h.advance().expect("the continuation stage completes"));
                    assert_eq!(
                        (result.submission.status, result.submission.stage),
                        (SubmissionStatus::Accepted, SubmissionStage::Submitted)
                    );
                    assert_eq!(h.pty.offers, offers(&[b"h", b"\r", b"\r"]));
                    assert_eq!(
                        h.coordinator.retained_checkpoint(id).unwrap().written_bytes,
                        3
                    );
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        minted,
                        "rebinding between stages mints no request token"
                    );
                    assert_eq!(opened(h), 1, "every stage stays in the original episode");
                    assert_eq!(
                        audit(h, id, &input, "synthetic-gated-deny").input.state,
                        storage::InputState::Accepted
                    );
                });
            }

            #[test]
            fn gated_first_stage_is_never_written_while_its_focus_gate_is_unmet() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "deny");
                    let deadline = soon(600);
                    let inspected = inspect(h, ids);
                    let input = command(
                        h,
                        &inspected,
                        "response:gated-unmet",
                        PermissionDecision::Allow,
                    );
                    let outcome = match h.respond_until(&input, deadline) {
                        Ok(None) => settle(h, deadline + Duration::from_millis(400)),
                        Ok(Some(reply)) => Ok(Some(response(reply))),
                        Err(refused) => Err(refused),
                    };
                    assert!(h.pty.offers.is_empty(), "an unmet gate writes no byte");
                    assert!(h.pty.program.is_empty());
                    if let Ok(Some(result)) = &outcome {
                        assert_ne!(result.submission.status, SubmissionStatus::Accepted);
                    }
                    if h.hooks.count("reserved") > 0 {
                        assert_eq!(
                            storage::get_input(&h.fixture.conn, h.hooks.input_id())
                                .unwrap()
                                .state,
                            storage::InputState::Ready,
                            "a response that wrote no byte stays ready"
                        );
                    }
                });
            }

            #[test]
            fn gated_stage_waits_while_the_previous_focus_persists_until_the_deadline() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "allow");
                    let deadline = soon(900);
                    admit(
                        h,
                        ids,
                        "response:gated-wait",
                        PermissionDecision::Deny,
                        deadline,
                    );
                    let id = write_stage(h, 1);
                    let waiting = settle(h, deadline - Duration::from_millis(150));
                    assert!(
                        matches!(waiting, Ok(None)),
                        "the unchanged previous focus keeps the response waiting"
                    );
                    assert_eq!(h.pty.offers, offers(&[b"h"]));
                    assert_eq!(
                        h.hooks.count("before"),
                        1,
                        "no later stage reaches BeforeWrite"
                    );
                    assert_eq!(
                        storage::get_input(&h.fixture.conn, id).unwrap().state,
                        storage::InputState::InProgress
                    );
                    let ended = settle(h, deadline + Duration::from_millis(500));
                    assert_uncertain(h, id, ended);
                    assert_eq!(h.pty.offers, offers(&[b"h"]));
                });
            }

            #[test]
            fn gated_unexpected_change_between_stages_fails_uncertain_without_another_byte() {
                for (case, screen) in [
                    (
                        "focus on an option without a choice",
                        gated_fixture_form(OPERATION, "always"),
                    ),
                    (
                        "focus resolved to no choice",
                        gated_fixture_form(OPERATION, "none"),
                    ),
                    (
                        "a different operation",
                        gated_fixture_form("PRIVATE_OTHER_OPERATION", "deny"),
                    ),
                    ("an unexpected continuation", gated_reason_dialog(None)),
                ] {
                    with_gated(|h, _| {
                        let ids = gated_form(h, "allow");
                        let deadline = soon(3000);
                        admit(
                            h,
                            ids,
                            "response:gated-change",
                            PermissionDecision::Deny,
                            deadline,
                        );
                        let id = write_stage(h, 1);
                        h.show(screen);
                        let outcome = settle(h, deadline);
                        assert_uncertain(h, id, outcome);
                        assert_eq!(h.pty.offers, offers(&[b"h"]), "{case}");
                    });
                }
            }

            #[test]
            fn gated_human_input_between_stages_fails_uncertain_without_another_stage() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "allow");
                    let deadline = soon(3000);
                    admit(
                        h,
                        ids,
                        "response:gated-human",
                        PermissionDecision::Deny,
                        deadline,
                    );
                    let id = write_stage(h, 1);
                    h.human.bytes.push_back(b'l');
                    h.show(gated_fixture_form(OPERATION, "deny"));
                    let outcome = settle(h, deadline);
                    assert_uncertain(h, id, outcome);
                    assert_eq!(h.pty.offers, offers(&[b"h", b"l"]));
                });
            }

            #[test]
            fn gated_continuation_binds_only_by_succession_in_the_same_episode() {
                type Disturbance = fn(&mut Host);
                let cases: [(&str, Disturbance); 4] = [
                    ("human input", |h| {
                        h.human.bytes.push_back(b'x');
                        h.drain();
                    }),
                    ("an observation gap", |h| {
                        h.controller
                            .observer_failed(
                                crate::runtime::runner::ObservationFailure::Parser,
                                &crate::runtime::runner::RealObservationClock,
                            )
                            .unwrap();
                    }),
                    ("the episode closing", |h| {
                        h.show(IDLE_COMPOSER.to_vec());
                        std::thread::sleep(Duration::from_millis(300));
                        h.drain();
                    }),
                    ("a different episode", |h| {
                        h.show(gated_fixture_form("PRIVATE_OTHER_OPERATION", "deny"));
                        std::thread::sleep(Duration::from_millis(150));
                        h.drain();
                    }),
                ];
                for (case, disturb) in cases {
                    with_gated(|h, _| {
                        let deadline = soon(2500);
                        let (_, id) =
                            deny_through_stage_two(h, "response:gated-succession", deadline);
                        disturb(h);
                        h.show(gated_reason_dialog(None));
                        let outcome = settle(h, deadline);
                        assert_uncertain(h, id, outcome);
                        assert_eq!(
                            h.pty
                                .offers
                                .iter()
                                .filter(|offer| offer.as_slice() == b"\r")
                                .count(),
                            1,
                            "{case}: the continuation stage is never written"
                        );
                    });
                }
            }

            #[test]
            fn gated_continuation_with_a_reason_never_satisfies_the_screen_gate() {
                with_gated(|h, _| {
                    let deadline = soon(1500);
                    let (_, id) = deny_through_stage_two(h, "response:gated-reason", deadline);
                    h.show(gated_reason_dialog(Some("typed reason")));
                    let outcome = settle(h, deadline + Duration::from_millis(500));
                    assert_uncertain(h, id, outcome);
                    assert_eq!(h.pty.offers, offers(&[b"h", b"\r"]));
                });
            }

            #[test]
            fn gated_inspection_of_a_continuation_offers_no_token_and_keeps_the_episode() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "deny");
                    let token = inspect(h, ids).request_token;
                    assert!(token.is_some());
                    let before = events(h);
                    h.show(gated_reason_dialog(None));
                    std::thread::sleep(Duration::from_millis(150));
                    h.drain();
                    let reason = blocked_reason(h, ids);
                    assert!(!reason.is_empty());
                    assert_eq!(events(h), before, "the continuation emits no event");
                    assert!(interaction_open(h, ids.1));
                    assert_eq!(opened(h), 1);
                    assert!(h.pty.offers.is_empty());
                });
            }

            #[test]
            fn gated_styling_only_focus_change_starts_a_new_request_generation() {
                with_gated(|h, _| {
                    let ids = gated_form(h, "allow");
                    let first = inspect(h, ids);
                    let stale =
                        command(h, &first, "response:gated-stale", PermissionDecision::Deny);
                    let generation = h.controller.request_generation();
                    let moved = gated_form(h, "deny");
                    assert_eq!(moved, ids, "a focus move keeps the episode");
                    assert!(h.controller.request_generation() > generation);
                    let second = inspect(h, ids);
                    assert!(second.request_token.is_some());
                    assert_ne!(second.request_token, first.request_token);
                    assert!(h.respond(&stale).is_err(), "the pre-move token is revoked");
                    assert_eq!(opened(h), 1);
                    assert!(h.pty.offers.is_empty());
                });
            }

            fn opencode_screen(fixture: &str, lines_from: &str) -> Vec<u8> {
                let mut screen: serde_json::Value = serde_json::from_str(fixture).unwrap();
                let source: serde_json::Value = serde_json::from_str(lines_from).unwrap();
                screen["lines"] = source["lines"].clone();
                native_fixture_bytes(&screen.to_string())
            }

            #[test]
            fn opencode_allow_enters_on_the_focused_allow_once() {
                with_opencode_inspection(|h, _, _| {
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-allow-pre-stage-1",
                        ),
                    ));
                    let ids = settled_ids(h);
                    let inspected = inspect(h, ids);
                    assert!(inspected.request_token.is_some());
                    assert_eq!(
                        inspected.supported_decisions,
                        [PermissionDecision::Allow, PermissionDecision::Deny]
                    );
                    let input = command(
                        h,
                        &inspected,
                        "response:opencode-allow",
                        PermissionDecision::Allow,
                    );
                    assert!(h.respond(&input).unwrap().is_none());
                    arm_stage(h, 1);
                    h.drain();
                    let result = response(h.advance().expect("the allow stage completes"));
                    assert_eq!(result.submission.status, SubmissionStatus::Accepted);
                    assert_eq!(h.pty.offers, offers(&[b"\r"]));
                });
            }

            #[test]
            fn opencode_deny_moves_the_focus_selects_reject_and_submits_the_empty_reason() {
                with_opencode_inspection(|h, calls, _| {
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let ids = settled_ids(h);
                    let inspected = inspect(h, ids);
                    let input = command(
                        h,
                        &inspected,
                        "response:opencode-deny",
                        PermissionDecision::Deny,
                    );
                    assert!(h.respond(&input).unwrap().is_none());
                    let minted = calls.load(Ordering::SeqCst);
                    write_stage(h, 1);
                    assert_eq!(h.pty.offers, offers(&[b"h"]));
                    h.show(opencode_screen(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-2",
                        ),
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    write_stage(h, 2);
                    assert_eq!(h.pty.offers, offers(&[b"h", b"\r"]));
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-3",
                        ),
                    ));
                    assert!(interaction_open(h, ids.1));
                    arm_stage(h, 3);
                    h.drain();
                    let result = response(h.advance().expect("the empty-reason stage completes"));
                    assert_eq!(
                        (result.submission.status, result.submission.stage),
                        (SubmissionStatus::Accepted, SubmissionStage::Submitted)
                    );
                    assert_eq!(h.pty.offers, offers(&[b"h", b"\r", b"\r"]));
                    assert_eq!(calls.load(Ordering::SeqCst), minted);
                    assert_eq!(opened(h), 1);
                });
            }

            #[test]
            fn opencode_focus_on_always_allow_after_the_first_deny_stage_fails_uncertain() {
                with_opencode_inspection(|h, _, _| {
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let ids = settled_ids(h);
                    let deadline = soon(3000);
                    let input = admit(
                        h,
                        ids,
                        "response:opencode-always",
                        PermissionDecision::Deny,
                        deadline,
                    );
                    let id = write_stage(h, 1);
                    h.show(opencode_screen(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-focus-always-1",
                        ),
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let outcome = settle(h, deadline);
                    assert_uncertain(h, id, outcome);
                    assert_eq!(h.pty.offers, offers(&[b"h"]));
                    assert_eq!(
                        storage::get_permission_response(&h.fixture.conn, id)
                            .unwrap()
                            .input
                            .operation_key,
                        input.operation_key
                    );
                });
            }

            #[test]
            fn opencode_reject_dialog_reached_by_human_input_offers_no_token() {
                with_opencode_inspection(|h, _, _| {
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let ids = settled_ids(h);
                    assert!(inspect(h, ids).request_token.is_some());
                    h.show(native_fixture_bytes(
                        crate::coding_agent::profiles::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-3",
                        ),
                    ));
                    std::thread::sleep(Duration::from_millis(150));
                    h.drain();
                    blocked_reason(h, ids);
                    assert!(interaction_open(h, ids.1));
                    assert_eq!(opened(h), 1);
                });
            }
        }
    }
    mod retirement_contracts {
        use super::super::*;
        use super::ordinary_prompt::support::{Host, with_host, with_host_until};
        use crate::runtime::runner::{ReceiptAction, ReceiptReply};
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        /// Fills the receipt cache with `count` completed entries modelled on the real retained
        /// entry `template`, under input IDs no durable row uses.
        fn prefill_completed(h: &mut Host, template: i64, count: usize) {
            let real = h
                .coordinator
                .operations
                .get(&template)
                .expect("real completed template");
            let RetainedSelection::Composer { method, stages, .. } = &real.selection else {
                panic!("an ordinary prompt retains its composer selection")
            };
            let fillers = (0..count)
                .map(|index| {
                    let input_id = 1_000_000 + i64::try_from(index).unwrap();
                    let mut operation = real.operation.clone();
                    operation.input_id = input_id;
                    operation.operation_key = format!("prompt:prefilled:{index}");
                    (
                        input_id,
                        RetainedInput {
                            operation,
                            bytes: real.bytes.clone(),
                            selection: RetainedSelection::Composer {
                                method: *method,
                                stages: stages.clone(),
                                next_stage: stages.len(),
                            },
                            stage: real.stage,
                            written_bytes: real.written_bytes,
                            output_seq: real.output_seq,
                            attempt_available: false,
                            leased: false,
                        },
                    )
                })
                .collect::<Vec<_>>();
            h.coordinator.operations.extend(fillers);
        }

        fn complete(h: &mut Host, key: &str) -> i64 {
            let request = h.request(key);
            let accepted = super::ordinary_prompt::support::settle(h, request);
            assert_eq!(accepted.status, SubmissionStatus::Accepted);
            assert_eq!(accepted.stage, SubmissionStage::Submitted);
            assert!(h.coordinator.pending.is_none());
            accepted.input_id.expect("actual completed input ID")
        }

        #[test]
        fn completed_coordinator_settlement_and_retirement_exceeds_cache_capacity_without_replay() {
            let until = Instant::now() + Duration::from_secs(30);
            with_host_until(until, |h| {
                let run = h.fixture.run.clone();
                let first_key = "prompt:capacity:0";
                let first_request = h.request(first_key);
                let expected_program = "ordinary fixture λ~".as_bytes();
                let evidence = "owned completed coordinator input";
                let mut first_settled = None;
                for index in 0..3 {
                    let detailed = index == 0;
                    let id = complete(h, &format!("prompt:capacity:{index}"));
                    if index == 0 {
                        let retained = cached(h, id);
                        assert_eq!(retained.0.state, storage::InputState::Accepted);
                        assert_eq!(retained.1, SubmissionStage::Submitted);
                        assert_eq!(retained.2, u64::try_from(expected_program.len()).unwrap());
                        assert!(!retained.4 && !retained.5);
                        prefill_completed(h, id, RECEIPT_CACHE_CAPACITY - 1);
                        assert_eq!(h.coordinator.operations.len(), RECEIPT_CACHE_CAPACITY);
                        let overflow = h.request("prompt:capacity:overflow");
                        assert!(
                            h.handle_until(overflow, until).is_err(),
                            "a full receipt cache refuses a new input"
                        );
                        assert!(
                            storage::find_input(
                                &h.fixture.conn,
                                run.run_id,
                                "prompt:capacity:overflow"
                            )
                            .unwrap()
                            .is_none()
                        );
                    }
                    assert_eq!(h.coordinator.operations.len(), RECEIPT_CACHE_CAPACITY);
                    assert_eq!(h.pty.program.len(), index + 1);
                    assert_eq!(h.pty.program.last().unwrap(), expected_program);
                    assert!(h.pty.manual.is_empty());
                    let settled_reply = h
                        .receipt(ReceiptAction::Reconcile {
                            input_id: id,
                            expected: storage::InputState::Accepted,
                            disposition: storage::InputDisposition::Submitted,
                            evidence_ref: evidence.into(),
                        })
                        .unwrap();
                    assert!(
                        matches!(settled_reply, InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
                    );
                    let settled = detailed.then(|| {
                        let settled = storage::get_input(&h.fixture.conn, id).unwrap();
                        assert_eq!(settled.run_id, run.run_id);
                        assert_eq!(settled.operation_key, format!("prompt:capacity:{index}"));
                        assert_eq!(settled.kind, storage::InputKind::Prompt);
                        assert_eq!(settled.state, storage::InputState::Settled);
                        assert_eq!(
                            settled.disposition,
                            Some(storage::InputDisposition::Submitted)
                        );
                        assert_eq!(settled.evidence_ref.as_deref(), Some(evidence));
                        assert_eq!(cached(h, id).0, settled);
                        settled
                    });
                    assert_retired(
                        h.receipt(ReceiptAction::Retire { input_id: id }).unwrap(),
                        id,
                    );
                    assert_eq!(
                        h.coordinator.operations.len(),
                        RECEIPT_CACHE_CAPACITY - 1,
                        "retirement releases exactly its own cache slot"
                    );
                    assert!(h.coordinator.pending.is_none());
                    if let Some(settled) = settled {
                        assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);
                        first_settled = Some(settled);
                    }
                    h.hooks.points.lock().unwrap().clear();
                }
                let (completed, unresolved): (i64, i64) = h.fixture.conn.query_row(
                "SELECT COUNT(*),COUNT(CASE WHEN state IN ('in_progress','accepted','uncertain') THEN 1 END) FROM member_input_operations WHERE run_id=?1",
                [run.run_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
                assert_eq!((completed, unresolved), (3, 0));
                let first =
                    first_settled.expect("first real completed operation retained as history");
                let changes = h.fixture.conn.total_changes();
                let reply = h
                    .handle_until(first_request, until)
                    .unwrap()
                    .expect("settled key returns history immediately");
                let InputControlReply::Submission(replayed) = reply else {
                    panic!("historical Prompt submission reply")
                };
                assert_eq!(replayed.status, SubmissionStatus::Accepted);
                assert_eq!(replayed.stage, SubmissionStage::Submitted);
                assert_eq!(replayed.input_id, Some(first.input_id));
                assert_eq!(h.coordinator.operations.len(), RECEIPT_CACHE_CAPACITY - 1);
                assert!(h.coordinator.pending.is_none());
                assert_eq!(h.fixture.conn.total_changes(), changes);
                assert_eq!(
                    storage::get_input(&h.fixture.conn, first.input_id).unwrap(),
                    first
                );
                assert_eq!(h.pty.offers.len(), 6, "three staged inputs, never replayed");
                assert_eq!(h.pty.program.len(), 3);
                assert!(h.pty.manual.is_empty());
                assert!(h.hooks.points.lock().unwrap().is_empty());
            });
        }

        fn assert_retired(reply: InputControlReply, expected: i64) {
            assert!(
                matches!(reply, InputControlReply::Receipt(ReceiptReply::Retired { input_id }) if input_id == expected)
            );
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
                let writer = h.pending_snapshot();
                let changes = h.fixture.conn.total_changes();
                let hooks = h.hooks.points.lock().unwrap().len();
                let reply = h
                    .receipt(ReceiptAction::Query {
                        operation_key: key.into(),
                    })
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
                    h.receipt(ReceiptAction::Query {
                        operation_key: "prompt:absent-receipt".into()
                    })
                    .unwrap(),
                    InputControlReply::Receipt(ReceiptReply::Missing)
                ));
                let mut stale = h.request("unused-receipt-key");
                stale.operation = InputControlOperation::Receipt(ReceiptAction::Query {
                    operation_key: key.into(),
                });
                stale.identity.owner_epoch += 1;
                assert!(h.handle(stale).is_err());
                assert!(
                    h.receipt(ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::InProgress,
                        disposition: storage::InputDisposition::NotSubmitted,
                        evidence_ref: "fixture:active-writer-is-not-zero-proof".into(),
                    })
                    .is_err(),
                    "reconciliation preserves the actual Armed writer and its available permit"
                );
                assert!(
                    h.pending_snapshot() == writer,
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
                    h.receipt(ReceiptAction::Query {
                        operation_key: key.into()
                    })
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
                    h.receipt(ReceiptAction::Query {
                        operation_key: key.into()
                    })
                    .is_err(),
                    "a retained attempt without its actual reservation is an invariant error"
                );
                assert!(h.pending_snapshot() == writer);
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
                let InputControlReply::Submission(partial) =
                    h.advance().expect("actual partial write")
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
                let Err(stale) = h.receipt(action(storage::InputState::Accepted)) else {
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
                let failed = h.receipt(action(storage::InputState::Uncertain));
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
                    matches!(h.receipt(action(storage::InputState::Uncertain)).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
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
                    matches!(h.receipt(ReceiptAction::Reconcile { input_id: id, expected: storage::InputState::Settled, disposition: storage::InputDisposition::AbandonedUncertain, evidence_ref: "fixture:preserve-first-evidence".into() }).unwrap(), InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
                );
                assert!(
                    h.receipt(ReceiptAction::Reconcile {
                        input_id: id,
                        expected: storage::InputState::Settled,
                        disposition: storage::InputDisposition::Submitted,
                        evidence_ref: "fixture:conflicting-wire-disposition".into()
                    })
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
                }) = h
                    .receipt(ReceiptAction::Query {
                        operation_key: key.into(),
                    })
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
                    h.receipt(ReceiptAction::Retire { input_id: id }).unwrap(),
                    id,
                );
                assert!(!h.coordinator.operations.contains_key(&id));
                let InputControlReply::Receipt(ReceiptReply::Present {
                    input_id,
                    submission,
                }) = h
                    .receipt(ReceiptAction::Query {
                        operation_key: key.into(),
                    })
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
                let InputControlReply::Submission(repeat) = h.handle(repeat).unwrap().unwrap()
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
                let canonical = super::ordinary_prompt::support::canonical_preview(&message);
                let mut program = canonical.as_bytes().to_vec();
                program.push(b'~');
                h.pty.expect_program(program.clone());
                let request =
                    super::ordinary_prompt::support::preview_request(h, &message, &canonical);
                let accepted = super::ordinary_prompt::support::settle(h, request);
                assert!(matches!(accepted.status, SubmissionStatus::Accepted));
                let id = accepted.input_id.expect("durable Preview ID");
                let before = cached(h, id);
                assert!(matches!(before.1, SubmissionStage::Submitted));
                assert!(!before.4 && !before.5);
                assert!(h.coordinator.pending.is_none());
                let offers = h.pty.offers.clone();
                assert_eq!(h.pty.program, vec![program.clone()]);

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
                let settled =
                    storage::settle_preview(&mut h.fixture.conn, &run.owner(), id).unwrap();
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
                assert!(h.handle(foreign_request).is_err());
                assert_eq!(cached(h, id), before);
                assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), settled);

                let unrelated_key = "prompt:retirement-active-writer";
                let unrelated = h.request(unrelated_key);
                assert!(h.handle(unrelated).unwrap().is_none());
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
                    .handle(retirement)
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
}
