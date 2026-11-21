use crate::broker::MemberStatus;
use crate::broker::Placement;
use crate::broker::runtime as storage;
use crate::broker::runtime::*;
use crate::config::Settings;
use crate::db::bounded_by_inherited;
use crate::db::with_database_budget;
use crate::pane_lock::PaneLock;
use crate::runtime::*;
use crate::time::Deadline;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use std::time::Instant;

pub(crate) struct PreparedSubmission<'a> {
    pub(crate) request: &'a SubmitInput,
    expiration: InputExpiration,
    deadline: Instant,
}
impl<'a> PreparedSubmission<'a> {
    pub(crate) fn new(
        request: &'a SubmitInput,
        deadline: Instant,
        clock: &dyn crate::time::Clock,
    ) -> Result<Self> {
        Ok(Self {
            request,
            expiration: InputExpiration::capture(deadline, clock)?,
            deadline,
        })
    }
}

#[derive(Debug)]
pub(crate) struct InputFailure {
    pub(crate) source: CafleetError,
    pub(crate) context: InputContext,
}
impl From<InputFailure> for CafleetError {
    fn from(failure: InputFailure) -> Self {
        let context = failure.context;
        let mut fields = Vec::new();
        for (name, value) in [
            ("run_id", context.run_id),
            ("interaction_id", context.interaction_id),
            ("input_id", context.input_id),
        ] {
            if let Some(value) = value {
                fields.push(format!("{name}={value}"));
            }
        }
        if let Some(key) = context.operation_key {
            fields.push(format!(
                "operation_key={}",
                serde_json::to_string(&key).expect("a string serializes")
            ));
        }
        if fields.is_empty() {
            failure.source
        } else {
            Self::WithDiagnostics {
                cause: Box::new(failure.source),
                diagnostics: vec![fields.join(" ")],
            }
        }
    }
}
#[derive(Debug, Default)]
pub(crate) struct InputContext {
    pub(crate) run_id: Option<i64>,
    pub(crate) interaction_id: Option<i64>,
    pub(crate) input_id: Option<i64>,
    pub(crate) operation_key: Option<String>,
}
impl InputContext {
    pub(crate) fn failure(self, source: CafleetError) -> InputFailure {
        InputFailure {
            source,
            context: self,
        }
    }
}
pub(crate) trait InputCommandHost {
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock>;
    fn lock(
        &mut self,
        settings: &Settings,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<Option<PaneLock>>;
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot>;
    fn exchange(
        &mut self,
        run: &RunRecord,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> Result<InputControlReply>;
}

pub(crate) struct NativeInputHost;
impl InputCommandHost for NativeInputHost {
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
        std::sync::Arc::new(crate::time::SystemClock)
    }

    fn lock(
        &mut self,
        settings: &Settings,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<Option<PaneLock>> {
        PaneLock::acquire_within(&settings.database_url, placement, deadline)
    }

    fn health(&mut self, run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot> {
        probe_health(run, deadline, &*self.clock())
    }

    fn exchange(
        &mut self,
        run: &RunRecord,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
        let mut client = crate::runtime::control::ControlClient::connect(
            &paths.control_socket,
            deadline,
            &*self.clock(),
        )?;
        client.exchange_input(request, deadline, &*self.clock())
    }
}

/// Probes the runner's health between two revalidations of the durable target.
fn bracket_health(
    conn: &mut Connection,
    host: &mut impl InputCommandHost,
    run: &RunRecord,
    deadline: Instant,
    validate: impl Fn(&mut Connection) -> Result<()>,
) -> Result<()> {
    validate(conn)?;
    host.health(run, deadline)?;
    validate(conn)
}

fn control_request(
    run: &RunRecord,
    expiration: &InputExpiration,
    operation: InputControlOperation,
) -> InputControlRequest {
    InputControlRequest {
        identity: ControlIdentity::from_run(run),
        expires_at_mono_ns: expiration.monotonic_ns(),
        operation,
    }
}

fn submission_context(run_id: i64, request: &SubmitInput) -> InputContext {
    InputContext {
        run_id: Some(run_id),
        operation_key: Some(request.operation_key.clone()),
        ..InputContext::default()
    }
}

fn validate_submission_request(run_id: i64, request: &SubmitInput) -> Result<()> {
    validate_operation_key(&request.operation_key)?;
    usage(
        run_id > 0 && crate::pane_lock::valid_input(&request.literal),
        "ordinary submission requires a positive run and one nonblank line without control characters",
    )?;
    usage(
        matches!(request.input_kind, InputKind::Prompt | InputKind::Exec),
        "ordinary input kind required",
    )
}

pub(crate) fn find_submission(
    conn: &mut Connection,
    identity: &PermissionReceiptIdentity,
    request: &SubmitInput,
    deadline: Deadline,
    clock: &dyn crate::time::Clock,
) -> std::result::Result<Option<InputOperation>, InputFailure> {
    let mut context = submission_context(identity.run_id, request);
    let result = (|| {
        validate_submission_request(identity.run_id, request)?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            let run = selected_run(&tx, identity.run_id)?;
            ensure(
                run.database_identity_json == identity.database_identity_json,
                "submission database identity mismatch",
            )?;
            let retained = storage::find_input(&tx, identity.run_id, &request.operation_key)?;
            if let Some(input) = &retained {
                context.input_id = Some(input.input_id);
                let digest = storage::input_payload_digest(identity.run_id, request);
                ensure(
                    input.kind == request.input_kind && input.payload_digest == digest,
                    "conflicting input operation key reuse",
                )?;
            }
            deadline.remaining_at(clock.now())?;
            tx.commit()?;
            Ok(retained)
        })
    })();
    result.map_err(|source| context.failure(source))
}

/// The outcome a durable input already determines; a `Ready` input needs live revalidation.
pub(super) fn durable_outcome(
    input: &InputOperation,
) -> Option<(SubmissionStatus, SubmissionStage, Option<String>)> {
    match (input.state, input.disposition) {
        (InputState::Ready, _) if input.deferred_at.is_some() => Some((
            SubmissionStatus::Queued,
            SubmissionStage::ZeroBytes,
            input.defer_reason.clone(),
        )),
        (InputState::Ready, _) => None,
        (InputState::Accepted, _) | (InputState::Settled, Some(InputDisposition::Submitted)) => {
            Some((SubmissionStatus::Accepted, SubmissionStage::Submitted, None))
        }
        (InputState::InProgress | InputState::Uncertain, _) => Some((
            SubmissionStatus::Uncertain,
            SubmissionStage::Unknown,
            uncertain_reason(input),
        )),
        (InputState::Settled, _) => Some((
            SubmissionStatus::Rejected,
            SubmissionStage::ZeroBytes,
            Some("input operation key is consumed".into()),
        )),
    }
}

fn uncertain_reason(input: &InputOperation) -> Option<String> {
    input.stop_cause.map(|cause| cause.as_str().to_owned())
}

fn historical_submission(input: &InputOperation) -> Option<SubmissionResult> {
    let (status, stage, reason) = durable_outcome(input)?;
    Some(SubmissionResult {
        status,
        input_id: Some(input.input_id),
        stage,
        output_seq: None,
        reason,
        delivery: Some(input.delivery()),
    })
}

fn submission_deadline(
    conn: &Connection,
    deadline: Deadline,
    clock: &dyn crate::time::Clock,
) -> Result<Deadline> {
    let deadline = Deadline::until(bounded_by_inherited(conn, deadline.instant()));
    deadline.remaining_at(clock.now())?;
    Ok(deadline)
}

fn same_association(
    view: &storage::RuntimeEventView,
    original: &storage::RuntimeEventView,
) -> bool {
    let interaction = |view: &storage::RuntimeEventView| {
        view.interaction
            .as_ref()
            .map(|i| (i.interaction_id, i.run_id, i.episode_seq, i.classification))
    };
    view.event == original.event && interaction(view) == interaction(original)
}

fn permission_placement(conn: &Connection, run: &RunRecord) -> Result<Placement> {
    crate::broker::get_member(conn, run.member_id, run.fleet_id)?
        .map(|member| member.placement)
        .ok_or_else(|| error("permission member placement unavailable"))
}

/// `messages` are the identity, contradiction, missing-durable and unmatched errors, in that order.
fn validate_reply_against_durable(
    reply: &SubmissionResult,
    durable: Option<&InputOperation>,
    messages: [&str; 4],
) -> Result<()> {
    let [identity, contradiction, missing, unmatched] = messages;
    match (reply.input_id, durable) {
        (Some(id), Some(input)) => {
            ensure(id == input.input_id, identity)?;
            let consistent = match reply.status {
                SubmissionStatus::Accepted => {
                    reply.stage == SubmissionStage::Submitted
                        && matches!(
                            (input.state, input.disposition),
                            (InputState::Accepted, _)
                                | (InputState::Settled, Some(InputDisposition::Submitted))
                        )
                }
                SubmissionStatus::Queued => {
                    reply.stage == SubmissionStage::ZeroBytes
                        && input.state == InputState::Ready
                        && input.deferred_at.is_some()
                }
                SubmissionStatus::Uncertain => {
                    matches!(input.state, InputState::InProgress | InputState::Uncertain)
                        && reply.reason == uncertain_reason(input)
                }
                SubmissionStatus::Held | SubmissionStatus::Rejected => {
                    reply.stage == SubmissionStage::ZeroBytes
                        && (input.state == InputState::Ready
                            || input.state == InputState::Settled
                                && input.disposition != Some(InputDisposition::Submitted))
                }
            };
            ensure(
                consistent && reply.delivery.as_ref() == Some(&input.delivery()),
                contradiction,
            )
        }
        (None, None) => ensure(
            matches!(
                reply.status,
                SubmissionStatus::Held | SubmissionStatus::Rejected
            ) && reply.stage == SubmissionStage::ZeroBytes,
            missing,
        ),
        _ => Err(error(unmatched)),
    }
}

pub(crate) fn validate_submission_target(
    conn: &Connection,
    settings: &Settings,
    expected: &RunRecord,
    placement: &Placement,
) -> Result<()> {
    validate_configured_database(
        conn,
        settings,
        "file-backed input database required",
        "submission connection and configured database differ",
    )?;
    let current = revalidate_run_snapshot(
        conn,
        expected,
        placement,
        SnapshotPolicy::Live,
        SnapshotMessages {
            run_changed: "submission run identity changed",
            placement_changed: "submission member placement changed",
        },
    )?;
    validate_run_paths(conn, &current)?;
    Ok(())
}

enum SubmissionPreparation<'a> {
    History(SubmissionResult),
    Locked {
        attempt: PreparedSubmission<'a>,
        held: PaneLock,
        deadline: Deadline,
    },
}

pub(crate) fn submit_wrapped_input(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    request: &SubmitInput,
    deadline: Deadline,
) -> std::result::Result<SubmissionResult, InputFailure> {
    submit_wrapped_input_with_host(
        conn,
        settings,
        expected_run,
        expected_placement,
        request,
        deadline,
        &mut NativeInputHost,
    )
}

