use super::*;
use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, *};
use crate::config::Settings;
use crate::input::{Deadline, PaneLock, bounded_by_inherited, with_database_budget};
use rusqlite::{Connection, OptionalExtension};
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
        clock: &mut impl InputDeadlineClock,
    ) -> Result<Self> {
        Ok(Self {
            request,
            expiration: InputExpiration::capture(deadline, clock)?,
            deadline,
        })
    }

    pub(crate) fn retirement_request(
        &self,
        run: &RunRecord,
        input_id: i64,
    ) -> Result<InputControlRequest> {
        ensure(input_id > 0, "positive retirement input ID required")?;
        Ok(InputControlRequest {
            identity: ControlIdentity::from_run(run),
            expires_at_mono_ns: self.expiration.monotonic_ns(),
            operation: InputControlOperation::Receipt(ReceiptAction::Retire { input_id }),
        })
    }
}

#[derive(Debug)]
pub(crate) struct InputFailure {
    pub(crate) source: CafleetError,
    pub(crate) run_id: Option<i64>,
    pub(crate) interaction_id: Option<i64>,
    pub(crate) input_id: Option<i64>,
    pub(crate) operation_key: Option<String>,
}
impl From<InputFailure> for CafleetError {
    fn from(failure: InputFailure) -> Self {
        let mut fields = Vec::new();
        for (name, value) in [
            ("run_id", failure.run_id),
            ("interaction_id", failure.interaction_id),
            ("input_id", failure.input_id),
        ] {
            if let Some(value) = value {
                fields.push(format!("{name}={value}"));
            }
        }
        if let Some(key) = failure.operation_key {
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
struct InputContext {
    run_id: Option<i64>,
    interaction_id: Option<i64>,
    input_id: Option<i64>,
    operation_key: Option<String>,
}
impl InputContext {
    fn failure(self, source: CafleetError) -> InputFailure {
        InputFailure {
            source,
            run_id: self.run_id,
            interaction_id: self.interaction_id,
            input_id: self.input_id,
            operation_key: self.operation_key,
        }
    }
}
pub(crate) trait InputCommandHost {
    fn lock(
        &mut self,
        settings: &Settings,
        run: &RunRecord,
        placement: &Placement,
        deadline: Deadline,
    ) -> std::result::Result<Option<PaneLock>, InputFailure>;
    fn health(
        &mut self,
        run: &RunRecord,
        deadline: Instant,
    ) -> std::result::Result<HealthSnapshot, InputFailure>;
    fn exchange(
        &mut self,
        run: &RunRecord,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure>;
}

pub(crate) struct NativeInputHost;
impl InputCommandHost for NativeInputHost {
    fn lock(
        &mut self,
        settings: &Settings,
        _: &RunRecord,
        placement: &Placement,
        deadline: Deadline,
    ) -> std::result::Result<Option<PaneLock>, InputFailure> {
        PaneLock::acquire_within(&settings.database_url, placement, deadline).map_err(host_failure)
    }

    fn health(
        &mut self,
        run: &RunRecord,
        deadline: Instant,
    ) -> std::result::Result<HealthSnapshot, InputFailure> {
        probe_health(run, deadline).map_err(host_failure)
    }

    fn exchange(
        &mut self,
        run: &RunRecord,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> std::result::Result<InputControlReply, InputFailure> {
        let result = (|| {
            let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
            let mut client =
                super::control::ControlClient::connect(&paths.control_socket, deadline)?;
            client.exchange_input(request, deadline)
        })();
        result.map_err(host_failure)
    }
}

fn host_failure(source: CafleetError) -> InputFailure {
    InputContext {
        run_id: None,
        interaction_id: None,
        input_id: None,
        operation_key: None,
    }
    .failure(source)
}

fn submission_context(run_id: i64, request: &SubmitInput) -> InputContext {
    InputContext {
        run_id: Some(run_id),
        interaction_id: None,
        input_id: None,
        operation_key: Some(request.operation_key.clone()),
    }
}

fn validate_submission_request(run_id: i64, request: &SubmitInput) -> Result<()> {
    validate_operation_key(&request.operation_key)?;
    usage(
        run_id > 0 && crate::input::valid_input(&request.literal),
        "ordinary submission requires a positive run and one nonblank line without control characters",
    )?;
    match request.input_kind {
        InputKind::Preview => usage(
            request.message_id.is_some_and(|id| {
                id > 0 && request.operation_key == format!("preview:{run_id}:{id}")
            }),
            "preview requires its run-bound message key",
        )?,
        InputKind::Prompt | InputKind::Exec => usage(
            request.message_id.is_none(),
            "explicit input has no preview message",
        )?,
        InputKind::PermissionResponse => {
            return Err(CafleetError::Usage("ordinary input kind required".into()));
        }
    }
    Ok(())
}

pub(crate) fn find_submission(
    conn: &mut Connection,
    identity: &PermissionReceiptIdentity,
    request: &SubmitInput,
    deadline: Deadline,
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
                let digest = storage::versioned_payload_digest(
                    input.payload_digest_version,
                    identity.run_id,
                    request,
                )?;
                ensure(
                    input.kind == request.input_kind
                        && input.payload_digest == digest
                        && input.message_id == request.message_id,
                    "conflicting input operation key reuse",
                )?;
            }
            deadline.remaining()?;
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
        (InputState::InProgress | InputState::Uncertain, _) => {
            Some((SubmissionStatus::Uncertain, SubmissionStage::Unknown, None))
        }
        (InputState::Settled, _) => Some((
            SubmissionStatus::Rejected,
            SubmissionStage::ZeroBytes,
            Some("input operation key is consumed".into()),
        )),
    }
}

fn historical_submission(input: &InputOperation) -> Option<SubmissionResult> {
    let (status, stage, reason) = durable_outcome(input)?;
    Some(SubmissionResult {
        status,
        input_id: Some(input.input_id),
        stage,
        output_seq: None,
        reason,
        delivery: input.delivery(),
    })
}

fn submission_deadline(conn: &Connection, deadline: Deadline) -> Result<Deadline> {
    let deadline = Deadline::until(bounded_by_inherited(conn, deadline.instant()));
    deadline.remaining()?;
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
        .and_then(|member| member.placement)
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
                }
                SubmissionStatus::Held | SubmissionStatus::Rejected => {
                    reply.stage == SubmissionStage::ZeroBytes
                        && (input.state == InputState::Ready
                            || input.state == InputState::Settled
                                && input.disposition != Some(InputDisposition::Submitted))
                }
            };
            ensure(
                consistent && reply.delivery == input.delivery(),
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
    let current = storage::validate_owner(conn, &expected.owner())?;
    ensure(
        stable_run_identity(&current) == stable_run_identity(expected)
            && current.status == RunStatus::Running
            && current.owner_kind == OwnerKind::Runner
            && current.native_version == expected.native_version
            && current.profile_version == expected.profile_version,
        "submission run identity changed",
    )?;
    ensure(
        crate::broker::active_member_fleet(conn, expected.member_id)? == Some(expected.fleet_id),
        "input member is inactive",
    )?;
    let member = crate::broker::get_member(conn, expected.member_id, expected.fleet_id)?
        .ok_or_else(|| error("input member is absent"))?;
    ensure(
        member.kind == crate::broker::records::MemberKind::Member
            && member.placement.as_ref() == Some(placement),
        "submission member placement changed",
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
    let identity = PermissionReceiptIdentity::for_run(expected_run);
    let retained = find_submission(conn, &identity, request, deadline)?;
    if let Some(result) = retained.as_ref().and_then(historical_submission) {
        return Ok(SubmissionPreparation::History(result));
    }
    let mut context = submission_context(expected_run.run_id, request);
    context.input_id = retained.as_ref().map(|input| input.input_id);
    (|| {
        let deadline = submission_deadline(conn, deadline)?;
        with_database_budget(conn, deadline, |conn| {
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
        let attempt =
            PreparedSubmission::new(request, deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, expected_run, expected_placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("pane input remained busy before submission"))?;
        deadline.remaining()?;
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
    let deadline = submission_deadline(
        conn,
        Deadline::until(deadline.instant().min(attempt.deadline)),
    )
    .map_err(|source| submission_context(expected_run.run_id, attempt.request).failure(source))?;
    let identity = PermissionReceiptIdentity::for_run(expected_run);
    let retained = find_submission(conn, &identity, attempt.request, deadline)?;
    if let Some(result) = retained.as_ref().and_then(historical_submission) {
        return Ok(result);
    }
    let mut context = submission_context(expected_run.run_id, attempt.request);
    context.input_id = retained.as_ref().map(|input| input.input_id);
    let result = (|| {
        deadline.remaining()?;
        held.validate_binding(&settings.database_url, expected_placement)?;
        with_database_budget(conn, deadline, |conn| {
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
        let health = host
            .health(expected_run, deadline.instant())
            .map_err(CafleetError::from)?;
        health.validate(expected_run)?;
        deadline.remaining()?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(&tx, settings, expected_run, expected_placement)?;
            let changed = tx.execute(
                "UPDATE member_runs SET reachability='reachable',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3",
                rusqlite::params![crate::time::now_stamp(), expected_run.run_id, expected_run.owner_epoch],
            )?;
            ensure(changed == 1, "submission health ownership changed")?;
            deadline.remaining()?;
            tx.commit().map_err(crate::error::CafleetError::from)
        })?;
        let input = attempt.request.clone();
        let operation = match input.input_kind {
            InputKind::Preview | InputKind::Prompt => InputControlOperation::SubmitPrompt(input),
            InputKind::Exec => InputControlOperation::SubmitExec(input),
            InputKind::PermissionResponse => unreachable!("validated ordinary input kind"),
        };
        let request = InputControlRequest {
            identity: ControlIdentity::from_run(expected_run),
            expires_at_mono_ns: attempt.expiration.monotonic_ns(),
            operation,
        };
        held.validate_binding(&settings.database_url, expected_placement)?;
        deadline.remaining()?;
        held.release()?;
        let reply = host.exchange(expected_run, &request, deadline.instant());
        let reply = reply.map_err(CafleetError::from)?;
        let InputControlReply::Submission(reply) = reply else {
            return Err(error("ordinary submission requires a submission reply"));
        };
        let recorded = find_submission(conn, &identity, attempt.request, deadline)
            .map_err(CafleetError::from)?;
        context.input_id = recorded.as_ref().map(|input| input.input_id);
        if !held.reacquire_within(Deadline::until(
            deadline
                .instant()
                .max(Instant::now() + std::time::Duration::from_secs(1)),
        ))? {
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
        with_database_budget(conn, deadline, |conn| {
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
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
        deadline.remaining()?;
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
    let current = storage::validate_owner(conn, &expected.owner())?;
    ensure(
        stable_run_identity(&current) == stable_run_identity(expected),
        "receipt run identity changed",
    )?;
    ensure(
        super::input_recovery::receipt_placement(conn, &current)?.as_ref() == Some(placement),
        "receipt member placement changed",
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
    reconcile_ordinary_with_host(
        conn,
        settings,
        input_id,
        disposition,
        evidence_ref,
        deadline,
        &mut NativeInputHost,
    )
}

fn reconcile_ordinary_with_host(
    conn: &mut Connection,
    settings: &Settings,
    input_id: i64,
    disposition: InputDisposition,
    evidence_ref: &str,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<InputOperation, InputFailure> {
    reconcile_receipt_with_hosts(
        conn,
        settings,
        input_id,
        disposition,
        evidence_ref,
        deadline,
        (host, &mut super::recovery::NativeRecoveryHost),
    )
}

fn reconcile_receipt_with_hosts(
    conn: &mut Connection,
    settings: &Settings,
    input_id: i64,
    disposition: InputDisposition,
    evidence_ref: &str,
    deadline: Deadline,
    hosts: (&mut impl InputCommandHost, &mut impl RecoveryHost),
) -> std::result::Result<InputOperation, InputFailure> {
    let (host, recovery) = hosts;
    let mut context = InputContext {
        run_id: None,
        interaction_id: None,
        input_id: (input_id > 0).then_some(input_id),
        operation_key: None,
    };
    let result = (|| {
        let deadline = submission_deadline(conn, deadline)?;
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
            context.interaction_id = input
                .permission_response
                .as_ref()
                .map(|r| r.metadata.interaction_id);
            let recorded_run = storage::get_run(&tx, input.run_id)?;
            let run = storage::validate_owner(&tx, &recorded_run.owner())?;
            let placement = if input.state == InputState::Settled {
                ensure(
                    input.disposition == Some(disposition),
                    "settled input disposition changed",
                )?;
                None
            } else {
                super::input_recovery::receipt_placement(&tx, &run)?
            };
            deadline.remaining()?;
            tx.commit()?;
            Ok((input, run, placement))
        })?;
        if original.state == InputState::Settled {
            return Ok(original);
        }
        let Some(placement) = placement else {
            return super::input_recovery::reconcile_input_with_host(
                conn,
                settings,
                &original,
                &run,
                None,
                None,
                disposition,
                evidence_ref,
                deadline.instant(),
                recovery,
            )
            .map_err(|failure| failure.source);
        };
        let expiration = InputExpiration::capture(deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, &run, &placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("receipt pane input remained busy"))?;
        let live = (|| {
            let read_target = |conn: &mut Connection| {
                with_database_budget(conn, deadline, |conn| {
                    let tx = conn.transaction()?;
                    held.validate_binding(&settings.database_url, &placement)?;
                    validate_receipt_target(&tx, settings, &run, &placement)?;
                    let input = storage::get_input(&tx, input_id)?;
                    ensure(
                        input.run_id == original.run_id
                            && input.operation_key == original.operation_key
                            && input.kind == original.kind
                            && input.payload_digest == original.payload_digest
                            && input.message_id == original.message_id
                            && input.created_at == original.created_at
                            && input.permission_response == original.permission_response,
                        "receipt immutable input identity changed",
                    )?;
                    deadline.remaining()?;
                    tx.commit()?;
                    Ok(input)
                })
            };
            read_target(conn)?;
            let health = host
                .health(&run, deadline.instant())
                .map_err(CafleetError::from)?;
            health.validate(&run)?;
            deadline.remaining()?;
            read_target(conn)?;
            let request = |action| InputControlRequest {
                identity: ControlIdentity::from_run(&run),
                expires_at_mono_ns: expiration.monotonic_ns(),
                operation: InputControlOperation::Receipt(action),
            };
            let query = request(ReceiptAction::Query {
                operation_key: original.operation_key.clone(),
            });
            let reply = host
                .exchange(&run, &query, deadline.instant())
                .map_err(CafleetError::from)?;
            ensure(
                matches!(reply, InputControlReply::Receipt(ReceiptReply::Present { input_id: actual, .. })
                if actual == input_id),
                "receipt query has no matching durable input identity",
            )?;
            deadline.remaining()?;
            read_target(conn)?;
            let reconcile = request(ReceiptAction::Reconcile {
                input_id,
                expected: original.state,
                disposition,
                evidence_ref: evidence_ref.into(),
            });
            host.exchange(&run, &reconcile, deadline.instant())
                .map_err(CafleetError::from)?;
            let settled = read_target(conn)?;
            ensure(
                settled.state == InputState::Settled
                    && settled.disposition == Some(disposition)
                    && settled.evidence_ref.as_deref() == Some(evidence_ref),
                "receipt settlement acknowledgment has no matching durable settlement",
            )?;
            deadline.remaining()?;
            let retirement = request(ReceiptAction::Retire { input_id });
            host.exchange(&run, &retirement, deadline.instant())
                .map_err(CafleetError::from)?;
            deadline.remaining()?;
            let recorded = read_target(conn)?;
            ensure(recorded == settled, "receipt changed after retirement")?;
            Ok(recorded)
        })();
        match live {
            Ok(settled) => Ok(settled),
            Err(original_failure) => {
                let guard = match super::input_recovery::ExclusiveReceiptGuard::acquire(
                    conn,
                    settings,
                    &original,
                    &run,
                    (Some(&placement), Some(&held)),
                    deadline.instant(),
                    recovery,
                ) {
                    Ok(guard) => guard,
                    Err(_) => return Err(original_failure),
                };
                guard.settle(conn, disposition, evidence_ref, recovery)
            }
        }
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
pub(crate) fn validate_permission_reference(value: &str) -> Result<()> {
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
    validate_request_token(&input.request_token)?;
    validate_operation_key(&input.operation_key)?;
    validate_permission_reference(&input.mode_evidence_ref)?;
    validate_permission_reference(&input.decision_evidence_ref)?;
    validate_permission_attribution(input.decision_source, input.director_mode)
}
pub(crate) fn validate_permission_attribution(
    source: DecisionSource,
    mode: DirectorMode,
) -> Result<()> {
    usage(
        source != DecisionSource::DirectorAuto || mode == DirectorMode::Auto,
        "director-auto requires auto mode",
    )
}
fn selected_run(conn: &Connection, run_id: i64) -> Result<RunRecord> {
    selected_run_reporting(conn, run_id, "permission_response")
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
            && view
                .interaction
                .as_ref()
                .is_some_and(|value| value.classification == InteractionClass::PermissionRequest),
        "permission run/event/interaction/Director association mismatch",
    )
}
fn metadata(
    input: &PermissionRespondInput,
    audit: &PermissionResponseRecord,
) -> PermissionResponseMetadata {
    PermissionResponseMetadata {
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
        answer_profile_version: audit.metadata.answer_profile_version.clone(),
        answer_rule_id: audit.metadata.answer_rule_id.clone(),
    }
}
pub(super) fn projection(
    run: &RunRecord,
    receipt: PermissionReceipt,
) -> Result<PermissionResponseResult> {
    let input = receipt.input;
    let metadata = receipt.response.metadata;
    let (status, stage, reason) = match input.state {
        InputState::Accepted => (SubmissionStatus::Accepted, SubmissionStage::Submitted, None),
        InputState::InProgress | InputState::Uncertain => (
            SubmissionStatus::Uncertain,
            SubmissionStage::Unknown,
            Some("prior transmission remains uncertain".into()),
        ),
        InputState::Settled => match input
            .disposition
            .expect("validated settled input has a disposition")
        {
            InputDisposition::Submitted => {
                (SubmissionStatus::Accepted, SubmissionStage::Submitted, None)
            }
            InputDisposition::NotSubmitted | InputDisposition::AbandonedUncertain => (
                SubmissionStatus::Rejected,
                SubmissionStage::ZeroBytes,
                Some("operation key is consumed".into()),
            ),
        },
        InputState::Ready => (
            SubmissionStatus::Held,
            SubmissionStage::ZeroBytes,
            Some("input requires live proven-zero revalidation".into()),
        ),
    };
    Ok(PermissionResponseResult {
        submission: SubmissionResult {
            status,
            input_id: Some(input.input_id),
            stage,
            output_seq: None,
            reason,
            delivery: None,
        },
        member_id: run.member_id,
        run_id: run.run_id,
        owner_epoch: metadata.expected_owner_epoch,
        interaction_id: metadata.interaction_id,
        event_id: metadata.event_id,
        decision: metadata.decision,
        decision_source: metadata.decision_source,
    })
}
pub(super) fn response_lookup(
    conn: &Connection,
    run: &RunRecord,
    input: &PermissionRespondInput,
    known_id: &mut Option<i64>,
) -> Result<Option<PermissionReceipt>> {
    let id = conn
        .query_row(
            "SELECT input_id FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
            rusqlite::params![run.run_id, input.operation_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(None);
    };
    *known_id = Some(id);
    let retained = storage::get_permission_response(conn, id)?;
    let request = NewPermissionResponse {
        operation_key: input.operation_key.clone(),
        metadata: metadata(input, &retained.response),
    };
    let identity = PermissionReceiptIdentity::for_run(run);
    let receipt = storage::lookup_permission_response(conn, &identity, &request)?
        .ok_or_else(|| error("retained permission response disappeared from its snapshot"))?;
    Ok(Some(receipt))
}

enum ResponsePreparation {
    History(PermissionResponseResult),
    Live(Box<(RunRecord, RuntimeEventView, Option<PermissionReceipt>)>),
}
fn prepare_response(
    conn: &mut Connection,
    input: &PermissionRespondInput,
    deadline: Deadline,
) -> std::result::Result<ResponsePreparation, InputFailure> {
    let mut context = InputContext {
        run_id: Some(input.expected_run_id),
        interaction_id: Some(input.interaction_id),
        input_id: None,
        operation_key: Some(input.operation_key.clone()),
    };
    let result = with_database_budget(conn, deadline, |conn| {
        let tx = conn.transaction()?;
        let run = selected_run(&tx, input.expected_run_id)?;
        let retained = response_lookup(&tx, &run, input, &mut context.input_id)?;
        let prepared = if let Some(receipt) = &retained
            && receipt.input.state != InputState::Ready
        {
            ResponsePreparation::History(projection(&run, receipt.clone())?)
        } else {
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
            let original = super::input::inspection_association(&tx, &run, &input.inspection())?;
            ResponsePreparation::Live(Box::new((run, original, retained)))
        };
        deadline.remaining()?;
        tx.commit()?;
        Ok(prepared)
    });
    result.map_err(|source| context.failure(source))
}

pub(crate) fn respond_permission_with_host(
    conn: &mut Connection,
    settings: &Settings,
    input: &PermissionRespondInput,
    deadline: Deadline,
    host: &mut impl InputCommandHost,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    let mut context = InputContext {
        run_id: Some(input.expected_run_id),
        interaction_id: Some(input.interaction_id),
        input_id: None,
        operation_key: Some(input.operation_key.clone()),
    };
    let deadline = match validate_respond(input).and_then(|()| submission_deadline(conn, deadline))
    {
        Ok(deadline) => deadline,
        Err(source) => return Err(context.failure(source)),
    };
    let prepared = prepare_response(conn, input, deadline)?;
    let (run, original, ready) = match prepared {
        ResponsePreparation::History(result) => return Ok(result),
        ResponsePreparation::Live(snapshot) => *snapshot,
    };
    context.input_id = ready.as_ref().map(|r| r.input.input_id);
    let result = (|| {
        let placement =
            with_database_budget(conn, deadline, |conn| permission_placement(conn, &run))?;
        let expiration = InputExpiration::capture(deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, &run, &placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate = |conn: &mut Connection| -> Result<()> {
            deadline.remaining()?;
            held.validate_binding(&settings.database_url, &placement)?;
            with_database_budget(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                validate_submission_target(&tx, settings, &run, &placement)?;
                let view = super::input::inspection_association(&tx, &run, &input.inspection())?;
                ensure(
                    same_association(&view, &original),
                    "permission response association changed",
                )?;
                if let Some(ready) = &ready {
                    ensure(
                        storage::get_permission_response(&tx, ready.input.input_id)? == *ready,
                        "ready response snapshot changed",
                    )?;
                }
                deadline.remaining()?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })
        };
        validate(conn)?;
        host.health(&run, deadline.instant())
            .map_err(CafleetError::from)?
            .validate(&run)?;
        validate(conn)?;
        let request = InputControlRequest {
            identity: ControlIdentity::from_run(&run),
            expires_at_mono_ns: expiration.monotonic_ns(),
            operation: InputControlOperation::PermissionRespond(input.clone()),
        };
        let reply = host
            .exchange(&run, &request, deadline.instant())
            .map_err(CafleetError::from)?;
        let InputControlReply::PermissionResponse(reply) = reply else {
            return Err(error("permission response requires its typed reply"));
        };
        let receipt = with_database_budget(conn, deadline, |conn| {
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
            let receipt = response_lookup(&tx, &run, input, &mut context.input_id)?;
            context.input_id = receipt.as_ref().map(|r| r.input.input_id);
            deadline.remaining()?;
            tx.commit()?;
            Ok(receipt)
        })?;
        validate_reply_against_durable(
            &reply.submission,
            receipt.as_ref().map(|receipt| &receipt.input),
            [
                "permission response reply input identity mismatch",
                "permission response reply contradicts durable state",
                "permission response outcome requires its durable input",
                "permission response reply has no matching durable input",
            ],
        )?;
        deadline.remaining()?;
        Ok(reply)
    })();
    result.map_err(|source| context.failure(source))
}
pub(crate) fn respond_permission(
    conn: &mut Connection,
    settings: &Settings,
    input: &PermissionRespondInput,
    deadline: Deadline,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    respond_permission_with_host(conn, settings, input, deadline, &mut NativeInputHost)
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
    let mut context = InputContext {
        run_id: None,
        interaction_id: Some(input.interaction_id),
        input_id: None,
        operation_key: None,
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
        let deadline = submission_deadline(conn, deadline)?;
        let (run, placement, original) = with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction()?;
            let interaction_exists = tx
                .query_row(
                    "SELECT 1 FROM member_interactions WHERE interaction_id=?1",
                    [input.interaction_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !interaction_exists {
                return Err(CafleetError::App(format!(
                    "Interaction {} not found",
                    input.interaction_id
                )));
            }
            let view = storage::get_event(&tx, input.event_id)?;
            let run = selected_run_reporting(
                &tx,
                view.event.run_id,
                "member_runs.database_identity_json",
            )?;
            context.run_id = Some(run.run_id);
            let placement = permission_placement(&tx, &run)?;
            validate_submission_target(&tx, settings, &run, &placement)?;
            let view = super::input::inspection_association(&tx, &run, input)?;
            deadline.remaining()?;
            tx.commit()?;
            Ok((run, placement, view))
        })?;
        let expiration = InputExpiration::capture(deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, &run, &placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate = |conn: &mut Connection| -> Result<()> {
            deadline.remaining()?;
            held.validate_binding(&settings.database_url, &placement)?;
            with_database_budget(conn, deadline, |conn| {
                let tx = conn.transaction()?;
                validate_submission_target(&tx, settings, &run, &placement)?;
                let view = super::input::inspection_association(&tx, &run, input)?;
                ensure(
                    same_association(&view, &original),
                    "permission inspection association changed",
                )?;
                deadline.remaining()?;
                tx.commit().map_err(crate::error::CafleetError::from)
            })
        };
        validate(conn)?;
        host.health(&run, deadline.instant())
            .map_err(CafleetError::from)?
            .validate(&run)?;
        validate(conn)?;
        let request = InputControlRequest {
            identity: ControlIdentity::from_run(&run),
            expires_at_mono_ns: expiration.monotonic_ns(),
            operation: InputControlOperation::PermissionInspect(input.clone()),
        };
        let reply = host
            .exchange(&run, &request, deadline.instant())
            .map_err(CafleetError::from)?;
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
                && reply.native_version == run.native_version
                && reply.profile_version == run.profile_version,
            "permission inspection reply identity mismatch",
        )?;
        ensure(
            reply.evidence["backend"].as_str() == Some(run.backend.as_str())
                && reply.evidence["native_version"].as_str() == run.native_version.as_deref()
                && reply.evidence["profile_version"].as_str() == run.profile_version.as_deref()
                && reply.evidence["screen_seq"].as_u64() == Some(reply.output_seq),
            "permission inspection evidence identity mismatch",
        )?;
        let missing = super::input::inspection_missing_evidence(&reply.evidence)?;
        ensure(
            reply
                .missing_evidence
                .iter()
                .map(MissingEvidence::as_str)
                .eq(missing.iter().map(MissingEvidence::as_str)),
            "permission inspection evidence completeness mismatch",
        )?;
        match reply.answer_capability {
            Coverage::Tested => ensure(
                reply.request_token.is_some()
                    && !reply.supported_decisions.is_empty()
                    && reply.answer_profile_version == run.profile_version
                    && reply.classification == Some(InteractionClass::PermissionRequest)
                    && reply.visibility == Visibility::Visible
                    && reply.capability_reason.is_none(),
                "permission inspection capability fields disagree",
            )?,
            Coverage::Unknown => ensure(
                reply.request_token.is_none()
                    && reply.supported_decisions.is_empty()
                    && reply
                        .capability_reason
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty()),
                "permission inspection unavailable fields disagree",
            )?,
        }
        for value in [&reply.answer_profile_version, &reply.answer_rule_id]
            .into_iter()
            .flatten()
        {
            ensure(
                !value.trim().is_empty() && value.len() <= 256,
                "permission inspection profile identity is invalid",
            )?;
        }
        deadline.remaining()?;
        Ok(*reply)
    })();
    result.map_err(|source| context.failure(source))
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::runtime::runner::test_support::Fixture;
    use crate::runtime::runner::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    pub(crate) const NOW: &str = "2026-10-26T22:00:00.000000+00:00";

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Retained {
        InProgress,
        Accepted,
        Uncertain,
        Submitted,
        NotSubmitted,
        Abandoned,
    }
    pub(crate) struct Case {
        pub(crate) fixture: Fixture,
        pub(crate) owner: RunOwner,
        pub(crate) event: RuntimeEventRecord,
        pub(crate) request: NewPermissionResponse,
        pub(crate) input_id: i64,
        pub(crate) settings: Settings,
        pub(crate) placement: crate::broker::records::Placement,
        pub(crate) _artifacts: RunArtifacts,
        pub(crate) _lock: Arc<RunLock>,
    }
    impl Case {
        pub(crate) fn command(&self) -> PermissionRespondInput {
            let metadata = &self.request.metadata;
            PermissionRespondInput {
                interaction_id: metadata.interaction_id,
                event_id: metadata.event_id,
                director_member_id: metadata.director_member_id,
                expected_run_id: self.owner.run_id,
                expected_owner_epoch: metadata.expected_owner_epoch,
                request_token: metadata.request_token.clone(),
                decision: metadata.decision,
                decision_source: metadata.decision_source,
                director_mode: metadata.director_mode,
                mode_evidence_ref: metadata.mode_evidence_ref.clone(),
                decision_evidence_ref: metadata.decision_evidence_ref.clone(),
                operation_key: self.request.operation_key.clone(),
            }
        }
        pub(crate) fn new(state: Retained) -> Self {
            Self::decision(
                state,
                PermissionDecision::Allow,
                DirectorMode::Auto,
                DecisionSource::DirectorAuto,
            )
        }
        pub(crate) fn decision(
            state: Retained,
            decision: PermissionDecision,
            mode: DirectorMode,
            source: DecisionSource,
        ) -> Self {
            Self::from_fixture(Fixture::new(), state, decision, mode, source)
        }
        pub(crate) fn from_fixture(
            mut fixture: Fixture,
            state: Retained,
            decision: PermissionDecision,
            mode: DirectorMode,
            source: DecisionSource,
        ) -> Self {
            let artifacts = fixture.artifacts();
            let lock = Arc::new(
                RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap(),
            );
            let owner = lock
                .transfer_to_runner(
                    &mut fixture.conn,
                    &fixture.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
            assert_eq!(fixture.conn.execute(
                "UPDATE member_runs SET status='running',native_version='0.162.0',coverage='tested',profile_version='fixture-detector-v1',started_at=?1 WHERE run_id=?2",
                rusqlite::params![NOW, fixture.run.run_id],
            ).unwrap(), 1);
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            storage::reserve_event_sequences(&mut fixture.conn, &owner).unwrap();
            let evidence = storage::evidence::EvidencePolicy::metadata().apply(&json!({
                "backend":"codex","native_version":fixture.run.native_version,"profile_version":"fixture-detector-v1",
                "rule_id":"fixture-permission","screen_seq":1,"operation":null,"choices":null,
                "native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false
            })).unwrap();
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
            let event = storage::persist_event(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live,
            )
            .unwrap();
            let answer_rule = match decision {
                PermissionDecision::Allow => "fixture-allow-once",
                PermissionDecision::Deny => "fixture-deny-once",
            };
            let capability = crate::coding_agent::AnswerCapability::fixture(
                crate::coding_agent::CodingAgent::Codex,
                fixture.run.native_version.as_deref().unwrap(),
                "fixture-answer-v1",
                "fixture-permission",
                answer_rule,
                decision,
                &["synthetic-history-storage".into()],
            )
            .unwrap();
            let request = NewPermissionResponse {
                operation_key: "fixture:retained-response".into(),
                metadata: PermissionResponseMetadata {
                    event_id: event.event_id,
                    interaction_id: event.interaction_id.unwrap(),
                    expected_owner_epoch: owner.owner_epoch,
                    request_token: "a".repeat(64),
                    decision,
                    director_member_id: fixture.run.director_member_id,
                    director_mode: mode,
                    decision_source: source,
                    mode_evidence_ref: "fixture:host-mode".into(),
                    decision_evidence_ref: "fixture:attributed-decision".into(),
                    answer_profile_version: "fixture-answer-v1".into(),
                    answer_rule_id: answer_rule.into(),
                },
            };
            let receipt = match storage::reserve_permission_response(
                &mut fixture.conn,
                &owner,
                &request,
                Some(&capability),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap()
            {
                PermissionReservation::Reserved(receipt) => receipt,
                PermissionReservation::Retained(_) => {
                    panic!("a fresh history fixture reserves its own identity")
                }
            };
            let input_id = receipt.input.input_id;
            match state {
                Retained::InProgress => {}
                Retained::Accepted | Retained::Uncertain => {
                    storage::record_input_state(
                        &mut fixture.conn,
                        &owner,
                        input_id,
                        InputState::InProgress,
                        if matches!(state, Retained::Accepted) {
                            InputState::Accepted
                        } else {
                            InputState::Uncertain
                        },
                    )
                    .unwrap();
                }
                Retained::Submitted | Retained::Abandoned => {
                    storage::settle_input(
                        &mut fixture.conn,
                        &owner,
                        input_id,
                        InputState::InProgress,
                        if matches!(state, Retained::Submitted) {
                            InputDisposition::Submitted
                        } else {
                            InputDisposition::AbandonedUncertain
                        },
                        "fixture:recorded-disposition",
                    )
                    .unwrap();
                }
                Retained::NotSubmitted => {
                    storage::record_input_state(
                        &mut fixture.conn,
                        &owner,
                        input_id,
                        InputState::InProgress,
                        InputState::Ready,
                    )
                    .unwrap();
                    storage::settle_input(
                        &mut fixture.conn,
                        &owner,
                        input_id,
                        InputState::Ready,
                        InputDisposition::NotSubmitted,
                        "fixture:recorded-zero-byte-disposition",
                    )
                    .unwrap();
                }
            }
            let read = storage::get_permission_response(&fixture.conn, input_id).unwrap();
            assert!(read.claim.is_some());
            assert_eq!(read.response.metadata, request.metadata);
            let settings =
                Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()), None);
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            Self {
                fixture,
                owner,
                event,
                request,
                input_id,
                settings,
                placement,
                _artifacts: artifacts,
                _lock: lock,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::runner::service::test_support::ServiceFixture;
    use crate::runtime::runner::service::test_support::input_service::{
        input_service_barrier, input_service_composer, input_service_records,
        wait_input_service_record, with_input_service,
    };
    use crate::runtime::runner::test_support::Fixture;
    use std::io::Write;
    use std::net::Shutdown;
    use std::time::Duration;

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

    /// Row counts of the input, permission-response and claim tables, in that order.
    fn permission_counts(conn: &Connection) -> [i64; 3] {
        [
            "member_input_operations",
            "member_permission_responses",
            "member_permission_response_claims",
        ]
        .map(|table| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
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
        use crate::runtime::runner::observation::test_support::synthetic;

        let mut fixture = ServiceFixture::configured(false, synthetic);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        fixture.master.write_all(b"form-inspection\r").unwrap();
        fixture.read_until("Operation: write form-inspection.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, message_id) = opened_interaction(&mut fixture, until);
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        let settings = Settings::for_database(
            std::path::Path::new(fixture.fixture.conn.path().unwrap()),
            Some("tmux"),
        );
        let before = permission_counts(&fixture.fixture.conn);
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
        assert_eq!(permission_counts(&fixture.fixture.conn), before);
        assert!(notified_at(&fixture.fixture.conn, message_id).is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-barrier");
        fixture.finish();
    }

    #[test]
    fn permission_inspection_client_uses_real_guard_original_sql_bound_and_validates_live_reply() {
        use crate::coding_agent::profiles::catalog::test_support::inspection_fixture_definition;
        use crate::runtime::runner::observation::test_support::synthetic;

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
        assert_eq!(permission_counts(&fixture.fixture.conn), [0, 0, 0]);
        assert!(notified_at(&fixture.fixture.conn, message_id).is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-client-barrier");
        fixture.finish();
    }

    #[test]
    fn permission_response_history_honors_expired_inherited_budget_before_sql_or_live_calls() {
        use crate::runtime::runner::input_client::test_support::{Case, Retained};
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let mut case = Case::new(Retained::Accepted);
        let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
        let input = case.command();
        let run = storage::get_run(&case.fixture.conn, input.expected_run_id).unwrap();
        let records = health_records(&case.fixture.conn);
        let changes = case.fixture.conn.total_changes();
        let calls = Arc::new(AtomicUsize::new(0));
        let budgets = Arc::new(Mutex::new(Vec::new()));
        let observed_calls = calls.clone();
        let observed_budgets = budgets.clone();
        let original = Instant::now() + Duration::from_millis(100);
        let supplied = Deadline::until(original + Duration::from_secs(2));
        let mut active = None;
        let mut expired = None;
        let mut active_calls = None;
        let mut expired_calls = None;
        let scoped =
            with_database_budget(&mut case.fixture.conn, Deadline::until(original), |conn| {
                conn.authorizer(Some(move |context: AuthContext<'_>| {
                    observed_calls.fetch_add(1, Ordering::SeqCst);
                    if matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "member_input_operations",
                            ..
                        }
                    ) {
                        observed_budgets
                            .lock()
                            .unwrap()
                            .push(Deadline::inherited_or_new().instant());
                    }
                    Authorization::Allow
                }))?;
                active = Some(respond_permission_with_host(
                    conn,
                    &case.settings,
                    &input,
                    supplied,
                    &mut NoLiveHost,
                ));
                active_calls = Some(calls.swap(0, Ordering::SeqCst));
                std::thread::sleep(
                    original.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
                assert!(
                    Instant::now() >= original,
                    "actual inherited frame has expired"
                );
                expired = Some(respond_permission_with_host(
                    conn,
                    &case.settings,
                    &input,
                    supplied,
                    &mut NoLiveHost,
                ));
                expired_calls = Some(calls.load(Ordering::SeqCst));
                conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                    .map_err(crate::error::CafleetError::from)
            });
        let expired = expired.expect("expired-frame invocation completed");
        let expired_calls = expired_calls.expect("SQL count sampled after expired invocation");
        scoped.unwrap();
        let active = active
            .expect("same history has a live-budget control")
            .unwrap();
        assert_eq!(active.submission.status, SubmissionStatus::Accepted);
        assert_eq!(active.submission.input_id, Some(case.input_id));
        assert!(active_calls.expect("active SQL count") > 0);
        let budgets = budgets.lock().unwrap();
        assert!(!budgets.is_empty(), "actual history SQL was authorized");
        assert!(budgets.iter().all(|bound| *bound <= original));
        drop(budgets);
        assert_eq!(case.fixture.conn.total_changes(), changes);
        assert_eq!(
            storage::get_run(&case.fixture.conn, input.expected_run_id).unwrap(),
            run
        );
        assert_eq!(health_records(&case.fixture.conn), records);
        assert_eq!(
            storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
            receipt
        );
        let failure = expired
            .err()
            .expect("expired inherited history refuses before lookup");
        assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
        assert_eq!(
            (failure.run_id, failure.interaction_id, failure.input_id),
            (
                Some(input.expected_run_id),
                Some(input.interaction_id),
                None
            )
        );
        assert_eq!(
            failure.operation_key.as_deref(),
            Some(input.operation_key.as_str())
        );
        assert!(!failure.source.message().contains(&input.request_token));
        assert_eq!(
            expired_calls, 0,
            "expired inherited authority starts no SQL"
        );
    }

    fn response_services(fixture: &mut Fixture, spec: &mut SpawnSpec) {
        crate::runtime::runner::observation::test_support::synthetic(fixture, spec);
        let mut catalog =
            crate::coding_agent::profiles::catalog::test_support::inspection_fixture_definition();
        catalog["profiles"][0]["capabilities"]["answers"][0]["stages"] = serde_json::json!([{
            "bytes_hex":"790d","required_rule_id":"synthetic-permission","required_choice_id":"allow-once"
        }]);
        overwrite_observation_catalog(fixture, catalog);
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
    fn response_command(
        fixture: &mut ServiceFixture,
        until: Instant,
        key: &str,
    ) -> (Settings, RunRecord, Placement, PermissionRespondInput) {
        fixture.master.write_all(b"form-response\r").unwrap();
        fixture.read_until("Operation: write form-response.txt");
        let (event_id, interaction_id) = loop {
            let row=with_database_budget(&mut fixture.fixture.conn,Deadline::until(until),|conn| {
                conn.query_row("SELECT event_id,interaction_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",[fixture.fixture.run.run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).optional().map_err(crate::error::CafleetError::from)
            }).unwrap();
            if let Some(ids) = row {
                break ids;
            }
            assert!(
                Instant::now() < until,
                "original response observation deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        let (settings, run, placement) = service_target(fixture);
        let current = inspect_permission(
            &mut fixture.fixture.conn,
            &settings,
            &PermissionInspectInput {
                event_id,
                interaction_id,
                director_member_id: run.director_member_id,
            },
            Deadline::until(until),
        )
        .unwrap();
        assert!(
            current
                .supported_decisions
                .contains(&PermissionDecision::Allow)
        );
        let input = PermissionRespondInput {
            event_id,
            interaction_id,
            director_member_id: run.director_member_id,
            expected_run_id: run.run_id,
            expected_owner_epoch: run.owner_epoch,
            request_token: current.request_token.expect("actual current service token"),
            decision: PermissionDecision::Allow,
            decision_source: DecisionSource::DirectorAuto,
            director_mode: DirectorMode::Auto,
            mode_evidence_ref: "fixture:service-mode".into(),
            decision_evidence_ref: "fixture:service-decision".into(),
            operation_key: key.into(),
        };
        (settings, run, placement, input)
    }
    fn service_response_audit(
        fixture: &ServiceFixture,
        input: &PermissionRespondInput,
    ) -> PermissionReceipt {
        let operation = storage::find_input(
            &fixture.fixture.conn,
            input.expected_run_id,
            &input.operation_key,
        )
        .unwrap()
        .expect("actual durable permission operation");
        let receipt =
            storage::get_permission_response(&fixture.fixture.conn, operation.input_id).unwrap();
        assert_eq!(receipt.input.kind, InputKind::PermissionResponse);
        assert_eq!(receipt.input.state, InputState::Accepted);
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
                m.director_mode,
                m.decision_source
            ),
            (
                &input.request_token,
                input.decision,
                input.director_mode,
                input.decision_source
            )
        );
        assert_eq!(
            (&m.mode_evidence_ref, &m.decision_evidence_ref),
            (&input.mode_evidence_ref, &input.decision_evidence_ref)
        );
        assert_eq!(
            (&*m.answer_profile_version, &*m.answer_rule_id),
            ("synthetic-profile-1", "inspection-allow-plan")
        );
        assert_eq!(
            receipt
                .claim
                .as_ref()
                .map(|c| (c.interaction_id, c.input_id)),
            Some((input.interaction_id, operation.input_id))
        );
        receipt
    }

    #[test]
    fn permission_response_actual_service_returns_typed_durable_audit_and_one_selected_answer() {
        let mut fixture = ServiceFixture::configured(false, response_services);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        let (settings, run, placement, input) =
            response_command(&mut fixture, until, "response:actual-live");
        let output_start = fixture.output.len();
        let mut host = SocketHost::new(&settings, &placement, until);
        let original = storage::get_event(&fixture.fixture.conn, input.event_id).unwrap();
        let database = settings.database_url.clone();
        let interaction_id = input.interaction_id;
        let mut closed = SocketHost::new(&settings, &placement, until);
        closed.after_health = Some(Box::new(move || {
            let conn = crate::db::connect(&database).unwrap();
            assert_eq!(conn.execute(
                "UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2 AND closed_at IS NULL",
                rusqlite::params![crate::time::now_stamp(), interaction_id],
            ).unwrap(), 1);
            let closed_at: Option<String> = conn
                .query_row(
                    "SELECT closed_at FROM member_interactions WHERE interaction_id=?1",
                    [interaction_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(closed_at.is_some());
        }));
        let refused = respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(until),
            &mut closed,
        );
        let restored = fixture.fixture.conn.execute(
            "UPDATE member_interactions SET closed_at=NULL,close_reason=NULL WHERE interaction_id=?1 AND closed_at IS NOT NULL",
            [input.interaction_id],
        );
        assert_eq!(restored.unwrap(), 1);
        assert_eq!(closed.calls, [1, 1, 0]);
        let refused = refused
            .err()
            .expect("a genuinely closed association refuses before exchange");
        assert_eq!(
            (refused.run_id, refused.interaction_id, refused.input_id),
            (Some(run.run_id), Some(input.interaction_id), None)
        );
        assert!(
            storage::find_input(&fixture.fixture.conn, run.run_id, &input.operation_key)
                .unwrap()
                .is_none()
        );
        let previous = original.interaction.as_ref().unwrap().evidence_json["screen_seq"]
            .as_u64()
            .unwrap();
        let process = ProcessIdentity::read(run.owner_pid).unwrap().unwrap();
        assert_eq!(process.start_identity, run.owner_start_identity);
        assert!(process.matches().unwrap(), "generation-bound owned runner");
        assert_eq!(i64::from(fixture.child.id()), process.pid);
        fixture.resize(24, 81);
        let resized = loop {
            let current = inspect_permission(
                &mut fixture.fixture.conn,
                &settings,
                &PermissionInspectInput {
                    event_id: input.event_id,
                    interaction_id: input.interaction_id,
                    director_member_id: run.director_member_id,
                },
                Deadline::until(until),
            )
            .unwrap();
            if current.output_seq > previous {
                break current;
            }
            assert!(
                Instant::now() < until,
                "actual runner observes the resize under the original command budget"
            );
            std::thread::sleep(
                Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
            );
        };
        assert_eq!(
            resized.request_token.as_deref(),
            Some(input.request_token.as_str()),
            "an equivalent resize keeps the request token"
        );
        let after_resize = storage::get_event(&fixture.fixture.conn, input.event_id).unwrap();
        assert_eq!(after_resize.event, original.event);
        assert_eq!(
            after_resize.interaction, original.interaction,
            "an equivalent resize schedules no evidence refresh"
        );
        let result = respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(until),
            &mut host,
        );
        assert_eq!(
            host.calls,
            [1, 1, 1],
            "the retained token after an equivalent resize permits one exchange: {:?}",
            result.as_ref().err()
        );
        let result = result.unwrap();
        assert_eq!(host.requests[0]["kind"], "permission_respond");
        for field in [
            "answer_profile_version",
            "answer_rule_id",
            "bytes",
            "stages",
        ] {
            assert!(host.requests[0].get(field).is_none());
        }
        let receipt = service_response_audit(&fixture, &input);
        assert_eq!(
            (
                result.submission.status,
                result.submission.stage,
                result.submission.input_id
            ),
            (
                SubmissionStatus::Accepted,
                SubmissionStage::Submitted,
                Some(receipt.input.input_id)
            )
        );
        assert_eq!(
            (
                result.member_id,
                result.run_id,
                result.owner_epoch,
                result.interaction_id,
                result.event_id,
                result.decision,
                result.decision_source
            ),
            (
                run.member_id,
                run.run_id,
                run.owner_epoch,
                input.interaction_id,
                input.event_id,
                input.decision,
                input.decision_source
            )
        );
        assert!(result.submission.output_seq.is_some());
        native_answers_through_barrier(
            &mut fixture,
            output_start,
            "response-live-barrier",
            &["review-answer=y"],
        );
        fixture.finish();
    }

    #[test]
    fn permission_response_actual_service_lost_reply_retains_audit_and_same_key_never_retypes() {
        let mut fixture = ServiceFixture::configured(false, response_services);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        let (settings, run, placement, input) =
            response_command(&mut fixture, until, "response:actual-lost-reply");
        let output_start = fixture.output.len();
        let mut host = SocketHost::new(&settings, &placement, until);
        host.reply = ReplyMode::Disconnected;
        let failure = match respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(until),
            &mut host,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("owned disconnected consumer receives no fabricated success"),
        };
        assert!(
            failure
                .source
                .message()
                .contains("owned reply consumer closed")
        );
        assert_eq!(
            (failure.run_id, failure.interaction_id),
            (Some(run.run_id), Some(input.interaction_id))
        );
        assert_eq!(
            failure.operation_key.as_deref(),
            Some(input.operation_key.as_str())
        );
        assert!(!failure.source.message().contains(&input.request_token));
        assert_eq!(host.calls, [1, 1, 1]);
        assert_eq!(host.requests[0]["kind"], "permission_respond");
        let receipt = service_response_audit(&fixture, &input);
        let mut receipts = SocketHost::new(&settings, &placement, until);
        receipts.exchange_limit = 3;
        let settled = reconcile_ordinary_with_host(
            &mut fixture.fixture.conn,
            &settings,
            receipt.input.input_id,
            InputDisposition::Submitted,
            "fixture:permission-submitted",
            Deadline::until(until),
            &mut receipts,
        )
        .unwrap();
        assert_eq!(receipts.calls, [1, 1, 3]);
        assert_eq!(
            receipts
                .requests
                .iter()
                .map(|request| request["action"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["query", "reconcile", "retire"]
        );
        assert_eq!(settled.input_id, receipt.input.input_id);
        assert_eq!(settled.state, InputState::Settled);
        assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
        assert_eq!(
            settled.evidence_ref.as_deref(),
            Some("fixture:permission-submitted")
        );
        let settled_receipt =
            storage::get_permission_response(&fixture.fixture.conn, receipt.input.input_id)
                .unwrap();
        assert_eq!(settled_receipt.response, receipt.response);
        assert_eq!(settled_receipt.claim, receipt.claim);
        let changes = fixture.fixture.conn.total_changes();
        let held =
            PaneLock::acquire_within(&settings.database_url, &placement, Deadline::until(until))
                .unwrap()
                .unwrap();
        let repeated = respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(until),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(
            (
                repeated.submission.status,
                repeated.submission.stage,
                repeated.submission.input_id
            ),
            (
                SubmissionStatus::Accepted,
                SubmissionStage::Submitted,
                Some(receipt.input.input_id)
            )
        );
        assert_eq!(
            repeated.owner_epoch,
            receipt.response.metadata.expected_owner_epoch
        );
        assert_eq!(repeated.submission.output_seq, None);
        let repeated_settlement = reconcile_ordinary_with_host(
            &mut fixture.fixture.conn,
            &settings,
            receipt.input.input_id,
            InputDisposition::Submitted,
            "fixture:replacement-reference",
            Deadline::until(until),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(repeated_settlement, settled);
        assert!(
            reconcile_ordinary_with_host(
                &mut fixture.fixture.conn,
                &settings,
                receipt.input.input_id,
                InputDisposition::AbandonedUncertain,
                "fixture:conflicting-reference",
                Deadline::until(until),
                &mut NoLiveHost,
            )
            .is_err()
        );
        assert_eq!(fixture.fixture.conn.total_changes(), changes);
        assert_eq!(
            storage::get_permission_response(&fixture.fixture.conn, receipt.input.input_id)
                .unwrap(),
            settled_receipt
        );
        drop(held);
        native_answers_through_barrier(
            &mut fixture,
            output_start,
            "response-lost-barrier",
            &["review-answer=y"],
        );
        fixture.finish();
    }

    #[test]
    fn permission_receipt_postcommit_expiry_preserves_claim_context_and_consumed_history() {
        let mut fixture = ServiceFixture::configured(false, response_services);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        let (settings, run, placement, input) =
            response_command(&mut fixture, until, "response:receipt-postcommit-expiry");
        let output_start = fixture.output.len();
        respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(until),
            &mut SocketHost::new(&settings, &placement, until),
        )
        .unwrap();
        let original = service_response_audit(&fixture, &input);
        let original_expiry = until;
        let mut host = SocketHost::new(&settings, &placement, original_expiry);
        host.exchange_limit = 3;
        host.expire_retirement = true;
        let failure = reconcile_ordinary_with_host(
            &mut fixture.fixture.conn,
            &settings,
            original.input.input_id,
            InputDisposition::Submitted,
            "fixture:permission-committed-before-expiry",
            Deadline::until(original_expiry),
            &mut host,
        )
        .unwrap_err();
        assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
        assert_eq!(
            (failure.run_id, failure.interaction_id, failure.input_id),
            (
                Some(run.run_id),
                Some(input.interaction_id),
                Some(original.input.input_id)
            )
        );
        assert_eq!(
            failure.operation_key.as_deref(),
            Some(input.operation_key.as_str())
        );
        assert!(!failure.source.message().contains(&input.request_token));
        assert!(Instant::now() >= original_expiry);
        assert_eq!(host.calls, [1, 1, 3]);
        let settled =
            storage::get_permission_response(&fixture.fixture.conn, original.input.input_id)
                .unwrap();
        assert_eq!(settled.input.state, InputState::Settled);
        assert_eq!(settled.input.disposition, Some(InputDisposition::Submitted));
        assert_eq!(
            settled.input.evidence_ref.as_deref(),
            Some("fixture:permission-committed-before-expiry")
        );
        assert_eq!(settled.response, original.response);
        assert_eq!(settled.claim, original.claim);
        let history_until = Instant::now() + Duration::from_secs(1);
        let held = PaneLock::acquire_within(
            &settings.database_url,
            &placement,
            Deadline::until(history_until),
        )
        .unwrap()
        .unwrap();
        let changes = fixture.fixture.conn.total_changes();
        let repeated = reconcile_ordinary_with_host(
            &mut fixture.fixture.conn,
            &settings,
            original.input.input_id,
            InputDisposition::Submitted,
            "fixture:later-reference",
            Deadline::until(history_until),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(repeated, settled.input);
        let history = respond_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &input,
            Deadline::until(history_until),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(
            (history.submission.status, history.submission.input_id),
            (SubmissionStatus::Accepted, Some(original.input.input_id))
        );
        assert_eq!(fixture.fixture.conn.total_changes(), changes);
        assert_eq!(
            storage::get_permission_response(&fixture.fixture.conn, original.input.input_id)
                .unwrap(),
            settled
        );
        drop(held);
        native_answers_through_barrier(
            &mut fixture,
            output_start,
            "permission-receipt-expiry-barrier",
            &["review-answer=y"],
        );
        fixture.finish();
    }

    #[test]
    fn permission_receipt_live_failure_preserves_original_error_without_exclusive_effects() {
        use crate::runtime::runner::input_client::test_support::{Case, Retained};
        use crate::runtime::runner::input_recovery::test_support::TypedHost;

        let mut case = Case::new(Retained::Accepted);
        let original = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
        let run = storage::get_run(&case.fixture.conn, original.input.run_id).unwrap();
        let records = health_records(&case.fixture.conn);
        let placement = crate::broker::get_member(&case.fixture.conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        let mut live = UnavailableReceiptHost {
            settings: case.settings.clone(),
            placement: placement.clone(),
            until,
            input_id: case.input_id,
            change_digest: false,
            calls: [0; 3],
        };
        let mut recovery = TypedHost::new(until);
        let failure = reconcile_receipt_with_hosts(
            &mut case.fixture.conn,
            &case.settings,
            case.input_id,
            InputDisposition::Submitted,
            "fixture:live-only-permission-receipt",
            Deadline::until(until),
            (&mut live, &mut recovery),
        )
        .unwrap_err();
        assert_eq!(live.calls, [1, 1, 0]);
        assert_eq!(failure.source.message(), "owned first live receipt failure");
        assert_eq!(
            (failure.run_id, failure.interaction_id, failure.input_id),
            (
                Some(run.run_id),
                Some(original.response.metadata.interaction_id),
                Some(case.input_id)
            )
        );
        assert_eq!(
            failure.operation_key.as_deref(),
            Some(original.input.operation_key.as_str())
        );
        assert_eq!(recovery.calls, vec!["snapshot"]);
        assert!(recovery.processes.is_empty());
        assert!(recovery.startups.is_empty());
        assert_eq!(
            storage::get_run(&case.fixture.conn, run.run_id).unwrap(),
            run
        );
        assert_eq!(
            storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
            original
        );
        assert_eq!(health_records(&case.fixture.conn), records);
        PaneLock::acquire_released(&case.settings.database_url, &placement)
            .unwrap()
            .expect("failed live invocation releases its genuine pane guard")
            .validate_binding(&case.settings.database_url, &placement)
            .unwrap();
    }

    #[test]
    fn exclusive_permission_receipt_client_preserves_uncertain_claim_and_continuous_guard() {
        use crate::runtime::runner::input_client::test_support::{Case, Retained};
        use crate::runtime::runner::input_recovery::test_support::TypedHost;

        for absent in [false, true] {
            let mut case = Case::new(Retained::Uncertain);
            case.fixture.conn.execute(
                "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=7,ended_at='2026-10-27T18:13:00Z' WHERE run_id=?1",
                [case.fixture.run.run_id],
            ).unwrap();
            let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
            let original =
                storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
            let placement =
                crate::broker::get_member(&case.fixture.conn, run.member_id, run.fleet_id)
                    .unwrap()
                    .unwrap()
                    .placement
                    .unwrap();
            if absent {
                assert!(
                    crate::broker::deregister_member(&mut case.fixture.conn, run.member_id)
                        .unwrap()
                );
            }
            let before = health_records(&case.fixture.conn);
            drop(case._lock);
            let until = Instant::now() + Duration::from_secs(2);
            let mut live = UnavailableReceiptHost {
                settings: case.settings.clone(),
                placement: placement.clone(),
                until,
                input_id: case.input_id,
                change_digest: false,
                calls: [0; 3],
            };
            let mut recovery = TypedHost::new(until);
            let settings = case.settings.clone();
            let pinned = placement.clone();
            recovery.action = Some(Box::new(move |_, _, actual| {
                assert_eq!(actual, until);
                if !absent {
                    assert!(
                        PaneLock::try_acquire(&settings.database_url, &pinned)
                            .unwrap()
                            .is_none()
                    );
                }
                Ok(())
            }));
            let result =
                with_database_budget(&mut case.fixture.conn, Deadline::until(until), |conn| {
                    Ok(reconcile_receipt_with_hosts(
                        conn,
                        &case.settings,
                        case.input_id,
                        InputDisposition::NotSubmitted,
                        "fixture:exclusive-uncertain-permission",
                        Deadline::until(until + Duration::from_secs(1)),
                        (&mut live, &mut recovery),
                    ))
                })
                .unwrap()
                .unwrap();
            assert_eq!(live.calls, if absent { [0; 3] } else { [1, 1, 0] });
            let current = storage::get_run(&case.fixture.conn, run.run_id).unwrap();
            let mut expected_run = run.clone();
            expected_run.owner_epoch += 1;
            expected_run.owner_kind = OwnerKind::Reconciler;
            let process = ProcessIdentity::current().unwrap();
            expected_run.owner_pid = process.pid;
            expected_run.owner_start_identity = process.start_identity;
            assert_eq!(current, expected_run);
            assert!(recovery.calls.contains(&"after"));
            assert!(
                recovery
                    .startups
                    .iter()
                    .any(|(checked, actual)| checked == &run
                        && actual.as_ref() == if absent { None } else { Some(&placement) })
            );
            let receipt =
                storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
            assert_eq!(result, receipt.input);
            let mut expected = original.input.clone();
            expected.state = InputState::Settled;
            expected.disposition = Some(InputDisposition::NotSubmitted);
            expected.evidence_ref = Some("fixture:exclusive-uncertain-permission".into());
            expected.updated_at = result.updated_at.clone();
            assert_eq!(result, expected);
            assert_eq!(receipt.response, original.response);
            assert_eq!(receipt.claim, original.claim);
            assert!(
                receipt.claim.is_some(),
                "owner exclusion is no zero-byte fact for Uncertain history"
            );
            assert_eq!(health_records(&case.fixture.conn), before);
            let changes = case.fixture.conn.total_changes();
            let repeated = reconcile_ordinary_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.input_id,
                InputDisposition::NotSubmitted,
                "fixture:later-unsubstituted-evidence",
                Deadline::until(until),
                &mut NoLiveHost,
            )
            .unwrap();
            assert_eq!(repeated, result);
            assert_eq!(case.fixture.conn.total_changes(), changes);
            assert_eq!(
                storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
                receipt
            );
            if !absent {
                PaneLock::acquire_released(&case.settings.database_url, &placement)
                    .unwrap()
                    .unwrap()
                    .validate_binding(&case.settings.database_url, &placement)
                    .unwrap();
            }
            RunLock::acquire_released(&case.fixture.conn, &current.owner())
                .unwrap()
                .unwrap()
                .validate(&case.fixture.conn, &current.owner())
                .unwrap();
        }
    }

    struct UnavailableReceiptHost {
        settings: Settings,
        placement: Placement,
        until: Instant,
        input_id: i64,
        change_digest: bool,
        calls: [usize; 3],
    }

    impl UnavailableReceiptHost {
        fn locked(&self, until: Instant) {
            assert_eq!(until, self.until);
            Deadline::until(until).remaining().unwrap();
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none()
            );
        }
    }

    impl InputCommandHost for UnavailableReceiptHost {
        fn lock(
            &mut self,
            settings: &Settings,
            _: &RunRecord,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            assert_eq!(settings.database_url, self.settings.database_url);
            assert_eq!(placement, &self.placement);
            assert_eq!(deadline.instant(), self.until);
            self.calls[0] += 1;
            assert_eq!(
                self.calls[0], 1,
                "one genuine pane acquisition spans both routes"
            );
            PaneLock::acquire_within(&settings.database_url, placement, deadline)
                .map_err(host_failure)
        }

        fn health(
            &mut self,
            _: &RunRecord,
            until: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            self.locked(until);
            self.calls[1] += 1;
            if self.change_digest {
                let path = crate::db::database_path(&self.settings.database_url).unwrap();
                let conn = Connection::open(path).unwrap();
                assert_eq!(
                    conn.execute(
                        "UPDATE member_input_operations SET payload_digest=?1 WHERE input_id=?2",
                        rusqlite::params!["e".repeat(64), self.input_id],
                    )
                    .unwrap(),
                    1
                );
            }
            Err(host_failure(error("owned first live receipt failure")))
        }

        fn exchange(
            &mut self,
            _: &RunRecord,
            _: &InputControlRequest,
            _: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
            self.calls[2] += 1;
            panic!("an unavailable health endpoint grants no native or receipt exchange")
        }
    }

    #[test]
    fn exclusive_receipt_client_keeps_one_guard_original_row_and_error_across_live_fallback() {
        use crate::runtime::runner::input_recovery::test_support::{Termination, TypedHost};

        for mode in [
            "eligible",
            "absent_placement",
            "startup_unknown",
            "changed_input",
            "settlement_failure",
        ] {
            let History {
                mut fixture,
                settings,
                placement,
                row,
                _artifacts,
                _run_lock,
                ..
            } = History::new(InputState::Uncertain, None);
            fixture.conn.execute(
                "UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=7,ended_at='2026-10-27T18:13:00Z' WHERE run_id=?1",
                [fixture.run.run_id],
            ).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            drop(_run_lock);
            let absent = mode == "absent_placement";
            if absent {
                assert!(
                    crate::broker::deregister_member(&mut fixture.conn, fixture.run.member_id)
                        .unwrap()
                );
            }
            let original = fixture.run.clone();
            let history = health_records(&fixture.conn);
            let until = Instant::now() + Duration::from_secs(2);
            let mut live = UnavailableReceiptHost {
                settings: settings.clone(),
                placement: placement.clone(),
                until,
                input_id: row.input_id,
                change_digest: mode == "changed_input",
                calls: [0; 3],
            };
            let mut recovery = TypedHost::new(until);
            if mode == "startup_unknown" {
                recovery.startup = Termination::Unknown;
            }
            let locked_settings = settings.clone();
            let locked_placement = placement.clone();
            recovery.action = Some(Box::new(move |_, _, actual| {
                assert_eq!(actual, until);
                if !absent {
                    assert!(
                        PaneLock::try_acquire(&locked_settings.database_url, &locked_placement)
                            .unwrap()
                            .is_none()
                    );
                }
                Ok(())
            }));
            if mode == "settlement_failure" {
                fixture.conn.execute_batch("CREATE TRIGGER routed_receipt_failure BEFORE UPDATE OF state ON member_input_operations WHEN NEW.state='settled' BEGIN SELECT RAISE(FAIL,'owned routed settlement failure'); END").unwrap();
            }
            let result = with_database_budget(&mut fixture.conn, Deadline::until(until), |conn| {
                Ok(reconcile_receipt_with_hosts(
                    conn,
                    &settings,
                    row.input_id,
                    InputDisposition::Submitted,
                    "fixture:routed-receipt",
                    Deadline::until(until + Duration::from_secs(1)),
                    (&mut live, &mut recovery),
                ))
            });
            let removed = if mode == "settlement_failure" {
                fixture
                    .conn
                    .execute_batch("DROP TRIGGER routed_receipt_failure")
            } else {
                Ok(())
            };
            assert!(
                removed.is_ok(),
                "primary={:?}; trigger_cleanup={removed:?}",
                result.as_ref().map(|value| value.as_ref().err())
            );
            let result = result.unwrap();
            assert_eq!(
                live.calls,
                if absent { [0; 3] } else { [1, 1, 0] },
                "{mode}"
            );
            let actual = storage::get_input(&fixture.conn, row.input_id).unwrap();
            let current = storage::get_run(&fixture.conn, original.run_id).unwrap();
            if matches!(mode, "eligible" | "absent_placement") {
                assert_eq!(result.unwrap(), actual);
                assert_eq!(actual.state, InputState::Settled);
                assert_eq!(actual.disposition, Some(InputDisposition::Submitted));
                assert_eq!(
                    actual.evidence_ref.as_deref(),
                    Some("fixture:routed-receipt")
                );
                let process = ProcessIdentity::current().unwrap();
                let mut expected = original.clone();
                expected.owner_epoch += 1;
                expected.owner_kind = OwnerKind::Reconciler;
                expected.owner_pid = process.pid;
                expected.owner_start_identity = process.start_identity;
                assert_eq!(current, expected);
                assert!(recovery.calls.contains(&"after"));
                assert!(recovery.processes.contains(&ProcessIdentity {
                    pid: original.owner_pid,
                    start_identity: original.owner_start_identity.clone()
                }));
                assert!(recovery.startups.iter().any(|(run, pinned)| {
                    run == &original
                        && pinned.as_ref() == if absent { None } else { Some(&placement) }
                }));
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.input_id, Some(row.input_id));
                assert_eq!(failure.run_id, Some(row.run_id));
                assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(row.operation_key.as_str())
                );
                if mode == "startup_unknown" {
                    assert_eq!(
                        failure.source.to_string(),
                        "owned first live receipt failure"
                    );
                    assert!(recovery.calls.contains(&"startup"));
                } else if mode == "settlement_failure" {
                    assert!(
                        failure
                            .source
                            .to_string()
                            .contains("owned routed settlement failure")
                    );
                    assert!(recovery.calls.contains(&"after"));
                }
                let mut expected = row.clone();
                if mode == "changed_input" {
                    expected.payload_digest = "e".repeat(64);
                    assert!(!recovery.calls.contains(&"after"));
                }
                assert_eq!(
                    actual, expected,
                    "{mode}: fallback retains the original input instead of recapturing it"
                );
                assert_eq!(current, original);
            }
            assert_eq!(health_records(&fixture.conn), history);
            if !absent {
                PaneLock::acquire_released(&settings.database_url, &placement)
                    .unwrap()
                    .expect("the acquired pane guard is released on every routed outcome")
                    .validate_binding(&settings.database_url, &placement)
                    .unwrap();
            }
            RunLock::acquire_released(&fixture.conn, &current.owner())
                .unwrap()
                .expect("the original namespace is released after settlement or refusal")
                .validate(&fixture.conn, &current.owner())
                .unwrap();
        }
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

    struct NoLiveHost;
    impl InputCommandHost for NoLiveHost {
        fn lock(
            &mut self,
            _: &Settings,
            _: &RunRecord,
            _: &Placement,
            _: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            panic!("historical or invalid input reaches no pane wait")
        }
        fn health(
            &mut self,
            _: &RunRecord,
            _: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            panic!("historical or invalid input reaches no health probe")
        }
        fn exchange(
            &mut self,
            _: &RunRecord,
            _: &InputControlRequest,
            _: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
            panic!("historical or invalid input reaches no socket exchange")
        }
    }

    enum ReplyMode {
        Live,
        UnknownId,
        Disconnected,
    }

    struct SocketHost {
        settings: Settings,
        placement: Placement,
        until: Instant,
        expiration: Option<u64>,
        calls: [usize; 3],
        requests: Vec<Value>,
        reply: ReplyMode,
        after_health: Option<Box<dyn FnMut()>>,
        health_reply: Option<HealthSnapshot>,
        exchange_limit: usize,
        expire_retirement: bool,
        wrong_query_id: Option<i64>,
        skip_reconcile: bool,
    }

    fn service_target(fixture: &ServiceFixture) -> (Settings, RunRecord, Placement) {
        let conn = &fixture.fixture.conn;
        let run = storage::get_run(conn, fixture.fixture.run.run_id).unwrap();
        let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        (
            Settings::for_database(std::path::Path::new(conn.path().unwrap()), Some("tmux")),
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
            }
        }
    }

    impl InputCommandHost for SocketHost {
        fn lock(
            &mut self,
            settings: &Settings,
            _: &RunRecord,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            self.calls[0] += 1;
            assert_eq!(deadline.instant(), self.until);
            PaneLock::acquire_within(&settings.database_url, placement, deadline)
                .map_err(host_failure)
        }

        fn health(
            &mut self,
            run: &RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            self.calls[1] += 1;
            assert_eq!(deadline, self.until);
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none()
            );
            let health = match self.health_reply.take() {
                Some(health) => health,
                None => super::super::probe_health(run, deadline).map_err(host_failure)?,
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
        ) -> std::result::Result<InputControlReply, InputFailure> {
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
                    &super::super::input_protocol::encode_input_request(request).unwrap()[4..],
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
            let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)
                .map_err(host_failure)?;
            let mut client =
                super::super::control::ControlClient::connect(&paths.control_socket, deadline)
                    .map_err(host_failure)?;
            if matches!(self.reply, ReplyMode::Disconnected) {
                client
                    .stream
                    .shutdown(Shutdown::Read)
                    .map_err(io_error)
                    .map_err(host_failure)?;
                let bytes = super::super::input_protocol::encode_input_request(request)
                    .map_err(host_failure)?;
                let mut offset = 0;
                while offset < bytes.len() {
                    check_deadline(Instant::now(), deadline).map_err(host_failure)?;
                    match client.stream.write(&bytes[offset..]) {
                        Ok(0) => panic!("owned external request disconnected during send"),
                        Ok(count) => offset += count,
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            super::super::control::pause(deadline).map_err(host_failure)?;
                        }
                        Err(cause) => return Err(host_failure(io_error(cause))),
                    }
                }
                let key = match &request.operation {
                    InputControlOperation::SubmitPrompt(input)
                    | InputControlOperation::SubmitExec(input) => &input.operation_key,
                    InputControlOperation::PermissionRespond(input) => &input.operation_key,
                    _ => panic!("disconnected fixture sends a durable input operation"),
                };
                let mut observed = crate::db::connect(&self.settings.database_url).unwrap();
                loop {
                    let retained =
                        with_database_budget(&mut observed, Deadline::until(deadline), |conn| {
                            storage::find_input(conn, run.run_id, key)
                        })
                        .map_err(host_failure)?;
                    if retained.is_some_and(|input| input.state == InputState::Accepted) {
                        break;
                    }
                    super::super::control::pause(deadline).map_err(host_failure)?;
                }
                drop(client);
                return Err(host_failure(CafleetError::App(
                    "owned reply consumer closed".into(),
                )));
            }
            let mut reply = client
                .exchange_input(request, deadline)
                .map_err(host_failure)?;
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
            Ok(reply)
        }
    }

    fn ordinary(run: &RunRecord, key: &str) -> SubmitInput {
        SubmitInput {
            operation_key: format!("prompt:{}:{key}", run.run_id),
            input_kind: InputKind::Prompt,
            message_id: None,
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
            Deadline::until(outer.min(Instant::now() + Duration::from_secs(2))),
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
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{Arc, Mutex};

        let mut case = History::new(InputState::Settled, Some(InputDisposition::Submitted));
        case.fixture.conn.execute("UPDATE members SET status='deregistered',deregistered_at='2026-10-27T17:26:00Z' WHERE member_id=?1", [case.fixture.run.member_id]).unwrap();
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let changes = case.fixture.conn.total_changes();
        let before_run = storage::get_run(&case.fixture.conn, case.row.run_id).unwrap();
        let records = health_records(&case.fixture.conn);
        let budgets = Arc::new(Mutex::new(Vec::new()));
        let observed = budgets.clone();
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
                    observed
                        .lock()
                        .unwrap()
                        .push(Deadline::inherited_or_new().instant());
                }
                Authorization::Allow
            }))
            .unwrap();
        let expired = reconcile_ordinary_with_host(
            &mut case.fixture.conn,
            &case.settings,
            case.row.input_id,
            InputDisposition::Submitted,
            "fixture:replacement-evidence",
            Deadline::until(Instant::now()),
            &mut NoLiveHost,
        );
        let expired_reads = budgets.lock().unwrap().len();
        budgets.lock().unwrap().clear();
        let original = Instant::now() + Duration::from_secs(2);
        let result =
            with_database_budget(&mut case.fixture.conn, Deadline::until(original), |conn| {
                Ok(reconcile_ordinary_with_host(
                    conn,
                    &case.settings,
                    case.row.input_id,
                    InputDisposition::Submitted,
                    "fixture:replacement-evidence",
                    Deadline::until(original + Duration::from_secs(2)),
                    &mut NoLiveHost,
                ))
            });
        let removed = case
            .fixture
            .conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        assert!(
            removed.is_ok(),
            "expired_error={:?}; history_error={:?}; authorizer_cleanup={removed:?}",
            expired.as_ref().err(),
            result.as_ref().map(|value| value.as_ref().err())
        );
        let failure = expired.unwrap_err();
        assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
        assert_eq!(expired_reads, 0);
        assert_eq!(
            result.unwrap().unwrap(),
            case.row,
            "same disposition preserves the actual original row and evidence"
        );
        let observed = budgets.lock().unwrap();
        assert!(!observed.is_empty());
        assert!(observed.iter().all(|deadline| *deadline <= original));
        drop(observed);
        assert!(
            reconcile_ordinary_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.row.input_id,
                InputDisposition::AbandonedUncertain,
                "fixture:conflicting-disposition",
                Deadline::until(original),
                &mut NoLiveHost
            )
            .is_err()
        );
        let foreign = Fixture::new();
        let foreign_settings = Settings {
            database_url: format!("sqlite:///{}", foreign.conn.path().unwrap()),
            ..case.settings.clone()
        };
        assert!(
            reconcile_ordinary_with_host(
                &mut case.fixture.conn,
                &foreign_settings,
                case.row.input_id,
                InputDisposition::Submitted,
                "fixture:foreign-database",
                Deadline::until(original),
                &mut NoLiveHost
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
                reconcile_ordinary_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    input_id,
                    InputDisposition::Submitted,
                    evidence,
                    Deadline::until(original),
                    &mut NoLiveHost
                )
                .is_err()
            );
        }
        case.assert_unchanged(changes);

        use crate::runtime::runner::input_client::test_support::{Case, Retained};
        let mut permission = Case::new(Retained::Submitted);
        let audit = storage::get_permission_response(&permission.fixture.conn, permission.input_id)
            .unwrap();
        let changes = permission.fixture.conn.total_changes();
        let retained = reconcile_ordinary_with_host(
            &mut permission.fixture.conn,
            &permission.settings,
            permission.input_id,
            InputDisposition::Submitted,
            "fixture:replacement-permission-reference",
            Deadline::until(Instant::now() + Duration::from_secs(1)),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(retained, audit.input);
        assert!(
            reconcile_ordinary_with_host(
                &mut permission.fixture.conn,
                &permission.settings,
                permission.input_id,
                InputDisposition::NotSubmitted,
                "fixture:conflicting-permission-reference",
                Deadline::until(Instant::now() + Duration::from_secs(1)),
                &mut NoLiveHost,
            )
            .is_err()
        );
        assert_eq!(permission.fixture.conn.total_changes(), changes);
        assert_eq!(
            storage::get_permission_response(&permission.fixture.conn, permission.input_id)
                .unwrap(),
            audit
        );
    }

    #[test]
    fn live_receipt_client_binds_query_and_commit_ack_before_one_real_retirement() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{Arc, Mutex};

        with_input_service(|fixture, outer| {
            let (settings, run, placement, request, original) =
                accepted_service_receipt(fixture, outer, "receipt-live");
            let other = storage::reserve_input(
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
            let failure = reconcile_ordinary_with_host(
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
            assert_eq!(failure.input_id, Some(original.input_id));
            assert_eq!(
                failure.operation_key.as_deref(),
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
                reconcile_ordinary_with_host(
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
            let budgets = Arc::new(Mutex::new(Vec::new()));
            let observed = budgets.clone();
            fixture
                .fixture
                .conn
                .authorizer(Some(move |context: AuthContext<'_>| {
                    if matches!(
                        context.action,
                        AuthAction::Read {
                            table_name: "member_input_operations",
                            ..
                        }
                    ) {
                        observed
                            .lock()
                            .unwrap()
                            .push(Deadline::inherited_or_new().instant());
                    }
                    Authorization::Allow
                }))
                .unwrap();
            let mut host = SocketHost::new(&settings, &placement, until);
            host.exchange_limit = 3;
            let settled =
                with_database_budget(&mut fixture.fixture.conn, Deadline::until(until), |conn| {
                    Ok(reconcile_ordinary_with_host(
                        conn,
                        &settings,
                        original.input_id,
                        InputDisposition::Submitted,
                        "fixture:live-reconcile",
                        Deadline::until(outer),
                        &mut host,
                    ))
                });
            let removed = fixture
                .fixture
                .conn
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
            assert!(
                removed.is_ok(),
                "settlement_error={:?}; authorizer_cleanup={removed:?}",
                settled.as_ref().map(|value| value.as_ref().err())
            );
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
            let observed = budgets.lock().unwrap();
            assert!(
                !observed.is_empty(),
                "actual client receipt reads use the supplied connection"
            );
            assert!(observed.iter().all(|deadline| *deadline <= until));
            drop(observed);
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
                &mut NoLiveHost,
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

    #[test]
    fn live_receipt_client_postcommit_expiry_retains_settlement_context_and_no_live_retry() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement, request, original) =
                accepted_service_receipt(fixture, outer, "receipt-expiry");
            let until = outer.min(Instant::now() + Duration::from_secs(2));
            let mut host = SocketHost::new(&settings, &placement, until);
            host.exchange_limit = 3;
            host.expire_retirement = true;
            let failure = reconcile_ordinary_with_host(
                &mut fixture.fixture.conn,
                &settings,
                original.input_id,
                InputDisposition::Submitted,
                "fixture:committed-before-expiry",
                Deadline::until(until),
                &mut host,
            )
            .unwrap_err();
            assert!(matches!(failure.source, CafleetError::DeadlineExceeded));
            assert_eq!(failure.run_id, Some(run.run_id));
            assert_eq!(failure.input_id, Some(original.input_id));
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
            assert!(!failure.source.to_string().contains(&request.literal));
            assert_eq!(host.calls, [1, 1, 3]);
            let settled = storage::get_input(&fixture.fixture.conn, original.input_id).unwrap();
            assert_eq!(settled.state, InputState::Settled);
            assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
            assert_eq!(
                settled.evidence_ref.as_deref(),
                Some("fixture:committed-before-expiry")
            );
            let repeated = reconcile_ordinary_with_host(
                &mut fixture.fixture.conn,
                &settings,
                original.input_id,
                InputDisposition::Submitted,
                "fixture:preserve-original-evidence",
                Deadline::until(outer),
                &mut NoLiveHost,
            )
            .unwrap();
            assert_eq!(repeated, settled);
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
            let artifacts = fixture.artifacts();
            let run_lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
            let owner = run_lock
                .transfer_to_runner(
                    &mut fixture.conn,
                    &fixture.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
            assert_eq!(
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
                        rusqlite::params!["2026-10-27T14:00:00.000000+00:00", owner.run_id],
                    )
                    .unwrap(),
                1
            );
            fixture.run = storage::get_run(&fixture.conn, owner.run_id).unwrap();
            let request = SubmitInput {
                operation_key: format!("prompt:{}:{}", owner.run_id, "b".repeat(64)),
                input_kind: InputKind::Prompt,
                message_id: None,
                literal: "retained ordinary λ literal".into(),
            };
            let row = storage::reserve_input(
                &mut fixture.conn,
                &owner,
                &NewInput {
                    operation_key: request.operation_key.clone(),
                    kind: request.input_kind,
                    payload_digest: crate::broker::runtime::input_payload_digest(
                        owner.run_id,
                        &request,
                    ),
                    message_id: request.message_id,
                },
            )
            .unwrap();
            if let Some(disposition) = disposition {
                storage::settle_input(
                    &mut fixture.conn,
                    &owner,
                    row.input_id,
                    InputState::InProgress,
                    disposition,
                    "fixture:external-history",
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
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            let settings = Settings::for_database(
                std::path::Path::new(fixture.conn.path().unwrap()),
                Some("tmux"),
            );
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
                InputState::InProgress,
                None,
                SubmissionStatus::Uncertain,
                SubmissionStage::Unknown,
            ),
            (
                InputState::Uncertain,
                None,
                SubmissionStatus::Uncertain,
                SubmissionStage::Unknown,
            ),
            (
                InputState::Settled,
                Some(InputDisposition::Submitted),
                SubmissionStatus::Accepted,
                SubmissionStage::Submitted,
            ),
            (
                InputState::Settled,
                Some(InputDisposition::AbandonedUncertain),
                SubmissionStatus::Rejected,
                SubmissionStage::ZeroBytes,
            ),
        ] {
            let mut case = History::new(state, disposition);
            case.fixture.conn.execute(
                "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                rusqlite::params!["2026-10-27T14:01:00.000000+00:00", case.fixture.run.member_id],
            ).unwrap();
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
                    Deadline::until(Instant::now() + Duration::from_secs(1))
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
                &mut NoLiveHost,
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
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{Arc, Mutex};

        let mut case = History::new(InputState::Accepted, None);
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        let changes = case.fixture.conn.total_changes();
        let control_deadline = Instant::now() + Duration::from_secs(2);
        let run_before = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
        let records_before = health_records(&case.fixture.conn);
        let control = PreparedSubmission::new(
            &case.request,
            control_deadline,
            &mut super::super::RealInputDeadlineClock,
        )
        .unwrap();
        let result = submit_wrapped_input_under_pane_lock_with_host(
            &mut case.fixture.conn,
            &case.settings,
            &case.fixture.run,
            &case.placement,
            &held,
            &control,
            Deadline::until(control_deadline),
            &mut NoLiveHost,
        )
        .unwrap();
        assert_eq!(result.status, SubmissionStatus::Accepted);
        assert_eq!(result.input_id, Some(case.row.input_id));

        let original = Instant::now() + Duration::from_millis(100);
        let expired_attempt = PreparedSubmission::new(
            &case.request,
            original,
            &mut super::super::RealInputDeadlineClock,
        )
        .unwrap();
        std::thread::sleep(
            original.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        assert!(
            Instant::now() >= original,
            "the checked prepared attempt actually expired"
        );
        let budgets = Arc::new(Mutex::new(Vec::new()));
        let observed = budgets.clone();
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
                    observed
                        .lock()
                        .unwrap()
                        .push(Deadline::inherited_or_new().instant());
                }
                Authorization::Allow
            }))
            .unwrap();
        let expired = submit_wrapped_input_under_pane_lock_with_host(
            &mut case.fixture.conn,
            &case.settings,
            &case.fixture.run,
            &case.placement,
            &held,
            &expired_attempt,
            Deadline::until(Instant::now() + Duration::from_secs(3)),
            &mut NoLiveHost,
        );
        let expired_reads = budgets.lock().unwrap().len();
        budgets.lock().unwrap().clear();

        let original_active = Instant::now() + Duration::from_secs(1);
        let active_attempt = PreparedSubmission::new(
            &case.request,
            original_active,
            &mut super::super::RealInputDeadlineClock,
        )
        .unwrap();
        let inherited = Deadline::until(Instant::now() + Duration::from_secs(2));
        let supplied = Deadline::until(Instant::now() + Duration::from_secs(3));
        let active = with_database_budget(&mut case.fixture.conn, inherited, |conn| {
            Ok(submit_wrapped_input_under_pane_lock_with_host(
                conn,
                &case.settings,
                &case.fixture.run,
                &case.placement,
                &held,
                &active_attempt,
                supplied,
                &mut NoLiveHost,
            ))
        });
        let removed = case
            .fixture
            .conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        removed.unwrap();
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
        assert_eq!(failure.run_id, Some(case.fixture.run.run_id));
        assert_eq!(
            failure.operation_key.as_deref(),
            Some(case.request.operation_key.as_str())
        );
        assert_eq!(failure.input_id, None);
        assert_eq!(
            expired_reads, 0,
            "an expired original budget starts no receipt SQL"
        );
        let active = active.unwrap().unwrap();
        assert_eq!(active.status, SubmissionStatus::Accepted);
        assert_eq!(active.input_id, Some(case.row.input_id));
        let budgets = budgets.lock().unwrap();
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
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let mut case = History::new(InputState::Accepted, None);
        let changes = case.fixture.conn.total_changes();
        let identity = case.identity();
        let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
        assert_eq!(
            find_submission(&mut case.fixture.conn, &identity, &case.request, deadline).unwrap(),
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
        let denied = find_submission(&mut case.fixture.conn, &identity, &case.request, deadline);
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
        for change in ["literal", "kind", "message"] {
            let mut request = SubmitInput {
                operation_key: case.request.operation_key.clone(),
                input_kind: case.request.input_kind,
                message_id: case.request.message_id,
                literal: case.request.literal.clone(),
            };
            match change {
                "literal" => request.literal.push('!'),
                "kind" => request.input_kind = InputKind::Exec,
                "message" => request.message_id = Some(123),
                _ => unreachable!(),
            }
            let failure = find_submission(&mut case.fixture.conn, &identity, &request, deadline)
                .expect_err("a reused key binds the original complete payload");
            assert!(!failure.source.to_string().contains(&request.literal));
            assert_eq!(failure.run_id, Some(case.fixture.run.run_id));
            assert_eq!(
                failure.operation_key.as_deref(),
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
                    &mut NoLiveHost
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
            find_submission(&mut case.fixture.conn, &foreign, &case.request, deadline).is_err()
        );
        case.assert_unchanged(changes);
        let missing = SubmitInput {
            operation_key: "prompt:missing".into(),
            input_kind: InputKind::Prompt,
            message_id: None,
            literal: "missing input".into(),
        };
        assert!(
            find_submission(&mut case.fixture.conn, &identity, &missing, deadline)
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
                PreparedSubmission::new(&request, until, &mut super::super::RealInputDeadlineClock)
                    .unwrap();
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
                    &mut NoLiveHost
                )
                .is_err()
            );
            let foreign = Fixture::new();
            let foreign_settings = Settings {
                database_url: format!("sqlite:///{}", foreign.conn.path().unwrap()),
                ..settings.clone()
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
                    &mut NoLiveHost
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
        fn lock(
            &mut self,
            settings: &Settings,
            run: &RunRecord,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            self.inner.lock(settings, run, placement, deadline)
        }

        fn health(
            &mut self,
            run: &RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            self.inner.health(run, deadline)
        }

        fn exchange(
            &mut self,
            run: &RunRecord,
            request: &InputControlRequest,
            deadline: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
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
    fn external_input_lock_reacquire_failure_after_the_runner_reply_reports_the_known_input_id() {
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            let request = ordinary(&run, "reacquire-failure");
            let until = outer.min(Instant::now() + Duration::from_secs(4));
            let attempt =
                PreparedSubmission::new(&request, until, &mut super::super::RealInputDeadlineClock)
                    .unwrap();
            let held = PaneLock::try_acquire(&settings.database_url, &placement)
                .unwrap()
                .unwrap();
            let mut inner = SocketHost::new(&settings, &placement, until);
            inner.expiration = Some(attempt.expiration.monotonic_ns());
            let mut host = LockTakingHost { inner, taken: None };
            let failure = match submit_wrapped_input_under_pane_lock_with_host(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &held,
                &attempt,
                Deadline::until(until),
                &mut host,
            ) {
                Err(failure) => failure,
                Ok(result) => panic!(
                    "a lock that cannot be reacquired fails: {:?}",
                    result.input_id
                ),
            };
            let row =
                storage::find_input(&fixture.fixture.conn, run.run_id, &request.operation_key)
                    .unwrap()
                    .expect("the runner committed the submission receipt");
            assert_eq!(
                failure.input_id,
                Some(row.input_id),
                "the failure names the input the runner already submitted"
            );
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
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
                PreparedSubmission::new(&request, until, &mut super::super::RealInputDeadlineClock)
                    .unwrap();
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
                    &mut NoLiveHost
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
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
            fixture.fixture.conn.execute("UPDATE member_runs SET reachability='suspect',last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1", [run.run_id]).unwrap();
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
            assert_eq!(failure.run_id, Some(run.run_id));
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(request.operation_key.as_str())
            );
            assert!(!failure.source.to_string().contains(&request.literal));
            assert!(
                update.load(Ordering::SeqCst),
                "genuine successful health updates the supplied connection"
            );
            let refreshed = storage::get_run(&fixture.fixture.conn, run.run_id).unwrap();
            assert_eq!(refreshed.reachability, Reachability::Reachable);
            assert!(refreshed.last_health_at.is_some());
            assert_ne!(
                refreshed.last_health_at.as_deref(),
                Some("2000-01-01T00:00:00Z")
            );
            let mut expected = before;
            expected.reachability = refreshed.reachability;
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
            storage::reserve_event_sequences(&mut case.fixture.conn, &run.owner()).unwrap();
            let evidence = storage::evidence::EvidencePolicy::metadata().apply(&serde_json::json!({"backend":"codex","native_version":"0.163.0","profile_version":null,"rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
            storage::persist_event(
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
                host.health_reply = Some(HealthSnapshot {
                    identity: ControlIdentity::from_run(&run),
                    runner,
                    child: None,
                    output_seq: 120,
                    observation: None,
                });
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
            assert_eq!(failure.run_id, Some(run.run_id));
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(case.request.operation_key.as_str())
            );
            assert!(!failure.source.to_string().contains(&case.request.literal));
            let mut after = storage::get_run(&case.fixture.conn, run.run_id).unwrap();
            after.reachability = before.reachability;
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
            let reserved = storage::reserve_input(
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
            assert_eq!(failure.run_id, Some(run.run_id));
            assert_eq!(
                failure.operation_key.as_deref(),
                Some(request.operation_key.as_str())
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
                &mut NoLiveHost,
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

    mod prepared_submission {
        use super::*;
        use crate::runtime::runner::input_clock::test_support::*;

        fn request() -> SubmitInput {
            SubmitInput {
                operation_key: format!("prompt:1:{}", "a".repeat(64)),
                input_kind: InputKind::Prompt,
                message_id: None,
                literal: "literal clock fixture".into(),
            }
        }
        #[test]
        fn clock_prepared_submission_captures_one_paired_sample() {
            let t = anchor();
            let request = request();
            let mut clock = ScriptClock::one(sample(ms(t, 10), 1_000_000_000, ms(t, 30)));
            let _prepared = PreparedSubmission::new(&request, ms(t, 100), &mut clock).unwrap();
            clock.consumed(1, 0);
        }

        #[test]
        fn clock_prepared_submission_source_failure_preserves_unused_later_sample() {
            let t = anchor();
            let request = request();
            let mut clock =
                ScriptClock::new(vec![Err(unavailable()), Ok(sample(t, 1_000_000_000, t))]);
            one_failure(
                PreparedSubmission::new(&request, ms(t, 100), &mut clock),
                &clock,
                1,
            );
        }

        #[test]
        fn clock_prepared_submission_expiry_rejects_without_resampling_new_budget() {
            let t = anchor();
            let request = request();
            let mut clock = ScriptClock::new(vec![
                Ok(sample(ms(t, 90), 1_000_000_000, ms(t, 100))),
                Ok(sample(t, 1_000_000_000, t)),
            ]);
            one_failure(
                PreparedSubmission::new(&request, ms(t, 100), &mut clock),
                &clock,
                1,
            );
        }
    }

    mod permission_history {
        use super::super::test_support::*;
        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::members::test_support::placement;
        use crate::broker::runtime::{self as storage, *};
        use crate::config::Settings;
        use crate::db::test_support::{TableSnapshot, table_snapshot};
        use crate::error::CafleetError;
        use crate::input::{Deadline, PaneLock, with_database_budget};
        use crate::runtime::runner::test_support::Fixture;
        use crate::runtime::runner::*;
        use rusqlite::Connection;
        use serde_json::json;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        struct Snapshot {
            tables: Vec<TableSnapshot>,
            changes: u64,
        }
        impl Snapshot {
            fn take(conn: &Connection) -> Self {
                Self {
                    tables: table_snapshot(conn, |_| true),
                    changes: conn.total_changes(),
                }
            }
            fn unchanged(&self, conn: &Connection) {
                let actual = Self::take(conn);
                assert_eq!(
                    actual.changes, self.changes,
                    "historical coordination performs no SQL mutation"
                );
                assert!(
                    actual.tables == self.tables,
                    "historical coordination preserves every table exactly"
                );
            }
        }

        mod inspect_errors {
            use super::*;
            use crate::runtime::runner::{PermissionInspectInput, inspect_permission};

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
                let settings = Settings::for_database(&path, None);
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
            fn permission_inspect_rejects_an_automatic_review_event_naming_an_existing_interaction()
            {
                for kind in [
                    RuntimeEventKind::AutomaticRefusalObserved,
                    RuntimeEventKind::AutomaticReviewUnavailable,
                ] {
                    let mut case = Case::new(Retained::Accepted);
                    let (_, interaction_id) = opened(&case);
                    let evidence = storage::evidence::EvidencePolicy::metadata()
                    .apply(&json!({
                        "backend":"codex","native_version":case.fixture.run.native_version,"profile_version":"fixture-detector-v1",
                        "rule_id":"fixture-auto-review","screen_seq":2,"operation":null,"choices":null,
                        "native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false
                    }))
                    .unwrap();
                    let annotation = storage::persist_event(
                        &mut case.fixture.conn,
                        &case.owner,
                        &ObservedTransition {
                            run_id: case.owner.run_id,
                            event_seq: 2,
                            episode_seq: None,
                            kind,
                            classification: None,
                            observed_at: NOW.into(),
                            evidence_json: evidence,
                            close_reason: None,
                            cause_json: None,
                        },
                        PersistenceMode::Live,
                    )
                    .unwrap();
                    let inputs = |conn: &Connection| -> i64 {
                        conn.query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                            row.get(0)
                        })
                        .unwrap()
                    };
                    let before = inputs(&case.fixture.conn);
                    let director = case.fixture.run.director_member_id;
                    let settings = case.settings.clone();
                    let error = inspect(
                        &mut case.fixture.conn,
                        &settings,
                        annotation.event_id,
                        interaction_id,
                        director,
                    );
                    assert_eq!(error.exit_code(), 1, "{kind:?}: {}", error.message());
                    assert_eq!(inputs(&case.fixture.conn), before, "{kind:?}");
                }
            }

            #[test]
            fn permission_inspect_reports_a_missing_interaction_before_event_run_or_identity_checks()
             {
                let case = Case::new(Retained::Accepted);
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
                let case = Case::new(Retained::Accepted);
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

        mod entry {
            use super::*;
            use crate::runtime::runner::{
                InputCommandHost, InputControlReply, InputControlRequest, InputFailure,
                PermissionRespondInput, PermissionResponseResult, SubmissionStage,
                SubmissionStatus, respond_permission_with_host,
            };

            #[derive(Default)]
            struct NoLive {
                calls: Vec<&'static str>,
            }
            impl NoLive {
                fn failure(&mut self, operation: &'static str, run: &RunRecord) -> InputFailure {
                    self.calls.push(operation);
                    InputFailure {
                        source: CafleetError::App(
                            "history fixture reached a forbidden live operation".into(),
                        ),
                        run_id: Some(run.run_id),
                        interaction_id: None,
                        input_id: None,
                        operation_key: None,
                    }
                }
            }
            impl InputCommandHost for NoLive {
                fn lock(
                    &mut self,
                    _: &Settings,
                    run: &RunRecord,
                    _: &crate::broker::records::Placement,
                    _: Deadline,
                ) -> std::result::Result<Option<PaneLock>, InputFailure> {
                    Err(self.failure("lock", run))
                }
                fn health(
                    &mut self,
                    run: &RunRecord,
                    _: Instant,
                ) -> std::result::Result<HealthSnapshot, InputFailure> {
                    Err(self.failure("health", run))
                }
                fn exchange(
                    &mut self,
                    run: &RunRecord,
                    _: &InputControlRequest,
                    _: Instant,
                ) -> std::result::Result<InputControlReply, InputFailure> {
                    Err(self.failure("exchange", run))
                }
            }
            fn invoke(
                conn: &mut Connection,
                settings: &Settings,
                request: &PermissionRespondInput,
                deadline: Deadline,
            ) -> std::result::Result<PermissionResponseResult, InputFailure> {
                let mut host = NoLive::default();
                let result = with_database_budget(conn, deadline, |conn| {
                    Ok(respond_permission_with_host(
                        conn, settings, request, deadline, &mut host,
                    ))
                })
                .unwrap();
                assert!(
                    host.calls.is_empty(),
                    "historical coordination reached live operations: {:?}",
                    host.calls
                );
                result
            }
            fn attempt(
                case: &mut Case,
                request: &PermissionRespondInput,
            ) -> std::result::Result<PermissionResponseResult, InputFailure> {
                invoke(
                    &mut case.fixture.conn,
                    &case.settings,
                    request,
                    Deadline::until(Instant::now() + Duration::from_secs(1)),
                )
            }
            fn success(
                result: std::result::Result<PermissionResponseResult, InputFailure>,
            ) -> PermissionResponseResult {
                match result {
                    Ok(result) => result,
                    Err(error) => {
                        panic!("valid retained response returns history: {}", error.source)
                    }
                }
            }
            fn assert_projection(case: &Case, result: &PermissionResponseResult, state: Retained) {
                assert_eq!(result.submission.input_id, Some(case.input_id));
                assert_eq!(result.member_id, case.fixture.run.member_id);
                assert_eq!(result.run_id, case.owner.run_id);
                assert_eq!(
                    result.owner_epoch,
                    case.request.metadata.expected_owner_epoch
                );
                assert_eq!(result.interaction_id, case.request.metadata.interaction_id);
                assert_eq!(result.event_id, case.event.event_id);
                assert_eq!(result.decision, case.request.metadata.decision);
                assert_eq!(
                    result.decision_source,
                    case.request.metadata.decision_source
                );
                assert_eq!(result.submission.output_seq, None);
                match state {
                    Retained::Accepted | Retained::Submitted => {
                        assert!(matches!(
                            result.submission.status,
                            SubmissionStatus::Accepted
                        ));
                        assert!(matches!(
                            result.submission.stage,
                            SubmissionStage::Submitted
                        ));
                    }
                    Retained::InProgress | Retained::Uncertain => {
                        assert!(matches!(
                            result.submission.status,
                            SubmissionStatus::Uncertain
                        ));
                        assert!(matches!(result.submission.stage, SubmissionStage::Unknown));
                    }
                    Retained::NotSubmitted | Retained::Abandoned => {
                        assert!(matches!(
                            result.submission.status,
                            SubmissionStatus::Rejected
                        ));
                        assert!(matches!(
                            result.submission.stage,
                            SubmissionStage::ZeroBytes
                        ));
                        assert!(
                            result
                                .submission
                                .reason
                                .as_ref()
                                .is_some_and(|reason| reason
                                    .to_ascii_lowercase()
                                    .contains("consum"))
                        );
                    }
                }
            }
            fn projection(state: Retained) {
                let mut case = Case::new(state);
                let before = Snapshot::take(&case.fixture.conn);
                let receipt =
                    storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
                let request = case.command();
                let result = success(attempt(&mut case, &request));
                assert_projection(&case, &result, state);
                before.unchanged(&case.fixture.conn);
                assert_eq!(
                    storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap(),
                    receipt
                );
            }
            #[test]
            fn retained_and_consumed_history_projects_each_state_without_live_calls() {
                for state in [
                    Retained::Accepted,
                    Retained::InProgress,
                    Retained::Uncertain,
                    Retained::Submitted,
                    Retained::NotSubmitted,
                    Retained::Abandoned,
                ] {
                    projection(state);
                }
            }

            #[test]
            fn history_preserves_both_decisions_and_every_valid_attributed_source_mode() {
                for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                    for (mode, source) in [
                        (DirectorMode::Auto, DecisionSource::DirectorAuto),
                        (DirectorMode::Auto, DecisionSource::User),
                        (DirectorMode::Manual, DecisionSource::User),
                        (DirectorMode::Unknown, DecisionSource::User),
                    ] {
                        let mut case = Case::decision(Retained::Accepted, decision, mode, source);
                        let before = Snapshot::take(&case.fixture.conn);
                        let request = case.command();
                        let result = success(attempt(&mut case, &request));
                        assert_projection(&case, &result, Retained::Accepted);
                        before.unchanged(&case.fixture.conn);
                    }
                }
            }

            #[derive(Clone, Copy, Debug)]
            enum HistoryChange {
                Closed,
                MemberInactive,
                DirectorInactive,
                FleetDeleted,
                OwnerFenced,
            }
            fn changed_history(change: HistoryChange) {
                let mut case = Case::new(Retained::Accepted);
                let run = &case.fixture.run;
                match change {
                    HistoryChange::Closed => {
                        assert_eq!(case.fixture.conn.execute(
                    "UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",
                    rusqlite::params![NOW,case.request.metadata.interaction_id]).unwrap(),1);
                    }
                    HistoryChange::MemberInactive | HistoryChange::DirectorInactive => {
                        assert_eq!(case.fixture.conn.execute(
                    "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                    rusqlite::params![NOW,if matches!(change,HistoryChange::MemberInactive){run.member_id}else{run.director_member_id}]).unwrap(),1);
                    }
                    HistoryChange::FleetDeleted => {
                        assert_eq!(
                            case.fixture
                                .conn
                                .execute(
                                    "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                                    rusqlite::params![NOW, run.fleet_id]
                                )
                                .unwrap(),
                            1
                        );
                    }
                    HistoryChange::OwnerFenced => {
                        assert_eq!(case.fixture.conn.execute(
                    "UPDATE member_runs SET status='lost',owner_kind='reconciler',owner_epoch=owner_epoch+1,ended_at=?1 WHERE run_id=?2",
                    rusqlite::params![NOW,run.run_id]).unwrap(),1);
                    }
                }
                let request = case.command();
                let before = Snapshot::take(&case.fixture.conn);
                let result = success(attempt(&mut case, &request));
                assert_projection(&case, &result, Retained::Accepted);
                before.unchanged(&case.fixture.conn);
                if matches!(change, HistoryChange::OwnerFenced) {
                    assert_eq!(
                        storage::get_run(&case.fixture.conn, case.owner.run_id)
                            .unwrap()
                            .owner_epoch,
                        result.owner_epoch + 1
                    );
                }
            }
            #[test]
            fn history_survives_each_later_lifecycle_change_without_live_preflight() {
                for change in [
                    HistoryChange::Closed,
                    HistoryChange::MemberInactive,
                    HistoryChange::DirectorInactive,
                    HistoryChange::FleetDeleted,
                    HistoryChange::OwnerFenced,
                ] {
                    changed_history(change);
                }
            }

            #[test]
            fn retained_result_precedes_the_actual_held_pane_lock() {
                let mut case = Case::new(Retained::Accepted);
                let guard = PaneLock::acquire_within(
                    &case.settings.database_url,
                    &case.placement,
                    Deadline::until(Instant::now() + Duration::from_secs(1)),
                )
                .unwrap()
                .unwrap();
                let request = case.command();
                let before = Snapshot::take(&case.fixture.conn);
                let result = success(attempt(&mut case, &request));
                assert_projection(&case, &result, Retained::Accepted);
                before.unchanged(&case.fixture.conn);
                assert!(
                    PaneLock::acquire_within(
                        &case.settings.database_url,
                        &case.placement,
                        Deadline::until(Instant::now() + Duration::from_millis(20))
                    )
                    .unwrap()
                    .is_none()
                );
                drop(guard);
            }
            #[test]
            fn reopened_connection_retries_settled_history_without_mutation() {
                let mut case = Case::new(Retained::Submitted);
                let request = case.command();
                let before = Snapshot::take(&case.fixture.conn);
                success(attempt(&mut case, &request));
                before.unchanged(&case.fixture.conn);
                let mut reopened = crate::db::connect(&case.settings.database_url).unwrap();
                let second = Snapshot::take(&reopened);
                let result = success(invoke(
                    &mut reopened,
                    &case.settings,
                    &request,
                    Deadline::until(Instant::now() + Duration::from_secs(1)),
                ));
                assert_projection(&case, &result, Retained::Submitted);
                second.unchanged(&reopened);
            }
            #[test]
            fn retained_lookup_reads_while_another_connection_holds_sqlite_write_ownership() {
                let mut case = Case::new(Retained::Accepted);
                let writer = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
                writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                let before = Snapshot::take(&case.fixture.conn);
                let request = case.command();
                let result = invoke(
                    &mut case.fixture.conn,
                    &case.settings,
                    &request,
                    Deadline::until(Instant::now() + Duration::from_millis(250)),
                );
                let held = !writer.is_autocommit();
                writer.execute_batch("ROLLBACK").unwrap();
                assert!(held);
                assert_projection(&case, &success(result), Retained::Accepted);
                before.unchanged(&case.fixture.conn);
            }
            #[test]
            fn retained_lookup_precedes_actual_unresolved_capacity_refusal() {
                let mut case = Case::new(Retained::Accepted);
                assert_eq!(case.fixture.conn.execute(
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) SELECT ?1,'fixture:capacity:'||x,'prompt',?2,'in_progress',?3,?3 FROM n",
                rusqlite::params![case.owner.run_id,"b".repeat(64),NOW]).unwrap(),4095);
                assert_eq!(case.fixture.conn.query_row("SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND state IN ('in_progress','accepted','uncertain')",[case.owner.run_id],|row|row.get::<_,i64>(0)).unwrap(),4096);
                let before = Snapshot::take(&case.fixture.conn);
                let request = case.command();
                let result = success(attempt(&mut case, &request));
                assert_projection(&case, &result, Retained::Accepted);
                before.unchanged(&case.fixture.conn);
            }

            #[test]
            fn each_conflicting_caller_binding_fails_before_live_calls_with_exact_rows() {
                for field in [
                    "interaction",
                    "event",
                    "director",
                    "run",
                    "epoch",
                    "token",
                    "decision",
                    "source",
                    "mode",
                    "mode_ref",
                    "decision_ref",
                ] {
                    let mut case = Case::decision(
                        Retained::Accepted,
                        PermissionDecision::Allow,
                        DirectorMode::Auto,
                        DecisionSource::User,
                    );
                    let mut request = case.command();
                    match field {
                        "interaction" => request.interaction_id += 1,
                        "event" => request.event_id += 1,
                        "director" => request.director_member_id = case.fixture.run.member_id,
                        "run" => request.expected_run_id += 1,
                        "epoch" => request.expected_owner_epoch += 1,
                        "token" => request.request_token = "b".repeat(64),
                        "decision" => request.decision = PermissionDecision::Deny,
                        "source" => request.decision_source = DecisionSource::DirectorAuto,
                        "mode" => request.director_mode = DirectorMode::Manual,
                        "mode_ref" => request.mode_evidence_ref = "fixture:other-mode".into(),
                        "decision_ref" => {
                            request.decision_evidence_ref = "fixture:other-decision".into()
                        }
                        _ => unreachable!(),
                    }
                    let before = Snapshot::take(&case.fixture.conn);
                    assert!(attempt(&mut case, &request).is_err(), "{field}");
                    before.unchanged(&case.fixture.conn);
                }
            }
            #[test]
            fn retained_ordinary_input_kind_cannot_impersonate_a_permission_response() {
                let mut case = Case::new(Retained::Accepted);
                storage::reserve_input(
                    &mut case.fixture.conn,
                    &case.owner,
                    &NewInput {
                        operation_key: "fixture:ordinary-input".into(),
                        kind: InputKind::Prompt,
                        payload_digest: "c".repeat(64),
                        message_id: None,
                    },
                )
                .unwrap();
                let mut request = case.command();
                request.operation_key = "fixture:ordinary-input".into();
                let before = Snapshot::take(&case.fixture.conn);
                assert!(attempt(&mut case, &request).is_err());
                before.unchanged(&case.fixture.conn);
            }
            #[test]
            fn historical_message_association_is_checked_despite_matching_canonical_digest() {
                let mut case = Case::new(Retained::Accepted);
                assert_eq!(
                    storage::permission_response_digest(case.owner.run_id, &case.request.metadata)
                        .unwrap(),
                    storage::get_input(&case.fixture.conn, case.input_id)
                        .unwrap()
                        .payload_digest
                );
                assert_eq!(
                    case.fixture
                        .conn
                        .execute(
                            "UPDATE messages SET to_member_id=?1 WHERE message_id=?2",
                            rusqlite::params![
                                case.fixture.run.member_id,
                                case.event.message_id.unwrap()
                            ]
                        )
                        .unwrap(),
                    1
                );
                let request = case.command();
                let source = stored_corruption(&case);
                let before = Snapshot::take(&case.fixture.conn);
                let result = attempt(&mut case, &request);
                known_corruption(&case, result, &source);
                before.unchanged(&case.fixture.conn);
            }

            fn stored_corruption(case: &Case) -> CafleetError {
                match storage::get_permission_response(&case.fixture.conn, case.input_id) {
                    Err(error) => error,
                    Ok(_) => panic!("the fixture exposes actual durable association corruption"),
                }
            }

            fn known_corruption(
                case: &Case,
                result: std::result::Result<PermissionResponseResult, InputFailure>,
                source: &CafleetError,
            ) {
                let failure = match result {
                    Err(failure) => failure,
                    Ok(_) => panic!("corrupt history rejects before live effects"),
                };
                assert_eq!(failure.run_id, Some(case.owner.run_id));
                assert_eq!(
                    failure.interaction_id,
                    Some(case.request.metadata.interaction_id)
                );
                assert_eq!(failure.input_id, Some(case.input_id));
                assert_eq!(
                    failure.operation_key.as_deref(),
                    Some(case.request.operation_key.as_str())
                );
                assert_eq!(
                    std::mem::discriminant(&failure.source),
                    std::mem::discriminant(source)
                );
                assert_eq!(failure.source.exit_code(), source.exit_code());
                assert_eq!(failure.source.message(), source.message());
            }
            #[test]
            fn history_rejects_missing_audit_or_possibly_transmitted_claim_without_live_fallback() {
                for remove_audit in [false, true] {
                    let mut case = Case::new(Retained::Accepted);
                    assert_eq!(
                        case.fixture
                            .conn
                            .execute(
                                "DELETE FROM member_permission_response_claims WHERE input_id=?1",
                                [case.input_id]
                            )
                            .unwrap(),
                        1
                    );
                    if remove_audit {
                        assert_eq!(
                            case.fixture
                                .conn
                                .execute(
                                    "DELETE FROM member_permission_responses WHERE input_id=?1",
                                    [case.input_id]
                                )
                                .unwrap(),
                            1
                        );
                    }
                    let request = case.command();
                    let source = stored_corruption(&case);
                    let before = Snapshot::take(&case.fixture.conn);
                    let result = attempt(&mut case, &request);
                    known_corruption(&case, result, &source);
                    before.unchanged(&case.fixture.conn);
                }
            }
            #[test]
            fn history_validates_actual_member_fleet_and_director_relationships() {
                for changed in ["member", "director", "fleet"] {
                    let mut case = Case::new(Retained::Accepted);
                    if changed == "fleet" {
                        let alternate = crate::broker::register_member(
                            &mut case.fixture.conn,
                            case.fixture.run.fleet_id,
                            "alternative",
                            "fixture",
                            None,
                        )
                        .unwrap()
                        .member_id;
                        assert_eq!(
                            case.fixture
                                .conn
                                .execute(
                                    "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
                                    rusqlite::params![alternate, case.fixture.run.fleet_id]
                                )
                                .unwrap(),
                            1
                        );
                    } else {
                        let other =
                            create_fleet_fixture(&mut case.fixture.conn, "other-history-fleet").0;
                        let member = if changed == "member" {
                            case.fixture.run.member_id
                        } else {
                            case.fixture.run.director_member_id
                        };
                        assert_eq!(
                            case.fixture
                                .conn
                                .execute(
                                    "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                                    rusqlite::params![other, member]
                                )
                                .unwrap(),
                            1
                        );
                    }
                    let request = case.command();
                    let before = Snapshot::take(&case.fixture.conn);
                    assert!(attempt(&mut case, &request).is_err(), "{changed}");
                    before.unchanged(&case.fixture.conn);
                }
            }
            #[test]
            fn copied_database_cannot_reuse_the_original_retained_identity() {
                let case = Case::new(Retained::Accepted);
                let copied = case.fixture._dir.path().join("copied-history.db");
                std::fs::copy(case.fixture.conn.path().unwrap(), &copied).unwrap();
                let settings = Settings::for_database(&copied, None);
                let mut conn = crate::db::connect(&settings.database_url).unwrap();
                let before = Snapshot::take(&conn);
                assert!(
                    invoke(
                        &mut conn,
                        &settings,
                        &case.command(),
                        Deadline::until(Instant::now() + Duration::from_secs(1))
                    )
                    .is_err()
                );
                before.unchanged(&conn);
            }
            #[test]
            fn semantic_source_mode_preflight_preserves_usage_category_before_history_or_live_effects()
             {
                for mode in [DirectorMode::Manual, DirectorMode::Unknown] {
                    let mut case = Case::new(Retained::Accepted);
                    let mut request = case.command();
                    request.director_mode = mode;
                    let before = Snapshot::take(&case.fixture.conn);
                    let failure = match attempt(&mut case, &request) {
                        Err(failure) => failure,
                        Ok(_) => panic!("director-auto requires auto before retained projection"),
                    };
                    assert!(matches!(failure.source, CafleetError::Usage(_)));
                    before.unchanged(&case.fixture.conn);
                }
            }

            fn causes() -> Vec<CafleetError> {
                vec![
                    CafleetError::Usage("fixture usage".into()),
                    CafleetError::App("fixture application".into()),
                    CafleetError::Value("fixture value".into()),
                    CafleetError::DatabaseBusy("fixture contention".into()),
                    CafleetError::DeadlineExceeded,
                    CafleetError::InvalidStoredValue {
                        field: "fixture.field".into(),
                        value: "invalid".into(),
                    },
                    CafleetError::WithDiagnostics {
                        cause: Box::new(CafleetError::Usage("nested usage".into())),
                        diagnostics: vec!["original diagnostic".into()],
                    },
                ]
            }
            #[test]
            fn input_failure_without_context_preserves_the_complete_source_category() {
                for cause in causes() {
                    let failure = InputFailure {
                        source: cause.clone(),
                        run_id: None,
                        interaction_id: None,
                        input_id: None,
                        operation_key: None,
                    };
                    let converted: CafleetError = failure.into();
                    assert_eq!(
                        std::mem::discriminant(&converted),
                        std::mem::discriminant(&cause)
                    );
                    assert_eq!(converted.exit_code(), cause.exit_code());
                    assert_eq!(converted.message(), cause.message());
                }
            }
            #[test]
            fn input_failure_context_preserves_cause_and_escapes_ordered_key_metadata() {
                let key = "fixture:key\nwith\"quotes\"\\tail";
                for original in causes() {
                    let failure = InputFailure {
                        source: original.clone(),
                        run_id: Some(29),
                        interaction_id: Some(17),
                        input_id: Some(31),
                        operation_key: Some(key.into()),
                    };
                    let converted: CafleetError = failure.into();
                    assert_eq!(converted.exit_code(), original.exit_code());
                    let CafleetError::WithDiagnostics { cause, diagnostics } = converted else {
                        panic!("known identities retain the source with context")
                    };
                    assert_eq!(
                        std::mem::discriminant(cause.as_ref()),
                        std::mem::discriminant(&original)
                    );
                    assert_eq!(cause.message(), original.message());
                    assert_eq!(diagnostics.len(), 1);
                    let context = &diagnostics[0];
                    let positions = ["run_id", "interaction_id", "input_id", "operation_key"]
                        .map(|name| context.find(name).expect("known context field"));
                    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
                    assert!(context.contains(&serde_json::to_string(key).unwrap()));
                    assert!(!context.contains('\n'));
                }
            }
            #[test]
            fn input_failure_context_omits_unknown_optional_identities() {
                let failure = InputFailure {
                    source: CafleetError::DeadlineExceeded,
                    run_id: None,
                    interaction_id: Some(17),
                    input_id: None,
                    operation_key: None,
                };
                let converted: CafleetError = failure.into();
                let CafleetError::WithDiagnostics { cause, diagnostics } = converted else {
                    panic!("one known identity is represented explicitly")
                };
                assert!(matches!(*cause, CafleetError::DeadlineExceeded));
                assert_eq!(diagnostics.len(), 1);
                assert!(diagnostics[0].contains("interaction_id"));
                for absent in ["run_id", "input_id", "operation_key"] {
                    assert!(!diagnostics[0].contains(absent));
                }
            }

            mod claim_and_association_history {
                use super::*;
                use rusqlite::hooks::{
                    AuthAction, AuthContext, Authorization, TransactionOperation,
                };
                use std::sync::Mutex;

                #[test]
                fn released_not_submitted_claim_remains_valid_consumed_history() {
                    let mut case = Case::new(Retained::NotSubmitted);
                    assert_eq!(
                        case.fixture
                            .conn
                            .execute(
                                "DELETE FROM member_permission_response_claims WHERE input_id=?1",
                                [case.input_id],
                            )
                            .unwrap(),
                        1
                    );
                    let receipt =
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .unwrap();
                    assert!(receipt.claim.is_none());
                    assert_eq!(receipt.input.state, InputState::Settled);
                    assert_eq!(
                        receipt.input.disposition,
                        Some(InputDisposition::NotSubmitted)
                    );
                    assert_eq!(receipt.response.metadata, case.request.metadata);
                    let request = case.command();
                    let before = Snapshot::take(&case.fixture.conn);
                    let result = success(attempt(&mut case, &request));
                    assert_projection(&case, &result, Retained::NotSubmitted);
                    before.unchanged(&case.fixture.conn);
                    assert_eq!(
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .unwrap(),
                        receipt
                    );
                }

                fn related(case: &Case) -> Case {
                    let mut conn = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
                    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                    let mut placement = placement(Some("%3"));
                    placement.coding_agent = "codex".into();
                    let member = crate::broker::register_member(
                        &mut conn,
                        case.fixture.run.fleet_id,
                        "related-history-worker",
                        "assignment",
                        Some(&placement),
                    )
                    .unwrap()
                    .member_id;
                    let next_id = conn
                        .query_row(
                            "SELECT COALESCE(MAX(run_id),0)+1 FROM member_runs",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap();
                    let process = ProcessIdentity::current().unwrap();
                    let original = &case.fixture.run;
                    let run = storage::create_run(
                        &mut conn,
                        &NewRun {
                            member_id: member,
                            fleet_id: original.fleet_id,
                            director_member_id: original.director_member_id,
                            backend: original.backend.clone(),
                            database_identity_json: original.database_identity_json.clone(),
                            launch_nonce: "b".repeat(64),
                            runtime_path: RuntimePaths::for_run(
                                &original.database_identity_json,
                                next_id,
                            )
                            .unwrap()
                            .run_dir
                            .to_str()
                            .unwrap()
                            .into(),
                            requested_mode: original.requested_mode.clone(),
                            resolved_mode: original.resolved_mode.clone(),
                            mode_source: original.mode_source,
                            permission_argv_json: original.permission_argv_json.clone(),
                            event_evidence: original.event_evidence,
                            native_version: original.native_version.clone(),
                            profile_version: None,
                            coverage: Coverage::Unknown,
                            owner_pid: process.pid,
                            owner_start_identity: process.start_identity,
                        },
                    )
                    .unwrap();
                    assert_eq!(run.run_id, next_id);
                    Case::from_fixture(
                        Fixture {
                            _dir: tempfile::tempdir().unwrap(),
                            conn,
                            run,
                        },
                        Retained::Accepted,
                        PermissionDecision::Allow,
                        DirectorMode::Auto,
                        DecisionSource::DirectorAuto,
                    )
                }

                #[test]
                fn existing_foreign_run_rejects_association_before_live_boundary() {
                    let mut original = Case::new(Retained::Accepted);
                    let foreign = related(&original);
                    assert_ne!(original.owner.run_id, foreign.owner.run_id);
                    assert_eq!(
                        original.owner.database_identity_json,
                        foreign.owner.database_identity_json
                    );
                    assert_eq!(
                        storage::get_run(&original.fixture.conn, foreign.owner.run_id).unwrap(),
                        storage::get_run(&foreign.fixture.conn, foreign.owner.run_id).unwrap()
                    );
                    let mut request = original.command();
                    request.expected_run_id = foreign.owner.run_id;
                    request.expected_owner_epoch = foreign.owner.owner_epoch;
                    request.operation_key = "fixture:absent-on-foreign-run".into();
                    assert_eq!(
                    original
                        .fixture
                        .conn
                        .query_row(
                            "SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
                            rusqlite::params![foreign.owner.run_id, request.operation_key],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap(),
                    0
                );
                    let before = Snapshot::take(&original.fixture.conn);
                    let failure = match attempt(&mut original, &request) {
                        Err(failure) => failure,
                        Ok(_) => {
                            panic!("an existing foreign run refuses the original event/interaction")
                        }
                    };
                    assert!(matches!(&failure.source,CafleetError::App(message)
                    if message=="permission run/event/interaction/Director association mismatch"));
                    assert_eq!(failure.source.exit_code(), 1);
                    assert_eq!(failure.run_id, Some(foreign.owner.run_id));
                    assert_eq!(
                        failure.interaction_id,
                        Some(original.request.metadata.interaction_id)
                    );
                    assert_eq!(failure.input_id, None);
                    assert_eq!(
                        failure.operation_key.as_deref(),
                        Some(request.operation_key.as_str())
                    );
                    before.unchanged(&original.fixture.conn);
                }

                #[test]
                fn second_run_matching_history_is_independently_valid() {
                    let original = Case::new(Retained::Accepted);
                    let mut foreign = related(&original);
                    let receipt =
                        storage::get_permission_response(&foreign.fixture.conn, foreign.input_id)
                            .unwrap();
                    assert_eq!(receipt.input.run_id, foreign.owner.run_id);
                    assert_ne!(
                        receipt.response.metadata.interaction_id,
                        original.request.metadata.interaction_id
                    );
                    let request = foreign.command();
                    let before = Snapshot::take(&foreign.fixture.conn);
                    let result = success(attempt(&mut foreign, &request));
                    assert_projection(&foreign, &result, Retained::Accepted);
                    before.unchanged(&foreign.fixture.conn);
                }

                #[derive(Debug)]
                enum ObservedSql {
                    Transaction(TransactionOperation),
                    Read {
                        table: String,
                        transaction_open: bool,
                    },
                }

                #[test]
                fn supplied_deferred_snapshot_survives_committed_association_change() {
                    let mut case = Case::new(Retained::Accepted);
                    let mode: String = case
                        .fixture
                        .conn
                        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                        .unwrap();
                    assert_eq!(mode.to_ascii_lowercase(), "wal");
                    let writer = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
                    writer.busy_timeout(Duration::from_millis(200)).unwrap();
                    let writer_mode: String = writer
                        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                        .unwrap();
                    assert_eq!(writer_mode.to_ascii_lowercase(), "wal");
                    let before = Snapshot::take(&case.fixture.conn);
                    let observations = Arc::new(Mutex::new(Vec::new()));
                    let observed = observations.clone();
                    let mutation = Arc::new(Mutex::new(None));
                    let committed = mutation.clone();
                    let mut writer = Some(writer);
                    let message_id = case.event.message_id.unwrap();
                    let wrong_recipient = case.fixture.run.member_id;
                    let mut transaction_open = false;
                    case.fixture
                    .conn
                    .authorizer(Some(move |context: AuthContext<'_>| {
                        match context.action {
                            AuthAction::Transaction { operation } => {
                                transaction_open = operation == TransactionOperation::Begin;
                                observed
                                    .lock()
                                    .unwrap()
                                    .push(ObservedSql::Transaction(operation));
                            }
                            AuthAction::Read { table_name, .. } => {
                                observed.lock().unwrap().push(ObservedSql::Read {
                                    table: table_name.into(),
                                    transaction_open,
                                });
                                if table_name == "member_permission_responses"
                                    && let Some(writer) = writer.take()
                                {
                                    let result = (|| -> rusqlite::Result<Snapshot> {
                                        let changed = writer.execute(
                                            "UPDATE messages SET to_member_id=?1 WHERE message_id=?2",
                                            rusqlite::params![wrong_recipient, message_id],
                                        )?;
                                        if changed != 1 || !writer.is_autocommit() {
                                            return Err(rusqlite::Error::InvalidQuery);
                                        }
                                        Ok(Snapshot::take(&writer))
                                    })();
                                    *committed.lock().unwrap() = Some(result);
                                }
                            }
                            _ => {}
                        }
                        Authorization::Allow
                    }))
                    .unwrap();
                    let request = case.command();
                    let result = attempt(&mut case, &request);
                    case.fixture
                        .conn
                        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                        .unwrap();
                    let observed = observations.lock().unwrap();
                    let transactions = observed
                        .iter()
                        .filter_map(|operation| match operation {
                            ObservedSql::Transaction(operation) => Some(*operation),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        transactions,
                        // rusqlite represents SQLite's COMMIT authorization action as Unknown.
                        vec![TransactionOperation::Begin, TransactionOperation::Unknown],
                        "{observed:?}"
                    );
                    assert!(case.fixture.conn.is_autocommit());
                    for table in [
                        "member_runs",
                        "member_input_operations",
                        "member_permission_responses",
                        "member_runtime_events",
                        "member_interactions",
                        "messages",
                        "member_permission_response_claims",
                    ] {
                        assert!(observed.iter().any(|operation|matches!(operation,ObservedSql::Read {table:actual,transaction_open:true} if actual==table)),"supplied-connection preparation inside the observed transaction for {table}: {observed:?}");
                    }
                    assert!(
                        observed.iter().all(|operation| !matches!(
                            operation,
                            ObservedSql::Read {
                                transaction_open: false,
                                ..
                            }
                        )),
                        "every supplied-connection read is enclosed by the single transaction: {observed:?}"
                    );
                    let committed = mutation
                    .lock()
                    .unwrap()
                    .take()
                    .expect("the actual supplied-connection audit read triggers the owned WAL writer")
                    .expect(
                        "the writer commits while the supplied transaction retains its read snapshot",
                    );
                    let after = Snapshot::take(&case.fixture.conn);
                    assert_eq!(
                        after.changes, before.changes,
                        "the coordinator makes zero SQL mutations"
                    );
                    assert!(
                        after.tables == committed.tables,
                        "all rows match only the intentional committed association change"
                    );
                    assert_eq!(
                        case.fixture
                            .conn
                            .query_row(
                                "SELECT to_member_id FROM messages WHERE message_id=?1",
                                [message_id],
                                |row| row.get::<_, i64>(0)
                            )
                            .unwrap(),
                        wrong_recipient
                    );
                    assert!(
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .is_err(),
                        "the new snapshot exposes the committed corruption"
                    );
                    assert_projection(&case, &success(result), Retained::Accepted);
                }
            }

            mod staged {
                use super::*;

                fn response_context(
                    case: &Case,
                    failure: &InputFailure,
                    key: &str,
                    input_id: Option<i64>,
                ) {
                    assert_eq!(failure.run_id, Some(case.owner.run_id));
                    assert_eq!(
                        failure.interaction_id,
                        Some(case.request.metadata.interaction_id)
                    );
                    assert_eq!(failure.input_id, input_id);
                    assert_eq!(failure.operation_key.as_deref(), Some(key));
                }

                fn ready() -> Case {
                    let mut case = Case::new(Retained::InProgress);
                    storage::record_input_state(
                        &mut case.fixture.conn,
                        &case.owner,
                        case.input_id,
                        InputState::InProgress,
                        InputState::Ready,
                    )
                    .unwrap();
                    let receipt =
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .unwrap();
                    assert_eq!(receipt.input.state, InputState::Ready);
                    assert_eq!(receipt.response.metadata, case.request.metadata);
                    assert!(receipt.claim.is_some());
                    case
                }

                #[test]
                fn permission_ready_history_reaches_live_lock_with_exact_old_metadata_and_claim() {
                    let mut case = ready();
                    let request = case.command();
                    let receipt =
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .unwrap();
                    let before = Snapshot::take(&case.fixture.conn);
                    let mut host = NoLive::default();
                    let failure = respond_permission_with_host(
                        &mut case.fixture.conn,
                        &case.settings,
                        &request,
                        Deadline::until(Instant::now() + Duration::from_secs(1)),
                        &mut host,
                    )
                    .err()
                    .expect("the controlled lock failure preserves Ready history");
                    assert_eq!(host.calls, ["lock"]);
                    response_context(&case, &failure, &request.operation_key, Some(case.input_id));
                    before.unchanged(&case.fixture.conn);
                    assert_eq!(
                        storage::get_permission_response(&case.fixture.conn, case.input_id)
                            .unwrap(),
                        receipt
                    );
                }

                #[test]
                fn permission_ready_conflict_precedes_live_lock_and_preserves_binding() {
                    for field in ["epoch", "token", "decision", "mode_ref", "decision_ref"] {
                        let mut case = ready();
                        let mut request = case.command();
                        match field {
                            "epoch" => request.expected_owner_epoch += 1,
                            "token" => request.request_token = "b".repeat(64),
                            "decision" => request.decision = PermissionDecision::Deny,
                            "mode_ref" => request.mode_evidence_ref = "fixture:other-mode".into(),
                            "decision_ref" => {
                                request.decision_evidence_ref = "fixture:other-decision".into()
                            }
                            _ => unreachable!(),
                        }
                        let before = Snapshot::take(&case.fixture.conn);
                        let receipt =
                            storage::get_permission_response(&case.fixture.conn, case.input_id)
                                .unwrap();
                        let failure = match attempt(&mut case, &request) {
                            Err(failure) => failure,
                            Ok(_) => panic!("conflicting Ready {field} rejects the attempt"),
                        };
                        assert!(
                            matches!(failure.source, CafleetError::InvalidStoredValue { .. }),
                            "Ready {field} preserves the metadata-conflict category: {}",
                            failure.source
                        );
                        response_context(
                            &case,
                            &failure,
                            &request.operation_key,
                            Some(case.input_id),
                        );
                        before.unchanged(&case.fixture.conn);
                        assert_eq!(
                            storage::get_permission_response(&case.fixture.conn, case.input_id)
                                .unwrap(),
                            receipt
                        );
                    }
                }
            }
        }
    }
}
