use crate::broker::MemberKind;
use crate::broker::Placement;
use crate::broker::runtime as storage;
use crate::broker::runtime::OwnerKind;
use crate::broker::runtime::RunRecord;
use crate::broker::runtime::RunStatus;
use crate::coding_agent::AnswerMatch;
use crate::coding_agent::AnswerPlan;
use crate::coding_agent::AnswerStage;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::ComposerOperation;
use crate::coding_agent::ProfileCatalog;
use crate::coding_agent::StageGate;
use crate::coding_agent::catalog::AnswerKey;
use crate::coding_agent::coding_agent;
use crate::db::bounded_by_inherited;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::pane_lock::PaneLock;
use crate::runtime::*;
use crate::time::Deadline;
use rusqlite::Connection;
use rusqlite::TransactionBehavior;
use std::collections::BTreeMap;
use std::io;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

pub(crate) trait RequestTokenSource: Send {
    fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()>;
}

const RECEIPT_CACHE_CAPACITY: usize = 4096;
pub(crate) const STAGE_SETTLE: Duration = Duration::from_millis(500);
const STAGE_WRITE_MARGIN: Duration = Duration::from_millis(250);
const DEFERRED_ATTEMPT_BUDGET: Duration = Duration::from_secs(5);
const DEFERRED_FINALIZATION: Duration = Duration::from_secs(1);
const COMPOSER_BUSY_BOUND: Duration = Duration::from_secs(180);
const POST_SUBMIT_VERIFICATION: Duration = Duration::from_secs(2);
const PASTE_CLOSE_RESERVE: Duration = Duration::from_millis(250);
const PASTE_OPEN: &[u8] = b"\x1b[200~";
const PASTE_CLOSE: &[u8] = b"\x1b[201~";
const FALLBACK_ESCAPE: u8 = 0x1b;
const FALLBACK_ENTER: u8 = 0x0d;

#[cfg(test)]
pub(crate) enum InputBoundary {
    Reserved,
    BeforeWrite,
    AfterWrite,
    TerminalState,
}

pub(crate) struct InputCheckpoint {
    #[cfg(test)]
    pub(crate) input_id: i64,
    pub(crate) stage: SubmissionStage,
    pub(crate) written_bytes: u64,
    pub(crate) output_seq: u64,
}

#[cfg(test)]
pub(crate) trait InputHooks: Send + Sync {
    fn checkpoint(
        &self,
        boundary: InputBoundary,
        state: &InputCheckpoint,
        deadline: Instant,
    ) -> Result<()>;
}

pub(crate) struct InputServices {
    pub(crate) catalog: ProfileCatalog,
    pub(crate) token_source: Box<dyn RequestTokenSource>,
    #[cfg(test)]
    pub(crate) stage_settle: Duration,
    #[cfg(test)]
    pub(crate) hooks: Arc<dyn InputHooks>,
}

pub(crate) struct InputCoordinator {
    run: RunRecord,
    backend: CodingAgent,
    lock: Arc<RunLock>,
    services: InputServices,
    manual_generation: u64,
    pointer_generation: u64,
    manual_accounting_failed: bool,
    placement: Placement,
    operations: BTreeMap<i64, RetainedInput>,
    pending: Option<PendingInput>,
    current_request: Option<(CurrentRequest, String)>,
    input_fenced: bool,
    observation_blocker: Option<String>,
    database_url: String,
    queue: std::collections::VecDeque<QueuedInput>,
    finalization: Option<PendingFinalization>,
    pending_stamp: Option<PendingStamp>,
    deferred_failed: bool,
    held_composer: Option<(i64, u64)>,
    output_seq: u64,
    bracketed_paste: bool,
}

/// An accepted deferred preview whose notification stamp and settlement storage prevented; like
/// a pending finalization it blocks later ordinary input and retries on the deferred timer.
struct PendingStamp {
    input_id: i64,
    stage: SubmissionStage,
}

/// A deferred input's terminal zero-byte transaction that storage prevented; it holds only
/// metadata, blocks every later ordinary input and retries on the deferred timer.
struct PendingFinalization {
    input_id: i64,
    expected: storage::InputState,
    reason: &'static str,
}

/// A transient storage obstacle that a later tick may clear; every other failure propagates.
fn storage_failure(cause: &CafleetError) -> bool {
    matches!(
        cause,
        CafleetError::DatabaseBusy(_) | CafleetError::DeadlineExceeded
    )
}

/// Runner-owned ordinary input accepted with a durable ready receipt; its literal stays in
/// memory only.
#[derive(Clone)]
struct QueuedInput {
    input_id: i64,
    redelivery: bool,
    stalled: bool,
    request: storage::NewInput,
    literal: String,
    operation: ComposerOperation,
    /// When the head first waited only on a composer that is not verified empty; the
    /// `idle_timeout` bound runs from here and resets whenever an interaction blocks the head.
    composer_busy_since: Option<Instant>,
    attempt_deadline: Option<Instant>,
    obstacle: &'static str,
}

type DeliverySelection = (
    storage::DeliveryMethod,
    Option<&'static str>,
    Arc<[Arc<[u8]>]>,
);

enum DeliveryChoice {
    Ready(DeliverySelection),
    Waiting,
    PlainLimit(String),
}

struct ComposerInput<'a> {
    operation: ComposerOperation,
    kind: storage::InputKind,
    message_id: Option<i64>,
    literal: &'a str,
}

struct DeferredAttempt {
    queued: QueuedInput,
    obstacle: &'static str,
}

#[derive(Clone)]
pub(crate) struct CurrentRequest {
    observed: ObservedRequest,
    interaction_id: i64,
    event_id: i64,
    message_id: i64,
    manual_generation: u64,
    pointer_generation: u64,
}
impl CurrentRequest {
    fn same_request(&self, other: &Self) -> bool {
        self.observed.request_generation == other.observed.request_generation
            && self.interaction_id == other.interaction_id
            && self.event_id == other.event_id
            && self.message_id == other.message_id
            && self.manual_generation == other.manual_generation
            && self.pointer_generation == other.pointer_generation
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
                    && matches!(
                        i.classification,
                        storage::InteractionClass::PermissionRequest
                            | storage::InteractionClass::Question
                    )
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

/// The operation-centred facts an inspected interaction lacks; a question lacks none of them.
pub(super) fn interaction_missing_evidence(
    classification: storage::InteractionClass,
    evidence: &serde_json::Value,
) -> Result<Vec<MissingEvidence>> {
    if classification == storage::InteractionClass::Question {
        storage::evidence::validate(evidence)?;
        Ok(Vec::new())
    } else {
        inspection_missing_evidence(evidence)
    }
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

fn answered_classification(key: &AnswerKey) -> storage::InteractionClass {
    match key {
        AnswerKey::Decision(_) => storage::InteractionClass::PermissionRequest,
        AnswerKey::Choice(_) => storage::InteractionClass::Question,
    }
}

fn answer_kind_mismatch(
    classification: storage::InteractionClass,
    key: &AnswerKey,
) -> Option<&'static str> {
    let question = classification == storage::InteractionClass::Question;
    match key {
        AnswerKey::Decision(_) if question => {
            Some("decision answers apply only to permission requests")
        }
        AnswerKey::Choice(_) if !question => {
            Some("choice answers apply only to question interactions")
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputPhase {
    AwaitReservation,
    AwaitBeforeWrite,
    Armed,
    Blocked,
    Closing,
    Verifying,
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
        offset: usize,
        closing: bool,
    },
    Permission {
        selection: Arc<PermissionSelection>,
        next_stage: usize,
    },
}

enum PendingReservation {
    Unreserved,
    HistoricalReady(Box<storage::PermissionReceipt>),
    Installed(i64),
}

enum PendingDelivery {
    Permission(Arc<PermissionSelection>),
    Composer(Box<ComposerDelivery>),
}

impl PendingDelivery {
    fn into_composer(self) -> Option<ComposerDelivery> {
        match self {
            Self::Composer(composer) => Some(*composer),
            Self::Permission(_) => None,
        }
    }
}

struct PendingInput {
    reservation: PendingReservation,
    request: storage::NewInput,
    bytes: Arc<[u8]>,
    delivery: PendingDelivery,
    deadline: Instant,
    phase: InputPhase,
}

struct ComposerDelivery {
    method: storage::DeliveryMethod,
    reason: Option<&'static str>,
    stages: Arc<[Arc<[u8]>]>,
    written_stages: usize,
    started: bool,
    settle_until: Option<Instant>,
    manual_generation: Option<u64>,
    pointer_generation: Option<u64>,
    stop: Option<ComposerStop>,
    stop_failure: Option<CafleetError>,
    verification: Option<PostSubmitVerification>,
    pane: Option<PaneLock>,
    literal: String,
    operation: ComposerOperation,
    deferred: Option<DeferredAttempt>,
}

#[derive(Clone, Copy)]
struct ComposerStop {
    cause: storage::StopCause,
    stage: storage::LeftStage,
    complete_prefix: bool,
}

struct PostSubmitVerification {
    until: Instant,
    output_seq: u64,
    episode_seq: Option<i64>,
}

impl ComposerDelivery {
    fn requires_empty_composer(&self) -> bool {
        self.method == storage::DeliveryMethod::Profile && !self.started
    }
}

enum OrdinaryBlocker {
    Interaction,
    ObserverUnavailable,
}

fn ordinary_blocker(status: &ObservationStatus) -> Option<OrdinaryBlocker> {
    use GateReason;
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

fn interaction_cause(status: &ObservationStatus) -> storage::StopCause {
    if status.eligibility == InputEligibility::Hold(GateReason::Candidate) {
        storage::StopCause::InteractionCandidate
    } else {
        storage::StopCause::InteractionOpen
    }
}

fn blocker_cause(status: &ObservationStatus) -> Option<storage::StopCause> {
    ordinary_blocker(status).map(|blocker| match blocker {
        OrdinaryBlocker::Interaction => interaction_cause(status),
        OrdinaryBlocker::ObserverUnavailable => storage::StopCause::ObserverUnavailable,
    })
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
    /// The durable input this attempt targets: a historical Ready receipt or its installed
    /// reservation.
    fn input_id(&self) -> Option<i64> {
        match &self.reservation {
            PendingReservation::Unreserved => None,
            PendingReservation::HistoricalReady(ready) => Some(ready.input.input_id),
            PendingReservation::Installed(id) => Some(*id),
        }
    }
    fn installed_input_id(&self) -> Option<i64> {
        match self.reservation {
            PendingReservation::Installed(id) => Some(id),
            PendingReservation::Unreserved | PendingReservation::HistoricalReady(_) => None,
        }
    }
    fn ready(&self) -> Option<&storage::PermissionReceipt> {
        match &self.reservation {
            PendingReservation::HistoricalReady(ready) => Some(&**ready),
            PendingReservation::Unreserved | PendingReservation::Installed(_) => None,
        }
    }
    fn permission(&self) -> Option<&Arc<PermissionSelection>> {
        match &self.delivery {
            PendingDelivery::Permission(selection) => Some(selection),
            PendingDelivery::Composer(_) => None,
        }
    }
    fn composer(&self) -> Option<&ComposerDelivery> {
        match &self.delivery {
            PendingDelivery::Composer(composer) => Some(&**composer),
            PendingDelivery::Permission(_) => None,
        }
    }
    fn composer_mut(&mut self) -> Option<&mut ComposerDelivery> {
        match &mut self.delivery {
            PendingDelivery::Composer(composer) => Some(&mut **composer),
            PendingDelivery::Permission(_) => None,
        }
    }
    fn deferred(&self) -> Option<&DeferredAttempt> {
        self.composer()?.deferred.as_ref()
    }
    fn deferred_mut(&mut self) -> Option<&mut DeferredAttempt> {
        self.composer_mut()?.deferred.as_mut()
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
                offset,
                closing,
            } => {
                if *closing {
                    ensure(
                        bytes == PASTE_CLOSE,
                        "paste cleanup writes only its closing marker",
                    )?;
                    self.permit = GuardPermit::ConsumedPreIo;
                    return Ok(self.deadline);
                }
                let proof = self
                    .proof
                    .as_ref()
                    .ok_or_else(|| error("native drain is absent"))?;
                ensure(
                    stages
                        .get(*next_stage)
                        .is_some_and(|stage| stage.get(*offset..) == Some(bytes)),
                    "composer stage selection changed",
                )?;
                if *method == storage::DeliveryMethod::Profile && *next_stage == 0 && *offset == 0 {
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
                transport.validate_permission(
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
                            stages,
                            next_stage,
                            offset,
                            closing,
                            ..
                        } => {
                            if *closing {
                                retained.stage = SubmissionStage::Partial;
                                self.permit = GuardPermit::ResultRecorded;
                                return Ok(());
                            }
                            *offset = 0;
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
                    if let RetainedSelection::Composer {
                        stages,
                        next_stage,
                        offset,
                        closing,
                        ..
                    } = &mut retained.selection
                        && !*closing
                        && *next_stage + 1 < stages.len()
                    {
                        *offset = offset
                            .checked_add(*count)
                            .ok_or_else(|| error("composer offset exhausted"))?;
                        retained.bytes = Arc::from(&stages[*next_stage][*offset..]);
                    }
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

#[cfg(test)]
impl InputCoordinator {
    fn set_post_submit_verification_until(&mut self, until: Instant) {
        self.pending
            .as_mut()
            .and_then(PendingInput::composer_mut)
            .and_then(|composer| composer.verification.as_mut())
            .expect("an active post-submit verification is required")
            .until = until;
    }

    /// Sets when the queued input began waiting only on a composer that is not verified empty.
    pub(crate) fn set_queued_composer_busy_since(&mut self, input_id: i64, since: &str) {
        let since = chrono::DateTime::parse_from_rfc3339(since).expect("RFC 3339 busy start");
        let elapsed = chrono::Utc::now()
            .signed_duration_since(since)
            .to_std()
            .expect("the busy start lies in the past");
        let attempt = self
            .pending
            .as_mut()
            .and_then(PendingInput::deferred_mut)
            .map(|deferred| &mut deferred.queued);
        let queued = self
            .queue
            .iter_mut()
            .chain(attempt)
            .find(|queued| queued.input_id == input_id)
            .expect("the input is queued or in its deferred attempt");
        queued.composer_busy_since = Some(Instant::now() - elapsed);
    }
}

impl InputCoordinator {
    #[cfg(not(test))]
    fn stage_settle(&self) -> Duration {
        STAGE_SETTLE
    }
    #[cfg(test)]
    fn stage_settle(&self) -> Duration {
        self.services.stage_settle
    }

    pub(super) fn inspect_received(
        &mut self,
        conn: &mut Connection,
        input: PermissionInspectInput,
        live: (&ObservationController, &mut Transport),
        deadline: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let (observation, transport) = live;
        let interaction_id = Some(input.interaction_id);
        let result =
            (|| {
                let deadline = bounded_by_inherited(conn, deadline);
                check_deadline(Instant::now(), deadline)?;
                let view = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction()?;
                    self.validate_live_run(&tx)?;
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
                let missing_evidence =
                    interaction_missing_evidence(interaction.classification, &evidence)?;
                let fresh = match transport.take_input_drain() {
                    Some(proof) => {
                        transport.validate_permission(&proof, deadline)?;
                        true
                    }
                    None => false,
                };
                let observed = observation.current_request();
                if let Some((current, _)) = &self.current_request
                    && (current.observed.request_generation != observation.request_generation()
                        || current.manual_generation != self.manual_generation
                        || current.pointer_generation != self.pointer_generation
                        || observed.is_none())
                {
                    self.current_request = None;
                }
                let mut supported_decisions = Vec::new();
                let mut supported_choices = Vec::new();
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
                        interaction_id: interaction.interaction_id,
                        event_id: view.event.event_id,
                        message_id,
                        manual_generation: self.manual_generation,
                        pointer_generation: self.pointer_generation,
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
                        if current.observed.answer_matches.iter().all(|m| {
                            m.plan().capability().answer_rule_id() == first.answer_rule_id()
                        }) {
                            answer_rule_id = Some(first.answer_rule_id().to_owned());
                        }
                        for matched in &current.observed.answer_matches {
                            match matched.plan().capability().key() {
                                AnswerKey::Decision(decision) => {
                                    supported_decisions.push(*decision)
                                }
                                AnswerKey::Choice(choice) => supported_choices.push(
                                    crate::runtime::control::SupportedChoice {
                                        choice_id: choice.clone(),
                                        label: matched.choice_label().to_owned(),
                                        scope: matched.plan().choice_scope(),
                                    },
                                ),
                            }
                        }
                        request_token = Some(token.clone());
                        reason = None;
                    }
                } else if fresh
                    && blocker.is_none()
                    && observation.unplanned_question_episode() == Some(interaction.episode_seq)
                {
                    reason = Some("question has no tested answer".into());
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
                        answer_rule_id,
                        answer_capability: if request_token.is_some() {
                            storage::Coverage::Tested
                        } else {
                            storage::Coverage::Unknown
                        },
                        capability_reason: reason,
                        supported_decisions,
                        supported_choices,
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

    /// Cancels queued work as `runner_stopped` and returns the settled input IDs, whose results
    /// the caller publishes before the run's exit event.
    pub(crate) fn fence(&mut self, deadline: Instant) -> Result<Vec<i64>> {
        self.current_request = None;
        self.input_fenced = true;
        let mut settled = Vec::new();
        if let Some((id, _, written)) = self.active_deferred() {
            self.pending = None;
            let mut conn = crate::db::connect(&self.database_url)?;
            if written {
                self.record_deferred_uncertain(
                    &mut conn,
                    id,
                    storage::StopCause::RunnerStopped,
                    deadline,
                )?;
            } else {
                let operation = with_database_deadline(&mut conn, deadline, |conn| {
                    storage::settle_zero_byte_deferred(
                        conn,
                        &self.run.owner(),
                        id,
                        storage::InputState::InProgress,
                        "runner_stopped",
                    )
                })?;
                let retained = self
                    .operations
                    .get_mut(&id)
                    .expect("a deferred attempt retains its receipt");
                retained.operation = operation;
                retained.attempt_available = false;
            }
            settled.push(id);
        }
        if !self.queue.is_empty() || self.finalization.is_some() || self.pending_stamp.is_some() {
            let mut conn = crate::db::connect(&self.database_url)?;
            let owner = self.run.owner();
            if let Some(pending) = self.pending_stamp.take() {
                with_database_deadline(&mut conn, deadline, |conn| {
                    storage::settle_preview(conn, &owner, pending.input_id)
                })?;
                settled.push(pending.input_id);
            }
            if let Some(pending) = self.finalization.take() {
                with_database_deadline(&mut conn, deadline, |conn| {
                    storage::settle_zero_byte_deferred(
                        conn,
                        &owner,
                        pending.input_id,
                        pending.expected,
                        pending.reason,
                    )
                })?;
                settled.push(pending.input_id);
            }
            while let Some(queued) = self.queue.pop_front() {
                with_database_deadline(&mut conn, deadline, |conn| {
                    storage::settle_zero_byte_deferred(
                        conn,
                        &owner,
                        queued.input_id,
                        storage::InputState::Ready,
                        "runner_stopped",
                    )
                })?;
                settled.push(queued.input_id);
            }
        }
        check_deadline(Instant::now(), deadline)?;
        Ok(settled)
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
        } else if status.retained_draft || status.retained_evidence {
            Some("observation update pending".into())
        } else if status.visibility != Visibility::Visible {
            Some("permission request is not visible".into())
        } else if status.eligibility != InputEligibility::Hold(GateReason::Interaction) {
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
                    && self.pointer_generation == bound.pointer_generation
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
                && self.pointer_generation == bound.pointer_generation
                && observation.request_generation() == bound.observed.request_generation
                && observed.request_generation == bound.observed.request_generation
                && observed.classification
                    == answered_classification(&selection.request.metadata.key)
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
        let Some(selection) = pending.permission().cloned() else {
            return Ok(StageGateProgress::Ready);
        };
        let id = pending
            .input_id()
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
        let bound_match = bound
            .observed
            .answer_matches
            .iter()
            .find(|matched| matched.plan() == &selection.plan);
        let bound_focus = || bound_match.and_then(AnswerMatch::focused_choice_id);
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
                && self.pointer_generation == bound.pointer_generation
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
                let observed_match = observed
                    .answer_matches
                    .iter()
                    .find(|matched| matched.plan() == &selection.plan);
                ensure(
                    observed.episode_seq == bound.observed.episode_seq
                        && observed.request_rule_id == bound.observed.request_rule_id
                        && observed.semantic == bound.observed.semantic
                        && observed.choices == bound.observed.choices
                        && observed_match.map(AnswerMatch::focus_capture)
                            == bound_match.map(AnswerMatch::focus_capture)
                        && observed_match.and_then(AnswerMatch::focused_choice_id)
                            == Some(choice.as_str()),
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
            .delivery = PendingDelivery::Permission(rebound);
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
        request: &InputControlRequest,
        input: &PermissionRespondInput,
        deadline: Instant,
        known_id: &mut Option<i64>,
    ) -> Result<Option<storage::PermissionReceipt>> {
        check_deadline(Instant::now(), deadline)?;
        crate::runtime::input_client::validate_respond(input)?;
        let mut expected = ControlIdentity::from_run(&self.run);
        expected.owner_epoch = input.expected_owner_epoch;
        ensure(
            request.identity == expected && input.expected_run_id == self.run.run_id,
            "permission response fixed identity mismatch",
        )?;
        with_database_deadline(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            ensure(
                storage::database_identity(&tx)? == self.run.database_identity_json,
                "input database identity changed",
            )?;
            let retained =
                crate::runtime::input_client::response_lookup(&tx, &self.run, input, known_id)?;
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
                crate::runtime::input_client::projection(&self.run, receipt.clone())
                    .map(|result| Some(InputControlReply::PermissionResponse(result)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn retained_permission_response(
        &self,
        conn: &mut Connection,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        let InputControlOperation::PermissionRespond(input) = &request.operation else {
            unreachable!("retained permission lookup receives a response operation");
        };
        let mut known_id = None;
        let result = (|| {
            let deadline = bounded_by_inherited(conn, deadline);
            let history = self.permission_history(conn, request, input, deadline, &mut known_id)?;
            self.retained_projection(&history)
        })();
        result.map_err(|source| self.response_failure(source, input, known_id))
    }

    fn permission_received(
        &mut self,
        conn: &mut Connection,
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
                self.permission_history(conn, &request, input, deadline, &mut known_id)?;
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
                    && binding.event_id == input.event_id,
                "permission response token identity changed",
            )?;
            if let Some(reason) = answer_kind_mismatch(binding.observed.classification, &input.key)
            {
                return Ok(Some(InputControlReply::PermissionResponse(
                    self.rejected_response(input, reason),
                )));
            }
            let Some(matched) = binding
                .observed
                .answer_matches
                .iter()
                .find(|matched| matched.plan().capability().key() == &input.key)
            else {
                return match &input.key {
                    AnswerKey::Decision(_) => {
                        Err(error("selected permission decision is unsupported"))
                    }
                    AnswerKey::Choice(choice) => Ok(Some(InputControlReply::PermissionResponse(
                        self.rejected_response(
                            input,
                            format!("choice {choice} has no tested answer for this question"),
                        ),
                    ))),
                };
            };
            let plan = matched.plan().clone();
            let cap = plan.capability();
            let selection = Arc::new(PermissionSelection {
                request: storage::NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: storage::PermissionResponseMetadata {
                        event_id: input.event_id,
                        interaction_id: input.interaction_id,
                        expected_owner_epoch: input.expected_owner_epoch,
                        request_token: input.request_token.clone(),
                        key: input.key.clone(),
                        director_member_id: input.director_member_id,
                        director_mode: input.director_mode,
                        decision_source: input.decision_source,
                        mode_evidence_ref: input.mode_evidence_ref.clone(),
                        decision_evidence_ref: input.decision_evidence_ref.clone(),
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
                self.validate_live(&tx)?;
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
                reservation: match history {
                    Some(ready) => PendingReservation::HistoricalReady(Box::new(ready)),
                    None => PendingReservation::Unreserved,
                },
                request: storage::NewInput {
                    operation_key: input.operation_key.clone(),
                    kind: storage::InputKind::PermissionResponse,
                    payload_digest: digest,
                    message_id: None,
                },
                bytes,
                delivery: PendingDelivery::Permission(selection),
                deadline,
                phase: InputPhase::AwaitReservation,
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
            key: metadata.key.clone(),
            decision_source: metadata.decision_source,
        }
    }

    fn rejected_response(
        &self,
        input: &PermissionRespondInput,
        reason: impl Into<String>,
    ) -> PermissionResponseResult {
        PermissionResponseResult {
            submission: SubmissionResult {
                status: SubmissionStatus::Rejected,
                input_id: None,
                stage: SubmissionStage::ZeroBytes,
                output_seq: None,
                reason: Some(reason.into()),
                delivery: None,
            },
            member_id: self.run.member_id,
            run_id: self.run.run_id,
            owner_epoch: input.expected_owner_epoch,
            interaction_id: input.interaction_id,
            event_id: input.event_id,
            key: input.key.clone(),
            decision_source: input.decision_source,
        }
    }

    fn run_failure(
        &self,
        source: CafleetError,
        interaction_id: Option<i64>,
        input_id: Option<i64>,
        operation_key: Option<String>,
    ) -> InputFailure {
        InputContext {
            run_id: Some(self.run.run_id),
            interaction_id,
            input_id,
            operation_key,
        }
        .failure(source)
    }

    fn response_failure(
        &self,
        source: CafleetError,
        input: &PermissionRespondInput,
        input_id: Option<i64>,
    ) -> InputFailure {
        self.run_failure(
            source,
            Some(input.interaction_id),
            input_id,
            Some(input.operation_key.clone()),
        )
    }

    fn input_context(&self) -> InputContext {
        InputContext {
            run_id: Some(self.run.run_id),
            interaction_id: self.pending.as_ref().and_then(|pending| {
                pending
                    .permission()
                    .as_ref()
                    .map(|selection| selection.request.metadata.interaction_id)
            }),
            input_id: self.pending.as_ref().and_then(|pending| pending.input_id()),
            operation_key: self
                .pending
                .as_ref()
                .map(|pending| pending.request.operation_key.clone()),
        }
    }

    fn failure(&self, source: CafleetError) -> InputFailure {
        self.input_context().failure(source)
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

    fn validate_live(&self, conn: &Connection) -> Result<()> {
        ensure(
            !self.manual_accounting_failed && !self.input_fenced,
            "manual accounting failure fenced input",
        )?;
        self.validate_live_run(conn)
    }

    fn validate_live_run(&self, conn: &Connection) -> Result<()> {
        let current = crate::runtime::input_client::revalidate_run_snapshot(
            conn,
            &self.run,
            &self.placement,
            crate::runtime::input_client::SnapshotPolicy::Live,
            crate::runtime::input_client::SnapshotMessages {
                run_changed: "input coordinator current run changed",
                placement_changed: "input member placement changed",
            },
        )?;
        self.lock.validate(conn, &current.owner())
    }

    fn validate_receipt_owner(&self, conn: &Connection) -> Result<()> {
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
        expected: &storage::PermissionReceipt,
        deadline: Instant,
    ) -> Result<ReceiptGuardBinding> {
        check_deadline(Instant::now(), deadline)?;
        self.validate_receipt_owner(conn)?;
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
        let binding = self.receipt_guard_snapshot(conn, expected, deadline)?;
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
        expected: &storage::PermissionReceipt,
        selection: &Arc<PermissionSelection>,
        live: (&mut Transport, &ObservationController),
        deadline: Instant,
    ) -> Result<Option<InputGuard<'a>>> {
        let (transport, observation) = live;
        self.validate_permission_binding(selection, observation)?;
        self.validate_live(conn)?;
        self.validate_permission_association(conn, selection)?;
        ensure(
            self.pending.as_ref().is_some_and(|p| {
                p.phase == InputPhase::AwaitReservation
                    && p.input_id() == Some(expected.input.input_id)
            }) && expected.input.state == storage::InputState::Ready
                && expected.response.metadata == selection.request.metadata,
            "exact pending ready rearm required",
        )?;
        let mut binding = self.receipt_guard_snapshot(conn, expected, deadline)?;
        ensure(binding.zero_known, "ready rearm requires actual zero bytes")?;
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        transport.validate_permission(&proof, deadline)?;
        ensure(
            proof.pointer_generation() == self.pointer_generation,
            "permission pointer generation changed",
        )?;
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
                retained.operation.same_identity(input),
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
        action: &ReceiptAction,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        use ReceiptAction;
        use ReceiptReply;
        if let ReceiptAction::Retire { input_id } = action {
            return self.retire_received(conn, *input_id, deadline);
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
                    self.validate_receipt_owner(&tx)?;
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
                    self.validate_receipt_owner(&tx)?;
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
                            let guard = self.acquire_receipt_guard(&tx, &receipt, deadline)?;
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
        input_id: i64,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        let deadline = bounded_by_inherited(conn, deadline);
        check_deadline(Instant::now(), deadline)?;
        let mut operation_key = None;
        let result = (|| {
            ensure(input_id > 0, "positive retirement input ID required")?;
            with_database_deadline(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                self.validate_receipt_owner(&tx)?;
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
            Ok(InputControlReply::Receipt(ReceiptReply::Retired {
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
            && !status.retained_draft
            && !status.retained_evidence
            && status.diagnostic.is_none()
    }

    fn verified_composer(&self, operation: ComposerOperation) -> bool {
        self.services.catalog.verified_composer(
            self.backend,
            self.run.native_version.as_deref(),
            operation,
        )
    }

    fn attempt_requirement(&self, operation: ComposerOperation, stages: usize) -> Result<Duration> {
        let settles = u32::try_from(stages.checked_sub(1).expect("a delivery has stages"))
            .expect("a delivery has bounded stages");
        Ok(self.stage_settle() * settles
            + STAGE_WRITE_MARGIN
            + if self.verified_composer(operation) {
                POST_SUBMIT_VERIFICATION
            } else {
                Duration::ZERO
            })
    }

    fn consumed_held_blocker(&self) -> Option<i64> {
        let (input_id, _) = self.held_composer?;
        let retained = self.operations.get(&input_id)?;
        (retained.operation.has_stalled_blocker_provenance()
            && (retained.stage != SubmissionStage::ZeroBytes || retained.written_bytes > 0)
            && !retained.leased
            && !retained.attempt_available
            && !self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.input_id() == Some(input_id)))
        .then_some(input_id)
    }

    fn unresolved_ordinary(&self, conn: &Connection, operation: ComposerOperation) -> Result<bool> {
        if self.verified_composer(operation) {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM member_input_operations WHERE run_id=?1 AND kind IN ('preview','prompt','exec') AND state='in_progress' AND input_id IS NOT ?2)",
                rusqlite::params![self.run.run_id, self.consumed_held_blocker()],
                |row| row.get(0),
            )?)
        } else {
            storage::has_unresolved_ordinary_attempt(conn, self.run.run_id)
        }
    }

    fn result(&self, operation: &storage::InputOperation) -> SubmissionResult {
        use storage::InputState;
        let (status, mut stage, reason) = crate::runtime::input_client::durable_outcome(operation)
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
            input_id: operation.input_id.into(),
            stage,
            output_seq: retained.map(|point| point.output_seq),
            reason,
            delivery: operation.delivery(),
        }
    }

    pub(super) fn handle_received(
        &mut self,
        conn: &mut Connection,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        self.receive(conn, request, observation, deadline, false)
    }

    /// The runner's own preview sweep request: it queues with zero bytes while another process
    /// holds the pane input lock, so a merely queued sweep request holds no lock.
    pub(super) fn handle_swept(
        &mut self,
        conn: &mut Connection,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        self.receive(conn, request, observation, deadline, true)
    }

    fn receive(
        &mut self,
        conn: &mut Connection,
        request: InputControlRequest,
        observation: &ObservationController,
        deadline: Instant,
        queue_behind_busy_pane: bool,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        if matches!(
            &request.operation,
            InputControlOperation::PermissionRespond(_)
        ) {
            return self.permission_received(conn, request, observation, deadline);
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
                request.identity == expected,
                "input request fixed identity mismatch",
            )?;
            if let InputControlOperation::Receipt(action) = &request.operation {
                match action {
                    ReceiptAction::Query { operation_key } => {
                        known_key = Some(operation_key.clone())
                    }
                    ReceiptAction::Retire { input_id }
                    | ReceiptAction::Reconcile { input_id, .. } => known_input_id = Some(*input_id),
                }
                request.identity.validate(&self.run)?;
                return self.receipt_received(conn, action, deadline).map(Some);
            }
            let (input, composer_operation) = match request.operation {
                InputControlOperation::SubmitPrompt(input) => (input, ComposerOperation::Prompt),
                InputControlOperation::SubmitExec(input) => (input, ComposerOperation::Exec),
                _ => unreachable!("serve_input routes only prompt and exec submissions here"),
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
                storage::preview_attempt_from_key(
                    self.run.run_id,
                    message_id,
                    &input.operation_key,
                )?;
            } else {
                ensure(
                    input.message_id.is_none(),
                    "explicit input has no preview message",
                )?;
            }
            validate_operation_key(&input.operation_key)?;
            ensure(
                crate::pane_lock::valid_input(&input.literal),
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
                let digest = storage::input_payload_digest(self.run.run_id, &input);
                ensure(
                    retained.kind == input.input_kind
                        && retained.payload_digest == digest
                        && retained.message_id == input.message_id,
                    "conflicting input operation key reuse",
                )?;
                return Ok(Some(InputControlReply::Submission(self.result(&retained))));
            }
            let digest = storage::input_payload_digest(self.run.run_id, &input);
            request.identity.validate(&self.run)?;
            let unresolved_deferred = with_database_deadline(conn, deadline, |conn| {
                self.unresolved_ordinary(conn, composer_operation)
            })?;
            self.clear_verified_hold(observation);
            let status = observation.status();
            let defer_reason = match ordinary_blocker(&status) {
                Some(OrdinaryBlocker::ObserverUnavailable) => {
                    return Ok(Some(InputControlReply::Submission(SubmissionResult::held(
                        "observer_unavailable",
                    ))));
                }
                Some(OrdinaryBlocker::Interaction) => Some(interaction_cause(&status).as_str()),
                None if self.pending.is_some()
                    || !self.queue.is_empty()
                    || self.finalization.is_some()
                    || self.pending_stamp.is_some()
                    || self.held_composer.is_some()
                    || unresolved_deferred
                    || (queue_behind_busy_pane
                        && PaneLock::try_acquire(&self.database_url, &self.placement)?
                            .is_none()) =>
                {
                    Some("earlier_input")
                }
                None => {
                    let selection = self.select_delivery(
                        conn,
                        observation,
                        ComposerInput {
                            operation: composer_operation,
                            kind: input.input_kind,
                            message_id: input.message_id,
                            literal: &input.literal,
                        },
                        false,
                        deadline,
                    )?;
                    match selection {
                        DeliveryChoice::Waiting => Some("composer_busy"),
                        DeliveryChoice::PlainLimit(cause) => {
                            return Ok(Some(InputControlReply::Submission(
                                SubmissionResult::rejected_before_write(cause),
                            )));
                        }
                        DeliveryChoice::Ready(selection)
                            if self.verified_composer(composer_operation)
                                && deadline.saturating_duration_since(Instant::now())
                                    < self.attempt_requirement(
                                        composer_operation,
                                        selection.2.len(),
                                    )? =>
                        {
                            Some("delivery_budget_exhausted")
                        }
                        DeliveryChoice::Ready(_) => None,
                    }
                }
            };
            let new_input = storage::NewInput {
                operation_key: input.operation_key.clone(),
                kind: input.input_kind,
                payload_digest: digest,
                message_id: input.message_id,
            };
            if let Some(defer_reason) = defer_reason {
                let reservation = with_database_deadline(conn, deadline, |conn| {
                    self.validate_live(conn)?;
                    storage::reserve_input_with_provenance(
                        conn,
                        &self.run.owner(),
                        &new_input,
                        storage::NewReservation {
                            selection: None,
                            defer_reason: Some(defer_reason),
                        },
                    )
                })?;
                let operation = match reservation {
                    storage::InputReservation::Inserted(operation) => {
                        self.queue.push_back(QueuedInput {
                            input_id: operation.input_id,
                            redelivery: false,
                            stalled: false,
                            request: new_input,
                            literal: input.literal,
                            operation: composer_operation,
                            composer_busy_since: None,
                            attempt_deadline: None,
                            obstacle: "delivery_budget_exhausted",
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
            with_database_deadline(conn, deadline, |conn| self.validate_live(conn))?;
            let (method, reason, stages) = match self.select_delivery(
                conn,
                observation,
                ComposerInput {
                    operation: composer_operation,
                    kind: input.input_kind,
                    message_id: input.message_id,
                    literal: &input.literal,
                },
                false,
                deadline,
            )? {
                DeliveryChoice::Ready(selection) => selection,
                DeliveryChoice::PlainLimit(cause) => {
                    return Ok(Some(InputControlReply::Submission(
                        SubmissionResult::rejected_before_write(cause),
                    )));
                }
                DeliveryChoice::Waiting => {
                    return Err(error("a busy composer queues the input before selection"));
                }
            };
            self.pending = Some(PendingInput {
                reservation: PendingReservation::Unreserved,
                request: new_input,
                bytes: stages[0].clone(),
                delivery: PendingDelivery::Composer(Box::new(ComposerDelivery {
                    method,
                    reason,
                    stages,
                    written_stages: 0,
                    started: false,
                    settle_until: None,
                    manual_generation: None,
                    pointer_generation: None,
                    stop: None,
                    stop_failure: None,
                    verification: None,
                    pane: None,
                    literal: input.literal,
                    operation: composer_operation,
                    deferred: None,
                })),
                deadline,
                phase: InputPhase::AwaitReservation,
            });
            Ok(None)
        })();
        result.map_err(|source| self.run_failure(source, None, known_input_id, known_key))
    }

    /// Chooses profile delivery for a verified empty composer with an executable plan, and
    /// delivery fallback otherwise, rendering a fallback preview with its dismissal clause.
    fn select_delivery(
        &self,
        conn: &mut Connection,
        observation: &ObservationController,
        input: ComposerInput<'_>,
        idle_timeout: bool,
        deadline: Instant,
    ) -> Result<DeliveryChoice> {
        let ComposerInput {
            operation,
            kind,
            message_id,
            literal,
        } = input;
        let composer = match self.run.native_version.as_deref() {
            Some(native) => {
                let backend = coding_agent(&self.run.backend)
                    .ok_or_else(|| error("unknown input backend"))?;
                self.services
                    .catalog
                    .composer_plan(backend, native, operation)
            }
            None => None,
        };
        let (method, reason) = match &composer {
            Some(_) if Self::composer_eligible(observation) => {
                (storage::DeliveryMethod::Profile, None)
            }
            Some(_) if idle_timeout => (storage::DeliveryMethod::Fallback, Some("idle_timeout")),
            Some(_) => return Ok(DeliveryChoice::Waiting),
            None => (
                storage::DeliveryMethod::Fallback,
                Some("submission_plan_unavailable"),
            ),
        };
        let render = |base: &str| {
            if method == storage::DeliveryMethod::Fallback && kind == storage::InputKind::Preview {
                crate::delivery::fallback_preview_line(base)
            } else {
                base.to_owned()
            }
        };
        let mut literal = render(literal);
        let mut framing = crate::coding_agent::catalog::LiteralFraming::Plain;
        if let Some(plan) = &composer {
            if plan.literal_framing()
                == crate::coding_agent::catalog::LiteralFraming::BracketedPaste
                && self.bracketed_paste
            {
                framing = crate::coding_agent::catalog::LiteralFraming::BracketedPaste;
            } else if !plan.plain_literal_allowed(&literal) {
                if kind != storage::InputKind::Preview {
                    return Ok(DeliveryChoice::PlainLimit(
                        plan.plain_limit_refusal(&literal, &self.run.backend),
                    ));
                }
                let message = with_database_deadline(conn, deadline, |conn| {
                    crate::broker::get_message(conn, message_id.expect("a preview has its message"))
                })?;
                literal = render(&crate::delivery::pointer_preview_line(&message));
            }
        }
        let stages = if let Some(plan) = &composer {
            let [literal, submit] = plan.stages(&literal, framing)?;
            let mut stages: Vec<Arc<[u8]>> = Vec::new();
            if method == storage::DeliveryMethod::Fallback {
                stages.push(Arc::from(&[FALLBACK_ESCAPE][..]));
                stages.extend([Arc::from(literal), Arc::from(&[FALLBACK_ENTER][..])]);
            } else {
                stages.extend([Arc::from(literal), Arc::from(submit)]);
            }
            Arc::from(stages)
        } else {
            fallback_stages(operation.prefix(), &literal)?
        };
        Ok(DeliveryChoice::Ready((method, reason, stages)))
    }

    /// A started deferred attempt holds the pane and advances its stages on every tick.
    pub(super) fn deferred_attempt_writing(&self) -> bool {
        self.pending
            .as_ref()
            .and_then(|pending| pending.composer())
            .is_some_and(|composer| composer.deferred.is_some() && composer.pane.is_some())
    }

    /// A deferred terminal transaction failed permanently: ordinary input is fenced and the runner
    /// must stop, so fenced recovery settles and publishes the receipt.
    pub(super) fn deferred_failed(&self) -> bool {
        self.deferred_failed
    }

    /// The runner already owns a preview's delivery: one waits in the FIFO or is being drained
    /// from it, or a preview's terminal transaction is pending storage.
    pub(super) fn owns_preview_delivery(&self) -> bool {
        self.finalization.is_some()
            || self.pending_stamp.is_some()
            || self
                .queue
                .iter()
                .chain(
                    self.pending
                        .as_ref()
                        .and_then(PendingInput::deferred)
                        .map(|deferred| &deferred.queued),
                )
                .any(|queued| queued.request.kind == storage::InputKind::Preview)
    }

    pub(super) fn has_deferred_work(&self) -> bool {
        !self.queue.is_empty()
            || self.finalization.is_some()
            || self.pending_stamp.is_some()
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.deferred().is_some())
    }

    fn publish_result(
        observation: &ObservationController,
        input_id: i64,
        stage: SubmissionStage,
        stop_cause: Option<storage::StopCause>,
    ) {
        observation.publish_delivery_result(storage::input_delivery_result_cause(
            input_id,
            stage.as_str(),
            stop_cause,
        ));
    }

    /// Settles queued or attempted zero-byte work as not submitted and publishes its result. When
    /// storage prevents the transaction, it stays a pending finalization that later ticks retry.
    fn cancel_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        input_id: i64,
        expected: storage::InputState,
        reason: &'static str,
    ) -> Result<()> {
        let finalization = Instant::now() + DEFERRED_FINALIZATION;
        let operation = match with_database_deadline(conn, finalization, |conn| {
            storage::settle_zero_byte_deferred(conn, &self.run.owner(), input_id, expected, reason)
        }) {
            Ok(operation) => operation,
            Err(cause) if storage_failure(&cause) => {
                eprintln!(
                    "member runtime: input {input_id} result is pending storage: {}",
                    cause.message()
                );
                self.finalization = Some(PendingFinalization {
                    input_id,
                    expected,
                    reason,
                });
                return Ok(());
            }
            Err(cause) => {
                self.input_fenced = true;
                self.deferred_failed = true;
                return Err(cause);
            }
        };
        let retried = operation.is_retried_preview();
        if let Some(retained) = self.operations.get_mut(&input_id) {
            retained.operation = operation;
            retained.attempt_available = false;
        }
        if !retried {
            Self::publish_result(observation, input_id, SubmissionStage::ZeroBytes, None);
        }
        Ok(())
    }

    /// Classifies the queued head's target before its first byte: `None` keeps it live, and a
    /// reason settles it. Ownership loss and storage failures return an error.
    fn deferred_target(&self, conn: &Connection) -> Result<Option<&'static str>> {
        ensure(
            !self.manual_accounting_failed && !self.input_fenced,
            "manual accounting failure fenced input",
        )?;
        let current = storage::validate_owner(conn, &self.run.owner())?;
        ensure(
            current.status == RunStatus::Running && current.owner_kind == OwnerKind::Runner,
            "input coordinator no longer owns a running run",
        )?;
        if stable_run_identity(&current) != stable_run_identity(&self.run)
            || current.native_version != self.run.native_version
        {
            return Ok(Some("target_changed"));
        }
        if crate::broker::active_member_fleet(conn, self.run.member_id)? != Some(self.run.fleet_id)
        {
            return Ok(Some("target_inactive"));
        }
        let Some(member) = crate::broker::get_member(conn, self.run.member_id, self.run.fleet_id)?
        else {
            return Ok(Some("target_inactive"));
        };
        if member.kind != MemberKind::Member || member.placement != self.placement {
            return Ok(Some("target_changed"));
        }
        self.lock.validate(conn, &current.owner())?;
        Ok(None)
    }

    fn hold_possible_ordinary_input(&mut self, input_id: i64) {
        let retained = self
            .operations
            .get(&input_id)
            .expect("released input retains its receipt");
        let operation = match retained.operation.kind {
            storage::InputKind::Prompt | storage::InputKind::Preview => ComposerOperation::Prompt,
            storage::InputKind::Exec => ComposerOperation::Exec,
            storage::InputKind::PermissionResponse => return,
        };
        if !self.verified_composer(operation)
            || retained.stage == SubmissionStage::ZeroBytes && retained.written_bytes == 0
            || retained.operation.state == storage::InputState::Accepted
            || retained.operation.disposition == Some(storage::InputDisposition::Submitted)
            || retained.operation.disposition == Some(storage::InputDisposition::LeftInComposer)
                && retained.operation.left_stage == Some(storage::LeftStage::EscapeOnly)
            || self
                .held_composer
                .is_some_and(|(blocker, _)| blocker == input_id)
        {
            return;
        }
        assert!(
            !retained.leased,
            "a held ordinary input has released its writer exclusion"
        );
        self.held_composer = Some((input_id, self.output_seq));
        if let Some(head) = self.queue.front_mut() {
            head.composer_busy_since = None;
            head.attempt_deadline = None;
        }
    }

    fn clear_verified_hold(&mut self, observation: &ObservationController) {
        if self
            .held_composer
            .is_some_and(|(_, output_seq)| self.output_seq > output_seq)
            && Self::composer_eligible(observation)
        {
            self.held_composer = None;
            if let Some(head) = self.queue.front_mut() {
                head.composer_busy_since = None;
                head.attempt_deadline = None;
            }
        }
    }

    /// Starts the queued head once no interaction blocks it, or settles it when its attempt
    /// budget expires or its preview was consumed.
    fn start_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<()> {
        if self.input_fenced {
            return Ok(());
        }
        self.clear_verified_hold(observation);
        if let Some(pending) = self.finalization.take() {
            self.cancel_deferred(
                conn,
                observation,
                pending.input_id,
                pending.expected,
                pending.reason,
            )?;
            if self.finalization.is_some() {
                return Ok(());
            }
        }
        if let Some(pending) = self.pending_stamp.take() {
            self.settle_accepted_preview(conn, observation, pending.input_id, pending.stage)?;
            if self.pending_stamp.is_some() {
                return Ok(());
            }
        }
        let Some(head) = self.queue.front() else {
            return Ok(());
        };
        let (input_id, message_id, operation) =
            (head.input_id, head.request.message_id, head.operation);
        let now = Instant::now();
        let target = with_database_deadline(conn, now + DEFERRED_FINALIZATION, |conn| {
            if let Some(message_id) = message_id
                && crate::broker::get_message(conn, message_id)?.status
                    != crate::broker::MessageStatus::InputRequired
            {
                return Ok(Some("message_consumed"));
            }
            self.deferred_target(conn)
        });
        match target {
            Ok(Some(reason)) => {
                self.queue.pop_front();
                return self.cancel_deferred(
                    conn,
                    observation,
                    input_id,
                    storage::InputState::Ready,
                    reason,
                );
            }
            Ok(None) => {}
            Err(cause) if storage_failure(&cause) => {
                self.note_head_obstacle("storage_unavailable");
                return Ok(());
            }
            Err(cause) => return Err(cause),
        }
        match with_database_deadline(conn, now + DEFERRED_FINALIZATION, |conn| {
            self.unresolved_ordinary(conn, operation)
        }) {
            Ok(false) => {}
            Ok(true) => return Ok(()),
            Err(cause) if storage_failure(&cause) => {
                self.note_head_obstacle("storage_unavailable");
                return Ok(());
            }
            Err(cause) => return Err(cause),
        }
        let blocker = ordinary_blocker(&observation.status());
        let consumed_held_blocker = self.consumed_held_blocker();
        let head = self
            .queue
            .front_mut()
            .expect("the checked queue head remains");
        if matches!(blocker, Some(OrdinaryBlocker::Interaction)) {
            head.attempt_deadline = None;
            head.composer_busy_since = None;
            return Ok(());
        }
        if blocker.is_none() {
            if let Some((blocking_input_id, _)) = self.held_composer {
                let since = *head.composer_busy_since.get_or_insert(now);
                head.attempt_deadline = None;
                if now.duration_since(since) >= COMPOSER_BUSY_BOUND && !head.stalled {
                    let blocking =
                        match with_database_deadline(conn, now + DEFERRED_FINALIZATION, |conn| {
                            storage::get_input(conn, blocking_input_id)
                        }) {
                            Ok(blocking) => blocking,
                            Err(cause) if storage_failure(&cause) => {
                                head.obstacle = "storage_unavailable";
                                return Ok(());
                            }
                            Err(cause) => return Err(cause),
                        };
                    ensure(
                        blocking.run_id == self.run.run_id,
                        "held composer blocker belongs to its run",
                    )?;
                    ensure(
                        blocking.has_stalled_blocker_provenance()
                            && (blocking.state != storage::InputState::InProgress
                                || consumed_held_blocker == Some(blocking_input_id)),
                        "held composer blocker retains its selected consumed attempt provenance",
                    )?;
                    head.stalled = true;
                    observation.publish_delivery_stalled(serde_json::json!({
                        "kind":"input_delivery_stalled", "input_id":input_id,
                        "blocking_input_id":blocking_input_id,
                        "waiting_since":crate::time::format_utc(chrono::Utc::now() - chrono::Duration::from_std(now.duration_since(since)).expect("bounded composer wait"))
                    }));
                }
                return Ok(());
            }
            let (kind, literal) = (head.request.kind, head.literal.clone());
            let redelivery = head.redelivery;
            let idle_timeout = !redelivery
                && head
                    .composer_busy_since
                    .is_some_and(|since| now.duration_since(since) >= COMPOSER_BUSY_BOUND);
            let choice = self.select_delivery(
                conn,
                observation,
                ComposerInput {
                    operation,
                    kind,
                    message_id,
                    literal: &literal,
                },
                idle_timeout,
                deadline,
            )?;
            match choice {
                DeliveryChoice::Ready(selection) => {
                    return self.attempt_deferred(conn, observation, deadline, now, selection);
                }
                DeliveryChoice::PlainLimit(_) => {
                    self.queue.pop_front();
                    return self.cancel_deferred(
                        conn,
                        observation,
                        input_id,
                        storage::InputState::Ready,
                        "plain_limit_exceeded",
                    );
                }
                DeliveryChoice::Waiting => {
                    let head = self
                        .queue
                        .front_mut()
                        .expect("the waiting queue head remains");
                    head.composer_busy_since.get_or_insert(now);
                    head.attempt_deadline = None;
                    return Ok(());
                }
            }
        }
        let head = self
            .queue
            .front_mut()
            .expect("the blocked queue head remains");
        head.composer_busy_since = None;
        head.obstacle = "observer_unavailable";
        let attempt_deadline = *head
            .attempt_deadline
            .get_or_insert((now + DEFERRED_ATTEMPT_BUDGET).min(deadline));
        if now >= attempt_deadline {
            self.queue.pop_front();
            return self.cancel_deferred(
                conn,
                observation,
                input_id,
                storage::InputState::Ready,
                "observer_unavailable",
            );
        }
        Ok(())
    }

    /// Starts the selected attempt for the queued head within its five-second budget.
    fn attempt_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        deadline: Instant,
        now: Instant,
        selection: DeliverySelection,
    ) -> Result<()> {
        let head = self.queue.front_mut().expect("queued head remains");
        let attempt_deadline = *head
            .attempt_deadline
            .get_or_insert((now + DEFERRED_ATTEMPT_BUDGET).min(deadline));
        let input_id = head.input_id;
        let message_id = head.request.message_id;
        let composer_operation = head.operation;
        if now >= attempt_deadline {
            let obstacle = head.obstacle;
            self.queue.pop_front();
            return self.cancel_deferred(
                conn,
                observation,
                input_id,
                storage::InputState::Ready,
                obstacle,
            );
        }
        let target = match with_database_deadline(conn, now + DEFERRED_FINALIZATION, |conn| {
            if let Some(message_id) = message_id {
                let message = crate::broker::get_message(conn, message_id)?;
                if message.status != crate::broker::MessageStatus::InputRequired {
                    return Ok(Some("message_consumed"));
                }
            }
            self.deferred_target(conn)
        }) {
            Ok(target) => target,
            Err(cause) if storage_failure(&cause) => {
                self.note_head_obstacle("storage_unavailable");
                return Ok(());
            }
            Err(cause) => return Err(cause),
        };
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
        let Some(pane) = PaneLock::try_acquire(&self.database_url, &self.placement)? else {
            self.note_head_obstacle("pane_input_busy");
            return Ok(());
        };
        let requirement = self.attempt_requirement(composer_operation, selection.2.len())?;
        if attempt_deadline.saturating_duration_since(now) < requirement {
            self.note_head_obstacle("delivery_budget_exhausted");
            return Ok(());
        }
        let (method, reason, stages) = selection;
        let operation = match with_database_deadline(conn, attempt_deadline, |conn| {
            storage::begin_deferred_attempt(conn, &self.run.owner(), input_id, method, reason)
        }) {
            Ok(operation) => operation,
            Err(cause) if storage_failure(&cause) => {
                self.note_head_obstacle("storage_unavailable");
                return Ok(());
            }
            Err(cause) => return Err(cause),
        };
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
                    offset: 0,
                    closing: false,
                },
                stage: SubmissionStage::ZeroBytes,
                written_bytes: 0,
                output_seq: 0,
                attempt_available: false,
                leased: false,
            },
        );
        self.pending = Some(PendingInput {
            reservation: PendingReservation::Installed(input_id),
            request: head.request.clone(),
            bytes: stages[0].clone(),
            delivery: PendingDelivery::Composer(Box::new(ComposerDelivery {
                method,
                reason,
                stages,
                written_stages: 0,
                started: false,
                settle_until: None,
                manual_generation: None,
                pointer_generation: None,
                stop: None,
                stop_failure: None,
                verification: None,
                pane: Some(pane),
                literal: head.literal.clone(),
                operation: head.operation,
                deferred: Some(DeferredAttempt {
                    queued: head,
                    obstacle: "delivery_budget_exhausted",
                }),
            })),
            deadline: attempt_deadline,
            phase: InputPhase::AwaitBeforeWrite,
        });
        self.checkpoint(
            input_id,
            #[cfg(test)]
            InputBoundary::Reserved,
            attempt_deadline,
        )
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
        let Some(composer) = pending.composer() else {
            return Ok(false);
        };
        if composer.deferred.is_none() {
            return Ok(false);
        }
        let id = pending
            .input_id()
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
        let composer_busy = !interaction
            && composer.requires_empty_composer()
            && !Self::composer_eligible(observation);
        let now = Instant::now();
        let expired = now >= pending.deadline;
        let requeue = interaction || composer_busy;
        if !requeue && !expired {
            return Ok(false);
        }
        if requeue {
            match with_database_deadline(conn, now + DEFERRED_FINALIZATION, |conn| {
                storage::record_input_state(
                    conn,
                    &self.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    storage::InputState::Ready,
                )
            }) {
                Ok(operation) => debug_assert_eq!(operation.state, storage::InputState::Ready),
                Err(cause) if storage_failure(&cause) => {
                    self.pending
                        .as_mut()
                        .and_then(|pending| pending.deferred_mut())
                        .expect("deferred attempt")
                        .obstacle = "storage_unavailable";
                    return Ok(true);
                }
                Err(cause) => return Err(cause),
            }
        }
        let pending = self.pending.take().expect("deferred pending input");
        let composer = pending
            .delivery
            .into_composer()
            .expect("deferred composer delivery");
        let deferred = composer.deferred.expect("deferred attempt");
        if requeue {
            self.operations.remove(&id);
            let mut queued = deferred.queued;
            queued.attempt_deadline = None;
            queued.composer_busy_since = if composer_busy {
                queued.composer_busy_since.or(Some(now))
            } else {
                None
            };
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

    /// Queues an immediate ordinary request that has written zero bytes once an interaction or a
    /// composer that is not verified empty supersedes its selected method, replying `queued`.
    fn immediate_preflight(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<Option<InputControlReply>> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(None);
        };
        let Some(composer) = pending.composer() else {
            return Ok(None);
        };
        let zero_bytes = composer.written_stages == 0
            && pending.input_id().is_none_or(|id| {
                self.operations
                    .get(&id)
                    .is_some_and(|retained| retained.stage == SubmissionStage::ZeroBytes)
            });
        if composer.deferred.is_some() || pending.permission().is_some() || !zero_bytes {
            return Ok(None);
        }
        let status = observation.status();
        let insufficient_budget = self.verified_composer(composer.operation)
            && pending.deadline.saturating_duration_since(Instant::now())
                < self.attempt_requirement(composer.operation, composer.stages.len())?;
        let defer_reason = match ordinary_blocker(&status) {
            Some(OrdinaryBlocker::Interaction) => interaction_cause(&status).as_str(),
            None if composer.requires_empty_composer() && !Self::composer_eligible(observation) => {
                "composer_busy"
            }
            None if insufficient_budget => "delivery_budget_exhausted",
            _ => return Ok(None),
        };
        let pending = self.pending.take().expect("immediate pending input");
        let input_id = pending.input_id();
        let composer = pending
            .delivery
            .into_composer()
            .expect("immediate composer delivery");
        let operation = match input_id {
            Some(id) => {
                self.operations.remove(&id);
                with_database_deadline(conn, deadline, |conn| {
                    storage::requeue_zero_byte_immediate(conn, &self.run.owner(), id, defer_reason)
                })?
            }
            None => {
                let reservation = with_database_deadline(conn, deadline, |conn| {
                    self.validate_live(conn)?;
                    storage::reserve_input_with_provenance(
                        conn,
                        &self.run.owner(),
                        &pending.request,
                        storage::NewReservation {
                            selection: None,
                            defer_reason: Some(defer_reason),
                        },
                    )
                })?;
                match reservation {
                    storage::InputReservation::Inserted(operation) => operation,
                    storage::InputReservation::Existing(operation) => {
                        return Ok(Some(InputControlReply::Submission(self.result(&operation))));
                    }
                }
            }
        };
        self.queue.push_front(QueuedInput {
            input_id: operation.input_id,
            redelivery: false,
            stalled: false,
            request: pending.request,
            literal: composer.literal,
            operation: composer.operation,
            composer_busy_since: None,
            attempt_deadline: None,
            obstacle: "delivery_budget_exhausted",
        });
        Ok(Some(InputControlReply::Submission(self.result(&operation))))
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
            return self.settle_accepted_preview(conn, observation, operation.input_id, stage);
        }
        if !operation.is_retried_preview() {
            Self::publish_result(observation, operation.input_id, stage, operation.stop_cause);
        }
        Ok(())
    }

    /// Stamps and settles an accepted deferred preview, then publishes its result. A transient
    /// storage failure keeps it as a pending stamp; a permanent one fences ordinary input.
    fn settle_accepted_preview(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
        input_id: i64,
        stage: SubmissionStage,
    ) -> Result<()> {
        match with_database_deadline(conn, Instant::now() + DEFERRED_FINALIZATION, |conn| {
            storage::settle_preview(conn, &self.run.owner(), input_id)
        }) {
            Ok(settled) => {
                self.operations.remove(&settled.input_id);
                Self::publish_result(observation, input_id, stage, None);
                Ok(())
            }
            Err(cause) if storage_failure(&cause) => {
                eprintln!(
                    "member runtime: input {input_id} notification stamp is pending storage: {}",
                    cause.message()
                );
                self.pending_stamp = Some(PendingStamp { input_id, stage });
                Ok(())
            }
            Err(cause) => {
                self.input_fenced = true;
                self.deferred_failed = true;
                Err(cause)
            }
        }
    }

    fn retained_checkpoint(&self, input_id: i64) -> Result<InputCheckpoint> {
        let retained = self
            .operations
            .get(&input_id)
            .ok_or_else(|| error("retained input operation is absent"))?;
        Ok(InputCheckpoint {
            #[cfg(test)]
            input_id,
            stage: retained.stage,
            written_bytes: retained.written_bytes,
            output_seq: retained.output_seq,
        })
    }

    fn checkpoint(
        &mut self,
        input_id: i64,
        #[cfg(test)] boundary: InputBoundary,

        deadline: Instant,
    ) -> Result<()> {
        let _checkpoint = self.retained_checkpoint(input_id)?;
        #[cfg(test)]
        let result = (|| -> Result<()> {
            #[cfg(test)]
            self.services
                .hooks
                .checkpoint(boundary, &_checkpoint, deadline)?;

            check_deadline(Instant::now(), deadline)
        })();
        #[cfg(not(test))]
        let result = check_deadline(Instant::now(), deadline);
        if result.is_err()
            && let Some(pending) = &mut self.pending
        {
            pending.phase = InputPhase::Blocked;
        }
        result
    }

    fn drain_requirement(&self) -> DrainRequirement {
        let pending = self.pending.as_ref();
        if pending.and_then(|p| p.permission()).is_some() {
            DrainRequirement::Observation
        } else if pending
            .and_then(|p| p.composer())
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
        let permission = self.pending.as_ref().and_then(|p| p.permission());
        if let Some(permission) = permission {
            self.validate_permission_binding(permission, observation)?;
        }
        let requirement = self.drain_requirement();
        let Some(proof) = transport.take_input_drain() else {
            return Ok(None);
        };
        if !Self::validate_taken_drain(transport, &proof, observation, requirement, deadline)? {
            self.note_observer_obstacle(observation);
            return Ok(None);
        }
        self.current_drain_seq(&proof, "input manual generation changed")
            .map(Some)
    }

    fn note_head_obstacle(&mut self, obstacle: &'static str) {
        self.queue
            .front_mut()
            .expect("queued head remains")
            .obstacle = obstacle;
    }

    /// Records the zero-byte obstacle a started deferred attempt last met, which its expiry
    /// reports.
    fn note_deferred_obstacle(&mut self, obstacle: &'static str) {
        if let Some(deferred) = self
            .pending
            .as_mut()
            .and_then(|pending| pending.deferred_mut())
        {
            deferred.obstacle = obstacle;
        }
    }

    fn note_observer_obstacle(&mut self, observation: &ObservationController) {
        if matches!(
            ordinary_blocker(&observation.status()),
            Some(OrdinaryBlocker::ObserverUnavailable)
        ) {
            self.note_deferred_obstacle("observer_unavailable");
        }
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
                transport.validate_permission(proof, deadline)?;
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
        operation: &PendingInput,
        deadline: Instant,
    ) -> Result<WriteValidation> {
        let effective = self.effective_deadline(conn, deadline)?;
        ensure(
            conn.is_autocommit(),
            "final input validation requires autocommit",
        )?;
        let result = with_database_budget(conn, Deadline::until(effective), |conn| {
            conn.busy_handler(Some(|_| false))?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            self.validate_live(&tx)?;
            let id = operation
                .input_id()
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
            if let Some(selection) = &operation.permission() {
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
    }

    /// Holds the concrete pane-input lock for a composer write; `false` while another writer
    /// holds it, so the attempt writes nothing and retries.
    fn hold_composer_pane(&mut self) -> Result<bool> {
        let Some(composer) = self.pending.as_mut().and_then(|p| p.composer_mut()) else {
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
        transport: &mut Transport,
        observation: &ObservationController,
        deadline: Instant,
    ) -> Result<Option<InputGuard<'a>>> {
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
            if let Some(permission) = &pending.permission() {
                self.validate_permission_binding(permission, observation)?;
            }
            if !Self::validate_taken_drain(transport, &proof, observation, requirement, effective)?
            {
                self.note_observer_obstacle(observation);
                return Ok(None);
            }
            if let Some(composer) = &pending.composer() {
                ensure(
                    composer
                        .manual_generation
                        .is_none_or(|generation| generation == self.manual_generation),
                    "human input arrived between composer stages",
                )?;
            }
            if let Some(permission) = &pending.permission() {
                let retained = self
                    .operations
                    .get(
                        &pending
                            .input_id()
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
                selected_answer_stage(&selection.plan, *next_stage)?;
                ensure(
                    selection.plan.capability().request_rule_id()
                        == selection.binding.observed.request_rule_id,
                    "permission stage rule or choice changed",
                )?;
            }
            let output_seq = self.current_drain_seq(&proof, "input manual generation changed")?;
            let id = pending
                .input_id()
                .ok_or_else(|| error("armed input has no reservation"))?;
            let retained = self
                .operations
                .get(&id)
                .ok_or_else(|| error("armed input reservation is absent"))?;
            ensure(
                retained.attempt_available && !retained.leased,
                "input attempt permit is consumed",
            )?;
            if self.validate_write_now(conn, pending, effective)? == WriteValidation::Contended {
                self.note_deferred_obstacle("storage_unavailable");
                return Ok(None);
            }
            check_deadline(Instant::now(), effective)?;
            Ok(Some((id, proof, effective, output_seq)))
        })()?;
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
        transport: &mut Transport,
        pty: &mut impl PtyIo,
        observation: &ObservationController,
        deadline: Instant,
    ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
        self.output_seq = transport.snapshot().screen_seq;
        let context = self.input_context();
        let mut terminal_bookkeeping = false;
        let result = (|| {
            if self.pending.is_some()
                && self.finalization.is_none()
                && let Some(head) = self.queue.front()
            {
                let (input_id, message_id) = (head.input_id, head.request.message_id);
                let reason =
                    with_database_deadline(conn, Instant::now() + DEFERRED_FINALIZATION, |conn| {
                        if let Some(message_id) = message_id
                            && crate::broker::get_message(conn, message_id)?.status
                                != crate::broker::MessageStatus::InputRequired
                        {
                            return Ok(Some("message_consumed"));
                        }
                        self.deferred_target(conn)
                    });
                match reason {
                    Ok(Some(reason)) => {
                        self.queue.pop_front();
                        self.cancel_deferred(
                            conn,
                            observation,
                            input_id,
                            storage::InputState::Ready,
                            reason,
                        )?;
                    }
                    Ok(None) => {}
                    Err(cause) if storage_failure(&cause) => {
                        self.note_head_obstacle("storage_unavailable")
                    }
                    Err(cause) => return Err(cause),
                }
            }
            if self.pending.is_none() {
                self.start_deferred(conn, observation, deadline)?;
                return Ok(None);
            }
            if self.deferred_preflight(conn, observation)? {
                return Ok(None);
            }
            if let Some(reply) = self.immediate_preflight(conn, observation, deadline)? {
                return Ok(Some(reply));
            }
            if let Some(reply) = self.refresh_zero_delivery(conn, observation)? {
                return Ok(Some(InputControlReply::Submission(reply)));
            }
            if self.pending.is_none() {
                return Ok(None);
            }
            let deferred = self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.deferred().is_some());
            let deadline = if deferred {
                deadline.min(self.pending.as_ref().expect("pending input").deadline)
            } else {
                deadline
            };
            let pending = self
                .pending
                .as_ref()
                .expect("pending input survives preflight");
            let effective = self.effective_deadline(conn, deadline)?;
            match pending.phase {
                InputPhase::Blocked => Ok(None),
                InputPhase::Closing => {
                    self.write_close_stage(conn, transport, pty, observation, effective)
                }
                InputPhase::Verifying => {
                    self.verify_submit(conn, transport, observation, effective)
                }
                InputPhase::AwaitReservation => {
                    self.reserve_pending(conn, transport, observation, effective)
                }
                InputPhase::AwaitBeforeWrite => self.arm_pending(
                    conn,
                    (transport, observation),
                    effective,
                    &mut terminal_bookkeeping,
                ),
                InputPhase::Armed => self.write_pending_stage(
                    conn,
                    (transport, observation),
                    pty,
                    effective,
                    &mut terminal_bookkeeping,
                ),
            }
        })();
        result.map_err(|source| {
            if let Some(id) = self
                .pending
                .as_ref()
                .and_then(PendingInput::installed_input_id)
            {
                self.hold_possible_ordinary_input(id);
            }
            let source = match self
                .pending
                .as_mut()
                .and_then(PendingInput::composer_mut)
                .and_then(|composer| composer.stop_failure.take())
            {
                Some(primary) => primary.with_diagnostic(source.message()),
                None => source,
            };
            if let Some(composer) = self.pending.as_mut().and_then(|p| p.composer_mut()) {
                composer.pane = None;
            }
            self.retire_failed_deferred(conn, observation);
            let source = if terminal_bookkeeping {
                source
            } else {
                self.retain_permission_failure(conn, deadline, source)
            };
            if self.pending.is_some() {
                self.failure(source)
            } else {
                context.failure(source)
            }
        })
    }

    fn reserved_input_id(&self) -> Result<i64> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.input_id())
            .ok_or_else(|| error("input writer has no reservation"))
    }

    fn refresh_zero_delivery(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
    ) -> Result<Option<SubmissionResult>> {
        let pending = self.pending.as_ref().expect("a pending input remains");
        let Some(composer) = pending.composer() else {
            return Ok(None);
        };
        if composer.started || pending.phase == InputPhase::Blocked {
            return Ok(None);
        }
        let (operation, literal, request, deadline, id, deferred, idle_timeout) = (
            composer.operation,
            composer.literal.clone(),
            pending.request.clone(),
            pending.deadline,
            pending.input_id(),
            composer.deferred.is_some(),
            composer.reason == Some("idle_timeout"),
        );
        let choice = self.select_delivery(
            conn,
            observation,
            ComposerInput {
                operation,
                kind: request.kind,
                message_id: request.message_id,
                literal: &literal,
            },
            idle_timeout,
            deadline,
        )?;
        let selection = match choice {
            DeliveryChoice::Waiting => return Ok(None),
            DeliveryChoice::PlainLimit(cause) => {
                let mut rejection = SubmissionResult::rejected_before_write(cause);
                if let Some(id) = id {
                    let retained = self
                        .operations
                        .get(&id)
                        .expect("a zero-byte input retains its receipt");
                    ensure(
                        retained.stage == SubmissionStage::ZeroBytes
                            && retained.written_bytes == 0
                            && !retained.leased,
                        "pre-write rejection requires its recorded writer exclusion",
                    )?;
                    if deferred {
                        self.cancel_deferred(
                            conn,
                            observation,
                            id,
                            storage::InputState::InProgress,
                            "plain_limit_exceeded",
                        )?;
                        self.pending = None;
                        self.operations.remove(&id);
                        return Ok(None);
                    }
                    let operation = with_database_deadline(conn, deadline, |conn| {
                        storage::record_input_state(
                            conn,
                            &self.run.owner(),
                            id,
                            storage::InputState::InProgress,
                            storage::InputState::Ready,
                        )
                    })?;
                    let retained = self
                        .operations
                        .get_mut(&id)
                        .expect("a zero-byte input retains its receipt");
                    retained.operation = operation;
                    retained.attempt_available = false;
                    rejection.input_id = Some(id);
                    rejection.delivery = retained.operation.delivery();
                }
                self.pending = None;
                return Ok(Some(rejection));
            }
            DeliveryChoice::Ready(selection) => selection,
        };
        let (method, reason, stages) = selection;
        if let Some(id) = id {
            let retained = self.operations.get(&id).expect("zero-byte receipt remains");
            ensure(
                retained.stage == SubmissionStage::ZeroBytes
                    && retained.written_bytes == 0
                    && !retained.leased,
                "zero-byte selection requires its recorded writer exclusion",
            )?;
            if retained.operation.delivery_method != Some(method)
                || retained.operation.delivery_reason.as_deref() != reason
            {
                let expected = retained.operation.clone();
                let operation = with_database_deadline(conn, deadline, |conn| {
                    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    self.validate_live(&tx)?;
                    ensure(
                        storage::get_input(&tx, id)? == expected,
                        "zero-byte selection receipt changed",
                    )?;
                    let count = tx.execute("UPDATE member_input_operations SET delivery_method=?1,delivery_reason=?2,updated_at=?3 WHERE input_id=?4 AND state='in_progress'", rusqlite::params![method.as_str(),reason,crate::time::now_stamp(),id])?;
                    ensure(
                        count == 1,
                        "zero-byte selection requires its in-progress receipt",
                    )?;
                    let operation = storage::get_input(&tx, id)?;
                    tx.commit()?;
                    Ok(operation)
                })?;
                self.operations
                    .get_mut(&id)
                    .expect("zero-byte receipt remains")
                    .operation = operation;
            }
            let retained = self
                .operations
                .get_mut(&id)
                .expect("zero-byte receipt remains");
            retained.bytes = stages[0].clone();
            retained.selection = RetainedSelection::Composer {
                method,
                stages: stages.clone(),
                next_stage: 0,
                offset: 0,
                closing: false,
            };
        }
        let pending = self
            .pending
            .as_mut()
            .expect("zero-byte pending input remains");
        pending.bytes = stages[0].clone();
        let composer = pending.composer_mut().expect("zero-byte composer remains");
        composer.method = method;
        composer.reason = reason;
        composer.stages = stages;
        Ok(None)
    }

    fn reserve_pending(
        &mut self,
        conn: &mut Connection,
        transport: &mut Transport,
        observation: &ObservationController,
        effective: Instant,
    ) -> Result<Option<InputControlReply>> {
        let ready = self
            .pending
            .as_ref()
            .and_then(|p| p.ready().cloned().zip(p.permission().cloned()));
        let (operation, output_seq) = if let Some((ready, selection)) = ready {
            let owner = self.run.owner();
            let Some(guard) = with_database_deadline(conn, effective, |conn| {
                self.acquire_rearm_guard(
                    conn,
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
            let Some(output_seq) = self.fresh_drain(transport, observation, effective)? else {
                return Ok(None);
            };
            let pending = self.pending.as_ref().expect("retained pending input");
            let operation = match &pending.delivery {
                PendingDelivery::Permission(selection) => {
                    let reservation = with_database_deadline(conn, effective, |conn| {
                        self.validate_live(conn)?;
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
                                crate::runtime::input_client::projection(&self.run, receipt)?,
                            );
                            self.pending = None;
                            return Ok(Some(reply));
                        }
                    }
                }
                PendingDelivery::Composer(composer) => {
                    let reservation = storage::NewReservation {
                        selection: Some((composer.method, composer.reason)),
                        defer_reason: None,
                    };
                    let reservation = with_database_deadline(conn, effective, |conn| {
                        self.validate_live(conn)?;
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
                selection: match &pending.delivery {
                    PendingDelivery::Permission(selection) => RetainedSelection::Permission {
                        selection: selection.clone(),
                        next_stage: 0,
                    },
                    PendingDelivery::Composer(composer) => RetainedSelection::Composer {
                        method: composer.method,
                        stages: composer.stages.clone(),
                        next_stage: 0,
                        offset: 0,
                        closing: false,
                    },
                },
                stage: SubmissionStage::ZeroBytes,
                written_bytes: 0,
                output_seq,
                attempt_available: false,
                leased: false,
            },
        );
        let pending = self.pending.as_mut().expect("reserved pending input");
        pending.reservation = PendingReservation::Installed(id);
        pending.phase = InputPhase::AwaitBeforeWrite;
        self.checkpoint(
            id,
            #[cfg(test)]
            InputBoundary::Reserved,
            effective,
        )?;
        Ok(None)
    }

    fn composer_interruption(
        &self,
        id: i64,
        observation: &ObservationController,
    ) -> Result<Option<storage::StopCause>> {
        let pending = self.pending.as_ref().expect("pending write remains");
        let Some(composer) = pending.composer() else {
            return Ok(None);
        };
        if !composer.started {
            return Ok(None);
        }
        let retained = self.operations.get(&id).expect("pending receipt remains");
        let RetainedSelection::Composer {
            stages,
            next_stage,
            offset,
            ..
        } = &retained.selection
        else {
            return Err(error("ordinary writer selection changed"));
        };
        let literal_index = usize::from(composer.method == storage::DeliveryMethod::Fallback);
        let between_chunks = *next_stage == literal_index && *offset > 0;
        let cause = if self.manual_accounting_failed {
            Some(storage::StopCause::ManualAccountingFailed)
        } else if composer.manual_generation != Some(self.manual_generation) {
            Some(storage::StopCause::ManualInput)
        } else if between_chunks && composer.pointer_generation != Some(self.pointer_generation) {
            Some(storage::StopCause::PointerInput)
        } else {
            blocker_cause(&observation.status())
        };
        if cause.is_some() {
            return Ok(cause);
        }
        let remaining = pending.deadline.saturating_duration_since(Instant::now());
        if (between_chunks && remaining <= PASTE_CLOSE_RESERVE)
            || (*next_stage + 1 == stages.len()
                && self.verified_composer(composer.operation)
                && remaining < POST_SUBMIT_VERIFICATION + STAGE_WRITE_MARGIN)
        {
            return Ok(Some(storage::StopCause::DeadlineExpired));
        }
        Ok(None)
    }

    fn arm_pending(
        &mut self,
        conn: &mut Connection,
        (transport, observation): (&mut Transport, &ObservationController),
        effective: Instant,
        terminal_bookkeeping: &mut bool,
    ) -> Result<Option<InputControlReply>> {
        let id = self.reserved_input_id()?;
        if let StageGateProgress::Waiting = self.advance_stage_gate(observation)? {
            return Ok(None);
        }
        if let Some(cause) = self.composer_interruption(id, observation)? {
            *terminal_bookkeeping = true;
            return self.interrupt_composer(conn, id, observation, effective, cause);
        }
        if self
            .pending
            .as_ref()
            .and_then(PendingInput::composer)
            .and_then(|composer| composer.settle_until)
            .is_some_and(|until| Instant::now() < until)
        {
            return Ok(None);
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
        self.checkpoint(
            id,
            #[cfg(test)]
            InputBoundary::BeforeWrite,
            effective,
        )?;
        Ok(None)
    }

    /// Stops a staged composer delivery; a deferred attempt publishes its partial result
    /// instead of replying.
    fn interrupt_composer(
        &mut self,
        conn: &mut Connection,
        id: i64,
        observation: &ObservationController,
        effective: Instant,
        cause: storage::StopCause,
    ) -> Result<Option<InputControlReply>> {
        let (mut stop, needs_close) = self.composer_stop(id, cause)?;
        if !stop.complete_prefix && cause.permits_left_in_composer() {
            stop.cause = storage::StopCause::ShortWrite;
        }
        if needs_close {
            self.prepare_paste_close(id, stop)?;
            return Ok(None);
        }
        self.stop_composer_stages(conn, id, observation, effective, stop)
    }

    fn prepare_paste_close(&mut self, id: i64, stop: ComposerStop) -> Result<()> {
        let pending = self.pending.as_mut().expect("stopped composer remains");
        pending.composer_mut().expect("ordinary composer").stop = Some(stop);
        pending.bytes = Arc::from(PASTE_CLOSE);
        pending.phase = InputPhase::Closing;
        let retained = self.operations.get_mut(&id).expect("stopped input remains");
        let RetainedSelection::Composer { closing, .. } = &mut retained.selection else {
            return Err(error("paste cleanup requires ordinary input"));
        };
        *closing = true;
        retained.bytes = Arc::from(PASTE_CLOSE);
        retained.attempt_available = false;
        Ok(())
    }

    fn finish_composer_stop(
        &mut self,
        conn: &mut Connection,
        id: i64,
        observation: &ObservationController,
        effective: Instant,
        stop: ComposerStop,
    ) -> Result<Option<InputControlReply>> {
        let failure = self
            .pending
            .as_mut()
            .and_then(PendingInput::composer_mut)
            .expect("stopped composer remains")
            .stop_failure
            .take();
        let settled = self.stop_composer_stages(conn, id, observation, effective, stop);
        match (failure, settled) {
            (Some(primary), Ok(_)) => Err(primary),
            (Some(primary), Err(bookkeeping)) => {
                Err(primary.with_diagnostic(bookkeeping.message()))
            }
            (None, result) => result,
        }
    }

    fn composer_stop(&self, id: i64, cause: storage::StopCause) -> Result<(ComposerStop, bool)> {
        let retained = self
            .operations
            .get(&id)
            .ok_or_else(|| error("stopped input is absent"))?;
        let composer = self
            .pending
            .as_ref()
            .and_then(PendingInput::composer)
            .ok_or_else(|| error("stopped composer is absent"))?;
        let RetainedSelection::Composer {
            stages,
            next_stage,
            offset,
            ..
        } = &retained.selection
        else {
            return Err(error("ordinary stop requires a composer selection"));
        };
        let literal_index = usize::from(composer.method == storage::DeliveryMethod::Fallback);
        let literal = &stages[literal_index];
        let written = if *next_stage > literal_index {
            literal.len()
        } else if *next_stage == literal_index {
            *offset
        } else {
            0
        };
        let stage = if *next_stage == stages.len() {
            storage::LeftStage::Submitted
        } else if written == 0
            && composer.method == storage::DeliveryMethod::Fallback
            && *next_stage > 0
        {
            storage::LeftStage::EscapeOnly
        } else if written == literal.len() {
            storage::LeftStage::LiteralComplete
        } else {
            storage::LeftStage::Partial
        };
        let prefix = composer.operation.prefix().len();
        let bracketed = literal
            .get(prefix..)
            .is_some_and(|bytes| bytes.starts_with(PASTE_OPEN));
        let complete_prefix = std::str::from_utf8(&literal[..written]).is_ok()
            && (!bracketed
                || written <= prefix
                || (written >= prefix + PASTE_OPEN.len()
                    && written <= literal.len() - PASTE_CLOSE.len())
                || written == literal.len());
        let needs_close =
            bracketed && written >= prefix + PASTE_OPEN.len() && written < literal.len();
        Ok((
            ComposerStop {
                cause,
                stage,
                complete_prefix,
            },
            needs_close,
        ))
    }

    fn write_pending_stage(
        &mut self,
        conn: &mut Connection,
        (transport, observation): (&mut Transport, &ObservationController),
        pty: &mut impl PtyIo,
        effective: Instant,
        terminal_bookkeeping: &mut bool,
    ) -> Result<Option<InputControlReply>> {
        let id = self.reserved_input_id()?;
        if let Some(cause) = self.composer_interruption(id, observation)? {
            *terminal_bookkeeping = true;
            return self.interrupt_composer(conn, id, observation, effective, cause);
        }
        let bytes = self
            .pending
            .as_ref()
            .expect("armed pending input")
            .bytes
            .clone();
        let Some(mut guard) = self.acquire_write_guard(conn, transport, observation, effective)?
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
            self.checkpoint(
                id,
                #[cfg(test)]
                InputBoundary::AfterWrite,
                effective,
            )
        } else {
            Ok(())
        };
        let advanced = |count: usize| -> Result<u64> {
            before_count
                .checked_add(
                    u64::try_from(count).map_err(|_| error("native input count exceeds range"))?,
                )
                .ok_or_else(|| error("native input count exhausted"))
        };
        let write = combine_manual_progress(write, after).and_then(|outcome| {
            let point = self.retained_checkpoint(id)?;
            match &outcome {
                ProgrammaticWrite::Complete { bytes: count } => ensure(
                    *count == bytes.len()
                        && matches!(
                            point.stage,
                            SubmissionStage::Partial | SubmissionStage::Submitted
                        )
                        && point.written_bytes == advanced(*count)?,
                    "complete native result conflicts with retained evidence",
                )?,
                ProgrammaticWrite::Partial { bytes: count } => ensure(
                    *count > 0
                        && *count < bytes.len()
                        && point.stage == SubmissionStage::Partial
                        && point.written_bytes == advanced(*count)?,
                    "partial native result conflicts with retained evidence",
                )?,
                ProgrammaticWrite::Pending => ensure(
                    point.stage == before_stage && point.written_bytes == before_count,
                    "pending native result conflicts with retained evidence",
                )?,
            }
            Ok(outcome)
        });
        let point = self.retained_checkpoint(id)?;
        let literal_stage = self
            .pending
            .as_ref()
            .and_then(PendingInput::composer)
            .is_some_and(|composer| {
                composer.written_stages
                    == usize::from(composer.method == storage::DeliveryMethod::Fallback)
            });
        if point.written_bytes > 0
            && let Some(composer) = self.pending.as_mut().and_then(PendingInput::composer_mut)
        {
            composer.started = true;
            composer
                .manual_generation
                .get_or_insert(self.manual_generation);
            if literal_stage {
                composer
                    .pointer_generation
                    .get_or_insert(self.pointer_generation);
            }
        }
        if matches!(&write, Ok(ProgrammaticWrite::Partial { .. })) && literal_stage {
            let next_bytes = self
                .operations
                .get(&id)
                .expect("partial receipt remains")
                .bytes
                .clone();
            let pending = self.pending.as_mut().expect("partial literal remains");
            pending.bytes = next_bytes;
            pending.phase = InputPhase::AwaitBeforeWrite;
            return Ok(None);
        }
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
            let settle = self.stage_settle();
            let pending = self.pending.as_mut().expect("pending multistage response");
            if let Some(composer) = pending.composer_mut() {
                composer.written_stages += 1;
                composer.settle_until = Some(Instant::now() + settle);
            }
            pending.bytes = next_bytes;
            pending.phase = InputPhase::AwaitBeforeWrite;
            return Ok(None);
        }
        if matches!(&write, Ok(ProgrammaticWrite::Complete { .. }))
            && point.stage == SubmissionStage::Submitted
            && let Some(composer) = self.pending.as_ref().and_then(PendingInput::composer)
            && self.verified_composer(composer.operation)
        {
            let verification = PostSubmitVerification {
                until: Instant::now() + POST_SUBMIT_VERIFICATION,
                output_seq: transport.snapshot().screen_seq,
                episode_seq: observation.active_episode_seq(),
            };
            let pending = self.pending.as_mut().expect("submitted input remains");
            pending
                .composer_mut()
                .expect("verified composer")
                .verification = Some(verification);
            pending.phase = InputPhase::Verifying;
            return Ok(None);
        }
        if matches!(&write, Ok(ProgrammaticWrite::Pending)) {
            self.pending
                .as_mut()
                .expect("pending zero-byte attempt")
                .phase = InputPhase::AwaitBeforeWrite;
            return Ok(None);
        }
        let stage = self
            .operations
            .get(&id)
            .expect("actual input outcome retained")
            .stage;
        let stop_cause = storage::StopCause::WriteFailed;
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.composer().is_some())
            && stage != SubmissionStage::ZeroBytes
            && (!matches!(&write, Ok(ProgrammaticWrite::Complete { .. }))
                || stage != SubmissionStage::Submitted)
        {
            *terminal_bookkeeping = true;
            let (mut stop, needs_close) = self.composer_stop(id, stop_cause)?;
            stop.complete_prefix = false;
            self.pending
                .as_mut()
                .and_then(PendingInput::composer_mut)
                .expect("failed composer remains")
                .stop_failure = write.err();
            if needs_close {
                self.prepare_paste_close(id, stop)?;
                return Ok(None);
            }
            return self.finish_composer_stop(conn, id, observation, effective, stop);
        }
        *terminal_bookkeeping = true;
        let persisted = with_database_deadline(conn, effective, |conn| {
            self.validate_live(conn)?;
            let next = match stage {
                SubmissionStage::Submitted => storage::InputState::Accepted,
                SubmissionStage::ZeroBytes => storage::InputState::Ready,
                SubmissionStage::Partial
                | SubmissionStage::Unknown
                | SubmissionStage::EscapeOnly
                | SubmissionStage::LiteralComplete => {
                    return storage::record_input_uncertain(
                        conn,
                        &self.run.owner(),
                        id,
                        stop_cause,
                    );
                }
            };
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
            self.hold_possible_ordinary_input(id);
        }
        let completed = match (write, persisted) {
            (Ok(_), Ok(operation)) => Ok(operation),
            (Err(primary), Ok(_)) | (Ok(_), Err(primary)) => Err(primary),
            (Err(primary), Err(bookkeeping)) => Err(primary.with_diagnostic(bookkeeping.message())),
        };
        self.pending.as_mut().expect("completed input").phase = InputPhase::Blocked;
        let operation = completed?;
        self.checkpoint(
            id,
            #[cfg(test)]
            InputBoundary::TerminalState,
            effective,
        )?;
        let pending = self.pending.as_ref().expect("completed input retained");
        if pending.deferred().is_some() {
            self.finish_deferred(conn, observation, &operation)?;
            self.pending = None;
            return Ok(None);
        }
        let reply = match &pending.permission() {
            Some(selection) => InputControlReply::PermissionResponse(
                self.permission_result(&selection.request.metadata, self.result(&operation)),
            ),
            None => InputControlReply::Submission(self.result(&operation)),
        };
        self.pending = None;
        Ok(Some(reply))
    }

    /// The started deferred attempt's input and whether it may have written a child byte.
    fn active_deferred(&self) -> Option<(i64, SubmissionStage, bool)> {
        let pending = self.pending.as_ref()?;
        pending.deferred()?;
        let id = pending
            .input_id()
            .expect("a deferred attempt has its receipt");
        let stage = self
            .operations
            .get(&id)
            .expect("a deferred attempt retains its receipt")
            .stage;
        let written = pending
            .composer()
            .as_ref()
            .is_some_and(|composer| composer.written_stages > 0);
        Some((id, stage, stage != SubmissionStage::ZeroBytes || written))
    }

    /// Records a still-`in_progress` deferred receipt as uncertain after a possible write and
    /// returns the receipt as stored.
    fn record_deferred_uncertain(
        &mut self,
        conn: &mut Connection,
        id: i64,
        cause: storage::StopCause,
        deadline: Instant,
    ) -> Result<storage::InputOperation> {
        self.hold_possible_ordinary_input(id);
        let current = with_database_deadline(conn, deadline, |conn| storage::get_input(conn, id))?;
        let operation = if current.state == storage::InputState::InProgress {
            with_database_deadline(conn, deadline, |conn| {
                storage::record_input_uncertain(conn, &self.run.owner(), id, cause)
            })?
        } else {
            current
        };
        let retained = self
            .operations
            .get_mut(&id)
            .expect("a deferred attempt retains its receipt");
        retained.operation = operation.clone();
        retained.attempt_available = false;
        Ok(operation)
    }

    /// Ends a deferred attempt that failed after a possible write: its receipt becomes uncertain,
    /// its result is published once and the queue waits for reconciliation. Without a durable
    /// uncertain receipt, recovery publishes the result instead.
    fn retire_failed_deferred(
        &mut self,
        conn: &mut Connection,
        observation: &ObservationController,
    ) {
        let Some((id, stage, written)) = self.active_deferred() else {
            return;
        };
        if !written {
            return;
        }
        self.hold_possible_ordinary_input(id);
        self.pending = None;
        match self.record_deferred_uncertain(
            conn,
            id,
            storage::StopCause::WriteFailed,
            Instant::now() + DEFERRED_FINALIZATION,
        ) {
            Ok(operation) if operation.state == storage::InputState::Uncertain => {
                Self::publish_result(observation, id, stage, operation.stop_cause);
            }
            Ok(operation) => {
                if let Err(cause) = self.finish_deferred(conn, observation, &operation) {
                    eprintln!(
                        "member runtime: input {id} result is pending recovery: {}",
                        cause.message()
                    );
                }
            }
            Err(cause) => eprintln!(
                "member runtime: input {id} result is pending recovery: {}",
                cause.message()
            ),
        }
    }

    fn stop_composer_stages(
        &mut self,
        conn: &mut Connection,
        id: i64,
        observation: &ObservationController,
        effective: Instant,
        stop: ComposerStop,
    ) -> Result<Option<InputControlReply>> {
        let pending = self.pending.as_mut().expect("stopped composer remains");
        pending.phase = InputPhase::Blocked;
        let request = pending.request.clone();
        let composer = pending.composer().expect("ordinary composer");
        let literal = composer.literal.clone();
        let composer_operation = composer.operation;
        let deferred = composer.deferred.is_some();
        let verified = self.verified_composer(composer_operation);
        let left = verified && stop.complete_prefix && stop.cause.permits_left_in_composer();
        let bookkeeping = if effective > Instant::now() {
            effective
        } else {
            Instant::now() + DEFERRED_FINALIZATION
        };
        let (operation, successor) = with_database_deadline(conn, bookkeeping, |conn| {
            self.validate_live_run(conn)?;
            if left && request.kind == storage::InputKind::Preview {
                let (input, next) = storage::settle_left_and_queue_redelivery(
                    conn,
                    &self.run.owner(),
                    id,
                    stop.cause,
                    stop.stage,
                )?;
                Ok((input, Some(next)))
            } else if left {
                storage::record_input_left_in_composer(
                    conn,
                    &self.run.owner(),
                    id,
                    stop.cause,
                    stop.stage,
                )
                .map(|input| (input, None))
            } else {
                storage::record_input_uncertain(conn, &self.run.owner(), id, stop.cause)
                    .map(|input| (input, None))
            }
        })?;
        let retained = self
            .operations
            .get_mut(&id)
            .expect("stopped receipt remains");
        retained.operation = operation.clone();
        retained.attempt_available = false;
        let stage = if left {
            match stop.stage {
                storage::LeftStage::EscapeOnly => SubmissionStage::EscapeOnly,
                storage::LeftStage::Partial => SubmissionStage::Partial,
                storage::LeftStage::LiteralComplete => SubmissionStage::LiteralComplete,
                storage::LeftStage::Submitted => SubmissionStage::Submitted,
            }
        } else {
            retained.stage
        };
        self.hold_possible_ordinary_input(id);
        if let Some(next) = successor {
            self.queue.push_front(QueuedInput {
                input_id: next.input_id,
                redelivery: true,
                stalled: false,
                request: storage::NewInput {
                    operation_key: next.operation_key,
                    kind: request.kind,
                    payload_digest: request.payload_digest,
                    message_id: request.message_id,
                },
                literal,
                operation: composer_operation,
                composer_busy_since: None,
                attempt_deadline: None,
                obstacle: "redelivery",
            });
        }
        self.checkpoint(
            id,
            #[cfg(test)]
            InputBoundary::TerminalState,
            bookkeeping,
        )?;
        Self::publish_result(observation, id, stage, operation.stop_cause);
        let reply = self.result(&operation);
        self.pending = None;
        Ok((!deferred).then_some(InputControlReply::Submission(reply)))
    }

    fn write_close_stage(
        &mut self,
        conn: &mut Connection,
        transport: &mut Transport,
        pty: &mut impl PtyIo,
        observation: &ObservationController,
        effective: Instant,
    ) -> Result<Option<InputControlReply>> {
        let id = self.reserved_input_id()?;
        if !self.hold_composer_pane()? {
            return Ok(None);
        }
        with_database_deadline(conn, effective, |conn| {
            self.validate_live_run(conn)?;
            self.lock.validate(conn, &self.run.owner())?;
            let retained = self.operations.get(&id).expect("closing input remains");
            ensure(
                storage::get_input(conn, id)? == retained.operation,
                "paste cleanup receipt changed",
            )
        })?;
        let retained = self.operations.get_mut(&id).expect("closing input remains");
        ensure(
            !retained.leased,
            "paste cleanup requires exclusive input authority",
        )?;
        retained.leased = true;
        let mut guard = InputGuard {
            retained: Some(retained),
            proof: None,
            binding: None,
            deadline: effective,
            permit: GuardPermit::Fresh,
            exclusion: std::marker::PhantomData,
        };
        let written = transport.write_programmatic(pty, PASTE_CLOSE, &mut guard, effective);
        drop(guard);
        let after = self.checkpoint(
            id,
            #[cfg(test)]
            InputBoundary::AfterWrite,
            effective,
        );
        let written = combine_manual_progress(written, after);
        if matches!(written, Ok(ProgrammaticWrite::Pending)) {
            return Ok(None);
        }
        let mut stop = self
            .pending
            .as_ref()
            .and_then(PendingInput::composer)
            .and_then(|composer| composer.stop)
            .expect("closing input has its stopped outcome");
        if !matches!(written, Ok(ProgrammaticWrite::Complete { .. })) {
            stop.cause = storage::StopCause::WriteFailed;
            stop.complete_prefix = false;
        }
        if let Err(cause) = written {
            let failure = &mut self
                .pending
                .as_mut()
                .and_then(PendingInput::composer_mut)
                .expect("closing composer remains")
                .stop_failure;
            *failure = Some(match failure.take() {
                Some(primary) => primary.with_diagnostic(cause.message()),
                None => cause,
            });
        } else if !matches!(written, Ok(ProgrammaticWrite::Complete { .. })) {
            let failure = &mut self
                .pending
                .as_mut()
                .and_then(PendingInput::composer_mut)
                .expect("closing composer remains")
                .stop_failure;
            if let Some(primary) = failure.take() {
                *failure = Some(
                    primary.with_diagnostic("paste closing marker was not completely written"),
                );
            }
        }
        self.finish_composer_stop(conn, id, observation, effective, stop)
    }

    fn verify_submit(
        &mut self,
        conn: &mut Connection,
        transport: &mut Transport,
        observation: &ObservationController,
        effective: Instant,
    ) -> Result<Option<InputControlReply>> {
        let id = self.reserved_input_id()?;
        let verification = self
            .pending
            .as_ref()
            .and_then(PendingInput::composer)
            .and_then(|composer| composer.verification.as_ref())
            .expect("active verification is required");
        let fresh = if let Some(proof) = transport.take_input_drain() {
            transport.validate_ordinary(&proof, effective)?;
            proof.output_seq() > verification.output_seq
        } else {
            false
        };
        let new_interaction = observation.active_episode_seq().is_some_and(|episode| {
            verification
                .episode_seq
                .is_none_or(|before| episode > before)
        }) && observation.status().eligibility
            != InputEligibility::Hold(GateReason::Candidate);
        if fresh && (Self::composer_eligible(observation) || new_interaction) {
            let operation = with_database_deadline(conn, effective, |conn| {
                self.validate_live_run(conn)?;
                storage::record_input_state(
                    conn,
                    &self.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    storage::InputState::Accepted,
                )
            })?;
            self.operations
                .get_mut(&id)
                .expect("verified input remains")
                .operation = operation.clone();
            let deferred = self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.deferred().is_some());
            if deferred {
                self.finish_deferred(conn, observation, &operation)?;
                self.pending = None;
                return Ok(None);
            }
            let reply = self.result(&operation);
            self.pending = None;
            return Ok(Some(InputControlReply::Submission(reply)));
        }
        if Instant::now() >= verification.until {
            let stop = ComposerStop {
                cause: storage::StopCause::SubmitUnconfirmed,
                stage: storage::LeftStage::Submitted,
                complete_prefix: true,
            };
            return self.stop_composer_stages(conn, id, observation, effective, stop);
        }
        Ok(None)
    }

    fn retain_permission_failure(
        &mut self,
        conn: &mut Connection,
        deadline: Instant,
        source: CafleetError,
    ) -> CafleetError {
        let Some(pending) = self.pending.as_mut() else {
            return source;
        };
        if pending.permission().is_none() {
            return source;
        }
        let binding = &pending
            .permission()
            .expect("a permission selection remains")
            .binding;
        let stop_cause = if self.manual_accounting_failed {
            storage::StopCause::ManualAccountingFailed
        } else if self.manual_generation != binding.manual_generation
            || self.pointer_generation != binding.pointer_generation
        {
            storage::StopCause::ManualInput
        } else {
            storage::StopCause::WriteFailed
        };
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
        let zero_bytes = retained.stage == SubmissionStage::ZeroBytes;
        let Ok(effective) = self.effective_deadline(conn, deadline) else {
            return source;
        };
        let persisted = with_database_deadline(conn, effective, |conn| {
            self.validate_live_run(conn)?;
            if zero_bytes {
                storage::record_input_state(
                    conn,
                    &self.run.owner(),
                    id,
                    storage::InputState::InProgress,
                    storage::InputState::Ready,
                )
            } else {
                storage::record_input_uncertain(conn, &self.run.owner(), id, stop_cause)
            }
        });
        match persisted {
            Ok(operation) => {
                self.operations
                    .get_mut(&id)
                    .expect("failed receipt retained")
                    .operation = operation;
                source
            }
            Err(bookkeeping) => source.with_diagnostic(bookkeeping.message()),
        }
    }

    /// Retires the pending immediate attempt; a started deferred attempt stays for `fence`,
    /// which settles it by its written bytes.
    pub(super) fn finish_pending_attempt(&mut self) {
        if self.active_deferred().is_some() {
            return;
        }
        if let Some(id) = self
            .pending
            .as_ref()
            .and_then(PendingInput::installed_input_id)
        {
            self.hold_possible_ordinary_input(id);
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
            Ok(member.placement)
        })?;
        let backend = coding_agent(&run.backend).ok_or_else(|| error("unknown input backend"))?;
        Ok(Self {
            placement,
            operations: BTreeMap::new(),
            pending: None,
            run,
            backend,
            lock,
            services,
            manual_generation: 0,
            pointer_generation: 0,
            manual_accounting_failed: false,
            current_request: None,
            input_fenced: false,
            observation_blocker: None,
            database_url,
            queue: std::collections::VecDeque::new(),
            finalization: None,
            pending_stamp: None,
            deferred_failed: false,
            held_composer: None,
            output_seq: 0,
            bracketed_paste: false,
        })
    }

    pub(super) fn progress_with_manual<T>(
        &mut self,
        transport: &mut Transport,
        step: impl FnOnce(&mut Transport) -> Result<T>,
    ) -> std::result::Result<T, InputFailure> {
        let before = transport.human_generation();
        let before_pointer = transport.pointer_generation();
        let progress = step(transport);
        self.output_seq = transport.snapshot().screen_seq;
        self.bracketed_paste = transport.bracketed_paste();
        self.observation_blocker = transport.drain_blocker().map(str::to_owned);
        let accounting = transport
            .human_generation()
            .checked_sub(before)
            .ok_or_else(|| error("manual input generation decreased"))
            .and_then(|count| {
                usize::try_from(count).map_err(|_| error("manual input delta exceeds count range"))
            })
            .and_then(|count| self.observe_manual(count));
        let pointer_accounting = transport
            .pointer_generation()
            .checked_sub(before_pointer)
            .ok_or_else(|| error("pointer input generation decreased"))
            .and_then(|count| {
                if count > 0 {
                    self.current_request = None;
                }
                self.pointer_generation = self
                    .pointer_generation
                    .checked_add(count)
                    .ok_or_else(|| error("pointer input generation exhausted"))?;
                Ok(())
            });
        let accounting = combine_manual_progress(accounting, pointer_accounting);
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
        (Err(primary), Err(accounting)) => Err(primary.with_diagnostic(accounting.message())),
    }
}

pub(crate) struct InputClockSample {
    pub(crate) before: Instant,
    pub(crate) monotonic_ns: u64,
    pub(crate) after: Instant,
}
impl InputClockSample {
    fn validate(&self) -> Result<()> {
        ensure(self.before <= self.after, "input clock bracket is reversed")
    }
}

pub(crate) trait InputDeadlineClock {
    fn sample(&mut self) -> Result<InputClockSample>;
}

pub(crate) struct RealInputDeadlineClock;
impl InputDeadlineClock for RealInputDeadlineClock {
    fn sample(&mut self) -> Result<InputClockSample> {
        let before = Instant::now();
        let time = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Known(
            rustix::time::ClockId::Monotonic,
        ))
        .map_err(io_error)?;
        let after = Instant::now();
        let seconds = u64::try_from(time.tv_sec)
            .map_err(|_| error("monotonic clock seconds must be nonnegative"))?;
        let nanoseconds = u64::try_from(time.tv_nsec)
            .map_err(|_| error("monotonic clock nanoseconds must be nonnegative"))?;
        ensure(
            nanoseconds < 1_000_000_000,
            "monotonic clock nanoseconds exceed their range",
        )?;
        let monotonic_ns = seconds
            .checked_mul(1_000_000_000)
            .and_then(|value| value.checked_add(nanoseconds))
            .ok_or_else(|| error("monotonic clock nanoseconds overflow"))?;
        Ok(InputClockSample {
            before,
            monotonic_ns,
            after,
        })
    }
}

pub(crate) struct InputExpiration(u64);
impl InputExpiration {
    pub(crate) fn capture(deadline: Instant, clock: &mut impl InputDeadlineClock) -> Result<Self> {
        let sample = clock.sample()?;
        sample.validate()?;
        let remaining = deadline
            .checked_duration_since(sample.after)
            .filter(|duration| !duration.is_zero())
            .ok_or(CafleetError::DeadlineExceeded)?;
        let remaining_ns = u64::try_from(remaining.as_nanos())
            .map_err(|_| error("input deadline duration exceeds monotonic nanoseconds"))?;
        let expiration = sample
            .monotonic_ns
            .checked_add(remaining_ns)
            .ok_or_else(|| error("input expiration overflows monotonic nanoseconds"))?;
        Ok(Self(expiration))
    }

    pub(crate) fn receive(
        &self,
        admitted_at: Instant,
        clock: &mut impl InputDeadlineClock,
    ) -> Result<Instant> {
        let sample = clock.sample()?;
        sample.validate()?;
        let remaining_ns = self
            .0
            .checked_sub(sample.monotonic_ns)
            .filter(|remaining| *remaining > 0)
            .ok_or(CafleetError::DeadlineExceeded)?;
        let expiration = sample
            .before
            .checked_add(Duration::from_nanos(remaining_ns))
            .ok_or_else(|| error("input expiration exceeds the local clock range"))?;
        let admission = admitted_at
            .checked_add(Duration::from_secs(5))
            .ok_or_else(|| error("input admission exceeds the local clock range"))?;
        let deadline = expiration.min(admission);
        check_deadline(sample.after, deadline)?;
        Ok(deadline)
    }

    pub(crate) fn monotonic_ns(&self) -> u64 {
        self.0
    }
}

pub(super) fn receive_input_expiration(
    expires_at_mono_ns: u64,
    admitted_at: Instant,
    clock: &mut impl InputDeadlineClock,
) -> Result<Instant> {
    InputExpiration(expires_at_mono_ns).receive(admitted_at, clock)
}

#[cfg(test)]
mod tests {
    mod manual_progress {
        use crate::broker::runtime as storage;
        use crate::coding_agent::ProfileCatalog;
        use crate::error::CafleetError;
        use crate::runtime::input::tests::ordinary_prompt::support::RunnerFixture;
        use crate::runtime::input::*;
        use std::io::ErrorKind;
        use std::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;
        use std::panic::resume_unwind;
        use std::time::Duration;
        use std::time::Instant;

        use crate::runtime::transport::test_support::{FakeDisplay, FakeInput, FakePty};

        impl RunnerFixture {
            fn coordinator(&mut self) -> InputCoordinator {
                InputCoordinator::new(
                    &mut self.fixture.conn,
                    &self.fixture.run,
                    self.lock.clone(),
                    crate::runtime::input::tests::ordinary_prompt::support::services(
                        ProfileCatalog::bundled().clone(),
                        crate::runtime::input::test_support::default_input_hooks(),
                        crate::runtime::input::STAGE_SETTLE,
                        "manual progress",
                    ),
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
                            &mut FakeInput::default(),
                            &mut FakePty::new(b"", 1),
                            &mut FakeDisplay::default(),
                            &mut self
                                .controller
                                .observer(&self.writer, &RealObservationClock),
                            self.until,
                        )
                        .unwrap(),
                    InputDrain::Quiescent
                );
            }
        }

        fn with_harness(body: impl FnOnce(&mut RunnerFixture)) {
            let mut harness = RunnerFixture::new(
                ProfileCatalog::bundled(),
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
                let mut human = FakeInput::default();
                human.chunks.push_back(b"first".to_vec());
                let mut pty = FakePty {
                    blocked: true,
                    ..FakePty::new(b"", 1)
                };
                let mut calls = 0;
                let result = coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        calls += 1;
                        transport.quiesce_for_input(
                            &mut human,
                            &mut pty,
                            &mut FakeDisplay::default(),
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
                            &mut FakeDisplay::default(),
                            &mut h.controller.observer(&h.writer, &RealObservationClock),
                        )
                    })
                    .unwrap();
                assert_eq!(zero.human_bytes, 0);
                assert_eq!(calls, 2);
                assert_eq!(h.transport.human_generation(), 5);
                assert_eq!(coordinator.manual_generation, 5);
                assert_eq!(h.transport.pending_input_bytes(), 5);

                human.chunks.push_back(b"+next".to_vec());
                pty.blocked = false;
                let next = coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        calls += 1;
                        transport.tick(
                            &mut human,
                            &mut pty,
                            &mut FakeDisplay::default(),
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
                let mut human = FakeInput::default();
                human.chunks.push_back(b"typed".to_vec());
                let mut pty = FakePty {
                    failure: Some(ErrorKind::BrokenPipe),
                    ..FakePty::new(b"", 1)
                };
                let mut calls = 0;
                let result = coordinator.progress_with_manual(&mut h.transport, |transport| {
                    calls += 1;
                    transport.tick(
                        &mut human,
                        &mut pty,
                        &mut FakeDisplay::default(),
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
                assert_eq!(pty.offers, vec![b"typed".to_vec()]);
                assert!(pty.written.is_empty());
                assert_eq!(pty.reads, 0);
                assert!(matches!(&failure.source, CafleetError::App(message)
                    if message == "member runtime: fixture PTY write failure"));
                assert_eq!(failure.source.exit_code(), 1);
                assert_eq!(failure.context.run_id, Some(h.fixture.run.run_id));
                assert_eq!(failure.context.interaction_id, None);
                assert_eq!(failure.context.input_id, None);
                assert_eq!(failure.context.operation_key, None);
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

    pub(in crate::runtime::input) mod ordinary_prompt {
        use crate::broker::runtime as storage;
        use crate::coding_agent::ProfileCatalog;
        use crate::error::CafleetError;
        use crate::runtime::input::*;
        use crate::runtime::test_support::Fixture;
        use std::io;
        use std::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;
        use std::panic::resume_unwind;
        use std::sync::Arc;
        use std::time::Duration;
        use std::time::Instant;

        use crate::runtime::input::tests::ordinary_prompt::support::*;

        fn submitted(reply: InputControlReply) -> SubmissionResult {
            match reply {
                InputControlReply::Submission(value) => value,
                _ => panic!("ordinary Prompt returns its actual submission"),
            }
        }

        #[test]
        fn checkpoint_hook_error_precedes_expiry_and_blocks_before_write() {
            use std::sync::Mutex;

            #[derive(Clone, Copy)]
            struct ObservedCheckpoint {
                input_id: i64,
                stage: SubmissionStage,
                written_bytes: u64,
                output_seq: u64,
                deadline: Instant,
            }

            struct ExpiringReservedHooks {
                fail: bool,
                observed: Mutex<Option<ObservedCheckpoint>>,
            }

            impl InputHooks for ExpiringReservedHooks {
                fn checkpoint(
                    &self,
                    boundary: InputBoundary,
                    state: &InputCheckpoint,
                    deadline: Instant,
                ) -> std::result::Result<(), CafleetError> {
                    assert!(matches!(boundary, InputBoundary::Reserved));
                    assert!(Instant::now() < deadline);
                    {
                        let mut observed = self.observed.lock().unwrap();
                        assert!(observed.is_none(), "one actual reservation checkpoint");
                        *observed = Some(ObservedCheckpoint {
                            input_id: state.input_id,
                            stage: state.stage,
                            written_bytes: state.written_bytes,
                            output_seq: state.output_seq,
                            deadline,
                        });
                    }
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    assert!(Instant::now() >= deadline);
                    if self.fail {
                        Err(CafleetError::App(
                            "fixture checkpoint failed after expiry".into(),
                        ))
                    } else {
                        Ok(())
                    }
                }
            }

            for fail in [true, false] {
                with_host(|h| {
                    let key = "prompt:checkpoint-error-expiry";
                    let cache_count = h.coordinator.operations.len();
                    assert!(h.handle(h.request(key)).unwrap().is_none());
                    let pending = h.pending_snapshot();
                    let hooks = Arc::new(ExpiringReservedHooks {
                        fail,
                        observed: Mutex::new(None),
                    });
                    h.coordinator.services.hooks = hooks.clone();
                    let deadline = Instant::now() + Duration::from_millis(150);
                    assert!(deadline < h.until);
                    let failure = (0..8)
                        .find_map(|_| {
                            h.drain();
                            match h.coordinator.advance(
                                &mut h.fixture.conn,
                                &mut h.transport,
                                &mut h.pty,
                                &h.controller,
                                deadline,
                            ) {
                                Err(failure) => Some(failure),
                                Ok(None) => None,
                                Ok(Some(_)) => panic!("the hook blocks before a reply"),
                            }
                        })
                        .expect("the real reservation reaches the configured checkpoint");
                    let ObservedCheckpoint {
                        input_id: id,
                        stage,
                        written_bytes: written,
                        output_seq,
                        deadline: actual_deadline,
                    } = hooks
                        .observed
                        .lock()
                        .unwrap()
                        .as_ref()
                        .copied()
                        .expect("the hook received the actual reserved input");
                    assert_eq!(actual_deadline, deadline);
                    assert!(Instant::now() >= deadline);
                    if fail {
                        assert!(
                            matches!(&failure.source, CafleetError::App(cause) if cause == "fixture checkpoint failed after expiry")
                        );
                    } else {
                        assert!(matches!(&failure.source, CafleetError::DeadlineExceeded));
                    }
                    assert_eq!(failure.context.run_id, Some(h.fixture.run.run_id));
                    assert_eq!(failure.context.input_id, Some(id));
                    assert_eq!(failure.context.operation_key.as_deref(), Some(key));
                    assert_eq!(stage, SubmissionStage::ZeroBytes);
                    assert_eq!(written, 0);
                    let retained = h.coordinator.retained_checkpoint(id).unwrap();
                    assert_eq!(
                        (
                            retained.input_id,
                            retained.stage,
                            retained.written_bytes,
                            retained.output_seq,
                        ),
                        (id, stage, written, output_seq)
                    );
                    let blocked = h.pending_snapshot();
                    assert_eq!(
                        (&blocked.0, &blocked.1, &blocked.2, blocked.3),
                        (&pending.0, &pending.1, &pending.2, pending.3)
                    );
                    assert!(matches!(blocked.4, InputPhase::Blocked));
                    assert_eq!(blocked.5, Some(id));
                    assert_eq!(h.coordinator.operations.len(), cache_count + 1);
                    let durable = storage::get_input(&h.fixture.conn, id).unwrap();
                    assert_eq!(durable.state, storage::InputState::InProgress);
                    let changes = h.fixture.conn.total_changes();
                    assert!(h.try_advance().unwrap().is_none());
                    assert!(h.pending_snapshot() == blocked);
                    assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), durable);
                    assert_eq!(h.fixture.conn.total_changes(), changes);
                    assert!(h.pty.offers.is_empty());
                    assert!(h.pty.stages.is_empty());
                    assert!(h.pty.program.is_empty());
                    assert!(h.pty.manual.is_empty());
                });
            }
        }

        #[test]
        fn prompt_reservation_and_each_hook_require_a_fresh_drain_before_one_real_write() {
            let profile = HostProfile {
                stage_settle: crate::runtime::input::STAGE_SETTLE,
                ..HostProfile::metadata()
            };
            with_profile_host(catalog(), encoded(), profile, |h| {
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
                let gaps = h.pty.stage_gaps();
                assert_eq!(gaps.len(), 1, "{gaps:?}");
                assert!(
                    gaps[0] >= crate::runtime::input::STAGE_SETTLE,
                    "the submit key follows the production settle: {gaps:?}"
                );
                assert!(!h.pty.offers.iter().any(|offer| offer.contains(&0x1b)));
                assert_eq!(delivery_selection(h, id), (Some("profile".into()), None));
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
        fn prompt_positive_short_writes_continue_to_one_confirmed_submission() {
            with_host(|host| {
                let id = host.arm("prompt:fixture-short");
                host.pty.short = Some(3);
                let reply = composer_reliability::finish(host, true);
                assert_eq!(reply.status, SubmissionStatus::Accepted);
                assert_eq!(reply.input_id, Some(id));
                assert_eq!(host.pty.written, encoded());
                assert_eq!(
                    storage::get_input(&host.fixture.conn, id).unwrap().state,
                    storage::InputState::Accepted
                );
                let checkpoint = host.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(checkpoint.written_bytes, encoded().len() as u64);
                assert!(host.hooks.count("after") > 1);
                let points = host.hooks.points.lock().unwrap();
                let after = points
                    .iter()
                    .rev()
                    .find(|point| point.boundary == "after" && point.id == Some(id))
                    .expect("the completed short writes retain their final checkpoint");
                assert_eq!(after.count, checkpoint.written_bytes);
                assert!(after.seq <= checkpoint.output_seq);
            });
        }

        #[test]
        fn prompt_failed_write_after_a_possible_byte_records_write_failed() {
            with_host(|h| {
                let id = h.arm("prompt:fixture-failed");
                h.pty.failure = Some(io::ErrorKind::BrokenPipe);
                h.drain();
                let Err(failure) = h.try_advance() else {
                    panic!("an actual PTY write failure surfaces");
                };
                assert!(
                    failure
                        .source
                        .message()
                        .contains("injected PTY write failure"),
                    "{failure:?}"
                );
                let stored = storage::get_input(&h.fixture.conn, id).unwrap();
                assert_eq!(stored.state, storage::InputState::Uncertain);
                assert_eq!(stored.stop_cause, Some(storage::StopCause::WriteFailed));
                h.pty.failure = None;
                let repeat = h.request("prompt:fixture-failed");
                let replay = submitted(h.handle(repeat).unwrap().expect("retained key lookup"));
                assert_eq!(replay.status, SubmissionStatus::Uncertain);
                assert_eq!(replay.reason.as_deref(), Some("write_failed"));
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
                let original = storage::test_support::reserve_input(
                    &mut independent,
                    &h.fixture.run.owner(),
                    &new,
                )
                .unwrap();
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
            use crate::runtime::input::tests::ordinary_prompt::*;

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
            use crate::runtime::input::tests::ordinary_prompt::*;

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
                    let message = host.message(LITERAL);
                    let key = crate::broker::runtime::preview_operation_key(
                        host.fixture.run.run_id,
                        message.message_id,
                        1,
                    )
                    .unwrap();
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
                            crate::broker::runtime::preview_operation_key(
                                key_run,
                                message.message_id,
                                1,
                            )
                            .unwrap(),
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
                    "prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-exec-plan"]},
                    "exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"6b6c","fixture_ids":["synthetic-exec-plan"]}
                });
                let exec_catalog = crate::coding_agent::catalog::test_support::load(&definition)
                    .unwrap()
                    .without_submission(
                        crate::coding_agent::CodingAgent::Codex,
                        crate::coding_agent::ComposerOperation::Prompt,
                    );
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

        mod composer_reliability {
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::{
                form, queued, result_events, row,
            };
            use crate::runtime::input::tests::ordinary_prompt::*;
            use serde_json::{Value, json};

            const DRAFT: &[u8] = b"\x1b[2J\x1b[1;1Hretained native composer";
            const OPEN: &[u8] = b"\x1b[200~";
            const CLOSE: &[u8] = b"\x1b[201~";

            fn after_native_submit(fixture_id: &str, body: impl FnOnce(&mut Host)) {
                use crate::coding_agent::catalog::LiteralFraming;
                use crate::coding_agent::catalog::test_support::bundled_definitions;
                use crate::coding_agent::detector::test_support::bundled_screen;
                use crate::coding_agent::detector::test_support::native_fixture_bytes;
                use crate::coding_agent::{CodingAgent, ComposerOperation};

                let definitions = bundled_definitions();
                let fixtures = definitions["fixtures"].as_array().unwrap();
                let fixture = fixtures
                    .iter()
                    .find(|entry| entry["id"] == fixture_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "the authoritative retained/working fixture is registered: {fixture_id}"
                        )
                    });
                let backend: CodingAgent =
                    serde_json::from_value(fixture["backend"].clone()).unwrap();
                let native_version = fixture["native_version"].as_str().unwrap();
                let idle = fixtures
                    .iter()
                    .find(|candidate| {
                        candidate["backend"] == fixture["backend"]
                            && candidate["native_version"] == fixture["native_version"]
                            && candidate["rows"] == fixture["rows"]
                            && candidate["cols"] == fixture["cols"]
                            && candidate["id"]
                                .as_str()
                                .unwrap()
                                .contains("-composer-prompt-idle-mode-2004-")
                    })
                    .expect(
                        "each measured backend/geometry has its captured initial empty composer",
                    );
                let operation = if fixture_id.contains("-composer-exec-") {
                    ComposerOperation::Exec
                } else {
                    assert!(fixture_id.contains("-composer-prompt-"), "{fixture_id}");
                    ComposerOperation::Prompt
                };
                let literal = "printf native-verification";
                let catalog = ProfileCatalog::bundled().clone();
                let plan = catalog
                    .composer_plan(backend, native_version, operation)
                    .unwrap();
                assert!(plan.plain_literal_allowed(literal));
                let expected = plan
                    .stages(literal, LiteralFraming::Plain)
                    .unwrap()
                    .concat();
                with_profile_host(
                    catalog,
                    expected.clone(),
                    HostProfile {
                        backend,
                        native_version,
                        size: TerminalSize {
                            rows: u16::try_from(fixture["rows"].as_u64().unwrap()).unwrap(),
                            cols: u16::try_from(fixture["cols"].as_u64().unwrap()).unwrap(),
                        },
                        screen: native_fixture_bytes(&bundled_screen(idle["id"].as_str().unwrap())),
                        ..HostProfile::metadata()
                    },
                    |host| {
                        host.confirm_submissions = false;
                        assert!(
                            !host.transport.bracketed_paste(),
                            "native screen replay leaves mode 2004 off"
                        );
                        let mut request = host.request("native:verification");
                        let input = SubmitInput {
                            operation_key: "native:verification".into(),
                            input_kind: if operation == ComposerOperation::Exec {
                                storage::InputKind::Exec
                            } else {
                                storage::InputKind::Prompt
                            },
                            message_id: None,
                            literal: literal.into(),
                        };
                        request.operation = if operation == ComposerOperation::Exec {
                            InputControlOperation::SubmitExec(input)
                        } else {
                            InputControlOperation::SubmitPrompt(input)
                        };
                        assert!(host.handle(request).unwrap().is_none());
                        until_written(host, expected.len());
                        assert_eq!(host.pty.written, expected, "{fixture_id}");
                        assert!(
                            host.coordinator.pending.is_some(),
                            "the complete submit key awaits native confirmation: {fixture_id}"
                        );
                        body(host);
                    },
                );
            }

            fn native_verification_reply(host: &mut Host) -> SubmissionResult {
                for _ in 0..256 {
                    if let Some(reply) = turn(host) {
                        return reply;
                    }
                }
                panic!("native verification completes within bounded real coordinator turns");
            }

            #[test]
            fn every_authoritative_retained_native_snapshot_leaves_submit_unconfirmed() {
                use crate::coding_agent::detector::test_support::{
                    bundled_screen, native_fixture_bytes,
                };
                for fixture_id in [
                    "claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68",
                    "claude-2.1.319-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68",
                    "claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
                    "claude-2.1.319-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
                    "codex-0.167.0-composer-exec-plain-42-args-literal-tapped-v2-herdr-119-39",
                    "codex-0.167.0-composer-exec-plain-42-args-literal-tapped-v2-tmux-120-39",
                    "hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-99",
                    "hermes-2026.9.24-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-100",
                    "hermes-2026.9.24-composer-prompt-plain-41-literal-tapped-v2-herdr-119-49",
                    "hermes-2026.9.24-composer-prompt-plain-41-literal-tapped-v2-tmux-120-50",
                    "opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-68",
                    "opencode-2.2.5-composer-exec-plain-41-args-literal-tapped-v2-tmux-120-68",
                    "opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
                    "opencode-2.2.5-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
                    "pi-1.5.0-composer-exec-plain-41-args-literal-tapped-v2-herdr-119-69",
                    "pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-herdr-119-33",
                    "pi-1.5.0-composer-prompt-plain-41-literal-tapped-v2-tmux-120-33",
                    "codex-0.167.0-composer-prompt-plain-1025-post-1900ms-tapped-v2-tmux-120-37",
                    "codex-0.167.0-composer-prompt-plain-1025-post-1900ms-tapped-v2-herdr-119-37",
                    "codex-0.167.0-composer-exec-plain-1025-args-post-1900ms-tapped-v2-tmux-120-37",
                    "codex-0.167.0-composer-exec-plain-1025-args-post-1900ms-tapped-v2-herdr-119-37",
                    "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-tmux-120-44",
                    "hermes-2026.9.24-composer-prompt-plain-4000-max-preview-chunks-post-500ms-tapped-v2-herdr-119-43",
                    "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-tmux-120-94",
                    "hermes-2026.9.24-composer-exec-plain-4000-args-max-preview-chunks-post-500ms-tapped-v2-herdr-119-93",
                    "codex-0.167.0-composer-prompt-retained-authoritative-no-submit-tapped-v2-herdr-119-2",
                    "codex-0.167.0-composer-prompt-retained-authoritative-no-submit-tapped-v2-tmux-120-2",
                    "pi-1.5.0-composer-exec-retained-authoritative-no-submit-tapped-v2-tmux-120-2",
                ] {
                    after_native_submit(fixture_id, |host| {
                        host.show(native_fixture_bytes(&bundled_screen(fixture_id)));
                        assert_ne!(
                            host.controller.health(&host.writer).eligibility,
                            InputEligibility::ComposerEmpty,
                            "{fixture_id}"
                        );
                        host.coordinator
                            .set_post_submit_verification_until(Instant::now());
                        let reply = native_verification_reply(host);
                        assert_left(
                            host,
                            &reply,
                            storage::StopCause::SubmitUnconfirmed,
                            storage::LeftStage::Submitted,
                        );
                        assert_eq!(reply.stage, SubmissionStage::Submitted, "{fixture_id}");
                    });
                }
            }

            #[test]
            fn measured_working_composers_and_claude_ghost_hint_confirm_written_submit_keys() {
                use crate::coding_agent::detector::test_support::{
                    bundled_screen, native_fixture_bytes,
                };
                for fixture_id in [
                    "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-tmux-120-61",
                    "pi-1.5.0-composer-exec-plain-4000-args-max-preview-chunks-post-1900ms-tapped-v2-herdr-119-61",
                    "hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-tmux-120-7",
                    "hermes-2026.9.24-composer-prompt-plain-40-post-1900ms-tapped-v2-herdr-119-7",
                    "claude-2.1.319-composer-prompt-key-probe-before-tapped-v2-tmux-120-1",
                ] {
                    after_native_submit(fixture_id, |host| {
                        host.show(native_fixture_bytes(&bundled_screen(fixture_id)));
                        let reply = native_verification_reply(host);
                        assert_eq!(reply.status, SubmissionStatus::Accepted, "{fixture_id}");
                        assert_eq!(reply.stage, SubmissionStage::Submitted, "{fixture_id}");
                        assert_eq!(stored(host, &reply).stop_cause, None, "{fixture_id}");
                    });
                }
            }

            fn configured(framing: &str, limit: Value) -> ProfileCatalog {
                let mut value = catalog_definition();
                for operation in ["prompt", "exec"] {
                    let plan = &mut value["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation];
                    plan["literal_framing"] = json!(framing);
                    plan["plain_literal_limit"] = limit.clone();
                }
                crate::coding_agent::catalog::test_support::load(&value).unwrap()
            }

            fn permission_configured(framing: &str, limit: Value) -> ProfileCatalog {
                let mut definition =
                    crate::coding_agent::catalog::test_support::inspection_fixture_definition();
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
                for operation in ["prompt", "exec"] {
                    let plan = &mut definition["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation];
                    plan["literal_framing"] = json!(framing);
                    plan["plain_literal_limit"] = limit.clone();
                }
                crate::coding_agent::catalog::test_support::load(&definition).unwrap()
            }

            fn program(literal: &str, bracketed: bool) -> Vec<u8> {
                let mut bytes = Vec::new();
                if bracketed {
                    bytes.extend_from_slice(OPEN);
                }
                bytes.extend_from_slice(literal.as_bytes());
                if bracketed {
                    bytes.extend_from_slice(CLOSE);
                }
                bytes.push(b'~');
                bytes
            }

            fn request(host: &Host, key: &str, literal: &str) -> InputControlRequest {
                let mut request = host.request(key);
                let InputControlOperation::SubmitPrompt(input) = &mut request.operation else {
                    panic!("the prompt harness creates a prompt request")
                };
                input.literal = literal.into();
                request
            }

            fn turn(host: &mut Host) -> Option<SubmissionResult> {
                if let Some(composer) = host
                    .coordinator
                    .pending
                    .as_mut()
                    .and_then(PendingInput::composer_mut)
                    && composer.settle_until.is_some()
                {
                    composer.settle_until = Some(Instant::now());
                }
                host.drain();
                host.advance().map(submitted)
            }

            fn until_written(host: &mut Host, bytes: usize) {
                for _ in 0..256 {
                    if host.pty.written.len() >= bytes {
                        return;
                    }
                    assert!(
                        turn(host).is_none(),
                        "the literal reaches its checkpoint before replying"
                    );
                }
                panic!("the actual writer reaches its checkpoint within bounded turns");
            }

            pub(super) fn finish(host: &mut Host, confirm: bool) -> SubmissionResult {
                for _ in 0..256 {
                    if host.coordinator.pending.is_some()
                        && host
                            .pty
                            .offers
                            .last()
                            .is_some_and(|bytes| bytes == b"~" || bytes == b"\r")
                    {
                        if confirm {
                            host.child_output(SCREEN);
                        } else {
                            host.coordinator
                                .set_post_submit_verification_until(Instant::now());
                        }
                    }
                    if let Some(reply) = turn(host) {
                        return reply;
                    }
                }
                panic!("the actual coordinator finishes within bounded turns");
            }

            fn stored(host: &Host, reply: &SubmissionResult) -> storage::InputOperation {
                storage::get_input(
                    &host.fixture.conn,
                    reply.input_id.expect("a written input has a receipt"),
                )
                .unwrap()
            }

            fn assert_left(
                host: &Host,
                reply: &SubmissionResult,
                cause: storage::StopCause,
                stage: storage::LeftStage,
            ) {
                assert_eq!(reply.status, SubmissionStatus::LeftInComposer);
                let input = stored(host, reply);
                assert_eq!(input.state, storage::InputState::Settled);
                assert_eq!(
                    input.disposition,
                    Some(storage::InputDisposition::LeftInComposer)
                );
                assert_eq!(input.stop_cause, Some(cause));
                assert_eq!(input.left_stage, Some(stage));
                assert_eq!(reply.reason.as_deref(), Some(cause.as_str()));
            }

            fn enable_paste(host: &mut Host) {
                host.child_output(b"\x1b[?2004h");
                host.drain();
                assert!(host.transport.bracketed_paste());
            }

            fn start(host: &mut Host, key: &str, literal: &str) {
                host.confirm_submissions = false;
                let request = request(host, key, literal);
                assert!(host.handle(request).unwrap().is_none());
            }

            fn stopped_preview(
                host: &mut Host,
                short: bool,
            ) -> (crate::broker::MessageRecord, i64) {
                host.confirm_submissions = false;
                let message = host.message("stopped preview");
                let literal = canonical_preview(&message);
                host.pty.expect_program(program(&literal, false));
                if short {
                    host.pty.short = Some(3);
                }
                let request = preview_request(host, &message, &literal);
                assert!(host.handle(request).unwrap().is_none());
                until_written(host, if short { 3 } else { literal.len() });
                host.show(DRAFT.to_vec());
                host.type_human(b"x");
                let reply = finish(host, false);
                assert_left(
                    host,
                    &reply,
                    storage::StopCause::ManualInput,
                    if short {
                        storage::LeftStage::Partial
                    } else {
                        storage::LeftStage::LiteralComplete
                    },
                );
                (message, reply.input_id.unwrap())
            }

            fn old_wait() -> String {
                crate::time::format_utc(chrono::Utc::now() - chrono::Duration::seconds(181))
            }

            #[test]
            fn pointer_input_during_the_settle_preserves_ordinary_submission() {
                for pointer in [b"\x1b[<35;10;20M".as_slice(), b"\x1b[M !!"] {
                    with_host(|host| {
                        start(host, "prompt:pointer-settle", LITERAL);
                        until_written(host, LITERAL.len());
                        let keyboard = host.transport.human_generation();
                        host.type_human(pointer);
                        let reply = finish(host, true);
                        assert_eq!(reply.status, SubmissionStatus::Accepted);
                        assert_eq!(host.transport.human_generation(), keyboard);
                        assert_eq!(host.transport.pointer_generation(), pointer.len() as u64);
                        assert_eq!(stored(host, &reply).stop_cause, None);
                    });
                }
            }

            #[test]
            fn keyboard_input_during_the_settle_records_a_complete_left_literal() {
                with_host(|host| {
                    start(host, "prompt:keyboard-settle", LITERAL);
                    until_written(host, LITERAL.len());
                    host.type_human(b"x");
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::ManualInput,
                        storage::LeftStage::LiteralComplete,
                    );
                    assert!(!host.pty.offers.iter().any(|bytes| bytes == b"~"));
                });
            }

            #[test]
            fn positive_short_literal_writes_continue_without_settling_between_chunks() {
                let literal = "あ".repeat(4000);
                with_catalog_host(
                    configured("plain", Value::Null),
                    program(&literal, false),
                    |host| {
                        host.pty.short = Some(1000);
                        start(host, "prompt:long-chunks", &literal);
                        until_written(host, 1000);
                        assert_eq!(host.pty.written, literal.as_bytes()[..1000]);
                        let keyboard = host.transport.human_generation();
                        let pointer = host.transport.pointer_generation();
                        let output_seq = host.transport.snapshot().screen_seq;
                        let visible_prefix =
                            std::str::from_utf8(&literal.as_bytes()[..999]).unwrap();
                        host.show(format!("\x1b[2J\x1b[1;1H{visible_prefix}").into_bytes());
                        assert!(host.transport.snapshot().screen_seq > output_seq);
                        assert_ne!(
                            host.controller.health(&host.writer).eligibility,
                            InputEligibility::ComposerEmpty,
                        );
                        assert_eq!(host.transport.human_generation(), keyboard);
                        assert_eq!(host.transport.pointer_generation(), pointer);
                        until_written(host, 2000);
                        assert_eq!(host.pty.written, literal.as_bytes()[..2000]);
                        assert!(
                            host.coordinator
                                .pending
                                .as_ref()
                                .unwrap()
                                .composer()
                                .unwrap()
                                .settle_until
                                .is_none()
                        );
                        let reply = finish(host, true);
                        assert_eq!(reply.status, SubmissionStatus::Accepted);
                        assert_eq!(host.pty.written, program(&literal, false));
                    },
                );
            }

            #[test]
            fn pointer_input_between_literal_chunks_stops_and_closes_the_open_paste() {
                let literal = "abcdefghi";
                with_catalog_host(
                    configured("bracketed_paste", Value::Null),
                    program(literal, true),
                    |host| {
                        enable_paste(host);
                        host.pty.short = Some(OPEN.len() + 3);
                        start(host, "prompt:pointer-chunks", literal);
                        until_written(host, OPEN.len() + 3);
                        host.pty.short = None;
                        host.type_human(b"\x1b[<0;10;20M");
                        let reply = finish(host, false);
                        assert_left(
                            host,
                            &reply,
                            storage::StopCause::PointerInput,
                            storage::LeftStage::Partial,
                        );
                        assert_eq!(host.pty.offers.last().unwrap(), CLOSE);
                        assert!(!host.pty.offers.iter().any(|bytes| bytes == b"~"));
                    },
                );
            }

            #[test]
            fn keyboard_input_between_chunks_records_the_complete_written_prefix() {
                with_host(|host| {
                    host.pty.short = Some(3);
                    start(host, "prompt:keyboard-chunks", LITERAL);
                    until_written(host, 3);
                    host.type_human(b"x");
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::ManualInput,
                        storage::LeftStage::Partial,
                    );
                });
            }

            #[test]
            fn a_stop_inside_a_utf8_scalar_closes_paste_and_remains_uncertain() {
                let literal = "あいう";
                with_catalog_host(
                    configured("bracketed_paste", Value::Null),
                    program(literal, true),
                    |host| {
                        enable_paste(host);
                        host.pty.short = Some(OPEN.len() + 1);
                        start(host, "prompt:scalar-prefix", literal);
                        until_written(host, OPEN.len() + 1);
                        host.pty.short = None;
                        host.type_human(b"x");
                        let reply = finish(host, false);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        assert_eq!(
                            stored(host, &reply).stop_cause,
                            Some(storage::StopCause::ShortWrite)
                        );
                        assert_eq!(host.pty.offers.last().unwrap(), CLOSE);
                    },
                );
            }

            #[test]
            fn a_stop_inside_the_opening_marker_does_not_emit_a_close_marker() {
                with_catalog_host(
                    configured("bracketed_paste", Value::Null),
                    program(LITERAL, true),
                    |host| {
                        enable_paste(host);
                        host.pty.short = Some(3);
                        start(host, "prompt:marker-prefix", LITERAL);
                        until_written(host, 3);
                        host.pty.short = None;
                        host.type_human(b"x");
                        let reply = finish(host, false);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        assert_eq!(
                            stored(host, &reply).stop_cause,
                            Some(storage::StopCause::ShortWrite)
                        );
                        assert!(!host.pty.offers.iter().any(|bytes| bytes == CLOSE));
                    },
                );
            }

            #[test]
            fn a_chunk_deadline_at_a_complete_prefix_records_partial_deadline_expired() {
                with_host(|host| {
                    host.pty.short = Some(3);
                    start(host, "prompt:chunk-deadline", LITERAL);
                    until_written(host, 3);
                    host.coordinator.pending.as_mut().unwrap().deadline =
                        Instant::now() + Duration::from_millis(100);
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::DeadlineExpired,
                        storage::LeftStage::Partial,
                    );
                });
            }

            #[test]
            fn a_verified_fallback_stop_after_escape_records_escape_only() {
                let mut expected = b"\x1b".to_vec();
                expected.extend_from_slice(LITERAL.as_bytes());
                expected.push(b'\r');
                with_catalog_host(configured("plain", Value::Null), expected, |host| {
                    host.show(DRAFT.to_vec());
                    let id = queued(
                        host,
                        request(host, "prompt:escape-only", LITERAL),
                        "composer_busy",
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(id, &old_wait());
                    until_written(host, 1);
                    assert_eq!(host.pty.written, b"\x1b");
                    host.type_human(b"x");
                    for _ in 0..64 {
                        let _ = turn(host);
                    }
                    let input = row(host, id);
                    assert_eq!(
                        input.disposition,
                        Some(storage::InputDisposition::LeftInComposer)
                    );
                    assert_eq!(input.left_stage, Some(storage::LeftStage::EscapeOnly));
                    assert_eq!(input.stop_cause, Some(storage::StopCause::ManualInput));
                });
            }

            #[test]
            fn bracketed_framing_is_selected_only_while_mode_2004_is_enabled() {
                for enabled in [false, true] {
                    with_catalog_host(
                        configured("bracketed_paste", Value::Null),
                        program(LITERAL, enabled),
                        |host| {
                            if enabled {
                                enable_paste(host);
                            }
                            start(host, "prompt:mode-selection", LITERAL);
                            let reply = finish(host, true);
                            assert_eq!(reply.status, SubmissionStatus::Accepted);
                            assert_eq!(host.pty.written, program(LITERAL, enabled));
                        },
                    );
                }
            }

            #[test]
            fn immediate_over_limit_prompt_is_refused_before_receipt_or_bytes() {
                assert_immediate_limit_rejection(storage::InputKind::Prompt);
            }

            #[test]
            fn immediate_over_limit_exec_is_refused_before_receipt_or_bytes() {
                assert_immediate_limit_rejection(storage::InputKind::Exec);
            }

            fn assert_immediate_limit_rejection(kind: storage::InputKind) {
                for framing in ["plain", "bracketed_paste"] {
                    with_catalog_host(
                        configured(framing, json!({"unit":"bytes","max":1})),
                        encoded(),
                        |host| {
                            let before = crate::broker::runtime::test_support::rows(
                                &host.fixture.conn,
                                "member_input_operations",
                            );
                            let mut input = request(host, "ordinary:over-limit", LITERAL);
                            if kind == storage::InputKind::Exec {
                                let InputControlOperation::SubmitPrompt(mut body) = input.operation
                                else {
                                    panic!("the harness creates a prompt body")
                                };
                                body.input_kind = kind;
                                input.operation = InputControlOperation::SubmitExec(body);
                            }
                            let reply = submitted(
                                host.handle(input)
                                    .unwrap()
                                    .expect("a pre-write refusal returns immediately"),
                            );
                            assert_eq!(reply.status, SubmissionStatus::Rejected);
                            assert_eq!(reply.stage, SubmissionStage::ZeroBytes);
                            assert_eq!(reply.input_id, None);
                            assert_eq!(reply.output_seq, None);
                            assert_eq!(reply.delivery, None);
                            let size = LITERAL.len()
                                + if kind == storage::InputKind::Exec {
                                    2
                                } else {
                                    0
                                };
                            let cause = if framing == "plain" {
                                "the codex profile has no bracketed-paste delivery"
                            } else {
                                "the native CLI has not enabled bracketed paste"
                            };
                            assert_eq!(
                                reply.reason,
                                Some(format!(
                                    "the input is {size} bytes, above the codex plain-input limit of 1 bytes, and {cause}"
                                ))
                            );
                            assert!(host.pty.offers.is_empty());
                            assert_eq!(
                                crate::broker::runtime::test_support::rows(
                                    &host.fixture.conn,
                                    "member_input_operations"
                                ),
                                before
                            );
                        },
                    );
                }
            }

            #[test]
            fn zero_byte_reselection_refuses_with_the_actual_ready_receipt_and_projection() {
                for kind in [storage::InputKind::Prompt, storage::InputKind::Exec] {
                    with_catalog_host(
                        configured("bracketed_paste", json!({"unit":"bytes","max":1})),
                        encoded(),
                        |host| {
                            enable_paste(host);
                            let mut input = request(host, "ordinary:mode-disabled", LITERAL);
                            if kind == storage::InputKind::Exec {
                                let InputControlOperation::SubmitPrompt(mut body) = input.operation
                                else {
                                    panic!("the harness creates a prompt body")
                                };
                                body.input_kind = kind;
                                input.operation = InputControlOperation::SubmitExec(body);
                            }
                            assert!(host.handle(input).unwrap().is_none());
                            assert!(turn(host).is_none());
                            assert!(host.pty.offers.is_empty());
                            let id = host
                                .coordinator
                                .pending
                                .as_ref()
                                .unwrap()
                                .input_id()
                                .unwrap();
                            let before = row(host, id);
                            assert_eq!(before.state, storage::InputState::InProgress);
                            let retained = host.coordinator.operations.get(&id).unwrap();
                            assert_eq!(retained.stage, SubmissionStage::ZeroBytes);
                            assert_eq!(retained.written_bytes, 0);
                            assert!(!retained.leased);
                            let mut screen = b"\x1b[?2004l".to_vec();
                            screen.extend_from_slice(SCREEN);
                            host.show(screen);
                            assert!(!host.transport.bracketed_paste());
                            let reply = finish(host, false);
                            assert_eq!(reply.status, SubmissionStatus::Rejected);
                            assert_eq!(reply.stage, SubmissionStage::ZeroBytes);
                            assert_eq!(reply.input_id, Some(id));
                            let after = row(host, id);
                            assert_eq!(after.state, storage::InputState::Ready);
                            assert_eq!(after.operation_key, before.operation_key);
                            assert_eq!(after.payload_digest, before.payload_digest);
                            assert_eq!(after.kind, kind);
                            assert_eq!(after.disposition, None);
                            assert_eq!(reply.delivery, after.delivery());
                            let size = LITERAL.len()
                                + if kind == storage::InputKind::Exec {
                                    2
                                } else {
                                    0
                                };
                            assert_eq!(
                                reply.reason,
                                Some(format!(
                                    "the input is {size} bytes, above the codex plain-input limit of 1 bytes, and the native CLI has not enabled bracketed paste"
                                ))
                            );
                            assert!(host.pty.offers.is_empty());
                        },
                    );
                }
            }

            #[test]
            fn over_limit_preview_uses_the_pointer_form_with_the_same_payload_identity() {
                with_catalog_host(
                    configured("plain", json!({"unit":"bytes","max":1024})),
                    encoded(),
                    |host| {
                        let message = host.message(&"あ".repeat(3000));
                        let canonical = canonical_preview(&message);
                        let expected_digest = storage::input_payload_digest(
                            host.fixture.run.run_id,
                            &SubmitInput {
                                operation_key: storage::preview_operation_key(
                                    host.fixture.run.run_id,
                                    message.message_id,
                                    1,
                                )
                                .unwrap(),
                                input_kind: storage::InputKind::Preview,
                                message_id: Some(message.message_id),
                                literal: canonical.clone(),
                            },
                        );
                        let request = preview_request(host, &message, &canonical);
                        assert!(host.handle(request).unwrap().is_none());
                        let reply = finish(host, true);
                        assert_eq!(reply.status, SubmissionStatus::Accepted);
                        let text = String::from_utf8(host.pty.written.clone()).unwrap();
                        assert!(text.contains(&format!(
                            "Body is 3000 characters; run cafleet message poll {} --json to read it.",
                            message.to_member_id
                        )));
                        assert!(!text.contains(&message.text));
                        assert_eq!(stored(host, &reply).payload_digest, expected_digest);
                    },
                );
            }

            #[test]
            fn queued_prompt_over_the_limit_settles_not_submitted_and_advances() {
                with_catalog_host(
                    permission_configured("plain", json!({"unit":"bytes","max":1})),
                    encoded(),
                    |host| {
                        form(host);
                        let id = queued(
                            host,
                            request(host, "prompt:queued-over-limit", LITERAL),
                            "interaction_open",
                        );
                        host.show(SCREEN.to_vec());
                        crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                            host,
                            |host| row(host, id).state == storage::InputState::Settled,
                        );
                        assert_eq!(host.controller.status().visibility, Visibility::NotVisible);
                        assert_eq!(
                            host.controller.status().eligibility,
                            InputEligibility::ComposerEmpty,
                        );
                        assert!(host.controller.active_episode_seq().is_none());
                        let input = row(host, id);
                        assert_eq!(
                            input.disposition,
                            Some(storage::InputDisposition::NotSubmitted)
                        );
                        assert_eq!(
                            input.delivery_reason.as_deref(),
                            Some("plain_limit_exceeded")
                        );
                        assert!(host.pty.offers.is_empty());
                        assert!(host.coordinator.queue.is_empty());
                    },
                );
            }

            #[test]
            fn submit_key_requires_the_full_verification_window_before_writing() {
                with_host(|host| {
                    start(host, "prompt:submit-budget", LITERAL);
                    until_written(host, LITERAL.len());
                    host.coordinator.pending.as_mut().unwrap().deadline =
                        Instant::now() + Duration::from_secs(1);
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::DeadlineExpired,
                        storage::LeftStage::LiteralComplete,
                    );
                    assert!(!host.pty.offers.iter().any(|bytes| bytes == b"~"));
                });
            }

            #[test]
            fn short_immediate_budget_queues_zero_bytes_then_uses_a_fresh_runner_budget() {
                with_host(|host| {
                    let request = request(host, "prompt:initial-budget", LITERAL);
                    let reply = submitted(
                        host.handle_until(request, Instant::now() + Duration::from_secs(1))
                            .unwrap()
                            .expect("short budget queues immediately"),
                    );
                    assert_eq!(reply.status, SubmissionStatus::Queued);
                    assert_eq!(reply.stage, SubmissionStage::ZeroBytes);
                    assert!(host.pty.offers.is_empty());
                    assert_eq!(
                        stored(host, &reply).defer_reason.as_deref(),
                        Some("delivery_budget_exhausted")
                    );
                    let input_id = reply.input_id.unwrap();
                    for _ in 0..256 {
                        if row(host, input_id).state == storage::InputState::Accepted {
                            break;
                        }
                        if !host.pty.program.is_empty() {
                            host.child_output(SCREEN);
                        }
                        let _ = turn(host);
                    }
                    assert_eq!(row(host, input_id).state, storage::InputState::Accepted);
                    assert_eq!(host.pty.written, encoded());
                });
            }

            #[test]
            fn deferred_head_budget_check_includes_the_verification_window() {
                with_catalog_host(
                    permission_configured("plain", Value::Null),
                    encoded(),
                    |host| {
                        form(host);
                        let id = queued(
                            host,
                            request(host, "prompt:deferred-budget", LITERAL),
                            "interaction_open",
                        );
                        host.show(SCREEN.to_vec());
                        host.coordinator
                            .queue
                            .front_mut()
                            .expect("the actual queued head")
                            .attempt_deadline = Some(Instant::now() + Duration::from_secs(1));
                        let _ = turn(host);
                        assert_eq!(row(host, id).state, storage::InputState::Ready);
                        assert!(host.pty.offers.is_empty());
                        assert_eq!(
                            host.coordinator
                                .queue
                                .front()
                                .expect("the same waiting head")
                                .obstacle,
                            "delivery_budget_exhausted"
                        );
                    },
                );
            }

            #[test]
            fn unconfirmed_submit_records_the_submitted_left_stage() {
                with_host(|host| {
                    start(host, "prompt:unconfirmed", LITERAL);
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::SubmitUnconfirmed,
                        storage::LeftStage::Submitted,
                    );
                    assert_eq!(host.pty.written, encoded());
                });
            }

            #[test]
            fn freshly_verified_empty_composer_confirms_the_written_submit_key() {
                with_host(|host| {
                    start(host, "prompt:confirmed", LITERAL);
                    let reply = finish(host, true);
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(stored(host, &reply).stop_cause, None);
                    assert_eq!(host.pty.written, encoded());
                });
            }

            #[test]
            fn redelivery_successor_waits_ahead_of_later_inputs_and_survives_the_idle_bound() {
                with_host(|host| {
                    let (message, stopped) = stopped_preview(host, false);
                    let next = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(next.preview_attempt, Some(2));
                    assert_eq!(next.defer_reason.as_deref(), Some("redelivery"));
                    let later = queued(
                        host,
                        request(host, "prompt:behind-redelivery", LITERAL),
                        "earlier_input",
                    );
                    let writes = host.pty.written.clone();
                    host.coordinator
                        .set_queued_composer_busy_since(next.input_id, &old_wait());
                    for _ in 0..64 {
                        let _ = turn(host);
                    }
                    assert_eq!(host.pty.written, writes);
                    assert_eq!(row(host, next.input_id).state, storage::InputState::Ready);
                    assert_eq!(row(host, later).state, storage::InputState::Ready);
                    assert_eq!(
                        row(host, stopped).disposition,
                        Some(storage::InputDisposition::LeftInComposer)
                    );
                });
            }

            #[test]
            fn consumed_held_preview_is_cancelled_on_the_next_queue_turn_without_writing() {
                with_host(|host| {
                    let (message, _) = stopped_preview(host, false);
                    let next = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    let writes = host.pty.written.clone();
                    assert!(
                        crate::broker::poll_messages(&host.fixture.conn, message.to_member_id)
                            .unwrap()
                            .iter()
                            .any(|consumed| consumed.message_id == message.message_id)
                    );
                    let _ = turn(host);
                    let cancelled = row(host, next.input_id);
                    assert_eq!(
                        cancelled.disposition,
                        Some(storage::InputDisposition::NotSubmitted)
                    );
                    assert_eq!(
                        cancelled.delivery_reason.as_deref(),
                        Some("message_consumed")
                    );
                    assert_eq!(host.pty.written, writes);
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(
                        result_events(host, next.input_id),
                        0,
                        "a consumed re-delivery successor publishes no delivery result"
                    );
                });
            }

            #[test]
            fn an_enter_press_that_verifies_the_composer_empty_delivers_the_waiting_head_then_the_later_input()
             {
                with_host(|host| {
                    let (message, stopped) = stopped_preview(host, false);
                    let waiting = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    let later = queued(
                        host,
                        request(host, "prompt:after-the-stalled-head", LITERAL),
                        "earlier_input",
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(waiting.input_id, &old_wait());
                    drain_stalled_events(host);
                    readable_stalled_head(host, waiting.input_id, stopped);
                    assert_eq!(
                        row(host, waiting.input_id).state,
                        storage::InputState::Ready
                    );
                    assert_eq!(row(host, later).state, storage::InputState::Ready);
                    assert_eq!(
                        host.coordinator.held_composer.map(|(input_id, _)| input_id),
                        Some(stopped),
                        "the left-in-composer blocker holds the composer"
                    );
                    let blocker = row(host, stopped);
                    let canonical = canonical_preview(&message);
                    let preview_program = [canonical.as_bytes(), b"~"].concat();
                    host.confirm_submissions = true;
                    host.confirm_after_programs([preview_program.clone(), encoded()]);
                    let before = host.pty.written.len();
                    host.type_human(b"\r");
                    host.show(SCREEN.to_vec());
                    crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                        host,
                        |host| {
                            matches!(
                                row(host, later).state,
                                storage::InputState::Accepted | storage::InputState::Settled
                            )
                        },
                    );
                    assert_eq!(
                        host.coordinator.held_composer, None,
                        "the verified empty composer clears the hold"
                    );
                    assert!(matches!(
                        row(host, waiting.input_id).state,
                        storage::InputState::Accepted | storage::InputState::Settled
                    ));
                    assert_eq!(
                        host.pty.written[before..],
                        [b"\r".as_slice(), &preview_program, &encoded()].concat(),
                        "the press submits the retained literal, then the waiting head and the later input deliver in order"
                    );
                    assert_eq!(row(host, stopped), blocker);
                });
            }

            #[test]
            fn stalled_held_head_publishes_one_event_with_the_blocking_input() {
                with_host(|host| {
                    let (message, stopped) = stopped_preview(host, false);
                    let next = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    host.coordinator
                        .set_queued_composer_busy_since(next.input_id, &old_wait());
                    for _ in 0..64 {
                        let _ = turn(host);
                    }
                    let mut statement = host.fixture.conn.prepare("SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2").unwrap();
                    let payloads = statement
                        .query_map(
                            rusqlite::params![host.fixture.run.run_id, next.input_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap();
                    assert_eq!(payloads.len(), 1);
                    let event: Value = serde_json::from_str(&payloads[0]).unwrap();
                    assert_eq!(event["blocking_input_id"], stopped);
                    assert_eq!(event["message_id"], message.message_id);
                    assert_eq!(event["director_action"]["kind"], "review_stalled_input");
                    assert_eq!(event["director_action"]["press_argv"], Value::Null);
                });
            }

            fn held_blocker_for_stalled_heads(host: &mut Host, immediate: bool) -> i64 {
                host.coordinator.services.hooks = Arc::new(FailLaterBeforeWrite {
                    inner: host.hooks.clone(),
                    armed: std::sync::atomic::AtomicBool::new(true),
                });
                host.pty.short = Some(3);
                let input = request(host, "prompt:reconciled-stalled-blocker", LITERAL);
                if immediate {
                    assert!(host.handle(input).unwrap().is_none());
                } else {
                    form(host);
                    queued(host, input, "interaction_open");
                    host.show(SCREEN.to_vec());
                }
                crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                    host,
                    |host| host.pty.written.len() == 3,
                );
                assert_eq!(host.pty.written, LITERAL.as_bytes()[..3]);
                let id = host
                    .coordinator
                    .pending
                    .as_ref()
                    .unwrap()
                    .input_id()
                    .unwrap();
                let failure = later_checkpoint_failure(host);
                assert!(
                    matches!(failure.source, CafleetError::DeadlineExceeded),
                    "{failure:?}"
                );
                assert_eq!(failure.context.input_id, Some(id));
                if immediate {
                    host.coordinator.finish_pending_attempt();
                    assert_eq!(row(host, id).state, storage::InputState::InProgress);
                } else {
                    assert_eq!(row(host, id).state, storage::InputState::Uncertain);
                    assert_eq!(
                        row(host, id).stop_cause,
                        Some(storage::StopCause::WriteFailed)
                    );
                }
                assert!(host.coordinator.pending.is_none());
                assert_eq!(host.pty.offers, [LITERAL.as_bytes().to_vec()]);
                host.pty.short = None;
                host.show(DRAFT.to_vec());
                host.confirm_after_programs([encoded()]);
                id
            }

            fn reconcile_stalled_blocker(
                host: &mut Host,
                id: i64,
                disposition: storage::InputDisposition,
            ) -> storage::InputOperation {
                let before = row(host, id);
                let result = host
                    .receipt(ReceiptAction::Reconcile {
                        input_id: id,
                        expected: before.state,
                        disposition,
                        evidence_ref:
                            "synthetic:operator-reconciled-positive-literal-held-composer".into(),
                    })
                    .unwrap();
                assert!(
                    matches!(result, InputControlReply::Receipt(ReceiptReply::Settled { input_id }) if input_id == id)
                );
                let after = row(host, id);
                assert_eq!(after.state, storage::InputState::Settled);
                assert_eq!(after.disposition, Some(disposition));
                assert_eq!(after.stop_cause, before.stop_cause);
                after
            }

            fn drain_stalled_events(host: &mut Host) {
                for _ in 0..4 {
                    for _ in 0..8 {
                        let _ = turn(host);
                    }
                    loop {
                        host.drain();
                        let writer = host.writer.status();
                        let health = host.controller.health(&host.writer);
                        assert!(
                            writer.diagnostic.is_none(),
                            "writer={writer:?}; health={health:?}"
                        );
                        assert_eq!(writer.gap_count, 0, "writer={writer:?}; health={health:?}");
                        if !health.degraded && health.pending_count == 0 && !health.pending_evidence
                        {
                            break;
                        }
                        assert!(
                            Instant::now() < host.until,
                            "writer={writer:?}; health={health:?}"
                        );
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }

            fn readable_stalled_head(host: &Host, head: i64, blocker: i64) -> Value {
                let ids = host.fixture.conn.prepare(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2 ORDER BY event_id",
                ).unwrap().query_map(rusqlite::params![host.fixture.run.run_id, head], |row| row.get::<_, i64>(0))
                    .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                assert_eq!(
                    ids.len(),
                    1,
                    "head={:?}; blocker={:?}; writer={:?}; health={:?}",
                    row(host, head),
                    row(host, blocker),
                    host.writer.status(),
                    host.controller.health(&host.writer)
                );
                let event = storage::get_event(&host.fixture.conn, ids[0]).unwrap();
                assert_eq!(event.event.run_id, host.fixture.run.run_id);
                assert_eq!(row(host, head).run_id, event.event.run_id);
                assert_eq!(row(host, blocker).run_id, event.event.run_id);
                let payload = event.event.payload_json;
                assert_eq!(payload["input_id"], head);
                assert_eq!(payload["blocking_input_id"], blocker);
                assert_eq!(payload["message_id"], json!(row(host, head).message_id));
                assert_eq!(payload["director_action"]["kind"], "review_stalled_input");
                payload
            }

            #[test]
            fn reconciled_held_blocker_before_first_stalled_publication_keeps_the_signal() {
                for immediate in [false, true] {
                    for disposition in [
                        storage::InputDisposition::NotSubmitted,
                        storage::InputDisposition::AbandonedUncertain,
                        storage::InputDisposition::Submitted,
                    ] {
                        crate::runtime::input::tests::ordinary_prompt::composer_busy::with_busy_host(
                            permission_configured("plain", Value::Null), encoded(), |host| {
                                let blocker = held_blocker_for_stalled_heads(host, immediate);
                                let head = queued(host, request(host, "prompt:stalled-after-reconcile", LITERAL), "earlier_input");
                                host.coordinator.set_queued_composer_busy_since(head, &old_wait());
                                let since = host.coordinator.queue.front().unwrap().composer_busy_since;
                                let reconciled = reconcile_stalled_blocker(host, blocker, disposition);
                                assert_eq!(host.coordinator.queue.front().unwrap().composer_busy_since, since);
                                let offers = host.pty.offers.clone();
                                let written = host.pty.written.clone();
                                drain_stalled_events(host);
                                assert_eq!(row(host, head).state, storage::InputState::Ready);
                                assert_eq!(host.pty.offers, offers);
                                assert_eq!(host.pty.written, written);
                                let event = readable_stalled_head(host, head, blocker);
                                drain_stalled_events(host);
                                assert_eq!(readable_stalled_head(host, head, blocker), event);
                                assert_eq!(row(host, blocker), reconciled);
                                finish_after_exceptional_clear(host, blocker, head);
                                assert_eq!(readable_stalled_head(host, head, blocker), event);
                                assert_eq!(row(host, blocker), reconciled);
                            },
                        );
                    }
                }
            }

            #[test]
            fn reconciled_already_notified_blocker_notifies_the_later_head_after_consumption_cancellation()
             {
                for disposition in [
                    storage::InputDisposition::NotSubmitted,
                    storage::InputDisposition::AbandonedUncertain,
                    storage::InputDisposition::Submitted,
                ] {
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::with_busy_host(
                        permission_configured("plain", Value::Null),
                        encoded(),
                        |host| {
                            let blocker = held_blocker_for_stalled_heads(host, false);
                            let message = host.message(
                                "waiting preview cancelled after its stalled notification",
                            );
                            let literal = canonical_preview(&message);
                            let first = queued(
                                host,
                                preview_request(host, &message, &literal),
                                "earlier_input",
                            );
                            let later = queued(
                                host,
                                request(host, "prompt:later-stalled-head", LITERAL),
                                "earlier_input",
                            );
                            host.coordinator
                                .set_queued_composer_busy_since(first, &old_wait());
                            let offers = host.pty.offers.clone();
                            let written = host.pty.written.clone();
                            drain_stalled_events(host);
                            let original = readable_stalled_head(host, first, blocker);
                            let since = host.coordinator.queue.front().unwrap().composer_busy_since;
                            let reconciled = reconcile_stalled_blocker(host, blocker, disposition);
                            assert_eq!(
                                host.coordinator.queue.front().unwrap().composer_busy_since,
                                since
                            );
                            assert!(
                                crate::broker::poll_messages(
                                    &host.fixture.conn,
                                    message.to_member_id
                                )
                                .unwrap()
                                .iter()
                                .any(|consumed| consumed.message_id == message.message_id)
                            );
                            crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(host, |host| row(host, first).state == storage::InputState::Settled);
                            assert_eq!(
                                row(host, first).disposition,
                                Some(storage::InputDisposition::NotSubmitted)
                            );
                            assert_eq!(
                                row(host, first).delivery_reason.as_deref(),
                                Some("message_consumed")
                            );
                            assert_eq!(host.coordinator.queue.front().unwrap().input_id, later);
                            assert_eq!(readable_stalled_head(host, first, blocker), original);
                            drain_stalled_events(host);
                            let later_events: i64 = host.fixture.conn.query_row(
                                "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2",
                                rusqlite::params![host.fixture.run.run_id, later], |row| row.get(0),
                            ).unwrap();
                            assert_eq!(
                                later_events, 0,
                                "the later head starts its own composer-only bound"
                            );
                            host.coordinator
                                .set_queued_composer_busy_since(later, &old_wait());
                            drain_stalled_events(host);
                            assert_eq!(host.pty.offers, offers);
                            assert_eq!(host.pty.written, written);
                            assert_eq!(row(host, later).state, storage::InputState::Ready);
                            let next = readable_stalled_head(host, later, blocker);
                            drain_stalled_events(host);
                            assert_eq!(readable_stalled_head(host, later, blocker), next);
                            assert_eq!(readable_stalled_head(host, first, blocker), original);
                            finish_after_exceptional_clear(host, blocker, later);
                            assert_eq!(row(host, blocker), reconciled);
                            assert_eq!(readable_stalled_head(host, later, blocker), next);
                            assert_eq!(readable_stalled_head(host, first, blocker), original);
                        },
                    );
                }
            }

            #[test]
            fn consumed_exceptional_in_progress_blocker_stalls_until_reconciliation_and_fresh_empty()
             {
                crate::runtime::input::tests::ordinary_prompt::composer_busy::with_busy_host(
                    permission_configured("plain", Value::Null),
                    encoded(),
                    |host| {
                        let blocker = held_blocker_for_stalled_heads(host, true);
                        let head = queued(
                            host,
                            request(host, "prompt:behind-consumed-in-progress", LITERAL),
                            "earlier_input",
                        );
                        host.coordinator
                            .set_queued_composer_busy_since(head, &old_wait());
                        let offers = host.pty.offers.clone();
                        let written = host.pty.written.clone();
                        drain_stalled_events(host);
                        assert_eq!(row(host, blocker).state, storage::InputState::InProgress);
                        assert_eq!(row(host, head).state, storage::InputState::Ready);
                        assert_eq!(host.pty.offers, offers);
                        assert_eq!(host.pty.written, written);
                        let original = readable_stalled_head(host, head, blocker);
                        let reconciled = reconcile_stalled_blocker(
                            host,
                            blocker,
                            storage::InputDisposition::AbandonedUncertain,
                        );
                        drain_stalled_events(host);
                        assert_eq!(readable_stalled_head(host, head, blocker), original);
                        finish_after_exceptional_clear(host, blocker, head);
                        assert_eq!(row(host, blocker), reconciled);
                        assert_eq!(readable_stalled_head(host, head, blocker), original);
                    },
                );
            }

            #[test]
            fn a_newly_opened_interaction_confirms_the_written_submit_key() {
                with_catalog_host(
                    permission_configured("plain", Value::Null),
                    encoded(),
                    |host| {
                        start(host, "prompt:interaction-confirmation", LITERAL);
                        until_written(host, encoded().len());
                        form(host);
                        let reply = (0..64)
                            .find_map(|_| turn(host))
                            .expect("the new interaction confirms the submitted input");
                        assert_eq!(reply.status, SubmissionStatus::Accepted);
                        assert_eq!(stored(host, &reply).stop_cause, None);
                    },
                );
            }

            #[test]
            fn consumed_held_blocker_exclusion_preserves_an_unrelated_in_progress_barrier() {
                crate::runtime::input::tests::ordinary_prompt::composer_busy::with_busy_host(
                    permission_configured("plain", Value::Null),
                    encoded(),
                    |host| {
                        let blocker = held_blocker_for_stalled_heads(host, true);
                        let owner = host.fixture.run.owner();
                        let unrelated = storage::test_support::reserve_input(
                            &mut host.fixture.conn,
                            &owner,
                            &storage::NewInput {
                                operation_key: "prompt:unrelated-in-progress-barrier".into(),
                                kind: storage::InputKind::Prompt,
                                payload_digest: "f".repeat(64),
                                message_id: None,
                            },
                        )
                        .unwrap();
                        assert_eq!(unrelated.state, storage::InputState::InProgress);
                        let head = queued(
                            host,
                            request(host, "prompt:behind-two-in-progress-inputs", LITERAL),
                            "earlier_input",
                        );
                        host.coordinator
                            .set_queued_composer_busy_since(head, &old_wait());
                        let offers = host.pty.offers.clone();
                        let written = host.pty.written.clone();
                        drain_stalled_events(host);
                        let events: i64 = host.fixture.conn.query_row(
                            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2",
                            rusqlite::params![host.fixture.run.run_id, head], |row| row.get(0),
                        ).unwrap();
                        assert_eq!(
                            events, 0,
                            "an unrelated active writer still blocks held-only progress"
                        );
                        assert_eq!(row(host, head).state, storage::InputState::Ready);
                        assert_eq!(row(host, unrelated.input_id), unrelated);
                        assert_eq!(host.pty.offers, offers);
                        assert_eq!(host.pty.written, written);
                        reconcile_stalled_blocker(
                            host,
                            unrelated.input_id,
                            storage::InputDisposition::NotSubmitted,
                        );
                        host.coordinator
                            .set_queued_composer_busy_since(head, &old_wait());
                        drain_stalled_events(host);
                        let event = readable_stalled_head(host, head, blocker);
                        assert_eq!(host.pty.offers, offers);
                        assert_eq!(host.pty.written, written);
                        let reconciled = reconcile_stalled_blocker(
                            host,
                            blocker,
                            storage::InputDisposition::AbandonedUncertain,
                        );
                        finish_after_exceptional_clear(host, blocker, head);
                        assert_eq!(row(host, blocker), reconciled);
                        assert_eq!(readable_stalled_head(host, head, blocker), event);
                    },
                );
            }

            #[test]
            fn partial_submit_key_is_uncertain_with_write_failed() {
                let mut definition = catalog_definition();
                for operation in ["prompt", "exec"] {
                    definition["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation]["literal_framing"] = json!("plain");
                    definition["profiles"][0]["capabilities"]["composer"]["submission"]
                        [operation]["plain_literal_limit"] = Value::Null;
                }
                definition["profiles"][0]["capabilities"]["composer"]["submission"]["prompt"]["submit_hex"] =
                    json!("6b6c");
                let catalog =
                    crate::coding_agent::catalog::test_support::load(&definition).unwrap();
                let mut bytes = LITERAL.as_bytes().to_vec();
                bytes.extend_from_slice(b"kl");
                with_catalog_host(catalog, bytes, |host| {
                    start(host, "prompt:short-submit", LITERAL);
                    until_written(host, LITERAL.len());
                    host.pty.short = Some(1);
                    let reply = finish(host, false);
                    assert_eq!(reply.status, SubmissionStatus::Uncertain);
                    assert_eq!(
                        stored(host, &reply).stop_cause,
                        Some(storage::StopCause::WriteFailed)
                    );
                });
            }

            struct FailAfterWrite {
                inner: Arc<Hooks>,
                armed: std::sync::atomic::AtomicBool,
            }

            struct FailLaterBeforeWrite {
                inner: Arc<Hooks>,
                armed: std::sync::atomic::AtomicBool,
            }

            impl InputHooks for FailLaterBeforeWrite {
                fn checkpoint(
                    &self,
                    boundary: InputBoundary,
                    state: &InputCheckpoint,
                    deadline: Instant,
                ) -> Result<()> {
                    let later_write =
                        matches!(boundary, InputBoundary::BeforeWrite) && state.written_bytes > 0;
                    self.inner.checkpoint(boundary, state, deadline)?;
                    if later_write && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return Err(CafleetError::DeadlineExceeded);
                    }
                    Ok(())
                }
            }

            fn later_checkpoint_failure(host: &mut Host) -> InputFailure {
                loop {
                    assert!(
                        Instant::now() < host.until,
                        "a later pre-write checkpoint fails after actual positive bytes"
                    );
                    host.drain();
                    match host.try_advance() {
                        Err(failure) => return failure,
                        Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                        Ok(Some(_)) => panic!("the later checkpoint retains its primary error"),
                    }
                }
            }

            fn exceptional_literal_stage(kind: storage::InputKind) -> Vec<u8> {
                let mut bytes = Vec::new();
                if kind == storage::InputKind::Exec {
                    bytes.extend_from_slice(b"! ");
                }
                bytes.extend_from_slice(LITERAL.as_bytes());
                bytes
            }

            fn exceptional_program(kind: storage::InputKind) -> Vec<u8> {
                let mut bytes = exceptional_literal_stage(kind);
                bytes.push(b'~');
                bytes
            }

            fn exceptional_literal(
                host: &mut Host,
                kind: storage::InputKind,
                short: Option<usize>,
                deferred: bool,
            ) -> (i64, i64) {
                let mut input = request(host, "ordinary:exceptional-literal", LITERAL);
                if kind == storage::InputKind::Exec {
                    let InputControlOperation::SubmitPrompt(mut body) = input.operation else {
                        panic!("the harness creates a prompt body")
                    };
                    body.input_kind = kind;
                    input.operation = InputControlOperation::SubmitExec(body);
                }
                host.coordinator.services.hooks = Arc::new(FailLaterBeforeWrite {
                    inner: host.hooks.clone(),
                    armed: std::sync::atomic::AtomicBool::new(true),
                });
                host.pty.short = short;
                let first = if deferred {
                    form(host);
                    Some(queued(host, input, "interaction_open"))
                } else {
                    assert!(host.handle(input).unwrap().is_none());
                    None
                };
                if deferred {
                    host.show(SCREEN.to_vec());
                }
                let literal = exceptional_literal_stage(kind);
                let written = short.unwrap_or(literal.len());
                crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                    host,
                    |host| host.pty.written.len() == written,
                );
                assert_eq!(host.pty.written, literal[..written]);
                let id = host
                    .coordinator
                    .pending
                    .as_ref()
                    .unwrap()
                    .input_id()
                    .unwrap();
                if let Some(first) = first {
                    assert_eq!(id, first);
                }
                let next = queued(
                    host,
                    request(host, "prompt:after-exceptional-literal", LITERAL),
                    "earlier_input",
                );
                let failure = later_checkpoint_failure(host);
                assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
                assert_eq!(failure.context.input_id, Some(id));
                assert_eq!(failure.context.run_id, Some(host.fixture.run.run_id));
                let failed = row(host, id);
                if deferred {
                    assert_eq!(failed.state, storage::InputState::Uncertain);
                    assert_eq!(failed.stop_cause, Some(storage::StopCause::WriteFailed));
                } else {
                    assert_eq!(failed.state, storage::InputState::InProgress);
                    assert_eq!(failed.stop_cause, None);
                    host.coordinator
                        .set_queued_composer_busy_since(next, &old_wait());
                    let since = host.coordinator.queue.front().unwrap().composer_busy_since;
                    host.coordinator.finish_pending_attempt();
                    host.coordinator.finish_pending_attempt();
                    assert_eq!(
                        host.coordinator.queue.front().unwrap().composer_busy_since,
                        since
                    );
                }
                assert_eq!(failed.disposition, None);
                assert!(host.coordinator.pending.is_none());
                assert_eq!(host.pty.offers.len(), 1);
                assert_eq!(host.pty.offers[0], literal);
                host.pty.short = None;
                host.confirm_after_programs([encoded()]);
                (id, next)
            }

            fn finish_after_exceptional_clear(host: &mut Host, first: i64, next: i64) {
                let before = host.pty.written.clone();
                let blocked = row(host, first);
                host.show(SCREEN.to_vec());
                crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                    host,
                    |host| row(host, next).state == storage::InputState::Accepted,
                );
                let delivered = row(host, next);
                assert_eq!(delivered.disposition, None);
                assert_eq!(
                    delivered.delivery_method,
                    Some(storage::DeliveryMethod::Profile)
                );
                assert_eq!(delivered.delivery_reason, None);
                assert_eq!(host.pty.written, [before, encoded()].concat());
                assert_eq!(
                    host.pty.offers[1..],
                    [LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                );
                assert_eq!(row(host, first), blocked);
            }

            #[test]
            fn exceptional_later_before_write_requires_fresh_empty_after_positive_literal_bytes() {
                for kind in [storage::InputKind::Prompt, storage::InputKind::Exec] {
                    for short in [Some(3), None] {
                        with_catalog_host(
                            permission_configured("plain", Value::Null),
                            exceptional_program(kind),
                            |host| {
                                let (first, next) = exceptional_literal(host, kind, short, true);
                                let before = host.pty.written.clone();
                                for _ in 0..64 {
                                    let _ = turn(host);
                                }
                                assert_eq!(row(host, next).state, storage::InputState::Ready);
                                assert_eq!(
                                    host.pty.written, before,
                                    "the stale empty frame grants no write"
                                );
                                assert_eq!(host.pty.offers.len(), 1);
                                finish_after_exceptional_clear(host, first, next);
                            },
                        );
                    }
                }
            }

            #[test]
            fn exceptional_later_before_write_holds_nonempty_composer_past_idle_bound_and_stalls() {
                for kind in [storage::InputKind::Prompt, storage::InputKind::Exec] {
                    for short in [Some(3), None] {
                        with_catalog_host(
                            permission_configured("plain", Value::Null),
                            exceptional_program(kind),
                            |host| {
                                let (first, next) = exceptional_literal(host, kind, short, true);
                                host.show(DRAFT.to_vec());
                                host.coordinator
                                    .set_queued_composer_busy_since(next, &old_wait());
                                let before = host.pty.written.clone();
                                for _ in 0..64 {
                                    let _ = turn(host);
                                }
                                assert_eq!(row(host, next).state, storage::InputState::Ready);
                                assert_eq!(
                                    host.pty.written, before,
                                    "held CAFleet text outlives the idle fallback bound"
                                );
                                assert_eq!(host.pty.offers.len(), 1);
                                crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until(
                                host,
                                |host| host.fixture.conn.query_row(
                                    "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2",
                                    rusqlite::params![host.fixture.run.run_id, next],
                                    |row| row.get::<_, i64>(0),
                                ).unwrap() > 0,
                            );
                                let payloads = host.fixture.conn.prepare(
                                "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2",
                            ).unwrap().query_map(rusqlite::params![host.fixture.run.run_id, next], |row| row.get::<_, String>(0))
                                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                                assert_eq!(payloads.len(), 1);
                                let event: Value = serde_json::from_str(&payloads[0]).unwrap();
                                assert_eq!(event["blocking_input_id"], first);
                                assert_eq!(event["input_id"], next);
                                assert_eq!(event["message_id"], Value::Null);
                                assert_eq!(host.pty.written, before);
                                finish_after_exceptional_clear(host, first, next);
                            },
                        );
                    }
                }
            }

            #[test]
            fn exceptional_later_before_write_immediate_retirement_preserves_the_hold_and_timer() {
                for kind in [storage::InputKind::Prompt, storage::InputKind::Exec] {
                    for short in [Some(3), None] {
                        crate::runtime::input::tests::ordinary_prompt::composer_busy::with_busy_host(
                            permission_configured("plain", Value::Null),
                            exceptional_program(kind),
                            |host| {
                                let (first, next) = exceptional_literal(host, kind, short, false);
                                let mut reconcile = host.request("receipt:exceptional-literal");
                                reconcile.operation =
                                    InputControlOperation::Receipt(ReceiptAction::Reconcile {
                                        input_id: first,
                                        expected: storage::InputState::InProgress,
                                        disposition: storage::InputDisposition::AbandonedUncertain,
                                        evidence_ref:
                                            "synthetic:positive-literal-later-before-write-failure"
                                                .into(),
                                    });
                                assert!(
                                    matches!(host.handle(reconcile).unwrap(), Some(InputControlReply::Receipt(ReceiptReply::Settled { input_id })) if input_id == first)
                                );
                                let abandoned = row(host, first);
                                assert_eq!(abandoned.state, storage::InputState::Settled);
                                assert_eq!(
                                    abandoned.disposition,
                                    Some(storage::InputDisposition::AbandonedUncertain)
                                );
                                let before = host.pty.written.clone();
                                for _ in 0..64 {
                                    let _ = turn(host);
                                }
                                assert_eq!(row(host, next).state, storage::InputState::Ready);
                                assert_eq!(host.pty.written, before);
                                assert_eq!(host.pty.offers.len(), 1);
                                host.show(DRAFT.to_vec());
                                let since =
                                    host.coordinator.queue.front().unwrap().composer_busy_since;
                                for _ in 0..64 {
                                    let _ = turn(host);
                                }
                                assert_eq!(
                                    host.coordinator.queue.front().unwrap().composer_busy_since,
                                    since
                                );
                                assert_eq!(row(host, next).state, storage::InputState::Ready);
                                assert_eq!(host.pty.written, before);
                                assert_eq!(host.pty.offers.len(), 1);
                                finish_after_exceptional_clear(host, first, next);
                                assert_eq!(row(host, first), abandoned);
                            },
                        );
                    }
                }
            }

            impl InputHooks for FailAfterWrite {
                fn checkpoint(
                    &self,
                    boundary: InputBoundary,
                    state: &InputCheckpoint,
                    deadline: Instant,
                ) -> Result<()> {
                    let after_write = matches!(boundary, InputBoundary::AfterWrite);
                    self.inner.checkpoint(boundary, state, deadline)?;
                    if after_write && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return Err(error("injected post-write checkpoint failure"));
                    }
                    Ok(())
                }
            }

            fn finish_fault(host: &mut Host, input_id: i64) -> InputFailure {
                for _ in 0..64 {
                    host.drain();
                    match host.try_advance() {
                        Err(failure) => return failure,
                        Ok(Some(_)) => panic!("a failed write preserves its primary InputFailure"),
                        Ok(None) => {
                            if !host.pty.offers.iter().any(|bytes| bytes == CLOSE) {
                                assert_eq!(
                                    storage::get_input(&host.fixture.conn, input_id)
                                        .unwrap()
                                        .state,
                                    storage::InputState::InProgress
                                );
                            }
                        }
                    }
                }
                panic!("paste cleanup finishes within bounded drained turns");
            }

            fn assert_fault_closed(host: &Host, input_id: i64) {
                assert_eq!(host.pty.offers.last().unwrap(), CLOSE);
                assert!(!host.pty.offers.iter().any(|bytes| bytes == b"~"));
                assert!(host.pty.program.is_empty());
                assert!(host.pty.manual.is_empty());
                let input = storage::get_input(&host.fixture.conn, input_id).unwrap();
                assert_eq!(input.state, storage::InputState::Uncertain);
                assert_eq!(input.stop_cause, Some(storage::StopCause::WriteFailed));
                assert_eq!(input.disposition, None);
                assert_eq!(input.left_stage, None);
            }

            #[test]
            fn known_open_paste_write_failure_attempts_close_and_preserves_the_primary_failure() {
                for close_fails in [false, true] {
                    with_catalog_host(
                        configured("bracketed_paste", Value::Null),
                        program(LITERAL, true),
                        |host| {
                            enable_paste(host);
                            host.pty.short = Some(OPEN.len() + 3);
                            start(host, "prompt:open-paste-write-failure", LITERAL);
                            until_written(host, OPEN.len() + 3);
                            let input_id = host
                                .coordinator
                                .pending
                                .as_ref()
                                .unwrap()
                                .input_id()
                                .unwrap();
                            host.pty.short = None;
                            host.pty.fail_next = Some(std::io::ErrorKind::BrokenPipe);
                            host.pty.failure =
                                close_fails.then_some(std::io::ErrorKind::PermissionDenied);
                            let failure = finish_fault(host, input_id);
                            assert_fault_closed(host, input_id);
                            assert!(
                                failure
                                    .source
                                    .message()
                                    .contains("injected one-shot PTY write failure")
                            );
                            let mut expected = [OPEN, &LITERAL.as_bytes()[..3]].concat();
                            if !close_fails {
                                expected.extend_from_slice(CLOSE);
                            }
                            assert_eq!(host.pty.written, expected);
                        },
                    );
                }
            }

            #[test]
            fn known_open_paste_checkpoint_failure_closes_before_terminal_settlement() {
                with_catalog_host(
                    configured("bracketed_paste", Value::Null),
                    program(LITERAL, true),
                    |host| {
                        enable_paste(host);
                        host.pty.short = Some(OPEN.len() + 3);
                        start(host, "prompt:open-paste-checkpoint-failure", LITERAL);
                        until_written(host, OPEN.len() + 3);
                        let input_id = host
                            .coordinator
                            .pending
                            .as_ref()
                            .unwrap()
                            .input_id()
                            .unwrap();
                        let hooks = Arc::new(FailAfterWrite {
                            inner: host.hooks.clone(),
                            armed: std::sync::atomic::AtomicBool::new(true),
                        });
                        host.coordinator.services.hooks = hooks;
                        let failure = finish_fault(host, input_id);
                        assert_fault_closed(host, input_id);
                        assert!(
                            failure
                                .source
                                .message()
                                .contains("injected post-write checkpoint failure")
                        );
                        assert!(host.pty.written.ends_with(CLOSE));
                    },
                );
            }

            #[test]
            fn failed_or_partial_close_stages_record_write_failed() {
                for failed in [false, true] {
                    with_catalog_host(
                        configured("bracketed_paste", Value::Null),
                        program(LITERAL, true),
                        |host| {
                            enable_paste(host);
                            host.pty.short = Some(OPEN.len() + 3);
                            start(host, "prompt:bad-close", LITERAL);
                            until_written(host, OPEN.len() + 3);
                            host.pty.short = None;
                            host.type_human(b"x");
                            host.drain();
                            if failed {
                                host.pty.failure = Some(std::io::ErrorKind::BrokenPipe);
                            } else {
                                host.pty.short = Some(1);
                            }
                            for _ in 0..64 {
                                match host.try_advance() {
                                    Err(_) | Ok(Some(_)) => break,
                                    Ok(None) => host.drain(),
                                }
                            }
                            let input = storage::find_input(
                                &host.fixture.conn,
                                host.fixture.run.run_id,
                                "prompt:bad-close",
                            )
                            .unwrap()
                            .unwrap();
                            assert_eq!(input.state, storage::InputState::Uncertain);
                            assert_eq!(input.stop_cause, Some(storage::StopCause::WriteFailed));
                            assert!(host.pty.offers.iter().any(|bytes| bytes == CLOSE));
                        },
                    );
                }
            }

            #[test]
            fn a_stop_inside_the_closing_marker_closes_paste_and_keeps_short_write_uncertainty() {
                with_catalog_host(
                    configured("bracketed_paste", Value::Null),
                    program(LITERAL, true),
                    |host| {
                        enable_paste(host);
                        let prefix = OPEN.len() + LITERAL.len() + 3;
                        host.pty.short = Some(prefix);
                        start(host, "prompt:closing-marker-prefix", LITERAL);
                        until_written(host, prefix);
                        host.pty.short = None;
                        host.type_human(b"x");
                        let reply = finish(host, false);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        assert_eq!(
                            stored(host, &reply).stop_cause,
                            Some(storage::StopCause::ShortWrite)
                        );
                        assert_eq!(host.pty.offers.last().unwrap(), CLOSE);
                    },
                );
            }

            #[test]
            fn redelivery_is_inserted_ahead_of_input_already_queued_at_the_stop() {
                with_host(|host| {
                    host.confirm_submissions = false;
                    let message = host.message("redelivery preserves message order");
                    let literal = canonical_preview(&message);
                    host.pty.expect_program(program(&literal, false));
                    assert!(
                        host.handle(preview_request(host, &message, &literal))
                            .unwrap()
                            .is_none()
                    );
                    until_written(host, literal.len());
                    let later = queued(
                        host,
                        request(host, "prompt:already-queued", LITERAL),
                        "earlier_input",
                    );
                    host.show(DRAFT.to_vec());
                    host.type_human(b"x");
                    let reply = finish(host, false);
                    assert_left(
                        host,
                        &reply,
                        storage::StopCause::ManualInput,
                        storage::LeftStage::LiteralComplete,
                    );
                    let next = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    let order = host
                        .coordinator
                        .queue
                        .iter()
                        .map(|input| input.input_id)
                        .collect::<Vec<_>>();
                    assert_eq!(order, [next.input_id, later]);
                });
            }

            #[test]
            fn verified_fifo_advances_past_uncertain_and_holds_the_following_ordinary_head() {
                with_catalog_host(
                    configured("plain", Value::Null),
                    program("あxx", false),
                    |host| {
                        host.pty.short = Some(1);
                        start(host, "prompt:uncertain-head", "あxx");
                        until_written(host, 1);
                        let later = queued(
                            host,
                            request(host, "prompt:behind-uncertain", LITERAL),
                            "earlier_input",
                        );
                        host.show(DRAFT.to_vec());
                        host.type_human(b"x");
                        let reply = finish(host, false);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        assert_eq!(
                            stored(host, &reply).stop_cause,
                            Some(storage::StopCause::ShortWrite)
                        );
                        let writes = host.pty.written.clone();
                        host.coordinator
                            .set_queued_composer_busy_since(later, &old_wait());
                        for _ in 0..64 {
                            let _ = turn(host);
                        }
                        assert_eq!(row(host, later).state, storage::InputState::Ready);
                        assert_eq!(
                            host.coordinator
                                .queue
                                .front()
                                .expect("the FIFO advanced to the ordinary head")
                                .input_id,
                            later
                        );
                        assert_eq!(host.pty.written, writes);
                    },
                );
            }

            #[test]
            fn clearing_the_held_composer_restores_the_human_draft_idle_timeout_rule() {
                with_catalog_host(
                    configured("plain", Value::Null),
                    program("あxx", false),
                    |host| {
                        host.pty.short = Some(1);
                        start(host, "prompt:clear-held", "あxx");
                        until_written(host, 1);
                        host.show(DRAFT.to_vec());
                        host.type_human(b"x");
                        let reply = finish(host, false);
                        assert_eq!(reply.status, SubmissionStatus::Uncertain);
                        host.pty.short = None;
                        host.show(SCREEN.to_vec());
                        let _ = turn(host);
                        host.show(DRAFT.to_vec());
                        let later = queued(
                            host,
                            request(host, "prompt:human-draft-after-clear", LITERAL),
                            "composer_busy",
                        );
                        host.coordinator
                            .set_queued_composer_busy_since(later, &old_wait());
                        let before = host.pty.written.len();
                        until_written(host, before + 1);
                        let input = row(host, later);
                        assert_eq!(
                            input.delivery_method,
                            Some(storage::DeliveryMethod::Fallback)
                        );
                        assert_eq!(input.delivery_reason.as_deref(), Some("idle_timeout"));
                        assert_eq!(host.pty.offers.last().unwrap(), b"\x1b");
                    },
                );
            }

            #[test]
            fn held_composer_time_behind_an_interaction_publishes_no_stalled_event() {
                with_catalog_host(
                    permission_configured("plain", Value::Null),
                    encoded(),
                    |host| {
                        let (message, _) = stopped_preview(host, false);
                        let waiting = storage::find_latest_preview_input(
                            &host.fixture.conn,
                            host.fixture.run.run_id,
                            message.message_id,
                        )
                        .unwrap()
                        .unwrap();
                        host.coordinator
                            .set_queued_composer_busy_since(waiting.input_id, &old_wait());
                        form(host);
                        for _ in 0..64 {
                            let _ = turn(host);
                        }
                        let count: i64 = host.fixture.conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' AND json_extract(payload_json,'$.input_id')=?2", rusqlite::params![host.fixture.run.run_id,waiting.input_id], |row| row.get(0)).unwrap();
                        assert_eq!(count, 0);
                        assert_eq!(
                            row(host, waiting.input_id).state,
                            storage::InputState::Ready
                        );
                    },
                );
            }

            #[test]
            fn nonverified_runs_keep_uncertain_fifo_heads_and_create_no_redelivery() {
                let catalog = configured("plain", Value::Null).without_submission(
                    crate::coding_agent::CodingAgent::Codex,
                    ComposerOperation::Prompt,
                );
                with_catalog_host(catalog, encoded(), |host| {
                    host.confirm_submissions = false;
                    let message = host.message("legacy unverified preview");
                    let literal = canonical_preview(&message);
                    let mut bytes = b"\x1b".to_vec();
                    bytes.extend_from_slice(literal.as_bytes());
                    bytes.push(b'\r');
                    host.pty.expect_program(bytes);
                    assert!(
                        host.handle(preview_request(host, &message, &literal))
                            .unwrap()
                            .is_none()
                    );
                    until_written(host, 1 + literal.len());
                    let later = queued(
                        host,
                        request(host, "prompt:legacy-fifo", LITERAL),
                        "earlier_input",
                    );
                    host.type_human(b"x");
                    let reply = finish(host, false);
                    assert_eq!(reply.status, SubmissionStatus::Uncertain);
                    let input = stored(host, &reply);
                    assert_eq!(input.stop_cause, Some(storage::StopCause::ManualInput));
                    assert_eq!(input.left_stage, None);
                    let latest = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(latest.input_id, input.input_id);
                    assert_eq!(latest.preview_attempt, Some(1));
                    let written = host.pty.written.clone();
                    for _ in 0..64 {
                        let _ = turn(host);
                    }
                    assert_eq!(row(host, later).state, storage::InputState::Ready);
                    assert_eq!(host.pty.written, written);
                });
            }

            #[test]
            fn an_inactive_target_cancels_the_waiting_preview_without_writing() {
                with_host(|host| {
                    let (message, _) = stopped_preview(host, false);
                    let waiting = storage::find_latest_preview_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        message.message_id,
                    )
                    .unwrap()
                    .unwrap();
                    let written = host.pty.written.clone();
                    host.fixture
                        .conn
                        .execute(
                            "UPDATE members SET status='deregistered' WHERE member_id=?1",
                            [host.fixture.run.member_id],
                        )
                        .unwrap();
                    let _ = host.try_advance();
                    let input = row(host, waiting.input_id);
                    assert_eq!(
                        input.disposition,
                        Some(storage::InputDisposition::NotSubmitted)
                    );
                    assert_eq!(input.delivery_reason.as_deref(), Some("target_inactive"));
                    assert_eq!(host.pty.written, written);
                });
            }
        }

        mod staged_delivery {
            use crate::runtime::input::tests::ordinary_prompt::*;

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
                    assert!(
                        gap >= host.stage_settle,
                        "each stage follows its {:?} settle: {gap:?}",
                        host.stage_settle
                    );
                }
            }

            pub(super) fn permission_catalog() -> ProfileCatalog {
                let mut definition =
                    crate::coding_agent::catalog::test_support::inspection_fixture_definition();
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
                prompt_plan_catalog(&definition)
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
            fn staged_human_input_during_the_settle_leaves_the_complete_literal() {
                with_host(|host| {
                    until_literal_written(host, prompt_request(host, "prompt:human-race"));
                    host.type_human(b"h");
                    let reply = finish_after_interruption(host);
                    assert_eq!(reply.status, SubmissionStatus::LeftInComposer);
                    assert_eq!(reply.stage, SubmissionStage::LiteralComplete);
                    assert_eq!(host.pty.stage_bytes(), vec![LITERAL.as_bytes().to_vec()]);
                    let id = reply.input_id.unwrap();
                    let stored = storage::get_input(&host.fixture.conn, id).unwrap();
                    assert_eq!(stored.state, storage::InputState::Settled);
                    assert_eq!(
                        stored.disposition,
                        Some(storage::InputDisposition::LeftInComposer)
                    );
                    assert_eq!(stored.left_stage, Some(storage::LeftStage::LiteralComplete));
                    assert_eq!(stored.stop_cause, Some(storage::StopCause::ManualInput));
                    assert_eq!(reply.reason.as_deref(), Some("manual_input"));
                    assert_eq!(
                        delivery_selection(host, id),
                        (Some("profile".into()), None),
                        "the selected method survives the settled left outcome"
                    );
                });
            }

            #[test]
            fn staged_runner_holds_the_pane_lock_across_the_settle_and_releases_it_after_submission()
             {
                with_host(|host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    let placement = host.placement();
                    until_literal_written(host, prompt_request(host, "prompt:locked"));
                    assert!(
                        crate::pane_lock::PaneLock::try_acquire(&url, &placement)
                            .unwrap()
                            .is_none(),
                        "the runner retains the pane lock between its stages"
                    );
                    let reply = finish_after_interruption(host);
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert!(
                        crate::pane_lock::PaneLock::acquire_released(&url, &placement)
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
                    let placement = host.placement();
                    let held = crate::pane_lock::PaneLock::try_acquire(&url, &placement)
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

            mod immediate_failure_bookkeeping {
                use crate::runtime::input::tests::ordinary_prompt::staged_delivery::*;

                fn expired() -> Instant {
                    let deadline = Instant::now();
                    std::thread::sleep(Duration::from_millis(2));
                    deadline
                }

                fn fail_advance_at_an_expired_deadline(host: &mut Host) {
                    host.drain();
                    let failed = host.coordinator.advance(
                        &mut host.fixture.conn,
                        &mut host.transport,
                        &mut host.pty,
                        &host.controller,
                        expired(),
                    );
                    assert!(
                        failed.is_err(),
                        "a written attempt fails at its expired deadline"
                    );
                }

                fn assert_recorded_uncertain_write_failed(host: &Host, input_id: i64) {
                    let row = storage::get_input(&host.fixture.conn, input_id).unwrap();
                    assert_eq!(row.state, storage::InputState::Uncertain, "{row:?}");
                    assert_eq!(
                        row.stop_cause,
                        Some(storage::StopCause::WriteFailed),
                        "{row:?}"
                    );
                    assert!(row.has_stalled_blocker_provenance(), "{row:?}");
                    assert_eq!(
                        host.coordinator.held_composer.map(|(blocker, _)| blocker),
                        Some(input_id),
                        "the written attempt still holds the composer"
                    );
                }

                #[test]
                fn a_written_immediate_prompt_failing_in_advance_after_its_literal_records_uncertain_write_failed()
                 {
                    with_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:d1-literal"));
                        let id = host.hooks.input_id();
                        fail_advance_at_an_expired_deadline(host);
                        assert_recorded_uncertain_write_failed(host, id);
                    });
                }

                #[test]
                fn a_written_immediate_prompt_failing_in_advance_after_its_submit_key_records_uncertain_write_failed()
                 {
                    with_host(|host| {
                        host.confirm_submissions = false;
                        until_literal_written(host, prompt_request(host, "prompt:d1-submit"));
                        let id = host.hooks.input_id();
                        std::thread::sleep(host.stage_settle + Duration::from_millis(50));
                        while host.pty.stage_bytes().len() < 2 {
                            assert!(Instant::now() < host.until, "the submit stage is written");
                            host.drain();
                            assert!(host.advance().is_none(), "no reply before verification");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        fail_advance_at_an_expired_deadline(host);
                        assert_recorded_uncertain_write_failed(host, id);
                    });
                }

                #[test]
                fn a_written_immediate_prompt_failing_in_its_pre_write_drain_records_uncertain_write_failed()
                 {
                    with_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:d2-drain"));
                        let id = host.hooks.input_id();
                        host.child_output(b"echo");
                        let drained = host.coordinator.progress_with_manual(
                            &mut host.transport,
                            |transport| {
                                transport.quiesce_for_input(
                                    &mut host.human,
                                    &mut host.pty,
                                    &mut host.output,
                                    &mut host.controller.observer(
                                        &host.writer,
                                        &crate::runtime::RealObservationClock,
                                    ),
                                    expired(),
                                )
                            },
                        );
                        let failure =
                            drained.expect_err("the pre-write drain fails at its deadline");
                        let _reply = host.coordinator.pending_attempt_failed(
                            &mut host.fixture.conn,
                            &host.controller,
                            failure,
                        );
                        host.coordinator.finish_pending_attempt();
                        assert_recorded_uncertain_write_failed(host, id);
                    });
                }
            }

            mod stop_cause {
                use crate::runtime::input::tests::ordinary_prompt::staged_delivery::*;

                const FOCUS_REPLY: &[u8] = b"\x1b[I";
                const CURSOR_REPORT: &[u8] = b"\x1b[24;80R";
                const OSC_REPLY: &[u8] = b"\x1b]11;rgb:0000/0000/0000\x1b\\";

                fn stored(host: &Host, reply: &SubmissionResult) -> storage::InputOperation {
                    storage::get_input(
                        &host.fixture.conn,
                        reply.input_id.expect("an attempted input has a receipt"),
                    )
                    .unwrap()
                }

                fn assert_stopped_with(
                    host: &Host,
                    reply: &SubmissionResult,
                    cause: storage::StopCause,
                ) {
                    assert_eq!(reply.status, SubmissionStatus::LeftInComposer, "{cause:?}");
                    assert_eq!(reply.stage, SubmissionStage::LiteralComplete, "{cause:?}");
                    assert_eq!(reply.reason.as_deref(), Some(cause.as_str()));
                    let row = stored(host, reply);
                    assert_eq!(row.state, storage::InputState::Settled);
                    assert_eq!(
                        row.disposition,
                        Some(storage::InputDisposition::LeftInComposer)
                    );
                    assert_eq!(row.left_stage, Some(storage::LeftStage::LiteralComplete));
                    assert_eq!(row.stop_cause, Some(cause));
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![LITERAL.as_bytes().to_vec()],
                        "no submit byte follows a stopped stage"
                    );
                }

                #[test]
                fn staged_claude_pending_question_notice_during_the_settle_stops_before_the_submit_key()
                 {
                    use crate::coding_agent::detector::test_support::bundled_screen;
                    use crate::coding_agent::detector::test_support::native_fixture_bytes;
                    use crate::runtime::input::tests::permission_inspection_contracts::with_claude_host;
                    with_claude_host(
                        "2.1.312",
                        "claude-2.1.312-auto-default-decline-continuation-1",
                        |host, _, _| {
                            let mut program = LITERAL.as_bytes().to_vec();
                            program.push(b'\r');
                            host.pty.expect_program(program);
                            until_literal_written(
                                host,
                                prompt_request(host, "prompt:pending-question-notice"),
                            );
                            host.show(native_fixture_bytes(&bundled_screen(
                                "claude-2.1.312-question-pending-notice-1",
                            )));
                            let reply = finish_after_interruption(host);
                            let cause = stored(host, &reply)
                                .stop_cause
                                .expect("a stopped stage records its cause");
                            assert!(
                                matches!(
                                    cause,
                                    storage::StopCause::InteractionCandidate
                                        | storage::StopCause::InteractionOpen
                                ),
                                "{cause:?}"
                            );
                            assert_stopped_with(host, &reply, cause);
                            assert!(
                                host.pty.program.is_empty(),
                                "the submit key is never written"
                            );
                            assert!(!host.pty.offers.iter().any(|offer| offer.contains(&0x1b)));
                        },
                    );
                }

                fn until_armed(host: &mut Host) {
                    std::thread::sleep(host.stage_settle + Duration::from_millis(50));
                    while host.pending_snapshot().4 != InputPhase::Armed {
                        assert!(Instant::now() < host.until, "the submit stage arms");
                        host.drain();
                        assert!(host.advance().is_none());
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    assert_eq!(host.pty.stage_bytes(), vec![LITERAL.as_bytes().to_vec()]);
                }

                fn candidate_form(host: &mut Host) {
                    host.show(
                        crate::coding_agent::catalog::test_support::inspection_fixture_form(
                            "write candidate.txt",
                            "synthetic-only",
                        ),
                    );
                    assert_eq!(
                        host.controller.health(&host.writer).eligibility,
                        InputEligibility::Hold(GateReason::Candidate)
                    );
                }

                fn with_interaction_host(body: impl FnOnce(&mut Host)) {
                    with_profile_host(
                        permission_catalog(),
                        encoded(),
                        HostProfile::metadata(),
                        body,
                    );
                }

                #[test]
                fn staged_terminal_replies_during_the_settle_still_write_the_submit_key() {
                    for terminal_reply in [FOCUS_REPLY, CURSOR_REPORT, OSC_REPLY] {
                        with_host(|host| {
                            until_literal_written(host, prompt_request(host, "prompt:reply"));
                            host.type_human(terminal_reply);
                            host.drain();
                            assert_eq!(
                                host.human.consumed.len(),
                                terminal_reply.len(),
                                "the reply is read during the settle"
                            );
                            let reply = finish_after_interruption(host);
                            assert_eq!(
                                reply.status,
                                SubmissionStatus::Accepted,
                                "{terminal_reply:?}"
                            );
                            assert_eq!(reply.stage, SubmissionStage::Submitted);
                            assert_eq!(
                                host.pty.stage_bytes(),
                                vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                            );
                            assert_eq!(
                                host.pty.manual, terminal_reply,
                                "the reply reaches the child unchanged"
                            );
                            let forwarded = host
                                .pty
                                .offers
                                .iter()
                                .position(|offer| offer.as_slice() == terminal_reply)
                                .expect("the reply is forwarded");
                            let submit = host
                                .pty
                                .offers
                                .iter()
                                .position(|offer| offer.as_slice() == b"~")
                                .expect("the submit key is written");
                            assert!(forwarded < submit, "the reply precedes the submit key");
                            let row = stored(host, &reply);
                            assert_eq!(row.state, storage::InputState::Accepted);
                            assert_eq!(row.stop_cause, None);
                        });
                    }
                }

                #[test]
                fn staged_lone_escape_during_the_settle_stops_with_manual_input_at_its_bound() {
                    with_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:lone-escape"));
                        let generation = host.transport.human_generation();
                        host.type_human(b"\x1b");
                        assert_eq!(host.quiesce_once(), InputDrain::Pending);
                        assert_eq!(host.human.consumed.len(), 1, "the Escape key is read");
                        assert_eq!(
                            host.transport.drain_blocker(),
                            Some("terminal reply pending")
                        );
                        assert_eq!(host.transport.human_generation(), generation);
                        host.transport
                            .backdate_reply_prefix(crate::runtime::transport::REPLY_PREFIX_BOUND);
                        host.drain();
                        assert_eq!(host.transport.human_generation(), generation + 1);
                        let reply = finish_after_interruption(host);
                        assert_stopped_with(host, &reply, storage::StopCause::ManualInput);
                        assert_eq!(host.pty.manual, b"\x1b");
                    });
                }

                #[test]
                fn staged_failed_manual_accounting_outranks_the_generation_change() {
                    with_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:accounting"));
                        host.coordinator.manual_generation = u64::MAX;
                        assert!(host.coordinator.observe_manual(1).is_err());
                        assert!(host.coordinator.manual_accounting_failed);
                        let reply = submitted(
                            host.advance()
                                .expect("the settle check stops the remaining stages"),
                        );
                        assert_stopped_with(
                            host,
                            &reply,
                            storage::StopCause::ManualAccountingFailed,
                        );
                    });
                }

                #[test]
                fn staged_keystroke_outranks_a_candidate_interaction() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:key-first"));
                        host.type_human(b"h");
                        candidate_form(host);
                        let reply = submitted(
                            host.advance()
                                .expect("the settle check stops the remaining stages"),
                        );
                        assert_stopped_with(host, &reply, storage::StopCause::ManualInput);
                    });
                }

                #[test]
                fn staged_candidate_during_the_settle_records_interaction_candidate() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:candidate"));
                        candidate_form(host);
                        let reply = submitted(
                            host.advance()
                                .expect("the settle check stops the remaining stages"),
                        );
                        assert_stopped_with(host, &reply, storage::StopCause::InteractionCandidate);
                    });
                }

                #[test]
                fn staged_open_interaction_during_the_settle_records_interaction_open() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:open"));
                        crate::runtime::input::tests::permission_inspection_contracts::stable_form(
                            host,
                        );
                        let reply = submitted(
                            host.advance()
                                .expect("the settle check stops the remaining stages"),
                        );
                        assert_stopped_with(host, &reply, storage::StopCause::InteractionOpen);
                    });
                }

                #[test]
                fn staged_observer_outage_during_the_settle_records_observer_unavailable() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:outage"));
                        let reply = loop {
                            assert!(Instant::now() < host.until, "the outage stops the stage");
                            host.controller
                                .observer_failed(
                                    crate::runtime::observation::ObservationFailure::Detector,
                                    &RealObservationClock,
                                )
                                .unwrap();
                            if let Some(reply) = host.advance() {
                                break submitted(reply);
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        };
                        assert_stopped_with(host, &reply, storage::StopCause::ObserverUnavailable);
                    });
                }

                #[test]
                fn staged_open_interaction_after_the_settle_records_interaction_open() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:armed-open"));
                        until_armed(host);
                        crate::runtime::input::tests::permission_inspection_contracts::stable_form(
                            host,
                        );
                        let reply = submitted(
                            host.advance()
                                .expect("the armed stage stops on the interaction"),
                        );
                        assert_stopped_with(host, &reply, storage::StopCause::InteractionOpen);
                        host.show(SCREEN.to_vec());
                        crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(
                            host,
                        );
                        assert_eq!(
                            host.pty.stage_bytes(),
                            vec![LITERAL.as_bytes().to_vec()],
                            "no submit byte is written after the interaction closes"
                        );
                    });
                }

                #[test]
                fn staged_candidate_after_the_settle_records_interaction_candidate() {
                    with_interaction_host(|host| {
                        until_literal_written(host, prompt_request(host, "prompt:armed-candidate"));
                        until_armed(host);
                        candidate_form(host);
                        let reply = submitted(
                            host.advance()
                                .expect("the armed stage stops on the interaction"),
                        );
                        assert_stopped_with(host, &reply, storage::StopCause::InteractionCandidate);
                    });
                }
            }
        }

        mod deferred_queue {
            use crate::broker::runtime::DeliveryMethod;
            use crate::broker::runtime::DeliveryProjection;
            use crate::broker::runtime::DeliveryStatus;
            use crate::pane_lock::PaneLock;
            use crate::runtime::input::tests::ordinary_prompt::staged_delivery::permission_catalog;
            use crate::runtime::input::tests::ordinary_prompt::*;

            pub(super) fn form(host: &mut Host) {
                crate::runtime::input::tests::permission_inspection_contracts::stable_form(host);
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

            pub(super) fn result_events(host: &Host, input_id: i64) -> i64 {
                host.fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2",
                        rusqlite::params![host.fixture.run.run_id, input_id],
                        |row| row.get(0),
                    )
                    .unwrap()
            }

            fn result_cause(host: &Host, input_id: i64) -> Option<serde_json::Value> {
                host.fixture
                    .conn
                    .query_row(
                        "SELECT json_extract(payload_json,'$.cause') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2",
                        rusqlite::params![host.fixture.run.run_id, input_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .unwrap()
                    .map(|text| serde_json::from_str(&text).unwrap())
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
                        crate::coding_agent::catalog::test_support::inspection_fixture_form(
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

            #[test]
            fn queue_timer_retries_a_busy_pane_lock_without_child_output_and_delivers_once_free() {
                with_permission_host(Duration::from_secs(10), |host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    form(host);
                    let id = queued(host, host.request("prompt:busy"), "interaction_open");
                    let held = PaneLock::try_acquire(&url, &host.placement())
                        .unwrap()
                        .unwrap();
                    close(host);
                    let busy_until = Instant::now() + Duration::from_millis(300);
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
                    let _held = PaneLock::try_acquire(&url, &host.placement())
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
            fn queued_preview_consumed_before_its_drain_settles_not_submitted_without_typing() {
                with_permission_host(Duration::from_secs(8), |host| {
                    let message = host.message("queued preview body");
                    form(host);
                    let request = preview_request(host, &message, &canonical_preview(&message));
                    let id = queued(host, request, "interaction_open");
                    assert!(
                        crate::broker::poll_messages(&host.fixture.conn, message.to_member_id)
                            .unwrap()
                            .iter()
                            .any(|consumed| consumed.message_id == message.message_id)
                    );
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
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(
                        result_events(host, id),
                        0,
                        "a preview consumed by poll publishes no delivery result"
                    );
                });
            }

            /// Lets each screen reach the controller, then outlives the drain deadline, as a slow
            /// observer callback does while injected keystrokes keep the child echoing.
            struct ExpiringObserver<O> {
                inner: O,
                deadline: Instant,
            }
            impl<O: crate::runtime::transport::ScreenObserver>
                crate::runtime::transport::ScreenObserver for ExpiringObserver<O>
            {
                fn observe(&mut self, screen: &crate::runtime::ScreenSnapshot) -> Result<()> {
                    let observed = self.inner.observe(screen);
                    std::thread::sleep(
                        self.deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(1),
                    );
                    observed
                }
                fn input_health(&self) -> crate::runtime::ObservationHealth {
                    self.inner.input_health()
                }
                fn parser_failure(&mut self) -> Result<()> {
                    self.inner.parser_failure()
                }
            }

            /// `complete_pending_input` on a written immediate attempt whose drain deadline expires
            /// inside the observer callback.
            fn fail_pending_drain_mid_observation(host: &mut Host) {
                host.child_output(b"echo of injected keys");
                let deadline = Instant::now() + Duration::from_millis(30);
                let drained =
                    host.coordinator
                        .progress_with_manual(&mut host.transport, |transport| {
                            transport.quiesce_for_input(
                                &mut host.human,
                                &mut host.pty,
                                &mut host.output,
                                &mut ExpiringObserver {
                                    inner: host.controller.observer(
                                        &host.writer,
                                        &crate::runtime::RealObservationClock,
                                    ),
                                    deadline,
                                },
                                deadline,
                            )
                        });
                let failure = drained.expect_err("the pre-write drain fails at its deadline");
                let _reply = host.coordinator.pending_attempt_failed(
                    &mut host.fixture.conn,
                    &host.controller,
                    failure,
                );
                host.coordinator.finish_pending_attempt();
            }

            /// One `tick_running` turn with no client attempt in flight: the transport tick, the
            /// observation poll, then the queue turn, which advances only on a quiescent drain.
            fn queue_turn(host: &mut Host) {
                host.coordinator
                    .progress_with_manual(&mut host.transport, |transport| {
                        transport.tick(
                            &mut host.human,
                            &mut host.pty,
                            &mut host.output,
                            &mut host
                                .controller
                                .observer(&host.writer, &crate::runtime::RealObservationClock),
                        )
                    })
                    .unwrap();
                host.controller
                    .poll(
                        &host.transport.snapshot(),
                        &host.writer,
                        &crate::runtime::RealObservationClock,
                    )
                    .unwrap();
                if matches!(host.quiesce_once(), InputDrain::Quiescent) {
                    let _ = host.try_advance();
                }
            }

            #[test]
            fn run_34_a_deadline_expired_immediate_write_releases_its_consumed_queued_head() {
                with_permission_host(Duration::from_secs(30), |host| {
                    assert!(
                        host.handle(host.request("prompt:run-34-blocker"))
                            .unwrap()
                            .is_none()
                    );
                    while host.pty.stages.is_empty() {
                        assert!(
                            Instant::now() < host.until,
                            "the blocker's literal is written"
                        );
                        host.drain();
                        assert!(host.advance().is_none());
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let blocker = host.hooks.input_id();
                    fail_pending_drain_mid_observation(host);
                    let blocking = row(host, blocker);
                    assert_eq!(blocking.state, storage::InputState::Uncertain);
                    assert_eq!(blocking.stop_cause, Some(storage::StopCause::WriteFailed));

                    let message = host.message("queued behind the blocker");
                    let request = preview_request(host, &message, &canonical_preview(&message));
                    let head = queued(host, request, "earlier_input");
                    let offers = host.pty.offers.len();
                    assert!(matches!(
                        host.receipt(crate::runtime::ReceiptAction::Reconcile {
                            input_id: blocker,
                            expected: storage::InputState::Uncertain,
                            disposition: storage::InputDisposition::Submitted,
                            evidence_ref: "audit:pane-shows-submitted".into(),
                        })
                        .unwrap(),
                        InputControlReply::Receipt(crate::runtime::ReceiptReply::Settled {
                            input_id
                        }) if input_id == blocker
                    ));
                    assert!(
                        crate::broker::poll_messages(&host.fixture.conn, message.to_member_id)
                            .unwrap()
                            .iter()
                            .any(|consumed| consumed.message_id == message.message_id)
                    );

                    host.child_output(SCREEN);
                    let bound = Instant::now() + Duration::from_secs(3);
                    while row(host, head).state == storage::InputState::Ready {
                        assert!(
                            Instant::now() < bound,
                            "the consumed head is released; drain blocker {:?}",
                            host.transport.drain_blocker()
                        );
                        queue_turn(host);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    assert_not_submitted(host, head, "message_consumed");
                    assert_eq!(
                        host.pty.offers.len(),
                        offers,
                        "the consumed preview is never typed"
                    );
                    assert_eq!(result_events(host, head), 0);
                });
            }

            #[test]
            fn a_preview_attempt_deadline_with_a_busy_pane_lock_settles_not_submitted_without_publishing()
             {
                with_permission_host(Duration::from_secs(30), |host| {
                    let url = format!("sqlite:///{}", host.fixture.conn.path().unwrap());
                    let message = host.message("preview behind a busy pane lock");
                    form(host);
                    let preview = queued(
                        host,
                        preview_request(host, &message, &canonical_preview(&message)),
                        "interaction_open",
                    );
                    let prompt = queued(
                        host,
                        host.request("prompt:busy-control"),
                        "interaction_open",
                    );
                    let _held = PaneLock::try_acquire(&url, &host.placement())
                        .unwrap()
                        .unwrap();
                    close(host);
                    turns_until(host, |host| {
                        row(host, prompt).state == storage::InputState::Settled
                    });
                    assert_not_submitted(host, preview, "pane_input_busy");
                    assert_not_submitted(host, prompt, "pane_input_busy");
                    turns_until(host, |host| result_events(host, prompt) == 1);
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(
                        result_events(host, preview),
                        0,
                        "the transient preview outcome publishes nothing; the sweep retries it"
                    );
                    assert_eq!(result_events(host, prompt), 1, "a prompt still publishes");
                    assert!(host.pty.offers.is_empty());
                });
            }

            #[test]
            fn a_preview_whose_target_changed_still_publishes_its_delivery_result() {
                with_permission_host(Duration::from_secs(60), |host| {
                    let message = host.message("preview for a moved target");
                    form(host);
                    let id = queued(
                        host,
                        preview_request(host, &message, &canonical_preview(&message)),
                        "interaction_open",
                    );
                    host.fixture
                        .conn
                        .execute(
                            "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1",
                            [host.fixture.run.member_id],
                        )
                        .unwrap();
                    close(host);
                    turns_until(host, |host| settled(host, id));
                    assert_not_submitted(host, id, "target_changed");
                    turns_until(host, |host| result_events(host, id) > 0);
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(result_events(host, id), 1, "the result is published once");
                });
            }

            #[test]
            fn orderly_shutdown_settles_a_queued_preview_runner_stopped_without_publishing() {
                with_permission_host(Duration::from_secs(30), |host| {
                    let message = host.message("preview queued at shutdown");
                    form(host);
                    let preview = queued(
                        host,
                        preview_request(host, &message, &canonical_preview(&message)),
                        "interaction_open",
                    );
                    let prompt = queued(
                        host,
                        host.request("prompt:shutdown-control"),
                        "interaction_open",
                    );
                    let settled = shut_down(host);
                    assert_eq!(settled, vec![preview, prompt]);
                    assert_not_submitted(host, preview, "runner_stopped");
                    assert_not_submitted(host, prompt, "runner_stopped");
                    assert_eq!(result_events(host, prompt), 1, "a prompt still publishes");
                    assert_eq!(
                        result_events(host, preview),
                        0,
                        "the next run's sweep retries the still-pending message"
                    );
                    assert!(host.pty.offers.is_empty());
                });
            }

            #[test]
            fn preview_receipts_use_the_canonical_digest_and_reject_another_literal() {
                with_permission_host(Duration::from_secs(5), |host| {
                    let message = host.message("digest preview body");
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
                    let expected = storage::input_payload_digest(
                        host.fixture.run.run_id,
                        &SubmitInput {
                            operation_key: input.operation_key.clone(),
                            input_kind: storage::InputKind::Preview,
                            message_id: Some(message.message_id),
                            literal: canonical,
                        },
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
                    let settled = host.coordinator.fence(host.until).unwrap();
                    assert_eq!(settled, vec![id]);
                    assert_not_submitted(host, id, "runner_stopped");
                    for input_id in settled {
                        host.controller.publish_delivery_result(
                            storage::input_delivery_result_cause(
                                input_id,
                                SubmissionStage::ZeroBytes.as_str(),
                                None,
                            ),
                        );
                    }
                    host.drain();
                    assert_eq!(
                        result_events(host, id),
                        1,
                        "the shutdown settlement publishes its deferred result"
                    );
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
                    assert_eq!(result_events(host, id), 1, "the result is published once");
                });
            }

            #[test]
            fn queue_head_settles_target_changed_for_a_moved_placement_and_target_inactive_for_a_deregistered_member()
             {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:moved"), "interaction_open");
                    host.fixture
                        .conn
                        .execute(
                            "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1",
                            [host.fixture.run.member_id],
                        )
                        .unwrap();
                    close(host);
                    turns_until(host, |host| settled(host, id));
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    let input = row(host, id);
                    assert_eq!(
                        input.disposition,
                        Some(storage::InputDisposition::NotSubmitted)
                    );
                    assert_eq!(input.delivery_reason.as_deref(), Some("target_changed"));
                });
                with_permission_host(Duration::from_secs(60), |host| {
                    host.fixture
                        .conn
                        .execute(
                            "UPDATE members SET status='deregistered' WHERE member_id=?1",
                            [host.fixture.run.member_id],
                        )
                        .unwrap();
                    assert_eq!(
                        host.coordinator
                            .deferred_target(&host.fixture.conn)
                            .unwrap(),
                        Some("target_inactive")
                    );
                });
            }

            /// Drives turns until the queued head's literal stage is written.
            fn until_head_literal(host: &mut Host) {
                while host.pty.stages.is_empty() {
                    assert!(
                        Instant::now() < host.until,
                        "the head's literal stage is written"
                    );
                    host.drain();
                    host.advance();
                    std::thread::sleep(Duration::from_millis(5));
                }
            }

            fn reconcile(host: &mut Host, input_id: i64) {
                let reply = host
                    .receipt(crate::runtime::ReceiptAction::Reconcile {
                        input_id,
                        expected: storage::InputState::Uncertain,
                        disposition: storage::InputDisposition::AbandonedUncertain,
                        evidence_ref: "audit:inspected-pane".into(),
                    })
                    .unwrap();
                assert!(
                    matches!(
                        reply,
                        InputControlReply::Receipt(crate::runtime::ReceiptReply::Settled {
                            input_id: settled
                        }) if settled == input_id
                    ),
                    "the coordinator reconciles the uncertain receipt"
                );
            }

            #[test]
            fn queued_stopped_inputs_publish_once_and_resume_after_the_composer_empties() {
                for case in [
                    "write_failure",
                    "human_input_after_the_settle",
                    "human_input_during_the_settle",
                ] {
                    with_permission_host(Duration::from_secs(60), |host| {
                        form(host);
                        let first =
                            queued(host, host.request("prompt:failing"), "interaction_open");
                        let next = queued(host, host.request("prompt:after"), "interaction_open");
                        close(host);
                        until_head_literal(host);
                        match case {
                            "write_failure" => host.pty.failure = Some(io::ErrorKind::BrokenPipe),
                            "human_input_after_the_settle" => {
                                std::thread::sleep(host.stage_settle + Duration::from_millis(100));
                                host.type_human(b"h");
                            }
                            _ => host.type_human(b"h"),
                        }
                        let target = if case == "write_failure" {
                            storage::InputState::Uncertain
                        } else {
                            storage::InputState::Settled
                        };
                        while row(host, first).state != target {
                            assert!(
                                Instant::now() < host.until,
                                "{case}: the head records its stopped outcome"
                            );
                            host.drain();
                            if let Err(failure) = host.try_advance() {
                                assert_eq!(case, "write_failure", "{failure:?}");
                                assert!(
                                    failure
                                        .source
                                        .message()
                                        .contains("injected PTY write failure"),
                                    "{failure:?}"
                                );
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        if case == "write_failure" {
                            assert_eq!(
                                row(host, first).stop_cause,
                                Some(storage::StopCause::WriteFailed),
                                "a failed deferred attempt records write_failed"
                            );
                        }
                        host.pty.failure = None;
                        turns_until(host, |host| result_events(host, first) > 0);
                        crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(
                            host,
                        );
                        assert_eq!(result_events(host, first), 1, "{case}");
                        let recorded = row(host, first)
                            .stop_cause
                            .expect("a stopped result records its cause");
                        assert_eq!(
                            result_cause(host, first),
                            Some(serde_json::json!({ "kind": recorded.as_str() })),
                            "{case}: the published cause is the receipt's"
                        );
                        assert_eq!(row(host, next).state, storage::InputState::Ready, "{case}");
                        if case == "write_failure" {
                            reconcile(host, first);
                        } else {
                            assert_eq!(
                                row(host, first).disposition,
                                Some(storage::InputDisposition::LeftInComposer)
                            );
                            assert_eq!(
                                row(host, first).left_stage,
                                Some(storage::LeftStage::LiteralComplete)
                            );
                        }
                        host.show(SCREEN.to_vec());
                        turns_until(host, |host| settled(host, next));
                        assert_eq!(
                            host.pty.offers[host.pty.offers.len() - 2..],
                            [LITERAL.as_bytes().to_vec(), b"~".to_vec()],
                            "{case}: the next queued input drains"
                        );
                        assert_eq!(result_events(host, first), 1, "{case}");
                    });
                }
            }

            #[test]
            fn preview_after_an_uncertain_head_waits_for_a_verified_empty_composer() {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let head = queued(
                        host,
                        host.request("prompt:uncertain-head"),
                        "interaction_open",
                    );
                    close(host);
                    until_head_literal(host);
                    host.pty.failure = Some(io::ErrorKind::BrokenPipe);
                    while row(host, head).state != storage::InputState::Uncertain {
                        assert!(Instant::now() < host.until, "the head becomes uncertain");
                        host.drain();
                        if let Err(failure) = host.try_advance() {
                            assert!(
                                failure
                                    .source
                                    .message()
                                    .contains("injected PTY write failure"),
                                "{failure:?}"
                            );
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    host.pty.failure = None;
                    let offers = host.pty.offers.len();
                    let message = host.message("preview behind an uncertain head");
                    let canonical = canonical_preview(&message);
                    let preview = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "earlier_input",
                    );
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(
                        host.pty.offers.len(),
                        offers,
                        "the preview receives zero bytes while the composer holds the stopped input: {:?}",
                        &host.pty.offers[offers..]
                    );
                    assert_eq!(row(host, preview).state, storage::InputState::Ready);
                    reconcile(host, head);
                    host.show(SCREEN.to_vec());
                    turns_until(host, |host| settled(host, preview));
                    assert_eq!(
                        host.pty.offers[offers..],
                        [canonical.into_bytes(), b"~".to_vec()]
                    );
                });
            }

            #[test]
            fn accepted_preview_stamp_failure_is_retried_and_published_once_without_retyping() {
                use std::sync::Arc;
                use std::sync::Mutex;
                with_permission_host(Duration::from_secs(60), |host| {
                    let message = host.message("preview whose stamp is busy once");
                    let canonical = canonical_preview(&message);
                    host.confirm_after_programs([[canonical.as_bytes(), b"~"].concat()]);
                    form(host);
                    let id = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "interaction_open",
                    );
                    let blocker = Arc::new(Mutex::new((
                        rusqlite::Connection::open(host.fixture.conn.path().unwrap()).unwrap(),
                        None::<Instant>,
                    )));
                    let observer = blocker.clone();
                    host.fixture
                        .conn
                        .authorizer(Some(move |context: rusqlite::hooks::AuthContext<'_>| {
                            if matches!(
                                context.action,
                                rusqlite::hooks::AuthAction::Transaction {
                                    operation: rusqlite::hooks::TransactionOperation::Begin
                                }
                            ) {
                                let mut held = observer.lock().unwrap();
                                let state: String = held
                                    .0
                                    .query_row(
                                        "SELECT state FROM member_input_operations WHERE input_id=?1",
                                        [id],
                                        |row| row.get(0),
                                    )
                                    .unwrap();
                                if held.1.is_none() && state == "accepted" {
                                    held.0.execute_batch("BEGIN IMMEDIATE").unwrap();
                                    held.1 = Some(Instant::now());
                                }
                            }
                            rusqlite::hooks::Authorization::Allow
                        }))
                        .unwrap();
                    close(host);
                    let stamp = |host: &Host| -> Option<String> {
                        host.fixture
                            .conn
                            .query_row(
                                "SELECT notified_at FROM messages WHERE message_id=?1",
                                [message.message_id],
                                |row| row.get(0),
                            )
                            .unwrap()
                    };
                    let mut released = false;
                    while stamp(host).is_none() || result_events(host, id) == 0 {
                        assert!(
                            Instant::now() < host.until,
                            "the stamp and result follow once storage recovers"
                        );
                        if !released {
                            let held = blocker.lock().unwrap();
                            if held
                                .1
                                .is_some_and(|since| since.elapsed() > Duration::from_millis(1200))
                            {
                                held.0.execute_batch("ROLLBACK").unwrap();
                                released = true;
                            }
                        }
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(
                        released,
                        "storage was busy for the accepted preview's stamp"
                    );
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(result_events(host, id), 1, "the result is published once");
                    assert_eq!(row(host, id).state, storage::InputState::Settled);
                    assert_eq!(
                        host.pty.offers,
                        vec![canonical.into_bytes(), b"~".to_vec()],
                        "the preview is typed once"
                    );
                    assert!(!host.coordinator.input_fenced);
                });
            }

            #[test]
            fn queue_head_interaction_after_its_zero_byte_attempt_expired_requeues_instead_of_settling()
             {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let id = queued(
                        host,
                        host.request("prompt:expired-then-form"),
                        "interaction_open",
                    );
                    let queued_receipt = row(host, id);
                    close(host);
                    while row(host, id).state == storage::InputState::Ready {
                        assert!(Instant::now() < host.until, "the deferred attempt starts");
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(host.pty.offers.is_empty());
                    host.coordinator.pending.as_mut().unwrap().deadline =
                        Instant::now() - Duration::from_millis(1);
                    form(host);
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    let requeued = row(host, id);
                    assert_eq!(
                        requeued.state,
                        storage::InputState::Ready,
                        "an open interaction requeues the expired zero-byte attempt"
                    );
                    assert_eq!(requeued.disposition, None);
                    assert_eq!(requeued.delivery_method, None);
                    assert_eq!(requeued.deferred_at, queued_receipt.deferred_at);
                    assert_eq!(requeued.defer_reason.as_deref(), Some("interaction_open"));
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    close(host);
                    turns_until(host, |host| settled(host, id));
                    assert_eq!(
                        host.pty.offers,
                        vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()],
                        "the requeued input drains once after closure"
                    );
                });
            }

            #[test]
            fn preview_queued_behind_a_left_immediate_attempt_waits_for_a_verified_empty_composer()
            {
                with_permission_host(Duration::from_secs(60), |host| {
                    let first = host.request("prompt:immediate-uncertain");
                    assert!(host.handle(first).unwrap().is_none());
                    let message = host.message("preview behind an immediate attempt");
                    let canonical = canonical_preview(&message);
                    let preview = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "earlier_input",
                    );
                    until_head_literal(host);
                    host.type_human(b"h");
                    let reply = loop {
                        assert!(Instant::now() < host.until, "the immediate attempt replies");
                        host.drain();
                        if let Some(reply) = host.advance() {
                            break submitted(reply);
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    };
                    assert_eq!(reply.status, SubmissionStatus::LeftInComposer);
                    assert_eq!(
                        row(host, reply.input_id.unwrap()).left_stage,
                        Some(storage::LeftStage::LiteralComplete)
                    );
                    let offers = host.pty.offers.len();
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(
                        host.pty.offers.len(),
                        offers,
                        "the queued preview receives zero bytes: {:?}",
                        &host.pty.offers[offers..]
                    );
                    assert_eq!(row(host, preview).state, storage::InputState::Ready);
                    host.show(SCREEN.to_vec());
                    turns_until(host, |host| settled(host, preview));
                    assert_eq!(
                        host.pty.offers[offers..],
                        [canonical.into_bytes(), b"~".to_vec()]
                    );
                });
            }

            #[test]
            fn deferred_attempt_expiring_after_storage_contention_settles_storage_unavailable() {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:contended"), "interaction_open");
                    close(host);
                    while host
                        .coordinator
                        .pending
                        .as_ref()
                        .is_none_or(|pending| pending.phase != InputPhase::Armed)
                    {
                        assert!(Instant::now() < host.until, "the deferred attempt arms");
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(host.pty.offers.is_empty());
                    let writer =
                        rusqlite::Connection::open(host.fixture.conn.path().unwrap()).unwrap();
                    host.drain();
                    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                    host.advance();
                    writer.execute_batch("ROLLBACK").unwrap();
                    assert!(
                        host.pty.offers.is_empty(),
                        "contended storage writes nothing: {:?}",
                        host.pty.offers
                    );
                    assert_eq!(row(host, id).state, storage::InputState::InProgress);
                    host.coordinator.pending.as_mut().unwrap().deadline =
                        Instant::now() - Duration::from_millis(1);
                    turns_until(host, |host| settled(host, id));
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    assert_not_submitted(host, id, "storage_unavailable");
                    turns_until(host, |host| result_events(host, id) > 0);
                    assert_eq!(result_events(host, id), 1);
                });
            }

            /// Applies the runner's orderly shutdown order: settle and publish the fenced results.
            fn shut_down(host: &mut Host) -> Vec<i64> {
                host.coordinator.finish_pending_attempt();
                let settled = host.coordinator.fence(host.until).unwrap();
                for (input_id, stage, cause) in storage::unpublished_delivery_results(
                    &host.fixture.conn,
                    host.fixture.run.run_id,
                )
                .unwrap()
                {
                    host.controller
                        .publish_delivery_result(storage::input_delivery_result_cause(
                            input_id, stage, cause,
                        ));
                }
                host.drain();
                settled
            }

            #[test]
            fn queue_orderly_shutdown_settles_an_active_deferred_attempt_by_its_written_bytes() {
                for written in [false, true] {
                    with_permission_host(Duration::from_secs(60), |host| {
                        form(host);
                        let id = queued(host, host.request("prompt:active"), "interaction_open");
                        close(host);
                        if written {
                            until_head_literal(host);
                        } else {
                            while row(host, id).state == storage::InputState::Ready {
                                assert!(Instant::now() < host.until, "the attempt starts");
                                host.drain();
                                host.advance();
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            assert!(host.pty.offers.is_empty());
                        }
                        assert_eq!(row(host, id).state, storage::InputState::InProgress);
                        assert_eq!(shut_down(host), vec![id], "written={written}");
                        let input = row(host, id);
                        if written {
                            assert_eq!(input.state, storage::InputState::Uncertain);
                            assert_eq!(
                                input.stop_cause,
                                Some(storage::StopCause::RunnerStopped),
                                "fence records runner_stopped for a written deferred attempt"
                            );
                            assert_eq!(
                                result_cause(host, id),
                                Some(serde_json::json!({"kind": "runner_stopped"}))
                            );
                        } else {
                            assert_not_submitted(host, id, "runner_stopped");
                            assert_eq!(result_cause(host, id), None);
                        }
                        assert_eq!(result_events(host, id), 1, "written={written}");
                    });
                }
            }

            #[test]
            fn queue_head_terminal_commit_survives_a_temporary_storage_failure_and_publishes_once()
            {
                with_permission_host(Duration::from_secs(60), |host| {
                    let id = queued_for_a_moved_target(host);
                    let writer =
                        rusqlite::Connection::open(host.fixture.conn.path().unwrap()).unwrap();
                    let advance_while_busy = |host: &mut Host| {
                        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                        host.advance();
                        writer.execute_batch("ROLLBACK").unwrap();
                    };
                    while host.coordinator.finalization.is_none() {
                        assert!(
                            Instant::now() < host.until,
                            "the busy terminal transaction becomes a pending finalization"
                        );
                        host.drain();
                        advance_while_busy(host);
                    }
                    let pending = host.coordinator.finalization.as_ref().unwrap();
                    assert_eq!(pending.input_id, id);
                    assert_eq!(pending.reason, "target_changed");
                    advance_while_busy(host);
                    assert!(
                        host.coordinator.finalization.is_some(),
                        "a retry while storage is still busy keeps the finalization pending"
                    );
                    assert_eq!(
                        row(host, id).state,
                        storage::InputState::Ready,
                        "the failed terminal transaction keeps the head's receipt"
                    );
                    assert_eq!(result_events(host, id), 0);
                    turns_until(host, |host| settled(host, id));
                    let input = row(host, id);
                    assert_eq!(
                        input.disposition,
                        Some(storage::InputDisposition::NotSubmitted)
                    );
                    assert_eq!(input.delivery_reason.as_deref(), Some("target_changed"));
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    turns_until(host, |host| result_events(host, id) > 0);
                    crate::runtime::input::tests::ordinary_prompt::composer_busy::hold_turns(host);
                    assert_eq!(result_events(host, id), 1, "the result is published once");
                });
            }

            /// Queues a head whose target placement then moves, so its next turn settles it
            /// `target_changed` through the terminal zero-byte transaction.
            fn queued_for_a_moved_target(host: &mut Host) -> i64 {
                form(host);
                let id = queued(host, host.request("prompt:storage"), "interaction_open");
                host.fixture
                    .conn
                    .execute(
                        "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1",
                        [host.fixture.run.member_id],
                    )
                    .unwrap();
                close(host);
                id
            }

            #[test]
            fn queue_head_permanent_settle_failure_surfaces_instead_of_retrying() {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:storage"), "interaction_open");
                    let later = queued(host, host.request("prompt:later"), "interaction_open");
                    host.fixture
                        .conn
                        .execute(
                            "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1",
                            [host.fixture.run.member_id],
                        )
                        .unwrap();
                    close(host);
                    host.fixture
                        .conn
                        .authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
                            if matches!(
                                context.action,
                                rusqlite::hooks::AuthAction::Update {
                                    table_name: "member_input_operations",
                                    ..
                                }
                            ) {
                                rusqlite::hooks::Authorization::Deny
                            } else {
                                rusqlite::hooks::Authorization::Allow
                            }
                        }))
                        .unwrap();
                    let failure = loop {
                        assert!(
                            Instant::now() < host.until,
                            "the permanent failure surfaces"
                        );
                        assert!(
                            host.coordinator.finalization.is_none(),
                            "a permanent failure never becomes a retried finalization"
                        );
                        host.drain();
                        if let Err(failure) = host.try_advance() {
                            break failure;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    };
                    assert!(
                        failure.source.message().contains("not authorized"),
                        "{failure:?}"
                    );
                    host.fixture
                        .conn
                        .authorizer(
                            None::<
                                fn(
                                    rusqlite::hooks::AuthContext<'_>,
                                )
                                    -> rusqlite::hooks::Authorization,
                            >,
                        )
                        .unwrap();
                    assert!(
                        host.coordinator.input_fenced,
                        "a permanent failure fences ordinary input for recovery"
                    );
                    for _ in 0..40 {
                        host.drain();
                        let _ = host.try_advance();
                    }
                    assert_eq!(row(host, id).state, storage::InputState::Ready);
                    assert_eq!(
                        row(host, later).state,
                        storage::InputState::Ready,
                        "no later queued input is dispatched after the failure"
                    );
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                });
            }

            #[test]
            fn queue_head_acquiring_the_pane_lock_too_late_for_its_settles_writes_zero_bytes() {
                with_permission_host(Duration::from_secs(60), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:late-lock"), "interaction_open");
                    let placement = host.coordinator.placement.clone();
                    let busy = PaneLock::try_acquire(&host.coordinator.database_url, &placement)
                        .unwrap()
                        .expect("another writer holds the pane lock");
                    close(host);
                    while host
                        .coordinator
                        .queue
                        .front()
                        .is_none_or(|head| head.attempt_deadline.is_none())
                    {
                        assert!(
                            Instant::now() < host.until,
                            "the head starts its attempt budget"
                        );
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    host.coordinator.queue.front_mut().unwrap().attempt_deadline =
                        Some(Instant::now() + Duration::from_millis(300));
                    drop(busy);
                    turns_until(host, |host| settled(host, id));
                    assert!(
                        host.pty.offers.is_empty(),
                        "a lock acquired with less than one settle left writes nothing: {:?}",
                        host.pty.offers
                    );
                    assert_not_submitted(host, id, "delivery_budget_exhausted");
                });
            }

            #[test]
            fn queue_head_under_an_observer_outage_settles_within_the_observer_budget() {
                with_permission_host(Duration::from_secs(90), |host| {
                    form(host);
                    let id = queued(host, host.request("prompt:outage"), "interaction_open");
                    close(host);
                    host.drain();
                    let mut backdated = false;
                    while !settled(host, id) {
                        assert!(
                            Instant::now() < host.until,
                            "the observer_unavailable attempt budget, not the 3-minute bound, settles the head"
                        );
                        host.controller
                            .observer_failed(
                                crate::runtime::observation::ObservationFailure::Detector,
                                &RealObservationClock,
                            )
                            .unwrap();
                        host.advance();
                        if !backdated
                            && let Some(head) = host.coordinator.queue.front_mut()
                            && head.attempt_deadline.is_some()
                        {
                            head.attempt_deadline =
                                Some(Instant::now() + Duration::from_millis(300));
                            backdated = true;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    assert!(backdated, "the outage started the head's attempt budget");
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    let input = row(host, id);
                    assert_eq!(
                        input.disposition,
                        Some(storage::InputDisposition::NotSubmitted)
                    );
                    assert_eq!(input.delivery_method, None);
                    assert_eq!(
                        input.delivery_reason.as_deref(),
                        Some("observer_unavailable")
                    );
                });
            }
        }

        mod method_rendering {
            use crate::broker::runtime::DeliveryMethod;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::queued;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::row;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until;
            use crate::runtime::input::tests::ordinary_prompt::*;

            const UNCLASSIFIED: &[u8] = b"\x1b[2J\x1b[1;1Hworking on something";

            fn planless_catalog() -> ProfileCatalog {
                prompt_plan_catalog(&catalog_definition()).without_submission(
                    crate::coding_agent::CodingAgent::Codex,
                    crate::coding_agent::ComposerOperation::Prompt,
                )
            }

            fn fallback_preview(canonical: &str) -> Vec<u8> {
                let mut bytes = b"\x1b".to_vec();
                bytes.extend_from_slice(
                    crate::delivery::fallback_preview_line(canonical).as_bytes(),
                );
                bytes.push(b'\r');
                bytes
            }

            fn preview_digest(host: &Host, message: i64, canonical: &str) -> String {
                storage::input_payload_digest(
                    host.fixture.run.run_id,
                    &SubmitInput {
                        operation_key: crate::broker::runtime::preview_operation_key(
                            host.fixture.run.run_id,
                            message,
                            1,
                        )
                        .unwrap(),
                        input_kind: storage::InputKind::Preview,
                        message_id: Some(message),
                        literal: canonical.into(),
                    },
                )
            }

            #[test]
            fn method_rendering_fallback_preview_appends_the_dismissal_clause_outside_its_digest() {
                with_catalog_host(planless_catalog(), Vec::new(), |host| {
                    let message = host.message("fallback preview body");
                    let canonical = canonical_preview(&message);
                    host.pty.expect_program(fallback_preview(&canonical));
                    let reply = settle(host, preview_request(host, &message, &canonical));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![
                            b"\x1b".to_vec(),
                            crate::delivery::fallback_preview_line(&canonical).into_bytes(),
                            b"\r".to_vec()
                        ]
                    );
                    let input =
                        storage::get_input(&host.fixture.conn, reply.input_id.unwrap()).unwrap();
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Fallback));
                    assert_eq!(
                        input.delivery_reason.as_deref(),
                        Some("submission_plan_unavailable")
                    );
                    assert_eq!(
                        input.payload_digest,
                        preview_digest(host, message.message_id, &canonical)
                    );
                });
            }

            #[test]
            fn method_rendering_queued_fallback_keeps_one_input_id_across_composer_changes_and_retries()
             {
                with_catalog_host(catalog(), Vec::new(), |host| {
                    let message = host.message("queued fallback body");
                    let canonical = canonical_preview(&message);
                    host.pty.expect_program(fallback_preview(&canonical));
                    host.show(UNCLASSIFIED.to_vec());
                    let id = queued(
                        host,
                        preview_request(host, &message, &canonical),
                        "composer_busy",
                    );
                    let digest = row(host, id).payload_digest;
                    host.show(b"\x1b[2J\x1b[1;1Hstill working".to_vec());
                    let repeated = submitted(
                        host.handle(preview_request(host, &message, &canonical))
                            .unwrap()
                            .expect("a repeated preview key replies from its receipt"),
                    );
                    assert_eq!(repeated.input_id, Some(id));
                    assert!(host.pty.offers.is_empty());
                    let expired = crate::time::format_utc(
                        chrono::Utc::now() - chrono::Duration::seconds(181),
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(id, &expired);
                    turns_until(host, |host| {
                        matches!(
                            row(host, id).state,
                            storage::InputState::Accepted | storage::InputState::Settled
                        )
                    });
                    host.show(SCREEN.to_vec());
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
                    assert_eq!(input.delivery_reason.as_deref(), Some("idle_timeout"));
                    assert_eq!(input.defer_reason.as_deref(), Some("composer_busy"));
                    let rows: i64 = host
                        .fixture
                        .conn
                        .query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                            row.get(0)
                        })
                        .unwrap();
                    assert_eq!(rows, 1);
                    assert_eq!(
                        input.payload_digest,
                        preview_digest(host, message.message_id, &canonical)
                    );
                });
            }
        }

        mod composer_busy {
            use crate::broker::runtime::DeliveryMethod;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::queued;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::row;
            use crate::runtime::input::tests::ordinary_prompt::deferred_queue::turns_until;
            use crate::runtime::input::tests::ordinary_prompt::*;

            const DRAFT: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic draft\r\nDraft end";

            /// The prompt-plan catalog, whose composer the draft screen leaves unverified.
            pub(super) fn busy_catalog() -> ProfileCatalog {
                prompt_plan_catalog(&catalog_definition())
            }

            /// The permission-form catalog with the prompt-plan composer.
            fn permission_busy_catalog() -> ProfileCatalog {
                let mut definition =
                    crate::coding_agent::catalog::test_support::inspection_fixture_definition();
                let composer = catalog_definition();
                definition["fixtures"]
                    .as_array_mut()
                    .unwrap()
                    .push(composer["fixtures"][1].clone());
                let empty = composer["profiles"][0]["rules"][0].clone();
                definition["profiles"][0]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(empty);
                definition["profiles"][0]["capabilities"]["composer"] =
                    composer["profiles"][0]["capabilities"]["composer"].clone();
                prompt_plan_catalog(&definition)
            }

            fn busy(host: &mut Host) {
                host.show(DRAFT.to_vec());
                assert!(matches!(
                    host.controller.health(&host.writer).eligibility,
                    InputEligibility::Hold(_)
                ));
            }

            fn fallback_program() -> Vec<u8> {
                let mut bytes = b"\x1b".to_vec();
                bytes.extend_from_slice(LITERAL.as_bytes());
                bytes.push(b'\r');
                bytes
            }

            /// Drives a fixed number of runner turns, enough for a misbehaving queue to write.
            pub(super) fn hold_turns(host: &mut Host) {
                for _ in 0..40 {
                    host.drain();
                    host.advance();
                }
            }

            /// A host whose deadline only bounds a hang, so suite load cannot fail a delivery.
            pub(super) fn with_busy_host(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                body: impl FnOnce(&mut Host),
            ) {
                with_profile_host_until(
                    catalog,
                    expected_program,
                    HostProfile::metadata(),
                    Instant::now() + Duration::from_secs(60),
                    body,
                );
            }

            fn delivered(host: &Host, input_id: i64) -> bool {
                matches!(
                    row(host, input_id).state,
                    storage::InputState::Accepted | storage::InputState::Settled
                )
            }

            #[test]
            fn composer_busy_queues_with_zero_bytes_and_drains_by_profile_when_the_composer_empties()
             {
                with_busy_host(busy_catalog(), encoded(), |host| {
                    host.show(DRAFT.to_vec());
                    assert!(matches!(
                        host.controller.health(&host.writer).eligibility,
                        InputEligibility::Hold(GateReason::Unknown)
                    ));
                    busy_then_profile(host);
                });
            }

            #[test]
            fn composer_busy_selection_requeues_with_zero_bytes_when_the_screen_blocks_before_the_first_write()
             {
                type Case = (fn() -> ProfileCatalog, fn(&mut Host), &'static str);
                let cases: [Case; 2] = [
                    (
                        crate::runtime::input::tests::ordinary_prompt::staged_delivery::permission_catalog,
                        crate::runtime::input::tests::ordinary_prompt::deferred_queue::form,
                        "interaction_open",
                    ),
                    (
                        busy_catalog,
                        |host| host.show(DRAFT.to_vec()),
                        "composer_busy",
                    ),
                ];
                for (catalog, block, reason) in cases {
                    with_busy_host(catalog(), encoded(), |host| {
                        let key = "prompt:blocked-after-selection";
                        let request = host.request(key);
                        assert!(
                            host.handle(request).unwrap().is_none(),
                            "{reason}: an eligible composer selects its method immediately"
                        );
                        block(host);
                        hold_turns(host);
                        assert!(
                            host.pty.offers.is_empty(),
                            "{reason}: no byte, Escape included: {:?}",
                            host.pty.offers
                        );
                        let requeued =
                            storage::find_input(&host.fixture.conn, host.fixture.run.run_id, key)
                                .unwrap()
                                .unwrap_or_else(|| panic!("{reason}: a durable requeued receipt"));
                        assert_eq!(requeued.state, storage::InputState::Ready, "{reason}");
                        assert_eq!(requeued.defer_reason.as_deref(), Some(reason));
                        assert_eq!(requeued.delivery_method, None, "{reason}");
                        host.show(SCREEN.to_vec());
                        turns_until(host, |host| delivered(host, requeued.input_id));
                        assert_eq!(host.pty.program, vec![encoded()], "{reason}");
                    });
                }
            }

            #[test]
            fn composer_busy_does_not_hold_a_working_member_whose_composer_row_is_empty() {
                with_busy_host(catalog(), encoded(), |host| {
                    host.show(
                        b"\x1b[2J\x1b[21;1HWorking\x1b[23;1HSynthetic state\r\nState end".to_vec(),
                    );
                    let reply = settle(host, host.request("prompt:working-empty-row"));
                    assert_eq!(reply.status, SubmissionStatus::Accepted);
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()],
                        "the native CLI queues profile input behind its current turn"
                    );
                    let input = row(host, reply.input_id.unwrap());
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Profile));
                    assert_eq!(input.deferred_at, None);
                });
            }

            fn busy_then_profile(host: &mut Host) {
                let request = host.request("prompt:busy-composer");
                let id = queued(host, request, "composer_busy");
                hold_turns(host);
                assert!(
                    host.pty.offers.is_empty(),
                    "a busy composer receives no byte, Escape included: {:?}",
                    host.pty.offers
                );
                assert_eq!(row(host, id).state, storage::InputState::Ready);
                host.show(SCREEN.to_vec());
                turns_until(host, |host| delivered(host, id));
                assert_eq!(
                    host.pty.stage_bytes(),
                    vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                );
                let input = row(host, id);
                assert_eq!(input.delivery_method, Some(DeliveryMethod::Profile));
                assert_eq!(input.delivery_reason, None);
                assert_eq!(input.defer_reason.as_deref(), Some("composer_busy"));
            }

            #[test]
            fn composer_busy_requeues_a_started_deferred_attempt_with_zero_bytes_before_its_first_stage()
             {
                with_profile_host_until(
                    permission_busy_catalog(),
                    encoded(),
                    HostProfile::metadata(),
                    Instant::now() + Duration::from_secs(60),
                    |host| {
                        crate::runtime::input::tests::ordinary_prompt::deferred_queue::form(host);
                        let request = host.request("prompt:busy-after-start");
                        let id = queued(host, request, "interaction_open");
                        let behind =
                            queued(host, host.request("prompt:behind"), "interaction_open");
                        host.show(SCREEN.to_vec());
                        while row(host, id).state == storage::InputState::Ready {
                            assert!(
                                Instant::now() < host.until,
                                "the deferred attempt starts on the verified empty composer"
                            );
                            host.drain();
                            host.advance();
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        assert_eq!(row(host, id).state, storage::InputState::InProgress);
                        assert!(host.pty.offers.is_empty());
                        host.show(DRAFT.to_vec());
                        hold_turns(host);
                        assert!(
                            host.pty.offers.is_empty(),
                            "a draft before the first stage keeps zero bytes: {:?}",
                            host.pty.offers
                        );
                        let requeued = row(host, id);
                        assert_eq!(requeued.state, storage::InputState::Ready);
                        assert_eq!(requeued.disposition, None);
                        assert_eq!(requeued.delivery_method, None);
                        assert_eq!(
                            requeued.defer_reason.as_deref(),
                            Some("interaction_open"),
                            "defer_reason keeps why the input first entered the queue"
                        );
                        assert_eq!(row(host, behind).state, storage::InputState::Ready);
                        host.show(SCREEN.to_vec());
                        turns_until(host, |host| delivered(host, id));
                        assert_eq!(
                            row(host, behind).state,
                            storage::InputState::Ready,
                            "the requeued input keeps its head position"
                        );
                        assert_eq!(
                            host.pty.offers,
                            vec![LITERAL.as_bytes().to_vec(), b"~".to_vec()]
                        );
                        assert_eq!(row(host, id).delivery_method, Some(DeliveryMethod::Profile));
                    },
                );
            }

            #[test]
            fn composer_busy_bound_starts_when_an_interaction_deferred_head_meets_a_busy_composer()
            {
                with_profile_host_until(
                    permission_busy_catalog(),
                    encoded(),
                    HostProfile::metadata(),
                    Instant::now() + Duration::from_secs(60),
                    |host| {
                        crate::runtime::input::tests::ordinary_prompt::deferred_queue::form(host);
                        let request = host.request("prompt:interaction-then-busy");
                        let id = queued(host, request, "interaction_open");
                        let long_queued = crate::time::format_utc(
                            chrono::Utc::now() - chrono::Duration::seconds(181),
                        );
                        host.coordinator
                            .set_queued_composer_busy_since(id, &long_queued);
                        host.drain();
                        host.advance();
                        busy(host);
                        hold_turns(host);
                        assert!(
                            host.pty.offers.is_empty(),
                            "time queued behind an interaction does not count toward the bound: {:?}",
                            host.pty.offers
                        );
                        assert_eq!(row(host, id).state, storage::InputState::Ready);
                        host.show(SCREEN.to_vec());
                        turns_until(host, |host| delivered(host, id));
                        let input = row(host, id);
                        assert_eq!(input.delivery_method, Some(DeliveryMethod::Profile));
                        assert_eq!(input.delivery_reason, None);
                    },
                );
            }

            #[test]
            fn composer_busy_requeued_immediate_input_keeps_its_place_ahead_of_later_queued_input()
            {
                with_busy_host(busy_catalog(), encoded(), |host| {
                    let first = host.request("prompt:first");
                    assert!(
                        host.handle(first).unwrap().is_none(),
                        "the first input selects its method immediately"
                    );
                    let later = queued(host, host.request("prompt:later"), "earlier_input");
                    busy(host);
                    hold_turns(host);
                    assert!(host.pty.offers.is_empty(), "{:?}", host.pty.offers);
                    let first = storage::find_input(
                        &host.fixture.conn,
                        host.fixture.run.run_id,
                        "prompt:first",
                    )
                    .unwrap()
                    .expect("the busy composer requeues the first input")
                    .input_id;
                    assert_eq!(row(host, first).state, storage::InputState::Ready);
                    host.show(SCREEN.to_vec());
                    turns_until(host, |host| {
                        if delivered(host, later) {
                            assert!(
                                delivered(host, first),
                                "the requeued first input drains before the later one"
                            );
                        }
                        delivered(host, first) && delivered(host, later)
                    });
                });
            }

            #[test]
            fn composer_busy_onset_survives_a_busy_requeue_and_still_reaches_idle_timeout() {
                with_busy_host(busy_catalog(), fallback_program(), |host| {
                    busy(host);
                    let request = host.request("prompt:flickering-composer");
                    let id = queued(host, request, "composer_busy");
                    let near_bound = crate::time::format_utc(
                        chrono::Utc::now() - chrono::Duration::seconds(178),
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(id, &near_bound);
                    host.show(SCREEN.to_vec());
                    while row(host, id).state == storage::InputState::Ready {
                        assert!(
                            Instant::now() < host.until,
                            "a verified empty composer starts the attempt"
                        );
                        host.drain();
                        host.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    assert!(host.pty.offers.is_empty());
                    host.show(DRAFT.to_vec());
                    hold_turns(host);
                    assert_eq!(row(host, id).state, storage::InputState::Ready);
                    assert!(
                        host.pty.offers.is_empty(),
                        "the busy requeue writes zero bytes: {:?}",
                        host.pty.offers
                    );
                    turns_until(host, |host| delivered(host, id));
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![
                            b"\x1b".to_vec(),
                            LITERAL.as_bytes().to_vec(),
                            b"\r".to_vec()
                        ],
                        "the retained onset reaches the 3-minute bound"
                    );
                    let input = row(host, id);
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Fallback));
                    assert_eq!(input.delivery_reason.as_deref(), Some("idle_timeout"));
                });
            }

            #[test]
            fn composer_busy_falls_back_with_idle_timeout_three_minutes_after_busy_onset() {
                with_busy_host(busy_catalog(), fallback_program(), |host| {
                    busy(host);
                    let request = host.request("prompt:idle-timeout");
                    let id = queued(host, request, "composer_busy");
                    let just_inside = crate::time::format_utc(
                        chrono::Utc::now() - chrono::Duration::seconds(170),
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(id, &just_inside);
                    hold_turns(host);
                    assert!(
                        host.pty.offers.is_empty(),
                        "no Escape before the 3-minute bound"
                    );
                    let expired = crate::time::format_utc(
                        chrono::Utc::now() - chrono::Duration::seconds(181),
                    );
                    host.coordinator
                        .set_queued_composer_busy_since(id, &expired);
                    turns_until(host, |host| delivered(host, id));
                    assert_eq!(
                        host.pty.stage_bytes(),
                        vec![
                            b"\x1b".to_vec(),
                            LITERAL.as_bytes().to_vec(),
                            b"\r".to_vec()
                        ]
                    );
                    let input = row(host, id);
                    assert_eq!(input.delivery_method, Some(DeliveryMethod::Fallback));
                    assert_eq!(input.delivery_reason.as_deref(), Some("idle_timeout"));
                    assert_eq!(input.defer_reason.as_deref(), Some("composer_busy"));
                });
            }
        }

        mod sql {
            use crate::db::with_database_budget;
            use crate::runtime::input::tests::ordinary_prompt::*;
            use crate::time::Deadline;
            use rusqlite::Connection;
            use rusqlite::hooks::AuthAction;
            use rusqlite::hooks::AuthContext;
            use rusqlite::hooks::Authorization;
            use rusqlite::hooks::TransactionOperation;
            use std::sync::atomic::AtomicBool;
            use std::sync::atomic::AtomicUsize;
            use std::sync::atomic::Ordering;

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
                    let until = h.until;
                    let coordinator = &mut h.coordinator;
                    let transport = &mut h.transport;
                    let pty = &mut h.pty;
                    let observation = &h.controller;
                    let started = Instant::now();
                    let pending = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
                        with_database_budget(conn, Deadline::until(until), |conn| {
                            let result = coordinator
                                .advance(conn, transport, pty, observation, until)
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
                    assert!(h.pty.program.is_empty());
                    assert!(matches!(
                        storage::get_input(&h.fixture.conn, id).unwrap().state,
                        storage::InputState::InProgress
                    ));
                    assert_eq!(h.hooks.count("before"), 1);

                    h.type_human(b"human");
                    h.child_output(SCREEN);
                    let previous_seq = h.transport.snapshot().screen_seq;
                    h.drain();
                    assert_eq!(h.human.consumed.len(), 5);
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
                        let until = h.until;
                        let coordinator = &mut h.coordinator;
                        let transport = &mut h.transport;
                        let observation = &h.controller;
                        let result = observed_sql(&mut h.fixture.conn, state.clone(), |conn| {
                            with_database_budget(conn, Deadline::until(outer), |conn| {
                                if !during {
                                    std::thread::sleep(
                                        outer.saturating_duration_since(Instant::now())
                                            + Duration::from_millis(1),
                                    );
                                }
                                match coordinator.acquire_write_guard(
                                    conn,
                                    transport,
                                    observation,
                                    until,
                                ) {
                                    Err(cause) => Err(cause),
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
                    let until = h.until;
                    let coordinator = &mut h.coordinator;
                    let transport = &mut h.transport;
                    let observation = &h.controller;
                    let mut guard =
                        with_database_budget(&mut h.fixture.conn, Deadline::until(outer), |conn| {
                            coordinator
                                .acquire_write_guard(conn, transport, observation, until)?
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

        pub(in crate::runtime::input) mod support {
            use crate::runtime::input::tests::ordinary_prompt::*;
            use crate::runtime::system::test_support::TestChild;
            use crate::runtime::writer::test_support::WriterFixture;
            use std::process::Command;
            use std::process::Stdio;
            use std::sync::Mutex;

            pub(super) const LITERAL: &str = "ordinary fixture λ";
            pub(super) const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
            pub(super) use crate::coding_agent::catalog::test_support::VERSION;
            use crate::runtime::transport::test_support::{FakeDisplay, FakeInput};

            pub(in crate::runtime::input) fn offers(values: &[&[u8]]) -> Vec<Vec<u8>> {
                values.iter().map(|value| value.to_vec()).collect()
            }

            /// Input services whose token source refuses use, naming `purpose` in the panic.
            pub(in crate::runtime::input) fn services(
                catalog: ProfileCatalog,
                hooks: Arc<dyn InputHooks>,
                stage_settle: Duration,
                purpose: &'static str,
            ) -> InputServices {
                InputServices {
                    catalog,
                    token_source: Box::new(PanicTokens(purpose)),
                    hooks,
                    stage_settle,
                }
            }

            pub(in crate::runtime::input) fn catalog_definition() -> serde_json::Value {
                let mut value = crate::coding_agent::catalog::test_support::synthetic_definitions();
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
                rule["extract"] = serde_json::json!({"operation":null,"choices":[]});
                rule["verified_noninteraction"] = serde_json::json!(true);
                value["profiles"][0]["capabilities"] = serde_json::json!({
                    "detection":null,"annotations":null,"answers":[],"composer":{
                        "rule_ids":["synthetic-composer-empty"],"fixture_ids":[fixture["id"]],
                        "submission":{"prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"7e","fixture_ids":["synthetic-prompt-plan"]},"exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-prompt-plan"]}}
                    }
                });
                value
            }

            pub(super) fn catalog() -> ProfileCatalog {
                prompt_plan_catalog(&catalog_definition())
            }

            /// A catalog built on [`catalog_definition`]'s composer, whose exec submission is a
            /// placeholder: only the prompt operation keeps an executable plan.
            pub(in crate::runtime::input) fn prompt_plan_catalog(
                definition: &serde_json::Value,
            ) -> ProfileCatalog {
                crate::coding_agent::catalog::test_support::load(definition)
                    .unwrap()
                    .without_submission(
                        crate::coding_agent::CodingAgent::Codex,
                        crate::coding_agent::ComposerOperation::Exec,
                    )
            }

            pub(super) fn encoded() -> Vec<u8> {
                let mut bytes = LITERAL.as_bytes().to_vec();
                bytes.push(b'~');
                bytes
            }

            /// Submits `request`, then drives drained turns until its first reply.
            pub(in crate::runtime::input) fn settle(
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

            pub(in crate::runtime::input) fn canonical_preview(
                message: &crate::broker::MessageRecord,
            ) -> String {
                crate::delivery::canonical_preview_line(message)
            }

            pub(in crate::runtime::input) fn preview_request(
                host: &Host,
                message: &crate::broker::MessageRecord,
                literal: &str,
            ) -> InputControlRequest {
                let key = crate::broker::runtime::preview_operation_key(
                    host.fixture.run.run_id,
                    message.message_id,
                    1,
                )
                .unwrap();
                let mut request = host.request(&key);
                request.operation = InputControlOperation::SubmitPrompt(SubmitInput {
                    operation_key: key,
                    input_kind: storage::InputKind::Preview,
                    message_id: Some(message.message_id),
                    literal: literal.into(),
                });
                request
            }

            pub(in crate::runtime::input) fn delivery_selection(
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

            pub(in crate::runtime::input) struct HostProfile<'a> {
                pub(crate) policy: storage::evidence::EvidencePolicy,
                pub(crate) event_evidence: &'static str,
                pub(crate) backend: crate::coding_agent::CodingAgent,
                pub(crate) native_version: &'a str,
                pub(crate) size: TerminalSize,
                pub(crate) screen: Vec<u8>,
                pub(crate) stage_settle: Duration,
            }
            pub(in crate::runtime::input) const TEST_STAGE_SETTLE: Duration =
                Duration::from_millis(50);
            impl HostProfile<'_> {
                pub(crate) fn synthetic(
                    policy: storage::evidence::EvidencePolicy,
                    event_evidence: &'static str,
                ) -> Self {
                    Self {
                        policy,
                        event_evidence,
                        backend: crate::coding_agent::CodingAgent::Codex,
                        native_version: VERSION,
                        size: TerminalSize { rows: 24, cols: 80 },
                        screen: SCREEN.to_vec(),
                        stage_settle: TEST_STAGE_SETTLE,
                    }
                }

                pub(crate) fn metadata() -> Self {
                    Self::synthetic(storage::evidence::EvidencePolicy::metadata(), "metadata")
                }
            }

            pub(in crate::runtime::input) struct PanicTokens(pub(crate) &'static str);
            impl RequestTokenSource for PanicTokens {
                fn fill(&mut self, _: &mut [u8; 32]) -> Result<()> {
                    panic!("{} creates no request token", self.0)
                }
            }

            #[derive(Clone, Debug)]
            pub(in crate::runtime::input) struct Point {
                pub(crate) boundary: &'static str,
                pub(crate) id: Option<i64>,
                pub(crate) count: u64,
                pub(crate) seq: u64,
            }

            pub(in crate::runtime::input) struct Hooks {
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
                pub(crate) fn reserved_input_ids(&self) -> Vec<i64> {
                    self.points
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|p| p.boundary == "reserved")
                        .map(|p| p.id.expect("actual durable input ID"))
                        .collect()
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

            /// `program` holds each completed programmatic input; `stages` holds every
            /// separately offered programmatic stage with its offer time, so settle intervals
            /// between a literal and its submit key are observable.
            pub(in crate::runtime::input) struct Pty {
                pub(crate) child: FakeInput,
                pub(crate) offers: Vec<Vec<u8>>,
                pub(crate) written: Vec<u8>,
                pub(crate) program: Vec<Vec<u8>>,
                pub(crate) stages: Vec<(Instant, Vec<u8>)>,
                pub(crate) manual: Vec<u8>,
                pub(crate) short: Option<usize>,
                pub(crate) fail_next: Option<io::ErrorKind>,
                pub(crate) failure: Option<io::ErrorKind>,
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
                    if let Some(kind) = self.fail_next.take() {
                        return Err(io::Error::new(kind, "injected one-shot PTY write failure"));
                    }
                    if let Some(kind) = self.failure {
                        return Err(io::Error::new(kind, "injected PTY write failure"));
                    }
                    let belongs_to_program =
                        self.expected_program[self.program_offset..].starts_with(bytes);
                    if belongs_to_program || bytes == b"\x1b[201~" {
                        let conn = crate::db::connect(&self.database_url).unwrap();
                        assert!(
                            self.hooks.reserved_input_ids().into_iter().any(|id| {
                                storage::get_input(&conn, id).unwrap().state
                                    == storage::InputState::InProgress
                            }),
                            "a programmatic stage writes only for an in-progress reservation"
                        );
                        assert!(self.hooks.count("before") > 0);
                        self.stages.push((Instant::now(), bytes.to_vec()));
                        let accepted = self.short.unwrap_or(bytes.len()).min(bytes.len());
                        self.written.extend_from_slice(&bytes[..accepted]);
                        if belongs_to_program {
                            self.program_offset += accepted;
                        }
                        if belongs_to_program && self.program_offset == self.expected_program.len()
                        {
                            self.program.push(self.expected_program.clone());
                            self.program_offset = 0;
                        }
                        Ok(accepted)
                    } else {
                        self.manual.extend_from_slice(bytes);
                        self.written.extend_from_slice(bytes);
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
            fn queue(input: &mut FakeInput, bytes: &[u8]) {
                assert!(
                    !bytes.is_empty(),
                    "an empty chunk would read as end of file"
                );
                input.chunks.push_back(bytes.to_vec());
            }

            pub(in crate::runtime::input) struct Child {
                native: TestChild,
                until: Instant,
            }
            impl Drop for Child {
                fn drop(&mut self) {
                    self.native.terminate_until(self.until);
                }
            }

            pub(in crate::runtime::input) struct RunSelection<'a> {
                pub(crate) backend: crate::coding_agent::CodingAgent,
                pub(crate) native_version: &'a str,
                pub(crate) event_evidence: &'static str,
            }

            /// Keeps exact input-event sequences independent of storage scheduling delays.
            struct InputFixtureWriterHooks;

            impl crate::runtime::writer::WriterHooks for InputFixtureWriterHooks {
                fn before(
                    &self,
                    _: crate::runtime::writer::WriterBoundary,
                    _: Instant,
                ) -> Result<()> {
                    Ok(())
                }

                fn work_slice(&self) -> Duration {
                    Duration::from_secs(30)
                }
            }

            /// A run owned by this process as its runner, with a started writer, observation
            /// controller and transport over a live `/bin/cat` native child.
            pub(in crate::runtime::input) struct RunnerFixture {
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
                pub(in crate::runtime::input) fn new(
                    catalog: &ProfileCatalog,
                    policy: storage::evidence::EvidencePolicy,
                    selection: Option<RunSelection>,
                    size: TerminalSize,
                    limits: TransportLimits,
                    until: Instant,
                ) -> Self {
                    let mut writer_fixture = WriterFixture::new();
                    let fixture = &mut writer_fixture.fixture;
                    if let Some(selection) = selection {
                        let selected =
                            catalog.capabilities(selection.backend, Some(selection.native_version));
                        let coverage = selected.detection;
                        fixture.conn.execute("UPDATE member_runs SET backend=?1,native_version=?2,coverage=?3,event_evidence=?4 WHERE run_id=?5",rusqlite::params![selection.backend.name(),selection.native_version,coverage.as_str(),selection.event_evidence,fixture.run.run_id]).unwrap();
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
                        ObservationController::new(&fixture.run, catalog, policy).unwrap();
                    let mut writer = EventWriter::start_with_hooks(
                        format!("sqlite:///{}", writer_fixture.fixture.conn.path().unwrap()),
                        writer_fixture.fixture.run.clone(),
                        writer_fixture.lock.clone(),
                        Arc::new(InputFixtureWriterHooks),
                    )
                    .unwrap();
                    writer.set_deadline(until);
                    let WriterFixture {
                        mut fixture,
                        artifacts,
                        lock,
                    } = writer_fixture;
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
                    status.diagnostic.is_none(),
                    "writer finish confirms join or reports degradation: {status:?}"
                );
                assert!(reaped.unwrap().is_some(), "owned native child was reaped");
            }

            pub(in crate::runtime::input) struct Host {
                pub(crate) fixture: Fixture,
                _artifacts: RunArtifacts,
                pub(crate) coordinator: InputCoordinator,
                pub(crate) writer: EventWriter,
                pub(crate) controller: ObservationController,
                pub(crate) transport: Transport,
                pub(crate) human: FakeInput,
                pub(crate) pty: Pty,
                pub(crate) output: FakeDisplay,
                pub(crate) hooks: Arc<Hooks>,
                pub(crate) until: Instant,
                pub(crate) stage_settle: Duration,
                pub(crate) confirm_submissions: bool,
                confirmation_frame: Vec<u8>,
                confirmation_programs: std::collections::VecDeque<Vec<u8>>,
                confirmed_bytes: usize,
                expires: u64,
                child: Child,
            }

            impl Host {
                /// Queues `bytes` as one read of native child output.
                pub(crate) fn child_output(&mut self, bytes: impl AsRef<[u8]>) {
                    queue(&mut self.pty.child, bytes.as_ref());
                }
                /// Queues `bytes` as one read of human terminal input.
                pub(crate) fn type_human(&mut self, bytes: impl AsRef<[u8]>) {
                    queue(&mut self.human, bytes.as_ref());
                }
                pub(in crate::runtime::input) fn pending_snapshot(
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
                        pending.input_id(),
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
                        size,
                        screen,
                        stage_settle,
                    } = profile;
                    let catalog = catalog();
                    let composer_selected =
                        catalog.capabilities(backend, Some(native_version)).composer
                            == storage::Coverage::Tested;
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
                        services(catalog, hooks.clone(), stage_settle, "ordinary Prompt"),
                        until,
                    )
                    .unwrap();
                    let pty = Pty {
                        child: FakeInput::new(&screen, screen.len().max(1)),
                        offers: Vec::new(),
                        written: Vec::new(),
                        program: Vec::new(),
                        stages: Vec::new(),
                        manual: Vec::new(),
                        short: None,
                        fail_next: None,
                        failure: None,
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
                        human: FakeInput::default(),
                        pty,
                        output: FakeDisplay::default(),
                        hooks,
                        until,
                        stage_settle,
                        confirm_submissions: true,
                        confirmation_frame: screen,
                        confirmation_programs: std::collections::VecDeque::new(),
                        confirmed_bytes: 0,
                        expires,
                        child,
                    };
                    host.drain();
                    if composer_selected {
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
                    self.child_output(&bytes);
                    self.drain();
                    if self.controller.health(&self.writer).eligibility
                        == InputEligibility::ComposerEmpty
                    {
                        self.confirmation_frame = bytes;
                    }
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
                        if matches!(result, InputDrain::Quiescent) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                pub(crate) fn advance(&mut self) -> Option<InputControlReply> {
                    self.try_advance().unwrap()
                }
                pub(crate) fn confirm_after_programs(
                    &mut self,
                    programs: impl IntoIterator<Item = Vec<u8>>,
                ) {
                    assert!(self.confirmation_programs.is_empty());
                    self.confirmation_programs.extend(programs);
                    assert!(!self.confirmation_programs.is_empty());
                    assert!(
                        self.confirmation_programs
                            .iter()
                            .all(|program| !program.is_empty())
                    );
                    self.confirmed_bytes = self.pty.written.len();
                }
                pub(crate) fn try_advance(
                    &mut self,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    let ordinary = self
                        .coordinator
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.composer().is_some());
                    let completed = self.pty.program.len();
                    let result = self.coordinator.advance(
                        &mut self.fixture.conn,
                        &mut self.transport,
                        &mut self.pty,
                        &self.controller,
                        self.until,
                    );
                    let complete_program = match self.confirmation_programs.front() {
                        Some(program) => {
                            self.pty.written.len() >= self.confirmed_bytes + program.len()
                                && self.pty.written.ends_with(program)
                        }
                        None => self.pty.program.len() > completed,
                    };
                    if ordinary && self.confirm_submissions && complete_program {
                        self.confirmation_programs.pop_front();
                        self.confirmed_bytes = self.pty.written.len();
                        self.child_output(self.confirmation_frame.clone());
                    }
                    result
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
                            crate::runtime::receive_input_expiration(
                                request.expires_at_mono_ns,
                                Instant::now(),
                                &mut crate::runtime::RealInputDeadlineClock,
                            )
                        })
                        .map(|received| received.min(deadline))
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
                        request,
                        &self.controller,
                        deadline,
                    )
                }
                pub(crate) fn receipt(
                    &mut self,
                    action: crate::runtime::ReceiptAction,
                ) -> std::result::Result<InputControlReply, InputFailure> {
                    let mut request = self.request("unused-receipt-key");
                    request.operation = InputControlOperation::Receipt(action);
                    self.handle(request)
                        .map(|reply| reply.expect("receipt actions reply immediately"))
                }
                pub(crate) fn respond(
                    &mut self,
                    input: &crate::runtime::PermissionRespondInput,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    self.respond_until(input, self.until)
                }
                pub(crate) fn respond_until(
                    &mut self,
                    input: &crate::runtime::PermissionRespondInput,
                    deadline: Instant,
                ) -> std::result::Result<Option<InputControlReply>, InputFailure> {
                    let mut request = self.request("unused-response-key");
                    request.operation = InputControlOperation::PermissionRespond(input.clone());
                    self.coordinator.handle_received(
                        &mut self.fixture.conn,
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
                pub(crate) fn placement(&self) -> crate::broker::Placement {
                    crate::broker::get_member(
                        &self.fixture.conn,
                        self.fixture.run.member_id,
                        self.fixture.run.fleet_id,
                    )
                    .unwrap()
                    .unwrap()
                    .placement
                }
                pub(crate) fn quiesce_once(&mut self) -> InputDrain {
                    self.coordinator
                        .progress_with_manual(&mut self.transport, |transport| {
                            transport.quiesce_for_input(
                                &mut self.human,
                                &mut self.pty,
                                &mut self.output,
                                &mut self
                                    .controller
                                    .observer(&self.writer, &crate::runtime::RealObservationClock),
                                self.until,
                            )
                        })
                        .unwrap()
                }
                pub(crate) fn message(&mut self, body: &str) -> crate::broker::MessageRecord {
                    let run = self.fixture.run.clone();
                    crate::broker::send_message(
                        &mut self.fixture.conn,
                        run.director_member_id,
                        run.member_id,
                        body,
                    )
                    .unwrap()
                }
            }
            pub(in crate::runtime::input) fn with_host(body: impl FnOnce(&mut Host)) {
                with_owned_host(Host::new(), body);
            }
            pub(in crate::runtime::input) fn with_host_until(
                until: Instant,
                body: impl FnOnce(&mut Host),
            ) {
                with_owned_host(Host::until(until), body);
            }
            pub(in crate::runtime::input) fn with_catalog_host(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                body: impl FnOnce(&mut Host),
            ) {
                with_profile_host(catalog, expected_program, HostProfile::metadata(), body);
            }
            pub(in crate::runtime::input) fn with_profile_host(
                catalog: ProfileCatalog,
                expected_program: Vec<u8>,
                profile: HostProfile,
                body: impl FnOnce(&mut Host),
            ) {
                with_owned_host(Host::with_catalog(catalog, expected_program, profile), body);
            }
            pub(in crate::runtime::input) fn with_profile_host_until(
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
        use crate::broker::runtime::Coverage;
        use crate::broker::runtime::InteractionClass;
        use crate::broker::runtime::PermissionDecision;
        use crate::coding_agent::catalog::test_support::inspection_fixture_definition;
        use crate::coding_agent::catalog::test_support::inspection_fixture_form;
        pub(super) use crate::coding_agent::detector::test_support::native_fixture_bytes;
        use crate::runtime::PermissionInspectInput;
        use crate::runtime::PermissionInspection;
        use crate::runtime::RuntimePaths;
        use crate::runtime::input::tests::ordinary_prompt::support::Host;
        use crate::runtime::input::tests::ordinary_prompt::support::HostProfile;
        use crate::runtime::input::tests::ordinary_prompt::support::catalog_definition;
        use crate::runtime::input::tests::ordinary_prompt::support::with_profile_host;
        use crate::runtime::input::*;
        use rusqlite::OptionalExtension;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;
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
            with_claude_host("2.1.302", "claude-2.1.302-idle-composer-1", body);
        }

        pub(super) fn with_claude_host(
            native_version: &'static str,
            idle_fixture: &str,
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_profile_host(
                ProfileCatalog::bundled().clone(),
                b"unused-program~".to_vec(),
                HostProfile {
                    backend: crate::coding_agent::CodingAgent::Claude,
                    native_version,
                    size: crate::runtime::TerminalSize {
                        rows: 36,
                        cols: 120,
                    },
                    screen: native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(idle_fixture),
                    ),
                    ..HostProfile::metadata()
                },
                |h| with_counted_tokens(h, body),
            );
        }

        pub(super) fn with_opencode_inspection(
            body: impl FnOnce(&mut Host, Arc<AtomicUsize>, Arc<AtomicBool>),
        ) {
            with_profile_host(
                ProfileCatalog::bundled().clone(),
                b"\r".to_vec(),
                HostProfile {
                    backend: crate::coding_agent::CodingAgent::Opencode,
                    native_version: "2.1.4",
                    size: crate::runtime::TerminalSize {
                        rows: 36,
                        cols: 120,
                    },
                    screen: native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-idle-composer-1",
                        ),
                    ),
                    ..HostProfile::metadata()
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
                crate::runtime::input::tests::ordinary_prompt::support::prompt_plan_catalog(
                    &definition,
                ),
                expected_program,
                profile,
                |h| with_counted_tokens(h, body),
            );
        }

        pub(super) fn form(h: &mut Host, operation: &str, focus: &str) {
            h.show(inspection_fixture_form(operation, focus));
        }

        pub(super) fn current_ids(h: &Host) -> Option<(i64, i64, i64, i64)> {
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
            h.child_output(
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
            let InputControlOperation::PermissionInspect(input) = request.operation else {
                panic!("an inspection request carries an inspection operation")
            };
            h.coordinator
                .inspect_received(
                    &mut h.fixture.conn,
                    input,
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
                let mutations: [Mutation; 13] = [
                    (
                        "request_generation",
                        Box::new(|r| r.observed.request_generation += 1),
                    ),
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
                        .map(|missing| missing.as_str())
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
                h.child_output(b"\x1b[?25l\x1b[?25h");
                h.drain();
                let redraw = inspect(h, ids);
                assert!(redraw.output_seq > original.output_seq);
                assert_eq!(h.controller.request_generation(), revision);
                assert_eq!(token(&redraw), previous);
                h.transport
                    .resize(
                        &mut h.pty,
                        crate::runtime::TerminalSize { rows: 24, cols: 81 },
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
                h.type_human(b"!");
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
        fn permission_inspection_refuses_stale_associations_and_expired_budget_without_effects() {
            with_inspection(|h, calls, _| {
                let ids = stable_form(h);
                let original = token(&inspect(h, ids));
                let before = counts(h);
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
                        crate::runtime::ObservationFailure::Parser,
                        &crate::runtime::RealObservationClock,
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
            use crate::coding_agent::CodingAgent;
            use crate::coding_agent::Detector;
            use crate::runtime::TerminalSize;
            use crate::runtime::TerminalState;
            let catalog =
                crate::coding_agent::catalog::test_support::load(&inspection_fixture_definition())
                    .unwrap();
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
                    .map(|missing| missing.as_str())
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
            assert_not_impl!(CurrentRequest: std::fmt::Debug, serde::Serialize);
        }
    }
    mod permission_response_contracts {
        use crate::broker::runtime::DecisionSource;
        use crate::broker::runtime::DirectorMode;
        use crate::broker::runtime::PermissionDecision;
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::catalog::AnswerKey;
        use crate::coding_agent::catalog::test_support::inspection_fixture_definition;
        use crate::runtime::ByteInput;
        use crate::runtime::PermissionInspection;
        use crate::runtime::PermissionRespondInput;
        use crate::runtime::PermissionResponseResult;
        use crate::runtime::ReceiptAction;
        use crate::runtime::ReceiptReply;
        use crate::runtime::TerminalSize;
        use crate::runtime::input::tests::ordinary_prompt::support::Host;
        use crate::runtime::input::tests::ordinary_prompt::support::Pty;
        use crate::runtime::input::tests::permission_inspection_contracts::counts;
        use crate::runtime::input::tests::permission_inspection_contracts::detail_rewrite;
        use crate::runtime::input::tests::permission_inspection_contracts::form;
        use crate::runtime::input::tests::permission_inspection_contracts::inspect;
        use crate::runtime::input::tests::permission_inspection_contracts::inspect_request;
        use crate::runtime::input::tests::permission_inspection_contracts::native_fixture_bytes;
        use crate::runtime::input::tests::permission_inspection_contracts::request as inspection_target;
        use crate::runtime::input::tests::permission_inspection_contracts::settled_ids;
        use crate::runtime::input::tests::permission_inspection_contracts::stable_detailed_form;
        use crate::runtime::input::tests::permission_inspection_contracts::stable_form;
        use crate::runtime::input::tests::permission_inspection_contracts::with_claude_host;
        use crate::runtime::input::tests::permission_inspection_contracts::with_claude_inspection;
        use crate::runtime::input::tests::permission_inspection_contracts::with_definition;
        use crate::runtime::input::tests::permission_inspection_contracts::with_inspection;
        use crate::runtime::input::tests::permission_inspection_contracts::with_redacted_inspection;
        use crate::runtime::input::*;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        fn command(
            h: &Host,
            inspection: &PermissionInspection,
            key: &str,
            decision: PermissionDecision,
        ) -> PermissionRespondInput {
            keyed_command(h, inspection, key, AnswerKey::Decision(decision))
        }
        fn keyed_command(
            h: &Host,
            inspection: &PermissionInspection,
            key: &str,
            answer: AnswerKey,
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
                key: answer,
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
                (&m.request_token, &m.key, m.decision_source, m.director_mode),
                (
                    &input.request_token,
                    &input.key,
                    input.decision_source,
                    input.director_mode
                )
            );
            assert_eq!(
                (&m.mode_evidence_ref, &m.decision_evidence_ref),
                (&input.mode_evidence_ref, &input.decision_evidence_ref)
            );
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
                    (|h: &mut Host| {
                        h.coordinator.fence(h.until).unwrap();
                    }) as fn(&mut Host),
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

        #[test]
        fn permission_screen_seq_only_redraw_schedules_no_refresh_and_keeps_the_settled_token() {
            with_inspection(|h, _, _| {
                let ids = stable_form(h);
                let inspected = inspect(h, ids);
                let input = command(h, &inspected, "response:redraw", PermissionDecision::Allow);
                let output_seq = h.transport.snapshot().screen_seq;
                let lock = Connection::open(h.fixture.conn.path().unwrap()).unwrap();
                lock.execute_batch("BEGIN IMMEDIATE").unwrap();
                // Every observed frame keeps identical cells with the cursor at the request end: the
                // observer runs after each control byte, so `[24;1HSelection: ` lands as one chunk.
                h.child_output(b"\x1b[?25l\x1b[0m\x1b[24;1HSelection: \x1b[?25h");
                let drained = h
                    .coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        transport.quiesce_for_input(
                            &mut h.human,
                            &mut h.pty,
                            &mut h.output,
                            &mut h
                                .controller
                                .observer(&h.writer, &crate::runtime::RealObservationClock),
                            h.until,
                        )
                    })
                    .unwrap();
                assert!(
                    h.transport.snapshot().screen_seq > output_seq,
                    "the redraw produced new output"
                );
                assert!(
                    matches!(drained, InputDrain::Quiescent),
                    "a screen_seq-only redraw needs no write, so it settles while the database is locked"
                );
                assert!(h.controller.current_request().is_some());
                let while_locked = inspect_request(h, inspection_target(h, ids), h.until).unwrap();
                assert_eq!(
                    while_locked.request_token, inspected.request_token,
                    "the settled request keeps its token without a database write"
                );
                lock.execute_batch("COMMIT").unwrap();
                drop(lock);
                h.drain();
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

        // One CSI sequence longer than the drain's per-tick child budget: it changes no cell or
        // cursor and yields only two observed frames, so the drain ends with output still arriving.
        fn neutral_output_flood(h: &mut Host) {
            let mut bytes = b"\x1b[".to_vec();
            bytes.extend(b"0;".repeat(2200));
            bytes.push(b'm');
            h.child_output(bytes);
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
            h.child_output(native_fixture_bytes(
                &crate::coding_agent::detector::test_support::bundled_screen(
                    "claude-2.1.302-default-permission-1",
                ),
            ));
            settled_ids(h)
        }

        fn claude_boxed_single_row_form(h: &mut Host) -> (i64, i64, i64, i64) {
            h.child_output(native_fixture_bytes(
                &crate::coding_agent::detector::test_support::bundled_screen(
                    "claude-2.1.302-boxed-single-row-permission-1",
                ),
            ));
            settled_ids(h)
        }

        type FormFeed = fn(&mut Host) -> (i64, i64, i64, i64);

        #[test]
        fn a_preview_during_a_visible_claude_permission_form_is_queued_without_child_bytes() {
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
                    let key = crate::broker::runtime::preview_operation_key(
                        run.run_id,
                        message.message_id,
                        1,
                    )
                    .unwrap();
                    let mut request = h.request(&key);
                    request.operation =
                        InputControlOperation::SubmitPrompt(crate::runtime::SubmitInput {
                            operation_key: key.clone(),
                            input_kind: storage::InputKind::Preview,
                            message_id: Some(message.message_id),
                            literal: crate::delivery::canonical_preview_line(&message),
                        });
                    let reply = h
                        .coordinator
                        .handle_received(&mut h.fixture.conn, request, &h.controller, h.until)
                        .unwrap();
                    let Some(InputControlReply::Submission(queued)) = reply else {
                        panic!(
                            "{fixture}: a preview during a visible form returns a Submission reply"
                        )
                    };
                    assert_eq!(
                        (queued.status, queued.stage),
                        (SubmissionStatus::Queued, SubmissionStage::ZeroBytes),
                        "{fixture}"
                    );
                    assert_eq!(
                        queued
                            .delivery
                            .as_ref()
                            .and_then(|delivery| delivery.reason.as_deref()),
                        Some("interaction_open"),
                        "{fixture}: the visible form, not a missing composer plan, defers the preview"
                    );
                    assert_eq!(
                        h.pty.offers, offers,
                        "{fixture}: no child byte, Escape included, is offered"
                    );
                    assert_eq!(
                        h.pty.program, program,
                        "{fixture}: no program byte is written"
                    );
                    let receipt = storage::find_input(&h.fixture.conn, run.run_id, &key)
                        .unwrap()
                        .unwrap_or_else(|| panic!("{fixture}: the queued preview has a receipt"));
                    assert_eq!(Some(receipt.input_id), queued.input_id, "{fixture}");
                    assert_eq!(receipt.state, storage::InputState::Ready, "{fixture}");
                    assert_eq!(
                        receipt.defer_reason.as_deref(),
                        Some("interaction_open"),
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
        fn a_preview_prompt_and_exec_during_dialog_a_queue_with_zero_bytes_and_each_drains_once() {
            use crate::runtime::input::tests::ordinary_prompt::support::canonical_preview;
            use crate::runtime::input::tests::ordinary_prompt::support::preview_request;
            const IDLE: &str = "claude-2.1.302-auto-idle-composer-1";
            const LITERAL: &str = "queued behind the auto-mode nudge";
            let fixture = |id: &str| {
                native_fixture_bytes(
                    &crate::coding_agent::detector::test_support::bundled_screen(id),
                )
            };
            for (version, form) in [
                ("2.1.309", "claude-2.1.309-read-outside-permission-1"),
                ("2.1.312", "claude-2.1.312-auto-default-nudge-1"),
            ] {
                with_claude_host(version, IDLE, |h, _, _| {
                    h.child_output(fixture(form));
                    settled_ids(h);
                    assert!(matches!(
                        h.controller.health(&h.writer).eligibility,
                        InputEligibility::Hold(_)
                    ));
                    let run = h.fixture.run.clone();
                    let message = crate::broker::send_message(
                        &mut h.fixture.conn,
                        run.director_member_id,
                        run.member_id,
                        "preview during Dialog A",
                    )
                    .unwrap();
                    let preview = canonical_preview(&message);
                    let mut program = preview.as_bytes().to_vec();
                    program.push(b'\r');
                    program.extend_from_slice(LITERAL.as_bytes());
                    program.push(b'\r');
                    program.extend_from_slice(b"! ");
                    program.extend_from_slice(LITERAL.as_bytes());
                    program.push(b'\r');
                    h.pty.expect_program(program.clone());
                    h.confirm_after_programs([
                        [preview.as_bytes(), b"\r"].concat(),
                        [LITERAL.as_bytes(), b"\r"].concat(),
                        [b"! ".as_slice(), LITERAL.as_bytes(), b"\r"].concat(),
                    ]);
                    let mut prompt = h.request("prompt:dialog-a");
                    prompt.operation = InputControlOperation::SubmitPrompt(SubmitInput {
                        operation_key: "prompt:dialog-a".into(),
                        input_kind: storage::InputKind::Prompt,
                        message_id: None,
                        literal: LITERAL.into(),
                    });
                    let mut exec = h.request("exec:dialog-a");
                    exec.operation = InputControlOperation::SubmitExec(SubmitInput {
                        operation_key: "exec:dialog-a".into(),
                        input_kind: storage::InputKind::Exec,
                        message_id: None,
                        literal: LITERAL.into(),
                    });
                    let mut queued_ids = Vec::new();
                    for (case, request) in [
                        ("preview", preview_request(h, &message, &preview)),
                        ("prompt", prompt),
                        ("exec", exec),
                    ] {
                        let Some(InputControlReply::Submission(queued)) =
                            h.handle(request).unwrap()
                        else {
                            panic!("a {case} during Dialog A returns a Submission reply")
                        };
                        assert_eq!(
                            (queued.status, queued.stage),
                            (SubmissionStatus::Queued, SubmissionStage::ZeroBytes),
                            "{case}"
                        );
                        assert_eq!(
                            queued
                                .delivery
                                .as_ref()
                                .and_then(|delivery| delivery.reason.as_deref()),
                            Some("interaction_open"),
                            "{case}"
                        );
                        queued_ids
                            .push(queued.input_id.expect("queued work has a durable input ID"));
                    }
                    for _ in 0..20 {
                        h.drain();
                        h.advance();
                    }
                    assert!(
                        h.pty.offers.is_empty(),
                        "no byte, Enter included, reaches Dialog A: {:?}",
                        h.pty.offers
                    );
                    h.show(fixture(IDLE));
                    while !queued_ids.iter().all(|id| {
                        matches!(
                            storage::get_input(&h.fixture.conn, *id).unwrap().state,
                            storage::InputState::Accepted | storage::InputState::Settled
                        )
                    }) {
                        assert!(Instant::now() < h.until, "the queued inputs drain in time");
                        h.drain();
                        h.advance();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    for _ in 0..20 {
                        h.drain();
                        h.advance();
                    }
                    assert_eq!(
                        h.pty.program,
                        vec![program],
                        "each input is delivered once, in order"
                    );
                    assert!(h.pty.manual.is_empty(), "{:?}", h.pty.manual);
                    for (index, id) in queued_ids.into_iter().enumerate() {
                        let input = storage::get_input(&h.fixture.conn, id).unwrap();
                        if index == 0 {
                            assert_eq!(input.state, storage::InputState::Settled, "preview {id}");
                            assert_eq!(
                                input.disposition,
                                Some(storage::InputDisposition::Submitted),
                                "preview {id}"
                            );
                        } else {
                            assert_eq!(input.state, storage::InputState::Accepted, "{id}");
                        }
                        assert_eq!(
                            input.delivery_method,
                            Some(crate::broker::runtime::DeliveryMethod::Profile)
                        );
                        assert_eq!(input.defer_reason.as_deref(), Some("interaction_open"));
                    }
                });
            }
        }

        #[test]
        fn permission_claude_blink_cursor_transit_keeps_generation_and_token_and_admits_respond() {
            let writes = blink_writes();
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
                    h.child_output(&write[..]);
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
                h.child_output(onto_form);
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
                h.child_output(detail_rewrite("refreshed"));
                assert!(matches!(h.quiesce_once(), InputDrain::Pending));
                assert_eq!(blocked_reason(h, ids), "observation evidence pending");
                neutral_output_flood(h);
                assert!(matches!(h.quiesce_once(), InputDrain::Pending));
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
                assert!(matches!(h.quiesce_once(), InputDrain::Pending));
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
                    (
                        failure.context.run_id,
                        failure.context.interaction_id,
                        failure.context.input_id
                    ),
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
                h.child_output(b"\x1b[0m");
                let lookup = |h: &mut Host, key: &str| {
                    let mut retained = input.clone();
                    retained.operation_key = key.into();
                    let mut request = h.request("unused-response-key");
                    request.operation = InputControlOperation::PermissionRespond(retained);
                    h.coordinator.retained_permission_response(
                        &mut h.fixture.conn,
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
                    h.pty.child.chunks.iter().map(Vec::len).sum::<usize>(),
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
                    (|h: &mut Host| {
                        h.coordinator.fence(h.until).unwrap();
                    }) as fn(&mut Host),
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
                    assert_eq!(failure.context.input_id, Some(id));
                    assert_eq!(
                        storage::get_permission_response(&h.fixture.conn, id).unwrap(),
                        original
                    );
                    assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
                });
            }
        }

        #[test]
        fn pointer_reports_revoke_permission_tokens_without_keyboard_generation_changes() {
            for pointer in [b"\x1b[<0;10;20M".as_slice(), b"\x1b[M !!"] {
                with_redacted_inspection(|host, _, _| {
                    let ids = stable_detailed_form(host, "opened");
                    let inspection = inspect(host, ids);
                    let input = command(
                        host,
                        &inspection,
                        "response:pointer-revoked",
                        PermissionDecision::Allow,
                    );
                    let keyboard = host.transport.human_generation();
                    host.type_human(pointer);
                    host.drain();
                    assert_eq!(host.transport.human_generation(), keyboard);
                    assert_eq!(host.transport.pointer_generation(), pointer.len() as u64);
                    assert!(host.respond(&input).is_err());
                    assert_eq!(counts(host), [0, 0, 0]);
                    assert!(host.pty.program.is_empty());
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
                h.type_human(b"!");
                h.drain();
                let failure = match h.respond(&stale) {
                    Err(failure) => failure,
                    Ok(_) => panic!("revoked token refuses the fresh response"),
                };
                assert_eq!(
                    (
                        failure.context.run_id,
                        failure.context.interaction_id,
                        failure.context.input_id
                    ),
                    (Some(h.fixture.run.run_id), Some(ids.1), None)
                );
                assert_eq!(
                    failure.context.operation_key.as_deref(),
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
                h.child_output(detail_rewrite("refreshed"));
                h.coordinator
                    .progress_with_manual(&mut h.transport, |transport| {
                        transport.quiesce_for_input(
                            &mut h.human,
                            &mut h.pty,
                            &mut h.output,
                            &mut h
                                .controller
                                .observer(&h.writer, &crate::runtime::RealObservationClock),
                            h.until,
                        )
                    })
                    .unwrap();
                loop {
                    let status = h.writer.status();
                    if status.diagnostic.is_some() {
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
                        &crate::runtime::RealObservationClock,
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
                while h.writer.status().diagnostic.is_some() {
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
                        &result.key,
                        result.decision_source
                    ),
                    (
                        input.interaction_id,
                        input.event_id,
                        &input.key,
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
                h.child_output(b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end");
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
                h.confirm_after_programs([[ordinary_literal.as_bytes(), b"~"].concat()]);
                let resumed = h
                    .coordinator
                    .handle_received(&mut h.fixture.conn, ordinary, &h.controller, h.until)
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
            assert_eq!(failure.context.input_id, Some(id));
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
                h.type_human(b"!");
                h.drain();
                let failure = h
                    .respond(&input)
                    .err()
                    .expect("actual manual generation revokes Ready authority");
                assert_eq!(
                    (
                        failure.context.run_id,
                        failure.context.interaction_id,
                        failure.context.input_id
                    ),
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
                .acquire_receipt_guard(&tx, row, h.until)
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
                .acquire_receipt_guard(&tx, row, h.until)
                .unwrap();
            let proof = guard
                .zero_byte_retirement(
                    &h.fixture.run.owner(),
                    row.input.input_id,
                    row.response.metadata.interaction_id,
                )
                .unwrap();
            tx.rollback().unwrap();
            storage::test_support::release_permission_claim(
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
                        matched.plan().capability().key()
                            == &AnswerKey::Decision(PermissionDecision::Allow)
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
                    pending.input_id(),
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
                        pending.input_id()
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
                    .acquire_receipt_guard(&tx, &original, h.until)
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
                        .acquire_receipt_guard(&tx, &original, h.until)
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
                assert_eq!(failed.context.input_id, Some(id));
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
            assert_not_impl!(InputGuard<'static>: Send, Clone, serde::Serialize);
            with_definition(inspection_fixture_definition(), b"y".to_vec(), |h, _, _| {
                let (input, id) = terminal_zero(h, "response:cacheless-zero");
                let ready = storage::get_permission_response(&h.fixture.conn, id).unwrap();
                h.coordinator.fence(h.until).unwrap();
                let mut empty = InputCoordinator::new(
                    &mut h.fixture.conn,
                    &h.fixture.run,
                    h.coordinator.lock.clone(),
                    crate::runtime::input::tests::ordinary_prompt::support::services(
                        crate::coding_agent::catalog::test_support::load(
                            &inspection_fixture_definition(),
                        )
                        .unwrap(),
                        h.hooks.clone(),
                        h.stage_settle,
                        "receipt-only proof",
                    ),
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
                    let guard = empty.acquire_receipt_guard(&tx, &ready, h.until).unwrap();
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
                storage::test_support::settle_input(
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
                    .acquire_receipt_guard(&tx, &attributed, h.until)
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
                    matches!(empty.handle_received(&mut h.fixture.conn, request, &h.controller, h.until).unwrap(), Some(InputControlReply::Receipt(ReceiptReply::Retired { input_id })) if input_id == id)
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
                        &input.key,
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
                        key: input.key.clone(),
                        director_member_id: input.director_member_id,
                        director_mode: input.director_mode,
                        decision_source: input.decision_source,
                        mode_evidence_ref: input.mode_evidence_ref.clone(),
                        decision_evidence_ref: input.decision_evidence_ref.clone(),
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
                        failure.context.operation_key.as_deref(),
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
                let expired = Instant::now() + Duration::from_millis(500);
                assert!(expired < h.until);
                let mut request = h.request("unused-cacheless-ready-expiry");
                request.operation = InputControlOperation::PermissionRespond(input.clone());
                assert!(
                    h.coordinator
                        .handle_received(&mut h.fixture.conn, request, &h.controller, expired,)
                        .unwrap()
                        .is_none()
                );
                let pending = h.coordinator.pending.as_ref().unwrap();
                assert_eq!((pending.input_id(), pending.deadline), (Some(id), expired));
                assert_eq!(pending.ready().unwrap(), &ready);
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
                    (
                        failure.context.run_id,
                        failure.context.interaction_id,
                        failure.context.input_id
                    ),
                    (
                        Some(h.fixture.run.run_id),
                        Some(input.interaction_id),
                        Some(id)
                    )
                );
                assert_eq!(
                    failure.context.operation_key.as_deref(),
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
                assert_eq!(h.coordinator.pending.as_ref().unwrap().input_id(), Some(id));
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
        fn permission_response_failure_after_its_first_stage_records_write_failed() {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:failed-after-stage");
                first_stage(h, id);
                form(h, "PRIVATE_INSPECTION_OPERATION", "synthetic-other");
                let Err(failure) = h.try_advance() else {
                    panic!("a changed focus fails the positive attempt");
                };
                assert_eq!(failure.context.input_id, Some(id));
                let point = h.coordinator.retained_checkpoint(id).unwrap();
                assert_eq!(
                    (point.stage, point.written_bytes),
                    (SubmissionStage::Partial, 1)
                );
                let receipt = audit(h, id, &input, "inspection-allow-plan");
                assert_eq!(receipt.input.state, storage::InputState::Uncertain);
                assert_eq!(
                    receipt.input.stop_cause,
                    Some(storage::StopCause::WriteFailed)
                );
                assert_eq!(h.pty.offers, vec![b"y".to_vec()]);
            });
        }

        #[test]
        fn permission_response_failed_later_stage_write_records_write_failed() {
            with_inspection(|h, _, _| {
                let (_, input, id) = admitted(h, "response:failed-later-write");
                first_stage(h, id);
                arm_stage(h, 2);
                h.pty.failure = Some(io::ErrorKind::BrokenPipe);
                h.drain();
                let Err(failure) = h.try_advance() else {
                    panic!("an actual PTY write failure surfaces");
                };
                assert!(
                    failure
                        .source
                        .message()
                        .contains("injected PTY write failure"),
                    "{failure:?}"
                );
                let receipt = audit(h, id, &input, "inspection-allow-plan");
                assert_eq!(receipt.input.state, storage::InputState::Uncertain);
                assert_eq!(
                    receipt.input.stop_cause,
                    Some(storage::StopCause::WriteFailed)
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
                    (
                        failure.context.run_id,
                        failure.context.interaction_id,
                        failure.context.input_id
                    ),
                    (
                        Some(h.fixture.run.run_id),
                        Some(input.interaction_id),
                        Some(id)
                    )
                );
                assert_eq!(
                    failure.context.operation_key.as_deref(),
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

        mod question {
            use crate::coding_agent::catalog::test_support::QUESTION_DECLINE_PLAN;
            use crate::coding_agent::catalog::test_support::QUESTION_LATER_PLAN;
            use crate::coding_agent::catalog::test_support::inspection_fixture_form;
            use crate::coding_agent::catalog::test_support::question_fixture_definition;
            use crate::coding_agent::catalog::test_support::question_fixture_form;
            use crate::coding_agent::catalog::test_support::question_notice_form;
            use crate::runtime::input::tests::permission_response_contracts::*;

            fn with_question(program: &[u8], body: impl FnOnce(&mut Host)) {
                with_definition(
                    question_fixture_definition(),
                    program.to_vec(),
                    |h, _, _| body(h),
                );
            }

            fn opened_ids(h: &mut Host) -> (i64, i64, i64, i64) {
                loop {
                    h.drain();
                    let ids =
                        crate::runtime::input::tests::permission_inspection_contracts::current_ids(
                            h,
                        );
                    if let Some(ids) = ids {
                        return ids;
                    }
                    assert!(
                        Instant::now() < h.until,
                        "the question opens within the fixture deadline"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }

            fn question(h: &mut Host, focused: &[&str]) -> (i64, i64, i64, i64) {
                h.show(question_fixture_form(focused));
                settled_ids(h)
            }

            fn choice(id: &str) -> AnswerKey {
                AnswerKey::Choice(id.into())
            }

            #[test]
            fn question_inspection_lists_its_tested_choices_in_rule_order_with_a_token() {
                with_question(b"3", |h| {
                    let ids = question(h, &["accept"]);
                    let value = inspect(h, ids);
                    assert_eq!(
                        value.classification,
                        Some(storage::InteractionClass::Question)
                    );
                    assert_eq!(value.visibility, Visibility::Visible);
                    assert_eq!(value.answer_capability, storage::Coverage::Tested);
                    assert!(value.supported_decisions.is_empty());
                    assert_eq!(
                        serde_json::to_value(&value.supported_choices).unwrap(),
                        serde_json::json!([
                            {"choice_id":"decline","label":"Keep current","scope":"persistent"},
                            {"choice_id":"later","label":"Ask later","scope":"persistent"}
                        ])
                    );
                    assert!(
                        value.answer_rule_id.is_none(),
                        "distinct choice plans have no unique common rule"
                    );
                    assert!(value.capability_reason.is_none());
                    assert!(value.missing_evidence.is_empty());
                    let token = value.request_token.expect("a tested question token");
                    assert_eq!(token.len(), 64);
                    assert_eq!(counts(h), [0, 0, 0]);
                    assert!(h.pty.offers.is_empty());
                });
            }

            #[test]
            fn question_with_one_tested_plan_names_its_answer_rule() {
                let mut definition = question_fixture_definition();
                definition["profiles"][0]["capabilities"]["answers"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|answer| answer["answer_rule_id"] != QUESTION_LATER_PLAN);
                with_definition(definition, b"\x0e\r".to_vec(), |h, _, _| {
                    let ids = question(h, &["accept"]);
                    let value = inspect(h, ids);
                    assert_eq!(value.answer_rule_id.as_deref(), Some(QUESTION_DECLINE_PLAN));
                    assert_eq!(
                        serde_json::to_value(&value.supported_choices).unwrap(),
                        serde_json::json!([
                            {"choice_id":"decline","label":"Keep current","scope":"persistent"}
                        ])
                    );
                });
            }

            #[test]
            fn question_without_a_tested_plan_reports_unknown_capability_and_no_token() {
                with_question(b"3", |h| {
                    h.show(question_notice_form());
                    let ids = opened_ids(h);
                    let value = inspect(h, ids);
                    assert_eq!(
                        value.classification,
                        Some(storage::InteractionClass::Question)
                    );
                    assert_eq!(value.answer_capability, storage::Coverage::Unknown);
                    assert_eq!(
                        value.capability_reason.as_deref(),
                        Some("question has no tested answer")
                    );
                    assert!(value.request_token.is_none());
                    assert!(value.answer_rule_id.is_none());
                    assert!(value.supported_decisions.is_empty());
                    assert!(value.supported_choices.is_empty());
                    assert!(value.missing_evidence.is_empty());
                });
            }

            #[test]
            fn a_non_fresh_question_with_tested_plans_keeps_the_generic_reason() {
                with_question(b"3", |h| {
                    let ids = question(h, &["accept"]);
                    let first = inspect(h, ids)
                        .request_token
                        .expect("a fresh tested question token");
                    let stale = inspect_request(h, inspection_target(h, ids), h.until).unwrap();
                    assert_eq!(
                        stale.classification,
                        Some(storage::InteractionClass::Question)
                    );
                    assert_eq!(stale.visibility, Visibility::Visible);
                    assert_eq!(
                        stale.capability_reason.as_deref(),
                        Some("current executable permission request unavailable"),
                        "a question with tested plans is reinspected, not unanswerable"
                    );
                    assert_eq!(stale.answer_capability, storage::Coverage::Unknown);
                    assert!(stale.request_token.is_none());
                    assert!(stale.answer_rule_id.is_none());
                    assert!(stale.supported_choices.is_empty());
                    let reinspected = inspect(h, ids);
                    assert_eq!(reinspected.request_token.as_deref(), Some(first.as_str()));
                    assert!(reinspected.capability_reason.is_none());
                    assert!(h.pty.offers.is_empty());
                });
            }

            #[test]
            fn an_ungated_choice_writes_its_stage_and_records_the_choice_audit() {
                with_question(b"3", |h| {
                    let ids = question(h, &["accept"]);
                    let inspected = inspect(h, ids);
                    let input =
                        keyed_command(h, &inspected, "response:question-later", choice("later"));
                    assert!(h.respond(&input).unwrap().is_none());
                    let id = arm_stage(h, 1);
                    h.drain();
                    let result = response(h.advance().expect("the single choice stage completes"));
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
                    assert_eq!(result.key, choice("later"));
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"3"])
                    );
                    let receipt = audit(h, id, &input, QUESTION_LATER_PLAN);
                    assert_eq!(receipt.input.state, storage::InputState::Accepted);
                    assert_eq!(receipt.response.metadata.key, choice("later"));
                });
            }

            #[test]
            fn a_gated_choice_writes_each_stage_on_its_marker_focus() {
                with_question(b"\x0e\r", |h| {
                    let ids = question(h, &["accept"]);
                    let inspected = inspect(h, ids);
                    let input = keyed_command(
                        h,
                        &inspected,
                        "response:question-decline",
                        choice("decline"),
                    );
                    assert!(h.respond(&input).unwrap().is_none());
                    let id = arm_stage(h, 1);
                    h.drain();
                    assert!(
                        h.advance().is_none(),
                        "the first choice stage keeps the response open"
                    );
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"\x0e"])
                    );
                    h.show(question_fixture_form(&["decline"]));
                    arm_stage(h, 2);
                    h.drain();
                    let result = response(h.advance().expect("the second choice stage completes"));
                    assert_eq!(
                        (result.submission.status, result.submission.stage),
                        (SubmissionStatus::Accepted, SubmissionStage::Submitted)
                    );
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"\x0e", b"\r"
                        ])
                    );
                    assert_eq!(
                        audit(h, id, &input, QUESTION_DECLINE_PLAN).input.state,
                        storage::InputState::Accepted
                    );
                });
            }

            fn rejected(h: &mut Host, input: &PermissionRespondInput, reason: &str) {
                let before = counts(h);
                let result = response(
                    h.respond(input)
                        .unwrap()
                        .expect("an answer-kind mismatch replies at once"),
                );
                assert_eq!(
                    (
                        result.submission.status,
                        result.submission.stage,
                        result.submission.input_id
                    ),
                    (SubmissionStatus::Rejected, SubmissionStage::ZeroBytes, None)
                );
                assert_eq!(result.submission.reason.as_deref(), Some(reason));
                assert_eq!(result.key, input.key);
                assert_eq!(counts(h), before, "a rejection reserves nothing");
                assert!(h.pty.offers.is_empty(), "a rejection writes no byte");
            }

            #[test]
            fn a_choice_on_a_permission_request_is_rejected_with_zero_bytes() {
                with_question(b"y\r", |h| {
                    h.show(inspection_fixture_form(
                        "PRIVATE_QUESTION_PROFILE",
                        "synthetic-only",
                    ));
                    let ids = settled_ids(h);
                    let inspected = inspect(h, ids);
                    assert!(inspected.supported_choices.is_empty());
                    let input = keyed_command(
                        h,
                        &inspected,
                        "response:choice-on-permission",
                        choice("decline"),
                    );
                    rejected(
                        h,
                        &input,
                        "choice answers apply only to question interactions",
                    );
                });
            }

            #[test]
            fn a_decision_on_a_question_is_rejected_with_zero_bytes() {
                with_question(b"3", |h| {
                    let ids = question(h, &["accept"]);
                    let inspected = inspect(h, ids);
                    for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                        let input =
                            command(h, &inspected, "response:decision-on-question", decision);
                        rejected(
                            h,
                            &input,
                            "decision answers apply only to permission requests",
                        );
                    }
                });
            }

            #[test]
            fn a_choice_without_a_tested_plan_is_rejected_with_zero_bytes() {
                with_question(b"3", |h| {
                    let ids = question(h, &["accept"]);
                    let inspected = inspect(h, ids);
                    for id in ["accept", "missing"] {
                        let input =
                            keyed_command(h, &inspected, "response:untested-choice", choice(id));
                        rejected(
                            h,
                            &input,
                            &format!("choice {id} has no tested answer for this question"),
                        );
                    }
                });
            }
        }

        mod gated {
            use crate::coding_agent::catalog::test_support::gated_fixture_definition;
            use crate::coding_agent::catalog::test_support::gated_fixture_form;
            use crate::coding_agent::catalog::test_support::gated_reason_dialog;
            use crate::runtime::input::tests::permission_inspection_contracts::with_opencode_inspection;
            use crate::runtime::input::tests::permission_response_contracts::*;
            use std::sync::atomic::AtomicUsize;
            use std::sync::atomic::Ordering;

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
                    Err(failure) => assert_eq!(failure.context.input_id, Some(id)),
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
                assert_eq!(
                    h.pty.offers,
                    crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h", b"\r"])
                );
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"\r"])
                    );
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"h", b"\r"
                        ])
                    );
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"h", b"\r", b"\r"
                        ])
                    );
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"])
                    );
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"])
                    );
                });
            }

            #[test]
            fn gated_unexpected_change_between_stages_fails_uncertain_without_another_byte() {
                for (case, screen) in [
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
                        assert_eq!(
                            h.pty.offers,
                            crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"]),
                            "{case}"
                        );
                    });
                }
            }

            #[test]
            fn gated_continuation_binds_only_by_succession_in_the_same_episode() {
                type Disturbance = fn(&mut Host);
                let cases: [(&str, Disturbance); 5] = [
                    ("human input", |h| {
                        h.type_human(b"x");
                        h.drain();
                    }),
                    ("pointer input", |h| {
                        h.type_human(b"\x1b[<0;10;20M");
                        h.drain();
                    }),
                    ("an observation gap", |h| {
                        h.controller
                            .observer_failed(
                                crate::runtime::ObservationFailure::Parser,
                                &crate::runtime::RealObservationClock,
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"h", b"\r"
                        ])
                    );
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
                        &crate::coding_agent::detector::test_support::bundled_screen(
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"\r"])
                    );
                });
            }

            #[test]
            fn opencode_deny_moves_the_focus_selects_reject_and_submits_the_empty_reason() {
                with_opencode_inspection(|h, calls, _| {
                    h.show(native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"])
                    );
                    h.show(opencode_screen(
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-2",
                        ),
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    write_stage(h, 2);
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"h", b"\r"
                        ])
                    );
                    h.show(native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
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
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[
                            b"h", b"\r", b"\r"
                        ])
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), minted);
                    assert_eq!(opened(h), 1);
                });
            }

            #[test]
            fn opencode_focus_on_always_allow_after_the_first_deny_stage_fails_uncertain() {
                with_opencode_inspection(|h, _, _| {
                    h.show(native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
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
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-focus-always-1",
                        ),
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let outcome = settle(h, deadline);
                    assert_uncertain(h, id, outcome);
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"])
                    );
                    assert_eq!(
                        storage::get_permission_response(&h.fixture.conn, id)
                            .unwrap()
                            .input
                            .operation_key,
                        input.operation_key
                    );
                });
            }

            /// The bundled OpenCode fixture `id` with the command box row `row` showing `command`.
            fn opencode_command(
                id: &str,
                row: usize,
                command: &str,
                trailer_col: usize,
            ) -> Vec<u8> {
                let mut screen: serde_json::Value = serde_json::from_str(
                    &crate::coding_agent::detector::test_support::bundled_screen(id),
                )
                .unwrap();
                let cols = usize::try_from(screen["cols"].as_u64().unwrap()).unwrap();
                for (offset, text) in [command, ""].into_iter().enumerate() {
                    let line = &mut screen["lines"][row + offset];
                    *line = serde_json::json!(format!(
                        "{text:<trailer_col$}█{}",
                        " ".repeat(cols - trailer_col - 1)
                    ));
                }
                native_fixture_bytes(&screen.to_string())
            }

            #[test]
            fn opencode_resize_across_w_split_between_deny_stages_fails_uncertain() {
                with_opencode_inspection(|h, _, _| {
                    let command = "  $ /tmp/x/permission-probe deny";
                    h.show(opencode_command(
                        "opencode-2.1.4-deny-pre-stage-1",
                        14,
                        command,
                        116,
                    ));
                    let ids = settled_ids(h);
                    let deadline = soon(3000);
                    admit(
                        h,
                        ids,
                        "response:opencode-resize",
                        PermissionDecision::Deny,
                        deadline,
                    );
                    let id = write_stage(h, 1);
                    let generation = h.controller.request_generation();
                    h.transport
                        .resize(
                            &mut h.pty,
                            crate::runtime::TerminalSize { rows: 36, cols: 57 },
                        )
                        .unwrap();
                    h.show(opencode_command(
                        "opencode-2.1.4-narrow-deny-pre-stage-2-057-1",
                        17,
                        command,
                        53,
                    ));
                    let outcome = settle(h, deadline);
                    assert_uncertain(h, id, outcome);
                    assert_eq!(
                        h.pty.offers,
                        crate::runtime::input::tests::ordinary_prompt::support::offers(&[b"h"]),
                        "the second stage is never written after the layout changes"
                    );
                    assert!(h.controller.request_generation() > generation);
                    assert_eq!(opened(h), 1, "unchanged command rows keep the episode");
                });
            }

            #[test]
            fn opencode_reject_dialog_reached_by_human_input_offers_no_token() {
                with_opencode_inspection(|h, _, _| {
                    h.show(native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
                            "opencode-2.1.4-deny-pre-stage-1",
                        ),
                    ));
                    let ids = settled_ids(h);
                    assert!(inspect(h, ids).request_token.is_some());
                    h.show(native_fixture_bytes(
                        &crate::coding_agent::detector::test_support::bundled_screen(
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
        use crate::runtime::ReceiptAction;
        use crate::runtime::ReceiptReply;
        use crate::runtime::input::tests::ordinary_prompt::support::Host;
        use crate::runtime::input::tests::ordinary_prompt::support::with_host;
        use crate::runtime::input::tests::ordinary_prompt::support::with_host_until;
        use crate::runtime::input::*;
        use std::time::Duration;

        /// Fills the receipt cache with `count` completed entries modelled on the real retained
        /// entry `template`, under input IDs no durable row uses.
        fn prefill_completed(h: &mut Host, template: i64, count: usize) {
            let real = h
                .coordinator
                .operations
                .get(&template)
                .expect("real completed template");
            let RetainedSelection::Composer {
                method,
                stages,
                offset,
                closing,
                ..
            } = &real.selection
            else {
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
                                offset: *offset,
                                closing: *closing,
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
            let accepted =
                crate::runtime::input::tests::ordinary_prompt::support::settle(h, request);
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
        fn live_receipt_query_rejects_changed_creation_stamp_before_effects() {
            with_host(|h| {
                let key = "prompt:receipt-cache-created-at";
                let id = h.arm(key);
                let cache = cached(h, id);
                let pending = h.pending_snapshot();
                let mut durable = storage::get_input(&h.fixture.conn, id).unwrap();
                assert_ne!(durable.created_at, "2000-01-02T03:04:05.000000+00:00");
                durable.created_at = "2000-01-02T03:04:05.000000+00:00".into();
                assert_eq!(
                    h.fixture
                        .conn
                        .execute(
                            "UPDATE member_input_operations SET created_at=?1 WHERE input_id=?2",
                            rusqlite::params![durable.created_at, id],
                        )
                        .unwrap(),
                    1
                );
                assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), durable);
                let changes = h.fixture.conn.total_changes();
                let hooks = h.hooks.points.lock().unwrap().len();
                let failure = match h.receipt(ReceiptAction::Query {
                    operation_key: key.into(),
                }) {
                    Err(failure) => failure,
                    Ok(_) => panic!("the receipt cache binds the original creation stamp"),
                };
                let CafleetError::WithDiagnostics { cause, diagnostics } = &failure.source else {
                    panic!("the selected receipt identity remains diagnostic context: {failure:?}");
                };
                assert!(
                    matches!(cause.as_ref(), CafleetError::App(message) if message == "receipt cache identity changed")
                );
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(
                    diagnostics[0],
                    format!(
                        "run_id={} input_id={} operation_key={}",
                        h.fixture.run.run_id,
                        id,
                        serde_json::to_string(key).unwrap()
                    )
                );
                assert_eq!(failure.context.run_id, Some(h.fixture.run.run_id));
                assert_eq!(failure.context.input_id, None);
                assert_eq!(failure.context.operation_key.as_deref(), Some(key));
                assert!(h.pending_snapshot() == pending);
                assert_eq!(cached(h, id), cache);
                assert_eq!(storage::get_input(&h.fixture.conn, id).unwrap(), durable);
                assert_eq!(h.fixture.conn.total_changes(), changes);
                assert_eq!(h.hooks.points.lock().unwrap().len(), hooks);
                assert!(h.pty.offers.is_empty());
                assert!(h.pty.stages.is_empty());
                assert!(h.pty.program.is_empty());
                assert!(h.pty.manual.is_empty());
            });
        }

        #[test]
        fn live_receipt_query_preserves_the_real_zero_byte_checkpoint_and_pending_writer() {
            with_host(|h| {
                let key = "prompt:live-receipt-query";
                let id = h.arm(key);
                let point = cached(h, id);
                assert!(matches!(point.1, SubmissionStage::ZeroBytes));
                assert!(point.4 && !point.5);
                h.child_output(b"\x1b[2J\x1b[HLater unclassified output");
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
                assert!(h.advance().is_none());
                assert!(h.coordinator.pending.is_some());
                let written = cached(h, id);
                assert_eq!(written.1, SubmissionStage::Partial);
                assert_eq!(written.2, 3);
                h.pty.failure = Some(std::io::ErrorKind::BrokenPipe);
                let failure = (0..4)
                    .find_map(|_| {
                        h.drain();
                        match h.try_advance() {
                            Err(failure) => Some(failure),
                            Ok(None) => None,
                            Ok(Some(_)) => panic!("the pending suffix fails before submission"),
                        }
                    })
                    .expect("the subsequent PTY write fails within bounded drained turns");
                assert!(
                    failure
                        .source
                        .message()
                        .contains("injected PTY write failure")
                );
                let original = storage::get_input(&h.fixture.conn, id).unwrap();
                assert_eq!(original.state, storage::InputState::Uncertain);
                assert_eq!(original.stop_cause, Some(storage::StopCause::WriteFailed));
                let point = cached(h, id);
                assert_eq!(point.1, SubmissionStage::Partial);
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
                assert_eq!(h.hooks.count("before"), 2);
                assert_eq!(h.hooks.count("after"), 2);
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
                let canonical =
                    crate::runtime::input::tests::ordinary_prompt::support::canonical_preview(
                        &message,
                    );
                let mut program = canonical.as_bytes().to_vec();
                program.push(b'~');
                h.pty.expect_program(program.clone());
                let request =
                    crate::runtime::input::tests::ordinary_prompt::support::preview_request(
                        h, &message, &canonical,
                    );
                let accepted =
                    crate::runtime::input::tests::ordinary_prompt::support::settle(h, request);
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
                        .retire_received(&mut h.fixture.conn, id, h.until)
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
                        .retire_received(&mut h.fixture.conn, id, Instant::now())
                else {
                    panic!("retirement requires a live original deadline")
                };
                assert!(matches!(expired, CafleetError::DeadlineExceeded));
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
                let unrelated_id = pending.input_id().expect("actual second reservation");
                let original_deadline = pending.deadline;
                let original_bytes = pending.bytes.clone();
                let unrelated_before = cached(h, unrelated_id);
                assert!(unrelated_before.4 && !unrelated_before.5);
                let Err(failure) = h
                    .coordinator
                    .retire_received(&mut h.fixture.conn, id, h.until)
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
                assert_eq!(pending.input_id(), Some(unrelated_id));
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
                h.child_output(b"\x1b[2J\x1b[HUnclassified screen");
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

                let changes = h.fixture.conn.total_changes();
                let again = h
                    .coordinator
                    .retire_received(&mut h.fixture.conn, id, h.until)
                    .unwrap();
                assert_retired(again, id);
                assert_eq!(h.fixture.conn.total_changes(), changes);
                h.fixture
                    .conn
                    .execute(
                        "UPDATE member_input_operations SET state='accepted',disposition=NULL WHERE input_id=?1",
                        [id],
                    )
                    .unwrap();
                assert!(
                    h.coordinator
                        .retire_received(&mut h.fixture.conn, id, h.until)
                        .is_err(),
                    "cache absence still checks the durable row on the supplied connection"
                );
                assert!(
                    h.coordinator
                        .retire_received(&mut h.fixture.conn, i64::MAX, h.until)
                        .is_err()
                );
                assert_eq!(cached(h, unrelated_id), other_cache);
                assert_eq!(h.pty.offers, offers);
            });
        }
    }

    use crate::runtime::InputExpiration;
    use crate::runtime::input::test_support::*;
    use std::time::Duration;
    use std::time::Instant;

    const EXPIRATION: u64 = 10_000_000_000;

    fn ns(origin: Instant, nanos: u64) -> Instant {
        origin
            .checked_add(Duration::from_nanos(nanos))
            .expect("the platform represents the fixed nanosecond fixture offset")
    }
    fn captured(deadline: Instant, clock: &mut ScriptClock, expected: u64) -> InputExpiration {
        let expiration = InputExpiration::capture(deadline, clock).unwrap();
        assert_eq!(expiration.monotonic_ns(), expected);
        clock.consumed(1, 0);
        expiration
    }
    fn expiration(origin: Instant) -> InputExpiration {
        captured(
            ms(origin, 1000),
            &mut ScriptClock::one(sample(origin, 9_000_000_000, origin)),
            EXPIRATION,
        )
    }
    fn received(
        expiration: &InputExpiration,
        admitted_at: Instant,
        clock: &mut ScriptClock,
        expected: Instant,
    ) {
        assert_eq!(expiration.receive(admitted_at, clock).unwrap(), expected);
        clock.consumed(1, 0);
    }

    #[test]
    fn clock_capture_uses_later_instant_remaining_without_bracket_extension() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 30)));
        captured(ms(t, 100), &mut clock, 1_070_000_000);
    }

    #[test]
    fn clock_capture_equal_bracket_preserves_one_nanosecond() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 10)));
        captured(ns(ms(t, 10), 1), &mut clock, 1_000_000_001);
        let mut origin = ScriptClock::one(sample(t, 0, t));
        captured(ns(t, 1), &mut origin, 1);
    }

    #[test]
    fn clock_capture_accepts_exact_unsigned_expiration_upper_bound() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
        captured(ns(t, 5), &mut clock, u64::MAX);
    }

    #[test]
    fn clock_capture_rejects_unsigned_expiration_overflow() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(t, u64::MAX - 5, t));
        one_failure(InputExpiration::capture(ns(t, 6), &mut clock), &clock, 0);
    }

    #[test]
    fn clock_capture_rejects_equal_or_expired_deadline_without_resampling() {
        for after_ms in [100, 101] {
            let t = anchor();
            let mut clock = ScriptClock::new(vec![
                Ok(sample(ms(t, 90), 1_000_000_000, ms(t, after_ms))),
                Ok(sample(t, 1_000_000_000, t)),
            ]);
            one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 1);
        }
    }

    #[test]
    fn clock_capture_rejects_reversed_instant_bracket() {
        let t = anchor();
        let mut clock = ScriptClock::one(sample(ms(t, 30), 1_000_000_000, ms(t, 10)));
        one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 0);
    }

    #[test]
    fn clock_capture_source_failure_consumes_one_sample_without_fallback() {
        let t = anchor();
        let mut clock = ScriptClock::new(vec![Err(unavailable()), Ok(sample(t, 1_000_000_000, t))]);
        one_failure(InputExpiration::capture(ms(t, 100), &mut clock), &clock, 1);
    }

    #[test]
    fn clock_receive_uses_earlier_instant_for_conservative_deadline() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100)));
        received(&expiration, t, &mut clock, ms(t, 3000));
    }

    #[test]
    fn clock_receive_caps_expiration_at_original_five_second_admission() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 9_000_000_000, ms(t, 1100)));
        received(&expiration, t, &mut clock, ms(t, 5000));
    }

    #[test]
    fn clock_receive_rejects_equal_or_past_monotonic_expiration() {
        for monotonic_ns in [EXPIRATION, EXPIRATION + 1] {
            let t = anchor();
            let expiration = expiration(t);
            let mut clock = ScriptClock::one(sample(ms(t, 1000), monotonic_ns, ms(t, 1100)));
            one_failure(expiration.receive(t, &mut clock), &clock, 0);
        }
    }

    #[test]
    fn clock_receive_rejects_bracket_delay_equal_to_or_past_projected_deadline() {
        for (before_ms, after_ms) in [(1000, 3000), (1000, 4000), (6000, 6100)] {
            let t = anchor();
            let expiration = expiration(t);
            let mut clock = ScriptClock::one(sample(
                ms(t, before_ms),
                EXPIRATION - 2_000_000_000,
                ms(t, after_ms),
            ));
            one_failure(expiration.receive(t, &mut clock), &clock, 0);
        }
    }

    #[test]
    fn clock_receive_rejects_reversed_instant_bracket() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock =
            ScriptClock::one(sample(ms(t, 2000), EXPIRATION - 2_000_000_000, ms(t, 1000)));
        one_failure(expiration.receive(t, &mut clock), &clock, 0);
    }

    #[test]
    fn clock_receive_source_failure_consumes_one_sample_without_fallback() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::new(vec![
            Err(unavailable()),
            Ok(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100))),
        ]);
        one_failure(expiration.receive(t, &mut clock), &clock, 1);
    }

    #[test]
    fn clock_receive_reuses_immutable_expiration_without_phase_extension() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::new(vec![
            Ok(sample(ms(t, 1000), EXPIRATION - 2_000_000_000, ms(t, 1100))),
            Ok(sample(ms(t, 2000), EXPIRATION - 1_000_000_000, ms(t, 2100))),
            Ok(sample(ms(t, 3000), EXPIRATION, ms(t, 3100))),
        ]);
        assert_eq!(expiration.receive(t, &mut clock).unwrap(), ms(t, 3000));
        clock.consumed(1, 2);
        assert_eq!(expiration.monotonic_ns(), EXPIRATION);
        assert_eq!(expiration.receive(t, &mut clock).unwrap(), ms(t, 3000));
        clock.consumed(2, 1);
        assert_eq!(expiration.monotonic_ns(), EXPIRATION);
        assert!(expiration.receive(t, &mut clock).is_err());
        clock.consumed(3, 0);
        assert_eq!(expiration.monotonic_ns(), EXPIRATION);
    }

    #[test]
    fn clock_receive_preserves_positive_single_nanosecond() {
        let t = anchor();
        let expiration = expiration(t);
        let mut clock = ScriptClock::one(sample(ms(t, 1000), EXPIRATION - 1, ms(t, 1000)));
        received(&expiration, t, &mut clock, ns(ms(t, 1000), 1));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::error::Result;
    use crate::runtime::input::{InputBoundary, InputCheckpoint, InputHooks};
    use std::sync::Arc;
    use std::time::Instant;
    struct DefaultInputHooks;
    impl InputHooks for DefaultInputHooks {
        fn checkpoint(&self, _: InputBoundary, _: &InputCheckpoint, _: Instant) -> Result<()> {
            Ok(())
        }
    }
    pub(in crate::runtime) fn default_input_hooks() -> Arc<dyn InputHooks> {
        Arc::new(DefaultInputHooks)
    }

    use crate::runtime::input::*;
    use std::collections::VecDeque;

    pub(crate) struct ScriptClock {
        pub(crate) samples: VecDeque<Result<InputClockSample>>,
        pub(crate) calls: usize,
    }
    impl ScriptClock {
        pub(crate) fn new(samples: Vec<Result<InputClockSample>>) -> Self {
            Self {
                samples: samples.into(),
                calls: 0,
            }
        }
        pub(crate) fn one(sample: InputClockSample) -> Self {
            Self::new(vec![Ok(sample)])
        }
        pub(crate) fn consumed(&self, calls: usize, remaining: usize) {
            assert_eq!(self.calls, calls);
            assert_eq!(self.samples.len(), remaining);
        }
    }
    impl InputDeadlineClock for ScriptClock {
        fn sample(&mut self) -> Result<InputClockSample> {
            self.calls += 1;
            self.samples
                .pop_front()
                .expect("the operation consumes only its specified paired clock samples")
        }
    }
    pub(crate) fn anchor() -> Instant {
        Instant::now()
            .checked_add(Duration::from_secs(60))
            .expect("the platform represents the fixed future clock anchor")
    }
    pub(crate) fn ms(origin: Instant, millis: u64) -> Instant {
        origin
            .checked_add(Duration::from_millis(millis))
            .expect("the platform represents the fixed millisecond fixture offset")
    }
    pub(crate) fn sample(before: Instant, monotonic_ns: u64, after: Instant) -> InputClockSample {
        InputClockSample {
            before,
            monotonic_ns,
            after,
        }
    }
    pub(crate) fn unavailable() -> CafleetError {
        CafleetError::App("fixture clock unavailable".into())
    }
    pub(crate) fn one_failure<T>(result: Result<T>, clock: &ScriptClock, remaining: usize) {
        assert!(
            result.is_err(),
            "the invalid clock operation returns an error"
        );
        clock.consumed(1, remaining);
    }
}