fn submit_wrapped_input_with_host(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    request: &SubmitInput,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<SubmissionResult, InputFailure> {
    match prepare_submission_with_host(
        conn,
        settings,
        expected_run,
        expected_placement,
        request,
        deadline,
        host,
    )? {
        SubmissionPreparation::History(result) => Ok(result),
        SubmissionPreparation::Locked {
            attempt,
            held,
            deadline,
        } => submit_wrapped_input_under_pane_lock_with_host(
            conn,
            settings,
            expected_run,
            expected_placement,
            &held,
            &attempt,
            deadline,
            host,
        ),
    }
}

fn prepare_submission_with_host<'a>(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    request: &'a SubmitInput,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<SubmissionPreparation<'a>, InputFailure> {
    let clock = host.clock();
    let identity = PermissionReceiptIdentity::for_run(expected_run);
    let retained = find_submission(conn, &identity, request, deadline, clock.as_ref())?;
    if let Some(result) = retained.as_ref().and_then(historical_submission) {
        return Ok(SubmissionPreparation::History(result));
    }
    let mut context = submission_context(expected_run.run_id, request);
    context.input_id = retained.as_ref().map(|input| input.input_id);
    (|| {
        let deadline = submission_deadline(conn, deadline, clock.as_ref())?;
        with_database_budget(conn, deadline, |conn| {
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
        let attempt = PreparedSubmission::new(request, deadline.instant(), clock.as_ref())?;
        let held = host
            .lock(settings, expected_placement, deadline)?
            .ok_or_else(|| error("pane input remained busy before submission"))?;
        deadline.remaining_at(clock.now())?;
        Ok(SubmissionPreparation::Locked {
            attempt,
            held,
            deadline,
        })
    })()
    .map_err(|source| context.failure(source))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the approved borrowed submission entry adds its host to the same fixed arguments"
)]
pub(crate) fn submit_wrapped_input_under_pane_lock_with_host(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    held: &PaneLock,
    attempt: &PreparedSubmission<'_>,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<SubmissionResult, InputFailure> {
    let clock = host.clock();
    let deadline = submission_deadline(
        conn,
        Deadline::until(deadline.instant().min(attempt.deadline)),
        clock.as_ref(),
    )
    .map_err(|source| submission_context(expected_run.run_id, attempt.request).failure(source))?;
    let identity = PermissionReceiptIdentity::for_run(expected_run);
    let retained = find_submission(conn, &identity, attempt.request, deadline, clock.as_ref())?;
    if let Some(result) = retained.as_ref().and_then(historical_submission) {
        return Ok(result);
    }
    let mut context = submission_context(expected_run.run_id, attempt.request);
    context.input_id = retained.as_ref().map(|input| input.input_id);
    let result = (|| {
        deadline.remaining_at(clock.now())?;
        held.validate_binding(&settings.database_url, expected_placement)?;
        with_database_budget(conn, deadline, |conn| {
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
        host.health(expected_run, deadline.instant())?;
        deadline.remaining_at(clock.now())?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(&tx, settings, expected_run, expected_placement)?;
            let changed = tx.execute(
                "UPDATE member_runs SET last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3",
                rusqlite::params![
                    clock.wall_stamp(),
                    expected_run.run_id,
                    expected_run.owner_epoch
                ],
            )?;
            ensure(changed == 1, "submission health ownership changed")?;
            deadline.remaining_at(clock.now())?;
            tx.commit().map_err(crate::error::CafleetError::from)
        })?;
        let input = attempt.request.clone();
        let operation = match input.input_kind {
            InputKind::Prompt => InputControlOperation::SubmitPrompt(input),
            InputKind::Exec => InputControlOperation::SubmitExec(input),
            InputKind::Preview | InputKind::Answer => {
                unreachable!("validated ordinary input kind")
            }
        };
        let request = control_request(expected_run, &attempt.expiration, operation);
        held.validate_binding(&settings.database_url, expected_placement)?;
        deadline.remaining_at(clock.now())?;
        held.release()?;
        let reply = host.exchange(expected_run, &request, deadline.instant())?;
        let InputControlReply::Submission(reply) = reply else {
            return Err(error("ordinary submission requires a submission reply"));
        };
        let recorded = find_submission(conn, &identity, attempt.request, deadline, clock.as_ref())?;
        context.input_id = recorded.as_ref().map(|input| input.input_id);
        let runner_owned = matches!(
            reply.status,
            SubmissionStatus::Accepted | SubmissionStatus::Queued
        );
        let bookkeeping = Deadline::until(
            deadline
                .instant()
                .max(clock.now() + std::time::Duration::from_secs(1)),
        );
        let reacquired = held.reacquire_within(bookkeeping)?;
        if !reacquired && !runner_owned {
            return Err(error(match context.input_id {
                Some(input_id) => format!(
                    "pane input lock remained busy after the runner reported {}; inspect the result with cafleet input show {input_id} --json",
                    reply.status.as_str()
                ),
                None => format!(
                    "pane input lock remained busy after the runner reported {}",
                    reply.status.as_str()
                ),
            }));
        }
        let validate = |conn: &mut Connection| {
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(conn, settings, expected_run, expected_placement)
        };
        if runner_owned {
            crate::db::complete_database_budget(conn, deadline, validate)?;
        } else {
            with_database_budget(conn, deadline, validate)?;
        }
        validate_reply_against_durable(
            &reply,
            recorded.as_ref(),
            [
                "submission reply input identity mismatch",
                "submission reply contradicts durable state",
                "submission outcome requires a durable input identity",
                "submission reply has no matching durable input",
            ],
        )?;
        if !runner_owned {
            deadline.remaining_at(clock.now())?;
        }
        Ok(reply)
    })();
    result.map_err(|source| context.failure(source))
}
fn validate_receipt_target(
    conn: &Connection,
    settings: &Settings,
    expected: &RunRecord,
    placement: &Placement,
) -> Result<()> {
    validate_receipt_database(conn, settings)?;
    let current = revalidate_run_snapshot(
        conn,
        expected,
        placement,
        SnapshotPolicy::HistoricalReceipt,
        SnapshotMessages {
            run_changed: "receipt run identity changed",
            placement_changed: "receipt member placement changed",
        },
    )?;
    validate_run_paths(conn, &current).map(|_| ())
}

pub(super) fn validate_receipt_database(conn: &Connection, settings: &Settings) -> Result<()> {
    validate_configured_database(
        conn,
        settings,
        "file-backed receipt database required",
        "receipt connection and configured database differ",
    )
}

fn validate_configured_database(
    conn: &Connection,
    settings: &Settings,
    memory_message: &str,
    mismatch_message: &str,
) -> Result<()> {
    let selected_path = conn.path().ok_or_else(|| error(memory_message))?;
    ensure(
        std::fs::canonicalize(selected_path).map_err(io_error)?
            == std::fs::canonicalize(crate::db::database_path(&settings.database_url)?)
                .map_err(io_error)?,
        mismatch_message,
    )
}

pub(crate) fn reconcile_input(
    conn: &mut Connection,
    settings: &Settings,
    input_id: i64,
    disposition: InputDisposition,
    evidence_ref: &str,
    deadline: Deadline,
) -> std::result::Result<InputOperation, InputFailure> {
    reconcile_receipt_with_host(
        conn,
        settings,
        input_id,
        disposition,
        evidence_ref,
        deadline,
        &mut NativeInputHost,
    )
}

fn reconcile_receipt_with_host(
    conn: &mut Connection,
    settings: &Settings,
    input_id: i64,
    disposition: InputDisposition,
    evidence_ref: &str,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<InputOperation, InputFailure> {
    let clock = host.clock();
    let mut context = InputContext {
        input_id: (input_id > 0).then_some(input_id),
        ..InputContext::default()
    };
    let result = (|| {
        let deadline = submission_deadline(conn, deadline, clock.as_ref())?;
        usage(input_id > 0, "positive reconciliation input ID required")?;
        usage(
            !evidence_ref.trim().is_empty(),
            "nonempty reconciliation evidence required",
        )?;
        let (original, run, placement) = with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            validate_receipt_database(&tx, settings)?;
            let input = storage::get_input(&tx, input_id)?;
            context.run_id = Some(input.run_id);
            context.operation_key = Some(input.operation_key.clone());
            let recorded_run = storage::get_run(&tx, input.run_id)?;
            let run = storage::validate_owner(&tx, &recorded_run.owner())?;
            let placement = if input.state == InputState::Settled {
                ensure(
                    input.disposition == Some(disposition),
                    "settled input disposition changed",
                )?;
                None
            } else {
                crate::runtime::input_client::receipt_placement(&tx, &run)?
            };
            deadline.remaining_at(clock.now())?;
            tx.commit()?;
            Ok((input, run, placement))
        })?;
        if original.state == InputState::Settled {
            return Ok(original);
        }
        let placement = placement.ok_or_else(|| {
            error("receipt reconciliation requires the member's live runner; the input stays uncertain")
        })?;
        let expiration = InputExpiration::capture(deadline.instant(), clock.as_ref())?;
        let held = host
            .lock(settings, &placement, deadline)?
            .ok_or_else(|| error("receipt pane input remained busy"))?;
        let read_target = |conn: &mut Connection| {
            with_database_budget(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                held.validate_binding(&settings.database_url, &placement)?;
                validate_receipt_target(&tx, settings, &run, &placement)?;
                let input = storage::get_input(&tx, input_id)?;
                ensure(
                    input.same_identity(&original),
                    "receipt immutable input identity changed",
                )?;
                deadline.remaining_at(clock.now())?;
                tx.commit()?;
                Ok(input)
            })
        };
        read_target(conn)?;
        host.health(&run, deadline.instant())?;
        deadline.remaining_at(clock.now())?;
        read_target(conn)?;
        let request =
            |action| control_request(&run, &expiration, InputControlOperation::Receipt(action));
        let query = request(ReceiptAction::Query {
            operation_key: original.operation_key.clone(),
        });
        let reply = host.exchange(&run, &query, deadline.instant())?;
        ensure(
            matches!(reply, InputControlReply::Receipt(ReceiptReply::Present { input_id: actual, .. })
                if actual == input_id),
            "receipt query has no matching durable input identity",
        )?;
        deadline.remaining_at(clock.now())?;
        read_target(conn)?;
        let reconcile = request(ReceiptAction::Reconcile {
            input_id,
            expected: original.state,
            disposition,
            evidence_ref: evidence_ref.into(),
        });
        host.exchange(&run, &reconcile, deadline.instant())?;
        let settled = read_target(conn)?;
        ensure(
            settled.state == InputState::Settled
                && settled.disposition == Some(disposition)
                && settled.evidence_ref.as_deref() == Some(evidence_ref),
            "receipt settlement acknowledgment has no matching durable settlement",
        )?;
        deadline.remaining_at(clock.now())?;
        let retirement = request(ReceiptAction::Retire { input_id });
        host.exchange(&run, &retirement, deadline.instant())?;
        deadline.remaining_at(clock.now())?;
        let recorded = read_target(conn)?;
        ensure(recorded == settled, "receipt changed after retirement")?;
        Ok(recorded)
    })();
    result.map_err(|source| context.failure(source))
}

fn usage(ok: bool, description: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(CafleetError::Usage(description.into()))
    }
}
pub(crate) fn validate_evidence_reference(value: &str) -> Result<()> {
    usage(
        !value.trim().is_empty()
            && value.len() <= 2048
            && !value.chars().any(|ch| ch <= '\u{1f}' || ch == '\u{7f}'),
        "a nonblank evidence reference of 1-2048 UTF-8 bytes without control characters is required",
    )
}
pub(crate) fn validate_request_token(value: &str) -> Result<()> {
    usage(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "a lowercase 64-hex request token is required",
    )
}
pub(crate) fn validate_operation_key(value: &str) -> Result<()> {
    usage(
        !value.trim().is_empty(),
        "a nonblank operation key is required",
    )
}
pub(super) fn validate_respond(input: &PermissionRespondInput) -> Result<()> {
    usage(
        [
            input.interaction_id,
            input.event_id,
            input.director_member_id,
            input.expected_run_id,
            input.expected_owner_epoch,
        ]
        .iter()
        .all(|id| *id > 0),
        "positive permission request IDs and owner epoch are required",
    )?;
    validate_request_token(&input.request_token)
}
fn selected_run(conn: &Connection, run_id: i64) -> Result<RunRecord> {
    selected_run_reporting(conn, run_id, "member_runs.database_identity_json")
}
fn selected_run_reporting(conn: &Connection, run_id: i64, field: &str) -> Result<RunRecord> {
    let identity = storage::database_identity(conn)?;
    let run = storage::get_run(conn, run_id)?;
    if identity != run.database_identity_json {
        return Err(CafleetError::InvalidStoredValue {
            field: field.into(),
            value: "selected database identity mismatch".into(),
        });
    }
    Ok(run)
}
fn association(
    conn: &Connection,
    run: &RunRecord,
    interaction_id: i64,
    event_id: i64,
    director: i64,
) -> Result<()> {
    let view = storage::get_event(conn, event_id)?;
    ensure(
        view.event.run_id == run.run_id
            && view.event.interaction_id == Some(interaction_id)
            && view.event.kind == RuntimeEventKind::InteractionOpened
            && view.event.notification_state == NotificationState::Inbox
            && director == run.director_member_id
            && view.interaction.as_ref().is_some_and(|value| {
                matches!(
                    value.classification,
                    InteractionClass::PermissionRequest | InteractionClass::Question
                )
            }),
        "permission run/event/interaction/Director association mismatch",
    )
}

fn response_context(input: &PermissionRespondInput) -> InputContext {
    InputContext {
        run_id: Some(input.expected_run_id),
        interaction_id: Some(input.interaction_id),
        input_id: None,
        operation_key: None,
    }
}

fn prepare_response(
    conn: &mut Connection,
    input: &PermissionRespondInput,
    deadline: Deadline,
    clock: &dyn crate::time::Clock,
) -> Result<(RunRecord, RuntimeEventView)> {
    with_database_budget(conn, deadline, |conn| {
        let tx = conn.transaction()?;
        let run = selected_run(&tx, input.expected_run_id)?;
        association(
            &tx,
            &run,
            input.interaction_id,
            input.event_id,
            input.director_member_id,
        )?;
        ensure(
            input.expected_owner_epoch == run.owner_epoch,
            "permission owner epoch changed",
        )?;
        let original =
            crate::runtime::input::inspection_association(&tx, &run, &input.inspection())?;
        deadline.remaining_at(clock.now())?;
        tx.commit()?;
        Ok((run, original))
    })
}

fn validate_response_identity(
    reply: &PermissionResponseResult,
    run: &RunRecord,
    input: &PermissionRespondInput,
) -> Result<()> {
    ensure(
        reply.member_id == run.member_id
            && reply.run_id == input.expected_run_id
            && reply.owner_epoch == input.expected_owner_epoch
            && reply.interaction_id == input.interaction_id
            && reply.event_id == input.event_id
            && reply.key == input.key
            && reply.submission.input_id.is_none(),
        "permission response reply identity mismatch",
    )
}

pub(crate) fn respond_permission_with_host(
    conn: &mut Connection,
    settings: &Settings,
    input: &PermissionRespondInput,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    let clock = host.clock();
    let context = response_context(input);
    let result = (|| {
        validate_respond(input)?;
        let deadline = submission_deadline(conn, deadline, clock.as_ref())?;
        let (run, original) = prepare_response(conn, input, deadline, clock.as_ref())?;
        let placement =
            with_database_budget(conn, deadline, |conn| permission_placement(conn, &run))?;
        let expiration = InputExpiration::capture(deadline.instant(), clock.as_ref())?;
        let held = host
            .lock(settings, &placement, deadline)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate = |conn: &mut Connection| -> Result<()> {
            deadline.remaining_at(clock.now())?;
            held.validate_binding(&settings.database_url, &placement)?;
            with_database_budget(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                validate_submission_target(&tx, settings, &run, &placement)?;
                let view =
                    crate::runtime::input::inspection_association(&tx, &run, &input.inspection())?;
                ensure(
                    same_association(&view, &original),
                    "permission response association changed",
                )?;
                deadline.remaining_at(clock.now())?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })
        };
        bracket_health(conn, host, &run, deadline.instant(), validate)?;
        let request = control_request(
            &run,
            &expiration,
            InputControlOperation::PermissionRespond(input.clone()),
        );
        let reply = host.exchange(&run, &request, deadline.instant())?;
        let InputControlReply::PermissionResponse(reply) = reply else {
            return Err(error("permission response requires its typed reply"));
        };
        validate_response_identity(&reply, &run, input)?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            held.validate_binding(&settings.database_url, &placement)?;
            validate_submission_target(&tx, settings, &run, &placement)?;
            association(
                &tx,
                &run,
                input.interaction_id,
                input.event_id,
                input.director_member_id,
            )?;
            deadline.remaining_at(clock.now())?;
            tx.commit().map_err(crate::error::CafleetError::from)
        })?;
        deadline.remaining_at(clock.now())?;
        Ok(reply)
    })();
    result.map_err(|source| context.failure(source))
}
pub(crate) fn inspect_permission(
    conn: &mut Connection,
    settings: &Settings,
    input: &PermissionInspectInput,
    deadline: Deadline,
) -> std::result::Result<PermissionInspection, InputFailure> {
    inspect_permission_with_host(conn, settings, input, deadline, &mut NativeInputHost)
}

fn inspect_permission_with_host(
    conn: &mut Connection,
    settings: &Settings,
    input: &PermissionInspectInput,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<PermissionInspection, InputFailure> {
    let clock = host.clock();
    let mut context = InputContext {
        interaction_id: Some(input.interaction_id),
        ..InputContext::default()
    };
    let result = (|| {
        usage(
            [
                input.interaction_id,
                input.event_id,
                input.director_member_id,
            ]
            .iter()
            .all(|id| *id > 0),
            "positive permission request IDs are required",
        )?;
        let deadline = submission_deadline(conn, deadline, clock.as_ref())?;
        let (run, placement, original) = with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            tx.query_row(
                "SELECT 1 FROM member_interactions WHERE interaction_id=?1",
                [input.interaction_id],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| {
                CafleetError::App(format!("Interaction {} not found", input.interaction_id))
            })?;
            let view = storage::get_event(&tx, input.event_id)?;
            let run = selected_run_reporting(
                &tx,
                view.event.run_id,
                "member_runs.database_identity_json",
            )?;
            context.run_id = Some(run.run_id);
            let placement = permission_placement(&tx, &run)?;
            validate_submission_target(&tx, settings, &run, &placement)?;
            let view = crate::runtime::input::inspection_association(&tx, &run, input)?;
            deadline.remaining_at(clock.now())?;
            tx.commit()?;
            Ok((run, placement, view))
        })?;
        let expiration = InputExpiration::capture(deadline.instant(), clock.as_ref())?;
        let held = host
            .lock(settings, &placement, deadline)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate = |conn: &mut Connection| -> Result<()> {
            deadline.remaining_at(clock.now())?;
            held.validate_binding(&settings.database_url, &placement)?;
            with_database_budget(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                validate_submission_target(&tx, settings, &run, &placement)?;
                let view = crate::runtime::input::inspection_association(&tx, &run, input)?;
                ensure(
                    same_association(&view, &original),
                    "permission inspection association changed",
                )?;
                deadline.remaining_at(clock.now())?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })
        };
        bracket_health(conn, host, &run, deadline.instant(), validate)?;
        let request = control_request(
            &run,
            &expiration,
            InputControlOperation::PermissionInspect(input.clone()),
        );
        let reply = host.exchange(&run, &request, deadline.instant())?;
        validate(conn)?;
        let InputControlReply::Inspection(reply) = reply else {
            return Err(error("permission inspection requires its typed reply"));
        };
        let interaction = original
            .interaction
            .as_ref()
            .expect("validated permission interaction");
        ensure(
            reply.message_id == original.event.message_id.expect("validated inbox message")
                && reply.episode_seq == interaction.episode_seq
                && reply.backend == run.backend
                && reply.native_version == run.native_version,
            "permission inspection reply identity mismatch",
        )?;
        ensure(
            reply.evidence["backend"].as_str() == Some(run.backend.as_str())
                && reply.evidence["native_version"].as_str() == run.native_version.as_deref()
                && reply.evidence["screen_seq"].as_u64() == Some(reply.output_seq),
            "permission inspection evidence identity mismatch",
        )?;
        let missing = crate::runtime::input::interaction_missing_evidence(
            interaction.classification,
            &reply.evidence,
        )?;
        ensure(
            reply.missing_evidence.iter().eq(missing.iter()),
            "permission inspection evidence completeness mismatch",
        )?;
        match reply.answer_capability {
            Coverage::Tested => ensure(
                run.native_version.is_some()
                    && reply.request_token.is_some()
                    && match interaction.classification {
                        InteractionClass::Question => {
                            reply.supported_decisions.is_empty()
                                && !reply.supported_choices.is_empty()
                        }
                        _ => {
                            !reply.supported_decisions.is_empty()
                                && reply.supported_choices.is_empty()
                        }
                    }
                    && reply.classification == Some(interaction.classification)
                    && reply.visibility == Visibility::Visible
                    && reply.capability_reason.is_none(),
                "permission inspection capability fields disagree",
            )?,
            Coverage::Unknown => ensure(
                reply.request_token.is_none()
                    && reply.supported_decisions.is_empty()
                    && reply.supported_choices.is_empty()
                    && reply
                        .capability_reason
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty()),
                "permission inspection unavailable fields disagree",
            )?,
        }
        if let Some(rule) = &reply.answer_rule_id {
            ensure(
                !rule.trim().is_empty() && rule.len() <= 256,
                "permission inspection answer rule is invalid",
            )?;
        }
        deadline.remaining_at(clock.now())?;
        Ok(*reply)
    })();
    result.map_err(|source| context.failure(source))
}

pub(super) enum SnapshotPolicy {
    Live,
    HistoricalReceipt,
}

pub(super) struct SnapshotMessages {
    pub(super) run_changed: &'static str,
    pub(super) placement_changed: &'static str,
}

pub(super) fn revalidate_run_snapshot(
    conn: &Connection,
    expected: &RunRecord,
    placement: &Placement,
    policy: SnapshotPolicy,
    messages: SnapshotMessages,
) -> Result<RunRecord> {
    let current = storage::validate_owner(conn, &expected.owner())?;
    match policy {
        SnapshotPolicy::Live => {
            ensure(
                stable_run_identity(&current) == stable_run_identity(expected)
                    && current.status == RunStatus::Running
                    && current.owner_kind == OwnerKind::Runner
                    && current.native_version == expected.native_version,
                messages.run_changed,
            )?;
            ensure(
                crate::broker::active_member_fleet(conn, expected.member_id)?
                    == Some(expected.fleet_id),
                "input member is inactive",
            )?;
            let member = crate::broker::get_member(conn, expected.member_id, expected.fleet_id)?
                .ok_or_else(|| error("input member is absent"))?;
            ensure(
                member.kind == crate::broker::MemberKind::Member && member.placement == *placement,
                messages.placement_changed,
            )?;
        }
        SnapshotPolicy::HistoricalReceipt => {
            ensure(
                stable_run_identity(&current) == stable_run_identity(expected),
                messages.run_changed,
            )?;
            ensure(
                receipt_placement(conn, &current)?.as_ref() == Some(placement),
                messages.placement_changed,
            )?;
        }
    }
    Ok(current)
}

pub(super) fn receipt_placement(conn: &Connection, run: &RunRecord) -> Result<Option<Placement>> {
    let (status, deleted): (MemberStatus, Option<String>) = conn
        .query_row(
            "SELECT m.status,f.deleted_at FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id WHERE m.member_id=?1 AND m.fleet_id=?2",
            params![run.member_id, run.fleet_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    let placement = crate::runtime::artifacts::placement_of(conn, run.member_id)?;
    ensure(
        placement.is_some() || status != MemberStatus::Active || deleted.is_some(),
        "active receipt member requires its recorded placement",
    )?;
    Ok(placement)
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::config::Settings;
    use crate::pane_lock::PaneLock;
    use crate::runtime::test_support::Fixture;
    use crate::runtime::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Instant;

    pub(crate) const NOW: &str = "2026-10-26T22:00:00.000000+00:00";

    pub(crate) struct Case {
        pub(crate) fixture: Fixture,
        pub(crate) owner: RunOwner,
        pub(crate) settings: Settings,
        pub(crate) _artifacts: RunArtifacts,
        pub(crate) _lock: Arc<RunLock>,
    }
    impl Case {
        pub(crate) fn new() -> Self {
            Self::from_fixture(Fixture::new())
        }
        pub(crate) fn from_fixture(mut fixture: Fixture) -> Self {
            let (artifacts, lock, owner) = fixture.running_owner();
            let lock = Arc::new(lock);
            assert_eq!(fixture.conn.execute(
                "UPDATE member_runs SET native_version='0.162.0',coverage='tested',started_at=?1 WHERE run_id=?2",
                rusqlite::params![NOW, fixture.run.run_id],
            ).unwrap(), 1);
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let evidence = crate::broker::runtime::metadata_evidence(&json!({
                    "backend":"codex","native_version":fixture.run.native_version,
                    "rule_id":"fixture-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false
                }))
                .unwrap();
            let transition = ObservedTransition {
                run_id: owner.run_id,
                event_seq: 1,
                episode_seq: Some(1),
                kind: RuntimeEventKind::InteractionOpened,
                classification: Some(InteractionClass::PermissionRequest),
                observed_at: NOW.into(),
                evidence_json: evidence,
                close_reason: None,
                cause_json: None,
            };
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live,
            )
            .unwrap();
            let settings =
                Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()));
            Self {
                fixture,
                owner,
                settings,
                _artifacts: artifacts,
                _lock: lock,
            }
        }
    }

    use crate::broker::Placement;

    use crate::time::Deadline;

    /// Refuses every live operation, recording the name of each one it was asked for.
    #[derive(Default)]
    pub(crate) struct NoLiveHost {
        pub(crate) calls: Vec<&'static str>,
    }

    impl NoLiveHost {
        fn refuse(&mut self, operation: &'static str) -> CafleetError {
            self.calls.push(operation);
            CafleetError::App(format!(
                "history fixture reached a forbidden live {operation}"
            ))
        }
    }

    impl InputCommandHost for NoLiveHost {
        fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
            std::sync::Arc::new(crate::time::SystemClock)
        }
        fn lock(&mut self, _: &Settings, _: &Placement, _: Deadline) -> Result<Option<PaneLock>> {
            Err(self.refuse("lock"))
        }

        fn health(&mut self, _: &RunRecord, _: Instant) -> Result<HealthSnapshot> {
            Err(self.refuse("health"))
        }

        fn exchange(
            &mut self,
            _: &RunRecord,
            _: &InputControlRequest,
            _: Instant,
        ) -> Result<InputControlReply> {
            Err(self.refuse("exchange"))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::test_support::record_read_budgets;
    use crate::runtime::input_client::test_support::NoLiveHost;
    use crate::runtime::input_client::*;
    use crate::runtime::service::test_support::ServiceFixture;
    use crate::runtime::service::test_support::input_service::input_service_barrier;
    use crate::runtime::service::test_support::input_service::input_service_composer;
    use crate::runtime::service::test_support::input_service::input_service_records;
    use crate::runtime::service::test_support::input_service::wait_input_service_record;
    use crate::runtime::service::test_support::input_service::with_input_service;
    use crate::runtime::test_support::Fixture;
    use std::io::Write;
    use std::net::Shutdown;
    use std::time::Duration;

    fn reconcile_with(
        conn: &mut Connection,
        settings: &Settings,
        input_id: i64,
        disposition: InputDisposition,
        evidence_ref: &str,
        deadline: Deadline,
        host: &mut impl InputCommandHost,
    ) -> std::result::Result<InputOperation, InputFailure> {
        reconcile_receipt_with_host(
            conn,
            settings,
            input_id,
            disposition,
            evidence_ref,
            deadline,
            host,
        )
    }

    /// The `(event_id, interaction_id, message_id)` of the run's opened interaction, polled
    /// until it is committed or `until` passes.
    fn opened_interaction(fixture: &mut ServiceFixture, until: Instant) -> (i64, i64, i64) {
        let run_id = fixture.fixture.run.run_id;
        loop {
            let row = with_database_budget(&mut fixture.fixture.conn, Deadline::until(until), |conn| {
                conn.query_row(
                    "SELECT event_id,interaction_id,message_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                    [run_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(crate::error::CafleetError::from)
            })
            .unwrap();
            if let Some(row) = row {
                return row;
            }
            assert!(
                Instant::now() < until,
                "the opened interaction commits before the original deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn input_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn notified_at(conn: &Connection, message_id: i64) -> Option<String> {
        conn.query_row(
            "SELECT notified_at FROM messages WHERE message_id=?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn inspection_barrier(fixture: &mut ServiceFixture, start: usize, barrier: &str) {
        native_answers_through_barrier(fixture, start, barrier, &[]);
    }
    fn native_answers_through_barrier(
        fixture: &mut ServiceFixture,
        start: usize,
        barrier: &str,
        prior: &[&str],
    ) {
        use std::io::Read;
        let until = Instant::now() + Duration::from_secs(2);
        let expected = format!("review-answer={barrier}");
        fixture
            .master
            .write_all(format!("{barrier}\r").as_bytes())
            .unwrap();
        loop {
            let interval = String::from_utf8_lossy(&fixture.output[start..]);
            if let Some(position) = interval.find(&expected)
                && interval[position..].contains('\n')
            {
                let answers = interval
                    .split_inclusive('\n')
                    .filter_map(|line| {
                        line.find("review-answer=")
                            .map(|position| line[position..].trim_end_matches(['\r', '\n']))
                    })
                    .collect::<Vec<_>>();
                let mut expected_answers = prior.to_vec();
                expected_answers.push(expected.as_str());
                assert_eq!(
                    answers, expected_answers,
                    "complete retained native answer interval: {interval:?}"
                );
                return;
            }
            assert!(
                Instant::now() < until,
                "complete manual barrier within its original two-second bound: {interval:?}"
            );
            let mut bytes = [0; 4096];
            match fixture.master.read(&mut bytes) {
                Ok(0) => panic!("owned outer PTY closed before complete inspection barrier"),
                Ok(count) => fixture.output.extend_from_slice(&bytes[..count]),
                Err(cause) if cause.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(cause) => panic!("owned inspection barrier read: {cause}"),
            }
            std::thread::sleep(
                Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
            );
        }
    }

    #[test]
    fn permission_inspection_actual_service_projects_current_unsupported_request_without_answering()
    {
        use crate::runtime::service::test_support::observation_services::synthetic;

        let mut fixture = ServiceFixture::configured(false, synthetic);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        fixture.master.write_all(b"form-inspection\r").unwrap();
        fixture.read_until("Operation: write form-inspection.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, message_id) = opened_interaction(&mut fixture, until);
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        let settings =
            Settings::for_database(std::path::Path::new(fixture.fixture.conn.path().unwrap()));
        let before = input_rows(&fixture.fixture.conn);
        let inspected = inspect_permission(
            &mut fixture.fixture.conn,
            &settings,
            &PermissionInspectInput {
                interaction_id,
                event_id,
                director_member_id: run.director_member_id,
            },
            Deadline::until(until),
        )
        .unwrap();
        assert_eq!(inspected.event_id, event_id);
        assert_eq!(inspected.interaction_id, interaction_id);
        assert_eq!(inspected.message_id, message_id);
        assert_eq!(inspected.run_id, run.run_id);
        assert_eq!(inspected.owner_epoch, run.owner_epoch);
        assert_eq!(inspected.member_id, run.member_id);
        assert_eq!(inspected.fleet_id, run.fleet_id);
        assert_eq!(inspected.director_member_id, run.director_member_id);
        assert_eq!(
            inspected.classification,
            Some(InteractionClass::PermissionRequest)
        );
        assert_eq!(inspected.visibility, Visibility::Visible);
        assert_eq!(inspected.answer_capability, Coverage::Unknown);
        assert!(inspected.capability_reason.is_some());
        assert!(inspected.supported_decisions.is_empty());
        assert!(inspected.request_token.is_none());
        assert_eq!(inspected.evidence["operation"], serde_json::Value::Null);
        assert_eq!(
            inspected
                .missing_evidence
                .iter()
                .copied()
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
        assert_eq!(input_rows(&fixture.fixture.conn), before);
        assert!(notified_at(&fixture.fixture.conn, message_id).is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-barrier");
        fixture.finish();
    }

    #[test]
    fn permission_inspection_reply_binds_backend_native_version_and_tested_capability_fields() {
        use crate::coding_agent::catalog::test_support::inspection_fixture_definition;
        use crate::runtime::service::test_support::observation_services::synthetic;

        let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
            synthetic(fixture, spec);
            overwrite_observation_catalog(fixture, inspection_fixture_definition());
        });
        fixture.start_running();
        let outer = Instant::now() + Duration::from_secs(5);
        fixture.master.write_all(b"form-reply-binding\r").unwrap();
        fixture.read_until("Operation: write form-reply-binding.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, _) = opened_interaction(&mut fixture, outer);
        let (settings, run, placement) = service_target(&fixture);
        let request = PermissionInspectInput {
            event_id,
            interaction_id,
            director_member_id: run.director_member_id,
        };
        let mut inspect = |tamper: Option<InspectionTamper>| {
            let mut host = SocketHost::new(&settings, &placement, outer);
            host.tamper_inspection = tamper;
            let result = inspect_permission_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &request,
                Deadline::until(outer),
                &mut host,
            );
            assert_eq!(host.calls, [1, 1, 1]);
            result
        };
        let live = inspect(None).unwrap();
        assert_eq!(live.answer_capability, Coverage::Tested);
        assert_eq!(live.backend, run.backend);
        assert_eq!(live.native_version, run.native_version);
        assert_eq!(
            live.evidence["native_version"],
            serde_json::json!(run.native_version)
        );
        assert!(live.answer_rule_id.is_none());
        let shared_rule = inspect(Some(Box::new(|reply| {
            reply.answer_rule_id = Some("inspection-shared-plan".into());
        })))
        .unwrap();
        assert_eq!(
            shared_rule.answer_rule_id.as_deref(),
            Some("inspection-shared-plan")
        );
        let tampers: Vec<(&str, InspectionTamper)> = vec![
            ("backend", Box::new(|reply| reply.backend = "claude".into())),
            (
                "native version",
                Box::new(|reply| reply.native_version = Some("0.0.2-synthetic".into())),
            ),
            (
                "absent native version",
                Box::new(|reply| reply.native_version = None),
            ),
            (
                "evidence backend",
                Box::new(|reply| reply.evidence["backend"] = serde_json::json!("claude")),
            ),
            (
                "evidence native version",
                Box::new(|reply| {
                    reply.evidence["native_version"] = serde_json::json!("0.0.2-synthetic")
                }),
            ),
            ("token", Box::new(|reply| reply.request_token = None)),
            (
                "decisions",
                Box::new(|reply| reply.supported_decisions = Vec::new()),
            ),
            (
                "classification",
                Box::new(|reply| reply.classification = None),
            ),
            (
                "visibility",
                Box::new(|reply| reply.visibility = Visibility::Obscured),
            ),
            (
                "reason",
                Box::new(|reply| reply.capability_reason = Some("unavailable".into())),
            ),
            (
                "blank rule",
                Box::new(|reply| reply.answer_rule_id = Some(" ".into())),
            ),
            (
                "oversized rule",
                Box::new(|reply| reply.answer_rule_id = Some("r".repeat(257))),
            ),
        ];
        for (label, tamper) in tampers {
            assert!(inspect(Some(tamper)).is_err(), "{label}");
        }
        let bounded = inspect(Some(Box::new(|reply| {
            reply.answer_rule_id = Some("r".repeat(256));
        })))
        .unwrap();
        assert_eq!(bounded.answer_rule_id.map(|rule| rule.len()), Some(256));
        inspection_barrier(&mut fixture, output_start, "reply-binding-barrier");
        fixture.finish();
    }

    #[test]
    fn permission_inspection_client_uses_real_guard_original_sql_bound_and_validates_live_reply() {
        use crate::coding_agent::catalog::test_support::inspection_fixture_definition;
        use crate::runtime::service::test_support::observation_services::synthetic;

        let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
            synthetic(fixture, spec);
            overwrite_observation_catalog(fixture, inspection_fixture_definition());
        });
        fixture.start_running();
        let outer = Instant::now() + Duration::from_secs(2);
        fixture
            .master
            .write_all(b"form-executable-inspection\r")
            .unwrap();
        fixture.read_until("Operation: write form-executable-inspection.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, message_id) = opened_interaction(&mut fixture, outer);
        let (settings, run, placement) = service_target(&fixture);
        let request = PermissionInspectInput {
            event_id,
            interaction_id,
            director_member_id: run.director_member_id,
        };
        let records = health_records(&fixture.fixture.conn);
        let mut expired = SocketHost::new(&settings, &placement, Instant::now());
        assert!(
            inspect_permission_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &request,
                Deadline::until(expired.until),
                &mut expired
            )
            .is_err()
        );
        assert_eq!(expired.calls, [0, 0, 0]);
        assert_eq!(health_records(&fixture.fixture.conn), records);

        let inherited = outer - Duration::from_millis(100);
        let mut host = SocketHost::new(&settings, &placement, inherited);
        let value = with_database_budget(
            &mut fixture.fixture.conn,
            Deadline::until(inherited),
            |conn| {
                inspect_permission_with_host(
                    conn,
                    &settings,
                    &request,
                    Deadline::until(outer),
                    &mut host,
                )
                .map_err(CafleetError::from)
            },
        )
        .unwrap();
        assert_eq!(host.calls, [1, 1, 1]);
        assert!(host.expiration.is_some());
        assert_eq!(
            (value.event_id, value.interaction_id, value.message_id),
            (event_id, interaction_id, message_id)
        );
        assert_eq!(value.answer_capability, Coverage::Tested);
        assert_eq!(
            value.supported_decisions,
            vec![PermissionDecision::Allow, PermissionDecision::Deny]
        );
        assert!(value.answer_rule_id.is_none());
        assert_eq!(value.request_token.as_ref().unwrap().len(), 64);
        assert!(value.evidence["operation"].is_null());
        assert!(
            value
                .missing_evidence
                .iter()
                .any(|v| matches!(v, MissingEvidence::ChoiceScope))
        );

        let original_sender: i64 = fixture
            .fixture
            .conn
            .query_row(
                "SELECT from_member_id FROM messages WHERE message_id=?1",
                [message_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original_sender, run.member_id);
        let replacement_sender = run.director_member_id;
        assert_ne!(
            replacement_sender, original_sender,
            "distinct valid fixture member identity"
        );
        let database = settings.database_url.clone();
        let mut changed = SocketHost::new(&settings, &placement, outer);
        changed.after_health = Some(Box::new(move || {
            let conn = crate::db::connect(&database).unwrap();
            let before: i64 = conn
                .query_row(
                    "SELECT from_member_id FROM messages WHERE message_id=?1",
                    [message_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(before, original_sender);
            assert_eq!(
                conn.execute(
                    "UPDATE messages SET from_member_id=?1 WHERE message_id=?2",
                    rusqlite::params![replacement_sender, message_id]
                )
                .unwrap(),
                1
            );
            let after: i64 = conn
                .query_row(
                    "SELECT from_member_id FROM messages WHERE message_id=?1",
                    [message_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_ne!(before, after);
            assert_eq!(after, replacement_sender);
        }));
        assert!(
            inspect_permission_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &request,
                Deadline::until(outer),
                &mut changed
            )
            .is_err()
        );
        assert_eq!(
            changed.calls,
            [1, 1, 0],
            "durable association is reread after actual health wait"
        );
        let actual_sender: i64 = fixture
            .fixture
            .conn
            .query_row(
                "SELECT from_member_id FROM messages WHERE message_id=?1",
                [message_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actual_sender, replacement_sender);
        assert_eq!(input_rows(&fixture.fixture.conn), 0);
        assert!(notified_at(&fixture.fixture.conn, message_id).is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-client-barrier");
        fixture.finish();
    }

    /// Replaces the catalog in the observation-services file that `synthetic` already wrote.
    fn overwrite_observation_catalog(fixture: &Fixture, catalog: serde_json::Value) {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(fixture._dir.path().join("observation-services.json"))
            .unwrap();
        file.write_all(&serde_json::to_vec(&serde_json::json!({"catalog":catalog})).unwrap())
            .unwrap();
        file.sync_all().unwrap();
    }

    struct History {
        fixture: Fixture,
        settings: Settings,
        placement: Placement,
        request: SubmitInput,
        row: InputOperation,
        _artifacts: RunArtifacts,
        _run_lock: RunLock,
    }

    enum ReplyMode {
        Live,
        UnknownId,
        Disconnected,
    }

    type InspectionTamper = Box<dyn FnMut(&mut PermissionInspection)>;

    struct SocketHost {
        settings: Settings,
        placement: Placement,
        until: Instant,
        expiration: Option<u64>,
        calls: [usize; 3],
        requests: Vec<Value>,
        reply: ReplyMode,
        after_health: Option<Box<dyn FnMut()>>,
        health_reply: Option<serde_json::Value>,
        exchange_limit: usize,
        expire_retirement: bool,
        wrong_query_id: Option<i64>,
        skip_reconcile: bool,
        tamper_inspection: Option<InspectionTamper>,
    }

    fn service_target(fixture: &ServiceFixture) -> (Settings, RunRecord, Placement) {
        let conn = &fixture.fixture.conn;
        let run = storage::get_run(conn, fixture.fixture.run.run_id).unwrap();
        let placement = member_placement(conn, &run);
        (
            Settings::for_database(std::path::Path::new(conn.path().unwrap())),
            run,
            placement,
        )
    }

    impl SocketHost {
        fn new(settings: &Settings, placement: &Placement, until: Instant) -> Self {
            Self {
                settings: settings.clone(),
                placement: placement.clone(),
                until,
                expiration: None,
                calls: [0; 3],
                requests: Vec::new(),
                reply: ReplyMode::Live,
                after_health: None,
                health_reply: None,
                exchange_limit: 1,
                expire_retirement: false,
                wrong_query_id: None,
                skip_reconcile: false,
                tamper_inspection: None,
            }
        }
    }

    impl InputCommandHost for SocketHost {
        fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
            std::sync::Arc::new(crate::time::SystemClock)
        }
        fn lock(
            &mut self,
            settings: &Settings,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, CafleetError> {
            self.calls[0] += 1;
            assert_eq!(deadline.instant(), self.until);
            PaneLock::acquire_within(&settings.database_url, placement, deadline)
        }

        fn health(
            &mut self,
            run: &RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, CafleetError> {
            self.calls[1] += 1;
            assert_eq!(deadline, self.until);
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none()
            );
            let health = match self.health_reply.take() {
                Some(reply) => HealthSnapshot::decode(reply, run)?,
                None => crate::runtime::probe_health(run, deadline, &crate::time::SystemClock)?,
            };
            if let Some(callback) = &mut self.after_health {
                callback();
            }
            Ok(health)
        }

        fn exchange(
            &mut self,
            run: &RunRecord,
            request: &InputControlRequest,
            deadline: Instant,
        ) -> std::result::Result<InputControlReply, CafleetError> {
            self.calls[2] += 1;
            assert!(
                self.calls[2] <= self.exchange_limit,
                "an explicit attempt uses only its declared exchanges"
            );
            assert_eq!(deadline, self.until);
            let client_holds_lock =
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none();
            if matches!(
                request.operation,
                InputControlOperation::SubmitPrompt(_) | InputControlOperation::SubmitExec(_)
            ) {
                assert!(
                    !client_holds_lock,
                    "a wrapped client awaits the runner's submission reply without the pane lock"
                );
            } else {
                assert!(
                    client_holds_lock,
                    "the actual guard spans every receipt exchange"
                );
            }
            assert_eq!(
                serde_json::to_value(&request.identity).unwrap(),
                serde_json::to_value(ControlIdentity::from_run(run)).unwrap()
            );
            if let Some(expiration) = self.expiration {
                assert_eq!(request.expires_at_mono_ns, expiration);
            } else {
                self.expiration = Some(request.expires_at_mono_ns);
            }
            self.requests.push(
                serde_json::from_slice(
                    &crate::runtime::control::encode_input_request(request).unwrap()[4..],
                )
                .unwrap(),
            );
            if self.skip_reconcile
                && let InputControlOperation::Receipt(ReceiptAction::Reconcile { input_id, .. }) =
                    request.operation
            {
                return Ok(InputControlReply::Receipt(ReceiptReply::Settled {
                    input_id,
                }));
            }
            if let InputControlOperation::Receipt(ReceiptAction::Retire { input_id }) =
                request.operation
            {
                let independent = crate::db::connect(&self.settings.database_url).unwrap();
                assert_eq!(
                    storage::get_input(&independent, input_id).unwrap().state,
                    InputState::Settled,
                    "Retire follows the committed durable reread"
                );
                if self.expire_retirement {
                    std::thread::sleep(
                        deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(1),
                    );
                    assert!(Instant::now() >= deadline);
                }
            }
            if matches!(self.reply, ReplyMode::UnknownId) {
                return Ok(InputControlReply::Submission(SubmissionResult {
                    status: SubmissionStatus::Accepted,
                    stage: SubmissionStage::Submitted,
                    input_id: Some(i64::MAX),
                    output_seq: Some(1),
                    reason: None,
                    delivery: Some(storage::DeliveryProjection {
                        status: storage::DeliveryStatus::Submitted,
                        method: Some(storage::DeliveryMethod::Profile),
                        reason: None,
                    }),
                }));
            }
            let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
            let mut client = crate::runtime::control::ControlClient::connect(
                &paths.control_socket,
                deadline,
                &crate::time::SystemClock,
            )?;
            if matches!(self.reply, ReplyMode::Disconnected) {
                client
                    .stream_mut()
                    .shutdown(Shutdown::Read)
                    .map_err(io_error)?;
                let bytes = crate::runtime::control::encode_input_request(request)?;
                let mut offset = 0;
                while offset < bytes.len() {
                    check_deadline(Instant::now(), deadline)?;
                    match client.stream_mut().write(&bytes[offset..]) {
                        Ok(0) => panic!("owned external request disconnected during send"),
                        Ok(count) => offset += count,
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            crate::runtime::control::pause(deadline, &crate::time::SystemClock)?;
                        }
                        Err(cause) => return Err(io_error(cause)),
                    }
                }
                let key = match &request.operation {
                    InputControlOperation::SubmitPrompt(input)
                    | InputControlOperation::SubmitExec(input) => &input.operation_key,
                    _ => panic!("disconnected fixture sends a durable input operation"),
                };
                let mut observed = crate::db::connect(&self.settings.database_url).unwrap();
                loop {
                    let retained =
                        with_database_budget(&mut observed, Deadline::until(deadline), |conn| {
                            storage::find_input(conn, run.run_id, key)
                        })?;
                    if retained.is_some_and(|input| input.state == InputState::Accepted) {
                        break;
                    }
                    crate::runtime::control::pause(deadline, &crate::time::SystemClock)?;
                }
                drop(client);
                return Err(CafleetError::App("owned reply consumer closed".into()));
            }
            let mut reply = client.exchange_input(request, deadline, &crate::time::SystemClock)?;
            if let Some(wrong) = self.wrong_query_id {
                let InputControlReply::Receipt(ReceiptReply::Present {
                    input_id,
                    submission,
                }) = &mut reply
                else {
                    panic!("owned Query returns a real Present receipt")
                };
                *input_id = wrong;
                submission.input_id = Some(wrong);
            }
            if let Some(tamper) = &mut self.tamper_inspection {
                let InputControlReply::Inspection(inspection) = &mut reply else {
                    panic!("a tampered exchange is a permission inspection")
                };
                tamper(inspection);
            }
            Ok(reply)
        }
    }

    fn ordinary(run: &RunRecord, key: &str) -> SubmitInput {
        SubmitInput {
            operation_key: format!("prompt:{}:{key}", run.run_id),
            input_kind: InputKind::Prompt,
            literal: format!("external {key} λ"),
        }
    }

    fn health_records(conn: &Connection) -> Vec<crate::db::test_support::TableSnapshot> {
        crate::db::test_support::table_snapshot(conn, |table| {
            matches!(
                table,
                "member_interactions" | "member_runtime_events" | "messages"
            )
        })
    }

    fn accepted_service_receipt(
        fixture: &mut ServiceFixture,
        outer: Instant,
        key: &str,
    ) -> (Settings, RunRecord, Placement, SubmitInput, InputOperation) {
        let (settings, run, placement) = service_target(fixture);
        let request = ordinary(&run, key);
        let result = submit_wrapped_input(
            &mut fixture.fixture.conn,
            &settings,
            &run,
            &placement,
            &request,
            Deadline::until(outer.min(Instant::now() + Duration::from_secs(5))),
        )
        .unwrap();
        assert_eq!(result.status, SubmissionStatus::Accepted);
        let row = storage::get_input(&fixture.fixture.conn, result.input_id.unwrap()).unwrap();
        assert_eq!(row.state, InputState::Accepted);
        wait_input_service_record(fixture, &request.literal, outer);
        input_service_barrier(fixture, outer);
        (settings, run, placement, request, row)
    }

    #[test]
    fn live_receipt_settled_history_is_immutable_before_live_calls_and_the_original_sql_deadline() {
        let mut case = History::new(InputState::Settled, Some(InputDisposition::Submitted));
        case.fixture
            .conn
            .execute(
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                [case.fixture.run.member_id],
            )
            .unwrap();
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let changes = case.fixture.conn.total_changes();
        let before_run = storage::get_run(&case.fixture.conn, case.row.run_id).unwrap();
        let records = health_records(&case.fixture.conn);
        let original = Instant::now() + Duration::from_secs(2);
        let (expired, expired_reads, result, history_reads) = {
            let mut reads = record_read_budgets(&mut case.fixture.conn, "member_input_operations");
            let expired = reconcile_with(
                &mut reads,
                &case.settings,
                case.row.input_id,
                InputDisposition::Submitted,
                "fixture:replacement-evidence",
                Deadline::until(Instant::now()),
                &mut NoLiveHost::default(),
            );
            let expired_reads = reads.take();
            let result = with_database_budget(&mut reads, Deadline::until(original), |conn| {
                Ok(reconcile_with(
                    conn,
                    &case.settings,
                    case.row.input_id,
                    InputDisposition::Submitted,
                    "fixture:replacement-evidence",
                    Deadline::until(original + Duration::from_secs(2)),
                    &mut NoLiveHost::default(),
                ))
            });
            (expired, expired_reads, result, reads.take())
        };
        let failure = expired.unwrap_err();
        assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
        assert!(expired_reads.is_empty());
        assert_eq!(
            result.unwrap().unwrap(),
            case.row,
            "same disposition preserves the actual original row and evidence"
        );
        assert!(!history_reads.is_empty());
        assert!(history_reads.iter().all(|deadline| *deadline <= original));
        assert!(
            reconcile_with(
                &mut case.fixture.conn,
                &case.settings,
                case.row.input_id,
                InputDisposition::AbandonedUncertain,
                "fixture:conflicting-disposition",
                Deadline::until(original),
                &mut NoLiveHost::default()
            )
            .is_err()
        );
        let foreign = Fixture::new();
        let foreign_settings = Settings {
            database_url: format!("sqlite:///{}", foreign.conn.path().unwrap()),
            database_url_explicit: true,
        };
        assert!(
            reconcile_with(
                &mut case.fixture.conn,
                &foreign_settings,
                case.row.input_id,
                InputDisposition::Submitted,
                "fixture:foreign-database",
                Deadline::until(original),
                &mut NoLiveHost::default()
            )
            .is_err()
        );
        case.assert_unchanged(changes);
        assert_eq!(
            storage::get_run(&case.fixture.conn, case.row.run_id).unwrap(),
            before_run
        );
        assert_eq!(health_records(&case.fixture.conn), records);
        held.validate_binding(&case.settings.database_url, &case.placement)
            .unwrap();

        for (input_id, evidence) in [(0, "fixture:invalid-id"), (case.row.input_id, " ")] {
            assert!(
                reconcile_with(
                    &mut case.fixture.conn,
                    &case.settings,
                    input_id,
                    InputDisposition::Submitted,
                    evidence,
                    Deadline::until(original),
                    &mut NoLiveHost::default()
                )
                .is_err()
            );
        }
        case.assert_unchanged(changes);
    }

    #[test]
    fn live_receipt_client_binds_query_and_commit_ack_before_one_real_retirement() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement, request, original) =
                accepted_service_receipt(fixture, outer, "receipt-live");
            let other = storage::test_support::reserve_input(
                &mut fixture.fixture.conn,
                &run.owner(),
                &NewInput {
                    operation_key: "prompt:other-live-receipt".into(),
                    kind: InputKind::Prompt,
                    payload_digest: "c".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            let until = outer.min(Instant::now() + Duration::from_secs(5));
            let mut wrong = SocketHost::new(&settings, &placement, until);
            wrong.wrong_query_id = Some(other.input_id);
            let failure = reconcile_with(
                &mut fixture.fixture.conn,
                &settings,
                original.input_id,
                InputDisposition::Submitted,
                "fixture:live-reconcile",
                Deadline::until(until),
                &mut wrong,
            )
            .unwrap_err();
            assert_eq!(wrong.calls, [1, 1, 1]);
            assert_eq!(failure.context.input_id, Some(original.input_id));
            assert_eq!(
                failure.context.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
                original
            );
            let mut unverified = SocketHost::new(&settings, &placement, until);
            unverified.exchange_limit = 2;
            unverified.skip_reconcile = true;
            assert!(
                reconcile_with(
                    &mut fixture.fixture.conn,
                    &settings,
                    original.input_id,
                    InputDisposition::Submitted,
                    "fixture:live-reconcile",
                    Deadline::until(until),
                    &mut unverified
                )
                .is_err(),
                "a typed Settled acknowledgment supplies no durable settlement"
            );
            assert_eq!(unverified.calls, [1, 1, 2]);
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
                original
            );
            let mut host = SocketHost::new(&settings, &placement, until);
            host.exchange_limit = 3;
            let (settled, observed) = {
                let mut reads =
                    record_read_budgets(&mut fixture.fixture.conn, "member_input_operations");
                let settled = with_database_budget(&mut reads, Deadline::until(until), |conn| {
                    Ok(reconcile_with(
                        conn,
                        &settings,
                        original.input_id,
                        InputDisposition::Submitted,
                        "fixture:live-reconcile",
                        Deadline::until(outer),
                        &mut host,
                    ))
                });
                (settled, reads.take())
            };
            let settled = settled.unwrap().unwrap();
            assert_eq!(settled.input_id, original.input_id);
            assert_eq!(settled.state, InputState::Settled);
            assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
            assert_eq!(
                settled.evidence_ref.as_deref(),
                Some("fixture:live-reconcile")
            );
            assert_eq!(host.calls, [1, 1, 3]);
            assert_eq!(
                host.requests
                    .iter()
                    .map(|value| value["action"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["query", "reconcile", "retire"]
            );
            assert!(
                host.requests
                    .iter()
                    .all(|value| value["kind"] == "receipt" && value.get("literal").is_none())
            );
            assert_eq!(host.requests[1]["expected_state"], "accepted");
            assert_eq!(host.requests[1]["evidence_ref"], "fixture:live-reconcile");
            assert_eq!(host.requests[2]["input_id"], original.input_id);
            assert!(
                !observed.is_empty(),
                "actual client receipt reads use the supplied connection"
            );
            assert!(observed.iter().all(|deadline| *deadline <= until));
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, other.input_id).unwrap(),
                other
            );
            let repeated = submit_wrapped_input_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(outer),
                &mut NoLiveHost::default(),
            )
            .unwrap();
            assert_eq!(repeated.input_id, Some(original.input_id));
            assert_eq!(repeated.status, SubmissionStatus::Accepted);
            input_service_barrier(fixture, outer);
            assert_eq!(
                input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == request.literal)
                    .count(),
                1
            );
        });
    }

    impl History {
        fn new(state: InputState, disposition: Option<InputDisposition>) -> Self {
            let mut fixture = Fixture::new();
            let (artifacts, run_lock, owner) = fixture.running_owner();
            let request = SubmitInput {
                operation_key: format!("prompt:{}:{}", owner.run_id, "b".repeat(64)),
                input_kind: InputKind::Prompt,
                literal: "retained ordinary λ literal".into(),
            };
            let row = storage::test_support::reserve_input(
                &mut fixture.conn,
                &owner,
                &NewInput {
                    operation_key: request.operation_key.clone(),
                    kind: request.input_kind,
                    payload_digest: crate::broker::runtime::input_payload_digest(
                        owner.run_id,
                        &request,
                    ),
                    message_id: None,
                },
            )
            .unwrap();
            if let Some(disposition) = disposition {
                storage::test_support::settle_input(
                    &mut fixture.conn,
                    &owner,
                    row.input_id,
                    InputState::InProgress,
                    disposition,
                    "fixture:external-history",
                )
                .unwrap();
            } else if state == InputState::Uncertain {
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    row.input_id,
                    storage::StopCause::WriteFailed,
                )
                .unwrap();
            } else if state != InputState::InProgress {
                storage::record_input_state(
                    &mut fixture.conn,
                    &owner,
                    row.input_id,
                    InputState::InProgress,
                    state,
                )
                .unwrap();
            }
            let row = storage::get_input(&fixture.conn, row.input_id).unwrap();
            assert_eq!(row.state, state);
            let placement = member_placement(&fixture.conn, &fixture.run);
            let settings =
                Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()));
            Self {
                fixture,
                settings,
                placement,
                request,
                row,
                _artifacts: artifacts,
                _run_lock: run_lock,
            }
        }

        fn identity(&self) -> PermissionReceiptIdentity {
            PermissionReceiptIdentity {
                database_identity_json: self.fixture.run.database_identity_json.clone(),
                run_id: self.fixture.run.run_id,
            }
        }

        fn assert_unchanged(&self, changes: u64) {
            assert_eq!(self.fixture.conn.total_changes(), changes);
            assert_eq!(
                storage::get_input(&self.fixture.conn, self.row.input_id).unwrap(),
                self.row
            );
        }
    }

    #[test]
    fn external_input_retained_history_precedes_busy_pane_and_inactive_owner() {
        for (state, disposition, status, stage) in [
            (
                InputState::Accepted,
                None,
                SubmissionStatus::Accepted,
                SubmissionStage::Submitted,
            ),
            (
                InputState::Uncertain,
                None,
                SubmissionStatus::Uncertain,
                SubmissionStage::Unknown,
            ),
            (
                InputState::Settled,
                Some(InputDisposition::AbandonedUncertain),
                SubmissionStatus::Rejected,
                SubmissionStage::ZeroBytes,
            ),
        ] {
            let mut case = History::new(state, disposition);
            case.fixture
                .conn
                .execute(
                    "UPDATE members SET status='deregistered' WHERE member_id=?1",
                    [case.fixture.run.member_id],
                )
                .unwrap();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            let changes = case.fixture.conn.total_changes();
            let identity = case.identity();
            assert_eq!(
                find_submission(
                    &mut case.fixture.conn,
                    &identity,
                    &case.request,
                    Deadline::until(Instant::now() + Duration::from_secs(1)),
                    &crate::time::SystemClock,
                )
                .unwrap(),
                Some(case.row.clone())
            );
            let result = submit_wrapped_input_with_host(
                &mut case.fixture.conn,
                &case.settings,
                &case.fixture.run,
                &case.placement,
                &case.request,
                Deadline::until(Instant::now() + Duration::from_millis(150)),
                &mut NoLiveHost::default(),
            )
            .unwrap();
            assert_eq!(result.input_id, Some(case.row.input_id));
            assert_eq!(result.status, status, "{state:?}/{disposition:?}");
            assert_eq!(result.stage, stage, "{state:?}/{disposition:?}");
            assert_eq!(result.output_seq, None);
            if status == SubmissionStatus::Rejected {
                assert!(
                    result
                        .reason
                        .as_ref()
                        .is_some_and(|reason| !reason.is_empty())
                );
            }
            case.assert_unchanged(changes);
            assert!(
                PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                    .unwrap()
                    .is_none()
            );
            drop(held);
        }
    }

    #[test]
    fn external_input_borrowed_history_keeps_the_prepared_deadline_before_first_sql() {
        let mut case = History::new(InputState::Accepted, None);
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let changes = case.fixture.conn.total_changes();
        let control_deadline = Instant::now() + Duration::from_secs(2);
        let run_before = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
        let records_before = health_records(&case.fixture.conn);
        let control =
            PreparedSubmission::new(&case.request, control_deadline, &crate::time::SystemClock)
                .unwrap();
        let result = submit_wrapped_input_under_pane_lock_with_host(
            &mut case.fixture.conn,
            &case.settings,
            &case.fixture.run,
            &case.placement,
            &held,
            &control,
            Deadline::until(control_deadline),
            &mut NoLiveHost::default(),
        )
        .unwrap();
        assert_eq!(result.status, SubmissionStatus::Accepted);
        assert_eq!(result.input_id, Some(case.row.input_id));

        let original = Instant::now() + Duration::from_millis(100);
        let expired_attempt =
            PreparedSubmission::new(&case.request, original, &crate::time::SystemClock).unwrap();
        std::thread::sleep(
            original.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        assert!(
            Instant::now() >= original,
            "the checked prepared attempt actually expired"
        );
        let original_active = Instant::now() + Duration::from_secs(1);
        let (expired, expired_reads, active, budgets) = {
            let mut reads = record_read_budgets(&mut case.fixture.conn, "member_input_operations");
            let expired = submit_wrapped_input_under_pane_lock_with_host(
                &mut reads,
                &case.settings,
                &case.fixture.run,
                &case.placement,
                &held,
                &expired_attempt,
                Deadline::until(Instant::now() + Duration::from_secs(3)),
                &mut NoLiveHost::default(),
            );
            let expired_reads = reads.take();
            let active_attempt =
                PreparedSubmission::new(&case.request, original_active, &crate::time::SystemClock)
                    .unwrap();
            let inherited = Deadline::until(Instant::now() + Duration::from_secs(2));
            let supplied = Deadline::until(Instant::now() + Duration::from_secs(3));
            let active = with_database_budget(&mut reads, inherited, |conn| {
                Ok(submit_wrapped_input_under_pane_lock_with_host(
                    conn,
                    &case.settings,
                    &case.fixture.run,
                    &case.placement,
                    &held,
                    &active_attempt,
                    supplied,
                    &mut NoLiveHost::default(),
                ))
            });
            (expired, expired_reads, active, reads.take())
        };
        case.assert_unchanged(changes);
        assert_eq!(
            storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap(),
            run_before
        );
        assert_eq!(health_records(&case.fixture.conn), records_before);
        assert!(
            PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .is_none()
        );
        let failure = expired
            .err()
            .expect("expired prepared history rejects before lookup");
        assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
        assert_eq!(failure.context.run_id, Some(case.fixture.run.run_id));
        assert_eq!(
            failure.context.operation_key.as_deref(),
            Some(case.request.operation_key.as_str())
        );
        assert_eq!(failure.context.input_id, None);
        assert!(
            expired_reads.is_empty(),
            "an expired original budget starts no receipt SQL"
        );
        let active = active.unwrap().unwrap();
        assert_eq!(active.status, SubmissionStatus::Accepted);
        assert_eq!(active.input_id, Some(case.row.input_id));
        assert!(
            !budgets.is_empty(),
            "the valid original budget reaches actual receipt SQL"
        );
        assert!(
            budgets.iter().all(|budget| *budget <= original_active),
            "first history SQL inherits the prepared minimum, not a later caller or outer budget"
        );
    }

    #[test]
    fn external_input_discovery_rejects_payload_association_and_database_conflicts() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;
        let mut case = History::new(InputState::Accepted, None);
        let changes = case.fixture.conn.total_changes();
        let identity = case.identity();
        let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
        assert_eq!(
            find_submission(
                &mut case.fixture.conn,
                &identity,
                &case.request,
                deadline,
                &crate::time::SystemClock
            )
            .unwrap(),
            Some(case.row.clone())
        );
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        case.fixture
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
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }))
            .unwrap();
        let denied = find_submission(
            &mut case.fixture.conn,
            &identity,
            &case.request,
            deadline,
            &crate::time::SystemClock,
        );
        let removed = case
            .fixture
            .conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        assert!(
            denied.is_err(),
            "supplied-connection read refusal is authoritative; cleanup={removed:?}"
        );
        removed.unwrap();
        assert!(
            reads.load(Ordering::SeqCst) > 0,
            "actual ordinary receipt SQL uses the caller connection"
        );
        case.assert_unchanged(changes);
        for change in ["literal", "kind"] {
            let mut request = SubmitInput {
                operation_key: case.request.operation_key.clone(),
                input_kind: case.request.input_kind,
                literal: case.request.literal.clone(),
            };
            match change {
                "literal" => request.literal.push('!'),
                "kind" => request.input_kind = InputKind::Exec,
                _ => unreachable!(),
            }
            let failure = find_submission(
                &mut case.fixture.conn,
                &identity,
                &request,
                deadline,
                &crate::time::SystemClock,
            )
            .expect_err("a reused key binds the original complete payload");
            assert!(!failure.source.to_string().contains(&request.literal));
            assert_eq!(failure.context.run_id, Some(case.fixture.run.run_id));
            assert_eq!(
                failure.context.operation_key.as_deref(),
                Some(case.request.operation_key.as_str())
            );
            case.assert_unchanged(changes);
            assert!(
                submit_wrapped_input_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    &case.fixture.run,
                    &case.placement,
                    &request,
                    deadline,
                    &mut NoLiveHost::default()
                )
                .is_err()
            );
        }
        let other = Fixture::new();
        let foreign = PermissionReceiptIdentity {
            database_identity_json: other.run.database_identity_json.clone(),
            run_id: identity.run_id,
        };
        assert!(
            find_submission(
                &mut case.fixture.conn,
                &foreign,
                &case.request,
                deadline,
                &crate::time::SystemClock
            )
            .is_err()
        );
        case.assert_unchanged(changes);
        let missing = SubmitInput {
            operation_key: "prompt:missing".into(),
            input_kind: InputKind::Prompt,
            literal: "missing input".into(),
        };
        assert!(
            find_submission(
                &mut case.fixture.conn,
                &identity,
                &missing,
                deadline,
                &crate::time::SystemClock
            )
            .unwrap()
            .is_none()
        );
        case.assert_unchanged(changes);
    }

    #[test]
    fn external_input_borrowed_lock_preserves_binding_expiration_and_actual_socket_submission() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "borrowed");
            let until = outer.min(Instant::now() + Duration::from_secs(10));
            let attempt =
                PreparedSubmission::new(&request, until, &crate::time::SystemClock).unwrap();
            let expiration = attempt.expiration.monotonic_ns();
            let held = PaneLock::try_acquire(&settings.database_url, &placement)
                .unwrap()
                .unwrap();
            let mut other_placement = placement.clone();
            other_placement.mux_pane_id = Some("%foreign".into());
            let wrong = PaneLock::try_acquire(&settings.database_url, &other_placement)
                .unwrap()
                .unwrap();
            let deadline = Deadline::until(until);
            assert!(
                submit_wrapped_input_under_pane_lock_with_host(
                    &mut fixture.fixture.conn,
                    &settings,
                    &run,
                    &placement,
                    &wrong,
                    &attempt,
                    deadline,
                    &mut NoLiveHost::default()
                )
                .is_err()
            );
            let foreign = Fixture::new();
            let foreign_settings = Settings {
                database_url: format!("sqlite:///{}", foreign.conn.path().unwrap()),
                database_url_explicit: true,
            };
            assert!(
                submit_wrapped_input_under_pane_lock_with_host(
                    &mut fixture.fixture.conn,
                    &foreign_settings,
                    &run,
                    &placement,
                    &held,
                    &attempt,
                    deadline,
                    &mut NoLiveHost::default()
                )
                .is_err()
            );
            assert!(
                storage::find_input(&fixture.fixture.conn, run.run_id, &request.operation_key)
                    .unwrap()
                    .is_none()
            );
            std::thread::sleep(Duration::from_millis(25));
            let mut host = SocketHost::new(&settings, &placement, until);
            host.expiration = Some(expiration);
            let result = submit_wrapped_input_under_pane_lock_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &held,
                &attempt,
                deadline,
                &mut host,
            )
            .unwrap();
            assert_eq!(host.calls, [0, 1, 1]);
            assert_eq!(host.requests[0]["literal"], request.literal);
            assert_eq!(host.requests[0]["kind"], "submit_prompt");
            assert_eq!(result.status, SubmissionStatus::Accepted);
            let row = storage::get_input(&fixture.fixture.conn, result.input_id.unwrap()).unwrap();
            assert_eq!(
                row.payload_digest,
                crate::broker::runtime::input_payload_digest(run.run_id, &request)
            );
            assert_eq!(row.state, InputState::Accepted);
            assert!(
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            drop(held);
            assert!(
                PaneLock::acquire_released(&settings.database_url, &placement)
                    .unwrap()
                    .is_some()
            );
            wait_input_service_record(fixture, &request.literal, outer);
            input_service_barrier(fixture, outer);
            assert_eq!(
                input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == request.literal)
                    .count(),
                1
            );
        });
    }

    /// Takes the pane lock as soon as the runner replies, so the client cannot reacquire it.
    struct LockTakingHost {
        inner: SocketHost,
        taken: Option<PaneLock>,
    }

    impl InputCommandHost for LockTakingHost {
        fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
            std::sync::Arc::new(crate::time::SystemClock)
        }
        fn lock(
            &mut self,
            settings: &Settings,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, CafleetError> {
            self.inner.lock(settings, placement, deadline)
        }

        fn health(
            &mut self,
            run: &RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, CafleetError> {
            self.inner.health(run, deadline)
        }

        fn exchange(
            &mut self,
            run: &RunRecord,
            request: &InputControlRequest,
            deadline: Instant,
        ) -> std::result::Result<InputControlReply, CafleetError> {
            let reply = self.inner.exchange(run, request, deadline)?;
            self.taken = PaneLock::acquire_within(
                &self.inner.settings.database_url,
                &self.inner.placement,
                Deadline::until(Instant::now() + Duration::from_secs(1)),
            )
            .unwrap();
            assert!(self.taken.is_some(), "the released pane lock is taken");
            Ok(reply)
        }
    }

    #[test]
    fn external_input_accepted_reply_succeeds_when_another_writer_takes_the_released_lock() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "lock-taken-after-reply");
            let until = outer.min(Instant::now() + Duration::from_secs(5));
            let attempt =
                PreparedSubmission::new(&request, until, &crate::time::SystemClock).unwrap();
            let held = PaneLock::try_acquire(&settings.database_url, &placement)
                .unwrap()
                .unwrap();
            let mut inner = SocketHost::new(&settings, &placement, until);
            inner.expiration = Some(attempt.expiration.monotonic_ns());
            let mut host = LockTakingHost { inner, taken: None };
            let result = submit_wrapped_input_under_pane_lock_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &held,
                &attempt,
                Deadline::until(until),
                &mut host,
            )
            .expect("a submission the runner accepted succeeds without the reacquired lock");
            let row =
                storage::find_input(&fixture.fixture.conn, run.run_id, &request.operation_key)
                    .unwrap()
                    .expect("the runner committed the submission receipt");
            assert_eq!(result.status, SubmissionStatus::Accepted);
            assert_eq!(result.input_id, Some(row.input_id));
            drop(host.taken.take());
            drop(held);
            wait_input_service_record(fixture, &request.literal, outer);
        });
    }

    #[test]
    fn external_input_full_run_and_placement_revalidate_before_effects_and_after_health_wait() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "identity");
            let until = outer.min(Instant::now() + Duration::from_secs(2));
            let deadline = Deadline::until(until);
            let attempt =
                PreparedSubmission::new(&request, until, &crate::time::SystemClock).unwrap();
            let held = PaneLock::try_acquire(&settings.database_url, &placement)
                .unwrap()
                .unwrap();
            let mut foreign_run = run.clone();
            foreign_run.director_member_id += 1;
            assert!(
                submit_wrapped_input_under_pane_lock_with_host(
                    &mut fixture.fixture.conn,
                    &settings,
                    &foreign_run,
                    &placement,
                    &held,
                    &attempt,
                    deadline,
                    &mut NoLiveHost::default()
                )
                .is_err()
            );
            let mut host = SocketHost::new(&settings, &placement, until);
            let database = settings.database_url.clone();
            let member_id = run.member_id;
            host.after_health = Some(Box::new(move || {
                assert_eq!(crate::db::connect(&database).unwrap().execute(
                    "UPDATE member_placements SET mux_session='foreign-session' WHERE member_id=?1",
                    [member_id]).unwrap(), 1);
            }));
            let outcome = submit_wrapped_input_under_pane_lock_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &held,
                &attempt,
                deadline,
                &mut host,
            );
            assert_eq!(
                fixture
                    .fixture
                    .conn
                    .execute(
                        "UPDATE member_placements SET mux_session=?1 WHERE member_id=?2",
                        rusqlite::params![placement.mux_session, run.member_id]
                    )
                    .unwrap(),
                1
            );
            assert!(outcome.is_err());
            assert_eq!(host.calls, [0, 1, 0]);
            assert!(
                storage::find_input(&fixture.fixture.conn, run.run_id, &request.operation_key)
                    .unwrap()
                    .is_none()
            );
            drop(held);
            input_service_barrier(fixture, outer);
            assert!(
                input_service_records(fixture)
                    .iter()
                    .all(|record| record["line"] != request.literal)
            );
        });
    }

    #[test]
    fn external_input_typed_success_requires_the_actual_supplied_connection_receipt() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;

        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1",
                    [run.run_id],
                )
                .unwrap();
            let before = storage::get_run(&fixture.fixture.conn, run.run_id).unwrap();
            let records = health_records(&fixture.fixture.conn);
            let update = Arc::new(AtomicBool::new(false));
            let observed = update.clone();
            fixture
                .fixture
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Update {
                            table_name: "member_runs",
                            ..
                        }
                    ) {
                        observed.store(true, Ordering::SeqCst);
                    }
                    Authorization::Allow
                }))
                .unwrap();
            let request = ordinary(&run, "unverified-reply");
            let until = outer.min(Instant::now() + Duration::from_secs(2));
            let mut host = SocketHost::new(&settings, &placement, until);
            host.reply = ReplyMode::UnknownId;
            let result = submit_wrapped_input_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(until),
                &mut host,
            );
            let removed = fixture
                .fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            assert!(
                result.is_err(),
                "unverified reply rejects; authorizer_cleanup={removed:?}"
            );
            removed.unwrap();
            let failure = result
                .err()
                .expect("a typed success alone proves no durable identity");
            assert_eq!(host.calls, [1, 1, 1]);
            assert_eq!(failure.context.run_id, Some(run.run_id));
            assert_eq!(
                failure.context.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
            assert!(!failure.source.to_string().contains(&request.literal));
            assert!(
                update.load(Ordering::SeqCst),
                "genuine successful health updates the supplied connection"
            );
            let refreshed = storage::get_run(&fixture.fixture.conn, run.run_id).unwrap();
            assert!(refreshed.last_health_at.is_some());
            assert_ne!(
                refreshed.last_health_at.as_deref(),
                Some("2000-01-01T00:00:00Z")
            );
            let mut expected = before;
            expected.last_health_at = refreshed.last_health_at.clone();
            assert_eq!(
                refreshed, expected,
                "health preserves lifecycle, coverage and event reservation"
            );
            assert_eq!(health_records(&fixture.fixture.conn), records);
            assert!(
                storage::find_input(&fixture.fixture.conn, run.run_id, &request.operation_key)
                    .unwrap()
                    .is_none()
            );
            input_service_barrier(fixture, outer);
            assert!(
                input_service_records(fixture)
                    .iter()
                    .all(|record| record["line"] != request.literal)
            );
        });
    }

    #[test]
    fn external_input_stale_or_unreachable_health_refuses_before_exchange_and_preserves_open_history()
     {
        for stale in [false, true] {
            let mut case = History::new(InputState::Ready, None);
            let run = case.fixture.run.clone();
            let evidence = crate::broker::runtime::metadata_evidence(&serde_json::json!({"backend":"codex","native_version":"0.163.0","rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false})).unwrap();
            storage::persist_transition(
                &mut case.fixture.conn,
                &run.owner(),
                &ObservedTransition {
                    run_id: run.run_id,
                    event_seq: 1,
                    episode_seq: Some(1),
                    kind: RuntimeEventKind::InteractionOpened,
                    classification: Some(InteractionClass::Question),
                    observed_at: "2026-10-27T16:44:00Z".into(),
                    evidence_json: evidence,
                    close_reason: None,
                    cause_json: None,
                },
                PersistenceMode::Live,
            )
            .unwrap();
            let before = storage::get_run(&case.fixture.conn, run.run_id).unwrap();
            let records = health_records(&case.fixture.conn);
            assert_eq!(
                records[0].rows.len(),
                1,
                "meaningful open interaction control"
            );
            let until = Instant::now() + Duration::from_secs(1);
            let mut host = SocketHost::new(&case.settings, &case.placement, until);
            if stale {
                let mut runner = ProcessIdentity::current().unwrap();
                runner.start_identity.push_str(":stale-health-reply");
                host.health_reply = Some(serde_json::json!({
                    "identity": ControlIdentity::from_run(&run),
                    "runner": runner,
                    "child": null,
                    "output_seq": 120,
                }));
            } else {
                assert!(
                    !RuntimePaths::for_run(&run.database_identity_json, run.run_id)
                        .unwrap()
                        .control_socket
                        .exists(),
                    "owned unreachable endpoint is absent"
                );
            }
            let result = submit_wrapped_input_with_host(
                &mut case.fixture.conn,
                &case.settings,
                &run,
                &case.placement,
                &case.request,
                Deadline::until(until),
                &mut host,
            );
            let Err(failure) = result else {
                panic!("stale or unreachable health refuses ordinary input")
            };
            assert_eq!(host.calls, [1, 1, 0]);
            assert_eq!(failure.context.run_id, Some(run.run_id));
            assert_eq!(
                failure.context.operation_key.as_deref(),
                Some(case.request.operation_key.as_str())
            );
            assert!(!failure.source.to_string().contains(&case.request.literal));
            let mut after = storage::get_run(&case.fixture.conn, run.run_id).unwrap();
            after.last_health_at = before.last_health_at.clone();
            assert_eq!(
                after, before,
                "refusal preserves lifecycle, owner, coverage and event reservation"
            );
            assert_eq!(health_records(&case.fixture.conn), records);
            assert_eq!(
                storage::get_input(&case.fixture.conn, case.row.input_id).unwrap(),
                case.row
            );
        }
    }

    #[test]
    fn external_input_ready_history_reaches_live_service_without_inventing_zero_byte_authority() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "ready-without-live-proof");
            let reserved = storage::test_support::reserve_input(
                &mut fixture.fixture.conn,
                &run.owner(),
                &NewInput {
                    operation_key: request.operation_key.clone(),
                    kind: request.input_kind,
                    payload_digest: crate::broker::runtime::input_payload_digest(
                        run.run_id, &request,
                    ),
                    message_id: None,
                },
            )
            .unwrap();
            let row = storage::record_input_state(
                &mut fixture.fixture.conn,
                &run.owner(),
                reserved.input_id,
                InputState::InProgress,
                InputState::Ready,
            )
            .unwrap();
            let until = outer.min(Instant::now() + Duration::from_secs(2));
            let mut host = SocketHost::new(&settings, &placement, until);
            let result = submit_wrapped_input_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(until),
                &mut host,
            )
            .unwrap();
            assert_eq!(host.calls, [1, 1, 1]);
            assert_eq!(result.status, SubmissionStatus::Held);
            assert_eq!(result.stage, SubmissionStage::ZeroBytes);
            assert_eq!(result.input_id, Some(row.input_id));
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
                row
            );
            input_service_barrier(fixture, outer);
            assert!(
                input_service_records(fixture)
                    .iter()
                    .all(|record| record["line"] != request.literal)
            );
        });
    }

    #[test]
    fn external_input_disconnected_reply_is_discoverable_and_matching_history_never_retypes() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "disconnected");
            let until = outer.min(Instant::now() + Duration::from_secs(2));
            let mut host = SocketHost::new(&settings, &placement, until);
            host.reply = ReplyMode::Disconnected;
            let failure = submit_wrapped_input_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(until),
                &mut host,
            )
            .err()
            .expect("the real disconnected consumer retains an error");
            assert_eq!(host.calls, [1, 1, 1]);
            assert_eq!(failure.context.run_id, Some(run.run_id));
            assert_eq!(
                failure.context.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
            assert!(
                matches!(&failure.source, CafleetError::App(cause) if cause == "owned reply consumer closed")
            );
            let presented: CafleetError = failure.into();
            let CafleetError::WithDiagnostics { cause, diagnostics } = presented else {
                panic!("a possible submission retains its original failure and context")
            };
            assert_eq!(cause.exit_code(), 1);
            assert!(
                matches!(*cause, CafleetError::App(ref message) if message == "owned reply consumer closed")
            );
            assert_eq!(
                diagnostics,
                vec![format!(
                    "run_id={} operation_key={}",
                    run.run_id,
                    serde_json::to_string(&request.operation_key).unwrap()
                )]
            );
            let identity = PermissionReceiptIdentity {
                database_identity_json: run.database_identity_json.clone(),
                run_id: run.run_id,
            };
            let row = find_submission(
                &mut fixture.fixture.conn,
                &identity,
                &request,
                Deadline::until(outer),
                &crate::time::SystemClock,
            )
            .unwrap()
            .expect("actual committed receipt survives reply loss");
            assert_eq!(row.state, InputState::Accepted);
            assert_eq!(
                row.payload_digest,
                crate::broker::runtime::input_payload_digest(run.run_id, &request)
            );
            let result = submit_wrapped_input_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(outer),
                &mut NoLiveHost::default(),
            )
            .unwrap();
            assert_eq!(result.status, SubmissionStatus::Accepted);
            assert_eq!(result.input_id, Some(row.input_id));
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
                row
            );
            wait_input_service_record(fixture, &request.literal, outer);
            input_service_barrier(fixture, outer);
            assert_eq!(
                input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == request.literal)
                    .count(),
                1
            );
            input_service_composer(fixture, outer);
        });
    }

    mod input_failure {
        use crate::runtime::input_client::tests::*;

        fn causes() -> [CafleetError; 2] {
            [
                CafleetError::Usage("fixture usage".into()),
                CafleetError::WithDiagnostics {
                    cause: Box::new(CafleetError::Usage("nested usage".into())),
                    diagnostics: vec!["original diagnostic".into()],
                },
            ]
        }

        #[test]
        fn input_failure_context_wraps_the_source_with_only_its_known_identities() {
            const KEY: &str = "fixture:key\nwith\"quotes\"\\tail";
            type Build = fn() -> InputContext;
            let cases: [(&str, Build, &[&str]); 3] = [
                ("no known identity", InputContext::default, &[]),
                (
                    "every identity",
                    || InputContext {
                        run_id: Some(29),
                        interaction_id: Some(17),
                        input_id: Some(31),
                        operation_key: Some(KEY.into()),
                    },
                    &["run_id", "interaction_id", "input_id", "operation_key"],
                ),
                (
                    "one identity",
                    || InputContext {
                        interaction_id: Some(17),
                        ..InputContext::default()
                    },
                    &["interaction_id"],
                ),
            ];
            for (name, context, known) in cases {
                for original in causes() {
                    let converted: CafleetError = context().failure(original.clone()).into();
                    assert_eq!(converted.exit_code(), original.exit_code(), "{name}");
                    if known.is_empty() {
                        assert_eq!(
                            std::mem::discriminant(&converted),
                            std::mem::discriminant(&original),
                            "{name}"
                        );
                        assert_eq!(converted.message(), original.message(), "{name}");
                        continue;
                    }
                    let CafleetError::WithDiagnostics { cause, diagnostics } = converted else {
                        panic!("{name}: known identities retain the source with context")
                    };
                    assert_eq!(
                        std::mem::discriminant(cause.as_ref()),
                        std::mem::discriminant(&original),
                        "{name}"
                    );
                    assert_eq!(cause.message(), original.message(), "{name}");
                    assert_eq!(diagnostics.len(), 1, "{name}");
                    let text = &diagnostics[0];
                    let positions: Vec<usize> = known
                        .iter()
                        .map(|field| text.find(field).expect("known context field"))
                        .collect();
                    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{name}");
                    for absent in ["run_id", "interaction_id", "input_id", "operation_key"]
                        .iter()
                        .filter(|field| !known.contains(field))
                    {
                        assert!(!text.contains(absent), "{name}: {absent}");
                    }
                    if known.contains(&"operation_key") {
                        assert!(
                            text.contains(&serde_json::to_string(KEY).unwrap()),
                            "{name}"
                        );
                    }
                    assert!(!text.contains('\n'), "{name}");
                }
            }
        }
    }

    mod permission_history {
        use crate::config::Settings;
        use crate::error::CafleetError;
        use crate::runtime::input_client::test_support::*;
        use crate::time::Deadline;
        use rusqlite::Connection;
        use std::time::Duration;
        use std::time::Instant;

        mod inspect_errors {
            use crate::runtime::PermissionInspectInput;
            use crate::runtime::input_client::tests::permission_history::*;
            use crate::runtime::inspect_permission;

            fn opened(case: &Case) -> (i64, i64) {
                case.fixture
                .conn
                .query_row(
                    "SELECT event_id,interaction_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                    [case.fixture.run.run_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
            }

            fn copied(case: &Case) -> (Settings, Connection) {
                let path = case.fixture._dir.path().join("copied-inspection.db");
                std::fs::copy(case.fixture.conn.path().unwrap(), &path).unwrap();
                let settings = Settings::for_database(&path);
                let conn = crate::db::connect(&settings.database_url).unwrap();
                (settings, conn)
            }

            fn inspect(
                conn: &mut Connection,
                settings: &Settings,
                event_id: i64,
                interaction_id: i64,
                director_member_id: i64,
            ) -> CafleetError {
                match inspect_permission(
                    conn,
                    settings,
                    &PermissionInspectInput {
                        event_id,
                        interaction_id,
                        director_member_id,
                    },
                    Deadline::until(Instant::now() + Duration::from_secs(1)),
                ) {
                    Err(failure) => failure.source,
                    Ok(_) => panic!("this inspection is refused"),
                }
            }

            #[test]
            fn permission_inspect_reports_a_missing_interaction_before_event_run_or_identity_checks()
             {
                let case = Case::new();
                let (event_id, interaction_id) = opened(&case);
                let director = case.fixture.run.director_member_id;
                let missing = interaction_id + 1000;
                let (settings, mut conn) = copied(&case);
                for (event, label) in [
                    (event_id, "an existing event"),
                    (event_id + 1000, "a missing event"),
                ] {
                    let error = inspect(&mut conn, &settings, event, missing, director);
                    assert_eq!(
                        error.message(),
                        format!("Interaction {missing} not found"),
                        "{label} with a foreign database identity"
                    );
                    assert_eq!(error.exit_code(), 1);
                }
            }

            #[test]
            fn permission_inspect_names_the_run_database_identity_field_on_a_mismatch() {
                let case = Case::new();
                let (event_id, interaction_id) = opened(&case);
                let (settings, mut conn) = copied(&case);
                let error = inspect(
                    &mut conn,
                    &settings,
                    event_id,
                    interaction_id,
                    case.fixture.run.director_member_id,
                );
                assert!(
                    matches!(&error, CafleetError::InvalidStoredValue { field, value }
                    if field == "member_runs.database_identity_json"
                        && value == "selected database identity mismatch"),
                    "{error:?}"
                );
                assert_eq!(
                    error.message(),
                    "invalid stored value for member_runs.database_identity_json: selected database identity mismatch"
                );
                assert_eq!(error.exit_code(), 1);
            }
        }
    }

    mod uncertain_reason {
        use crate::broker::runtime::test_support::Fixture as StorageFixture;
        use crate::runtime::input_client::tests::*;

        const MESSAGES: [&str; 4] = ["identity", "contradiction", "missing", "unmatched"];

        fn uncertain(cause: Option<storage::StopCause>) -> storage::InputOperation {
            let mut fixture = StorageFixture::new();
            let owner = fixture.activate();
            let input = fixture.input(&owner, "prompt:reason", InputKind::Prompt, None);
            match cause {
                Some(cause) => storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    cause,
                )
                .unwrap(),
                None => input,
            }
        }

        fn reply(input: &storage::InputOperation, reason: Option<&str>) -> SubmissionResult {
            SubmissionResult {
                status: SubmissionStatus::Uncertain,
                input_id: Some(input.input_id),
                stage: SubmissionStage::Unknown,
                output_seq: None,
                reason: reason.map(str::to_owned),
                delivery: Some(input.delivery()),
            }
        }

        #[test]
        fn durable_outcome_reports_the_recorded_cause_as_the_uncertain_reason() {
            const CASES: &[(&str, Option<storage::StopCause>)] = &[
                ("a recorded cause", Some(storage::StopCause::ShortWrite)),
                ("a causeless in-progress receipt", None),
            ];
            for &(name, cause) in CASES {
                let input = uncertain(cause);
                if cause.is_none() {
                    assert_eq!(input.state, InputState::InProgress, "{name}");
                }
                assert_eq!(
                    durable_outcome(&input),
                    Some((
                        SubmissionStatus::Uncertain,
                        SubmissionStage::Unknown,
                        cause.map(|cause| cause.as_str().to_owned())
                    )),
                    "{name}"
                );
            }
        }

        #[test]
        fn uncertain_reply_must_carry_the_receipt_cause() {
            let input = uncertain(Some(storage::StopCause::ShortWrite));
            validate_reply_against_durable(
                &reply(&input, Some("short_write")),
                Some(&input),
                MESSAGES,
            )
            .unwrap();
            for wrong in [None, Some("write_failed"), Some("short write")] {
                let error =
                    validate_reply_against_durable(&reply(&input, wrong), Some(&input), MESSAGES)
                        .unwrap_err();
                assert!(
                    error.message().contains("contradiction"),
                    "{wrong:?}: {}",
                    error.message()
                );
            }
        }

        #[test]
        fn uncertain_reply_for_a_causeless_receipt_carries_no_reason() {
            let input = uncertain(None);
            validate_reply_against_durable(&reply(&input, None), Some(&input), MESSAGES).unwrap();
            let error = validate_reply_against_durable(
                &reply(&input, Some("manual_input")),
                Some(&input),
                MESSAGES,
            )
            .unwrap_err();
            assert!(
                error.message().contains("contradiction"),
                "{}",
                error.message()
            );
        }
    }

    use crate::broker::Placement;
    use crate::broker::runtime as storage;
    use crate::config::Settings;
    use crate::db::with_database_budget;
    use crate::pane_lock::PaneLock;

    use crate::time::Deadline;
    use rusqlite::Connection;

    use std::time::Instant;

    fn member_placement(conn: &Connection, run: &RunRecord) -> Placement {
        crate::broker::get_member(conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
    }

    #[test]
    fn absent_placement_receipt_stays_uncertain_without_live_calls() {
        use crate::runtime::input_client::test_support::Case;

        let mut case = Case::new();
        let input_id = storage::test_support::reserve_input(
            &mut case.fixture.conn,
            &case.owner,
            &storage::NewInput {
                operation_key: "prompt:absent-placement".into(),
                kind: InputKind::Prompt,
                payload_digest: "d".repeat(64),
                message_id: None,
            },
        )
        .unwrap()
        .input_id;
        storage::record_input_state(
            &mut case.fixture.conn,
            &case.owner,
            input_id,
            InputState::InProgress,
            InputState::Accepted,
        )
        .unwrap();
        let member_id = case.fixture.run.member_id;
        case.fixture
            .conn
            .execute(
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                [member_id],
            )
            .unwrap();
        case.fixture
            .conn
            .execute(
                "DELETE FROM member_placements WHERE member_id=?1",
                [member_id],
            )
            .unwrap();
        let original = storage::get_input(&case.fixture.conn, input_id).unwrap();
        let mut host = NoLiveHost::default();
        let failure = reconcile_with(
            &mut case.fixture.conn,
            &case.settings,
            input_id,
            InputDisposition::Submitted,
            "fixture:absent-placement-receipt",
            Deadline::until(Instant::now() + Duration::from_secs(2)),
            &mut host,
        )
        .unwrap_err();
        assert_eq!(
            failure.source.message(),
            "receipt reconciliation requires the member's live runner; the input stays uncertain"
        );
        assert!(host.calls.is_empty());
        assert_eq!(
            storage::get_input(&case.fixture.conn, input_id).unwrap(),
            original
        );
    }
}
