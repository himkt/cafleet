use super::*;
use crate::broker::members::db_err;
use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, *};
use crate::config::Settings;
use crate::input::{Deadline, PaneLock, inherited_database_deadline, with_database_budget};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub(crate) struct PreparedSubmission<'a> {
    pub request: &'a SubmitInput,
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
            protocol_version: CONTROL_PROTOCOL_VERSION,
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

fn submission_digest(run_id: i64, request: &SubmitInput) -> Result<String> {
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
    let bytes = canonical_json(&serde_json::json!({
        "run_id":run_id,"kind":request.input_kind.as_str(),
        "message_id":request.message_id,"literal":request.literal,
    }));
    Ok(Sha256::digest(bytes.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn find_submission(
    conn: &mut Connection,
    identity: &PermissionReceiptIdentity,
    request: &SubmitInput,
    deadline: Deadline,
) -> std::result::Result<Option<InputOperation>, InputFailure> {
    let mut context = submission_context(identity.run_id, request);
    let result = (|| {
        let digest = submission_digest(identity.run_id, request)?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            let run = selected_run(&tx, identity.run_id)?;
            ensure(
                run.database_identity_json == identity.database_identity_json,
                "submission database identity mismatch",
            )?;
            let retained = storage::find_input(&tx, identity.run_id, &request.operation_key)?;
            if let Some(input) = &retained {
                context.input_id = Some(input.input_id);
                ensure(
                    input.kind == request.input_kind
                        && input.payload_digest == digest
                        && input.message_id == request.message_id,
                    "conflicting input operation key reuse",
                )?;
            }
            deadline.remaining()?;
            tx.commit().map_err(db_err)?;
            Ok(retained)
        })
    })();
    result.map_err(|source| context.failure(source))
}

fn historical_submission(input: &InputOperation) -> Option<SubmissionResult> {
    let (status, stage, reason) = match (input.state, input.disposition) {
        (InputState::Ready, _) => return None,
        (InputState::Accepted, _) | (InputState::Settled, Some(InputDisposition::Submitted)) => {
            (SubmissionStatus::Accepted, SubmissionStage::Submitted, None)
        }
        (InputState::InProgress | InputState::Uncertain, _) => {
            (SubmissionStatus::Uncertain, SubmissionStage::Unknown, None)
        }
        (InputState::Settled, _) => (
            SubmissionStatus::Rejected,
            SubmissionStage::ZeroBytes,
            Some("input operation key is consumed".into()),
        ),
    };
    Some(SubmissionResult {
        status,
        input_id: Some(input.input_id),
        stage,
        output_seq: None,
        reason,
    })
}

fn submission_deadline(conn: &Connection, deadline: Deadline) -> Result<Deadline> {
    let effective = inherited_database_deadline(conn).map_or(deadline.instant(), |bound| {
        bound.instant().min(deadline.instant())
    });
    let deadline = Deadline::until(effective);
    deadline.remaining()?;
    Ok(deadline)
}

pub(crate) fn validate_submission_target(
    conn: &Connection,
    settings: &Settings,
    expected: &RunRecord,
    placement: &Placement,
) -> Result<()> {
    let selected_path = conn
        .path()
        .ok_or_else(|| error("file-backed input database required"))?;
    ensure(
        std::fs::canonicalize(selected_path).map_err(io_error)?
            == std::fs::canonicalize(crate::db::database_path(&settings.database_url)?)
                .map_err(io_error)?,
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
    match prepare_submission_with_host(
        conn,
        settings,
        expected_run,
        expected_placement,
        request,
        deadline,
        &mut NativeInputHost,
    )? {
        SubmissionPreparation::History(result) => Ok(result),
        SubmissionPreparation::Locked {
            attempt,
            held,
            deadline,
        } => submit_wrapped_input_under_pane_lock(
            conn,
            settings,
            expected_run,
            expected_placement,
            &held,
            &attempt,
            deadline,
        ),
    }
}

#[cfg(test)]
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
    let identity = PermissionReceiptIdentity {
        database_identity_json: expected_run.database_identity_json.clone(),
        run_id: expected_run.run_id,
    };
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

pub(crate) fn submit_wrapped_input_under_pane_lock(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    held: &PaneLock,
    attempt: &PreparedSubmission<'_>,
    deadline: Deadline,
) -> std::result::Result<SubmissionResult, InputFailure> {
    submit_wrapped_input_under_pane_lock_with_host(
        conn,
        settings,
        expected_run,
        expected_placement,
        held,
        attempt,
        deadline,
        &mut NativeInputHost,
    )
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
    let identity = PermissionReceiptIdentity {
        database_identity_json: expected_run.database_identity_json.clone(),
        run_id: expected_run.run_id,
    };
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
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_err)?;
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(&tx, settings, expected_run, expected_placement)?;
            let changed = tx.execute(
                "UPDATE member_runs SET reachability='reachable',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3",
                rusqlite::params![crate::time::format_utc(crate::time::now_utc()), expected_run.run_id, expected_run.owner_epoch],
            ).map_err(db_err)?;
            ensure(changed == 1, "submission health ownership changed")?;
            deadline.remaining()?;
            tx.commit().map_err(db_err)
        })?;
        let input = SubmitInput {
            operation_key: attempt.request.operation_key.clone(),
            input_kind: attempt.request.input_kind,
            message_id: attempt.request.message_id,
            literal: attempt.request.literal.clone(),
        };
        let operation = match input.input_kind {
            InputKind::Preview | InputKind::Prompt => InputControlOperation::SubmitPrompt(input),
            InputKind::Exec => InputControlOperation::SubmitExec(input),
            InputKind::PermissionResponse => unreachable!("validated ordinary input kind"),
        };
        let request = InputControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: ControlIdentity::from_run(expected_run),
            expires_at_mono_ns: attempt.expiration.monotonic_ns(),
            operation,
        };
        held.validate_binding(&settings.database_url, expected_placement)?;
        deadline.remaining()?;
        let reply = host
            .exchange(expected_run, &request, deadline.instant())
            .map_err(CafleetError::from)?;
        let InputControlReply::Submission(reply) = reply else {
            return Err(error("ordinary submission requires a submission reply"));
        };
        let recorded = find_submission(conn, &identity, attempt.request, deadline)
            .map_err(CafleetError::from)?;
        context.input_id = recorded.as_ref().map(|input| input.input_id);
        with_database_budget(conn, deadline, |conn| {
            held.validate_binding(&settings.database_url, expected_placement)?;
            validate_submission_target(conn, settings, expected_run, expected_placement)
        })?;
        match (reply.input_id, recorded.as_ref()) {
            (Some(id), Some(input)) => {
                ensure(
                    id == input.input_id,
                    "submission reply input identity mismatch",
                )?;
                let matches = match reply.status {
                    SubmissionStatus::Accepted => {
                        reply.stage == SubmissionStage::Submitted
                            && matches!(
                                (input.state, input.disposition),
                                (InputState::Accepted, _)
                                    | (InputState::Settled, Some(InputDisposition::Submitted))
                            )
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
                ensure(matches, "submission reply contradicts durable state")?;
            }
            (None, None) => ensure(
                matches!(
                    reply.status,
                    SubmissionStatus::Held | SubmissionStatus::Rejected
                ) && reply.stage == SubmissionStage::ZeroBytes,
                "submission outcome requires a durable input identity",
            )?,
            _ => return Err(error("submission reply has no matching durable input")),
        }
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
    let selected_path = conn
        .path()
        .ok_or_else(|| error("file-backed receipt database required"))?;
    ensure(
        std::fs::canonicalize(selected_path).map_err(io_error)?
            == std::fs::canonicalize(crate::db::database_path(&settings.database_url)?)
                .map_err(io_error)?,
        "receipt connection and configured database differ",
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
            let tx = conn.transaction().map_err(db_err)?;
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
            tx.commit().map_err(db_err)?;
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
                    let tx = conn.transaction().map_err(db_err)?;
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
                    tx.commit().map_err(db_err)?;
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
                protocol_version: CONTROL_PROTOCOL_VERSION,
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
                matches!(reply, InputControlReply::Receipt(ReceiptReply::Present { input_id: actual, submission })
                if actual == input_id && submission.input_id == Some(input_id)),
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
            let reply = host
                .exchange(&run, &reconcile, deadline.instant())
                .map_err(CafleetError::from)?;
            ensure(
                matches!(reply, InputControlReply::Receipt(ReceiptReply::Settled { input_id: actual }) if actual == input_id),
                "receipt settlement acknowledgment identity mismatch",
            )?;
            let settled = read_target(conn)?;
            ensure(
                settled.state == InputState::Settled
                    && settled.disposition == Some(disposition)
                    && settled.evidence_ref.as_deref() == Some(evidence_ref),
                "receipt settlement acknowledgment has no matching durable settlement",
            )?;
            deadline.remaining()?;
            let retirement = request(ReceiptAction::Retire { input_id });
            let reply = host
                .exchange(&run, &retirement, deadline.instant())
                .map_err(CafleetError::from)?;
            ensure(
                matches!(reply, InputControlReply::Receipt(ReceiptReply::Retired { input_id: actual }) if actual == input_id),
                "receipt retirement acknowledgment identity mismatch",
            )?;
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
fn invalid(description: &str) -> CafleetError {
    CafleetError::InvalidStoredValue {
        field: "permission_response".into(),
        value: description.into(),
    }
}
fn selected_run(conn: &Connection, run_id: i64) -> Result<RunRecord> {
    let identity = storage::database_identity(conn)?;
    let run = storage::get_run(conn, run_id)?;
    if identity != run.database_identity_json {
        return Err(invalid("selected database identity mismatch"));
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
        .optional()
        .map_err(db_err)?;
    let Some(id) = id else {
        return Ok(None);
    };
    *known_id = Some(id);
    let retained = storage::get_permission_response(conn, id)?;
    let request = NewPermissionResponse {
        operation_key: input.operation_key.clone(),
        metadata: metadata(input, &retained.response),
    };
    let identity = PermissionReceiptIdentity {
        database_identity_json: run.database_identity_json.clone(),
        run_id: run.run_id,
    };
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
        let tx = conn.transaction().map_err(db_err)?;
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
            let original = super::input::inspection_association(
                &tx,
                &run,
                &PermissionInspectInput {
                    interaction_id: input.interaction_id,
                    event_id: input.event_id,
                    director_member_id: input.director_member_id,
                },
            )?;
            ResponsePreparation::Live(Box::new((run, original, retained)))
        };
        deadline.remaining()?;
        tx.commit().map_err(db_err)?;
        Ok(prepared)
    });
    result.map_err(|source| context.failure(source))
}

fn copy_response_input(input: &PermissionRespondInput) -> PermissionRespondInput {
    PermissionRespondInput {
        interaction_id: input.interaction_id,
        event_id: input.event_id,
        director_member_id: input.director_member_id,
        expected_run_id: input.expected_run_id,
        expected_owner_epoch: input.expected_owner_epoch,
        request_token: input.request_token.clone(),
        decision: input.decision,
        decision_source: input.decision_source,
        director_mode: input.director_mode,
        mode_evidence_ref: input.mode_evidence_ref.clone(),
        decision_evidence_ref: input.decision_evidence_ref.clone(),
        operation_key: input.operation_key.clone(),
    }
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
        let placement = with_database_budget(conn, deadline, |conn| {
            crate::broker::get_member(conn, run.member_id, run.fleet_id)?
                .and_then(|member| member.placement)
                .ok_or_else(|| error("permission member placement unavailable"))
        })?;
        let expiration = InputExpiration::capture(deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, &run, &placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate =
            |conn: &mut Connection| -> Result<()> {
                deadline.remaining()?;
                held.validate_binding(&settings.database_url, &placement)?;
                with_database_budget(conn, deadline, |conn| {
                    let tx = conn.transaction().map_err(db_err)?;
                    validate_submission_target(&tx, settings, &run, &placement)?;
                    let view = super::input::inspection_association(
                        &tx,
                        &run,
                        &PermissionInspectInput {
                            interaction_id: input.interaction_id,
                            event_id: input.event_id,
                            director_member_id: input.director_member_id,
                        },
                    )?;
                    ensure(
                        view.event == original.event
                            && view.interaction.as_ref().map(|i| {
                                (i.interaction_id, i.run_id, i.episode_seq, i.classification)
                            }) == original.interaction.as_ref().map(|i| {
                                (i.interaction_id, i.run_id, i.episode_seq, i.classification)
                            }),
                        "permission response association changed",
                    )?;
                    if let Some(ready) = &ready {
                        ensure(
                            storage::get_permission_response(&tx, ready.input.input_id)? == *ready,
                            "ready response snapshot changed",
                        )?;
                    }
                    deadline.remaining()?;
                    tx.commit().map_err(db_err)
                })
            };
        validate(conn)?;
        host.health(&run, deadline.instant())
            .map_err(CafleetError::from)?
            .validate(&run)?;
        validate(conn)?;
        let request = InputControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: ControlIdentity::from_run(&run),
            expires_at_mono_ns: expiration.monotonic_ns(),
            operation: InputControlOperation::PermissionRespond(copy_response_input(input)),
        };
        let reply = host
            .exchange(&run, &request, deadline.instant())
            .map_err(CafleetError::from)?;
        let InputControlReply::PermissionResponse(reply) = reply else {
            return Err(error("permission response requires its typed reply"));
        };
        ensure(
            reply.member_id == run.member_id
                && reply.run_id == run.run_id
                && reply.owner_epoch == input.expected_owner_epoch
                && reply.interaction_id == input.interaction_id
                && reply.event_id == input.event_id
                && reply.decision == input.decision
                && reply.decision_source == input.decision_source,
            "permission response reply identity mismatch",
        )?;
        let receipt = with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction().map_err(db_err)?;
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
            tx.commit().map_err(db_err)?;
            Ok(receipt)
        })?;
        match (reply.submission.input_id, receipt.as_ref()) {
            (Some(id), Some(receipt)) => {
                ensure(
                    id == receipt.input.input_id,
                    "permission response reply input identity mismatch",
                )?;
                let valid = match reply.submission.status {
                    SubmissionStatus::Accepted => {
                        reply.submission.stage == SubmissionStage::Submitted
                            && (receipt.input.state == InputState::Accepted
                                || receipt.input.state == InputState::Settled
                                    && receipt.input.disposition
                                        == Some(InputDisposition::Submitted))
                    }
                    SubmissionStatus::Uncertain => matches!(
                        receipt.input.state,
                        InputState::InProgress | InputState::Uncertain
                    ),
                    SubmissionStatus::Held | SubmissionStatus::Rejected => {
                        reply.submission.stage == SubmissionStage::ZeroBytes
                            && (receipt.input.state == InputState::Ready
                                || receipt.input.state == InputState::Settled
                                    && receipt.input.disposition
                                        != Some(InputDisposition::Submitted))
                    }
                };
                ensure(valid, "permission response reply contradicts durable state")?;
            }
            (None, None) => ensure(
                matches!(
                    reply.submission.status,
                    SubmissionStatus::Held | SubmissionStatus::Rejected
                ) && reply.submission.stage == SubmissionStage::ZeroBytes,
                "permission response outcome requires its durable input",
            )?,
            _ => {
                return Err(error(
                    "permission response reply has no matching durable input",
                ));
            }
        }
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
            let tx = conn.transaction().map_err(db_err)?;
            let view = storage::get_event(&tx, input.event_id)?;
            let run = selected_run(&tx, view.event.run_id)?;
            context.run_id = Some(run.run_id);
            let placement = crate::broker::get_member(&tx, run.member_id, run.fleet_id)?
                .and_then(|member| member.placement)
                .ok_or_else(|| error("permission member placement unavailable"))?;
            validate_submission_target(&tx, settings, &run, &placement)?;
            let view = super::input::inspection_association(&tx, &run, input)?;
            deadline.remaining()?;
            tx.commit().map_err(db_err)?;
            Ok((run, placement, view))
        })?;
        let expiration = InputExpiration::capture(deadline.instant(), &mut RealInputDeadlineClock)?;
        let held = host
            .lock(settings, &run, &placement, deadline)
            .map_err(CafleetError::from)?
            .ok_or_else(|| error("permission pane input lock is busy"))?;
        let validate =
            |conn: &mut Connection| -> Result<()> {
                deadline.remaining()?;
                held.validate_binding(&settings.database_url, &placement)?;
                with_database_budget(conn, deadline, |conn| {
                    let tx = conn.transaction().map_err(db_err)?;
                    validate_submission_target(&tx, settings, &run, &placement)?;
                    let view = super::input::inspection_association(&tx, &run, input)?;
                    ensure(
                        view.event == original.event
                            && view.interaction.as_ref().map(|i| {
                                (i.interaction_id, i.run_id, i.episode_seq, i.classification)
                            }) == original.interaction.as_ref().map(|i| {
                                (i.interaction_id, i.run_id, i.episode_seq, i.classification)
                            }),
                        "permission inspection association changed",
                    )?;
                    deadline.remaining()?;
                    tx.commit().map_err(db_err)
                })
            };
        validate(conn)?;
        host.health(&run, deadline.instant())
            .map_err(CafleetError::from)?
            .validate(&run)?;
        validate(conn)?;
        let request = InputControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: ControlIdentity::from_run(&run),
            expires_at_mono_ns: expiration.monotonic_ns(),
            operation: InputControlOperation::PermissionInspect(PermissionInspectInput {
                event_id: input.event_id,
                interaction_id: input.interaction_id,
                director_member_id: input.director_member_id,
            }),
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
            reply.event_id == input.event_id
                && reply.interaction_id == input.interaction_id
                && reply.message_id == original.event.message_id.expect("validated inbox message")
                && reply.run_id == run.run_id
                && reply.owner_epoch == run.owner_epoch
                && reply.member_id == run.member_id
                && reply.fleet_id == run.fleet_id
                && reply.director_member_id == run.director_member_id
                && reply.episode_seq == interaction.episode_seq
                && reply.backend == run.backend
                && reply.native_version == run.native_version
                && reply.profile_version == run.profile_version,
            "permission inspection reply identity mismatch",
        )?;
        storage::evidence::validate(&reply.evidence)?;
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
        if let Some(token) = &reply.request_token {
            validate_request_token(token)?;
        }
        let expected_order = [PermissionDecision::Allow, PermissionDecision::Deny]
            .into_iter()
            .filter(|decision| reply.supported_decisions.contains(decision))
            .collect::<Vec<_>>();
        ensure(
            reply.supported_decisions == expected_order,
            "permission inspection decisions are not an ordered subset",
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
mod tests {
    use super::*;
    use crate::runtime::runner_tests::Fixture;
    use crate::runtime::{
        InputServiceFixture, input_service_barrier, input_service_composer, input_service_records,
        wait_input_service_record, with_input_service,
    };
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::net::Shutdown;
    use std::time::Duration;

    fn inspection_barrier(fixture: &mut InputServiceFixture, start: usize, barrier: &str) {
        native_answers_through_barrier(fixture, start, barrier, &[]);
    }
    fn native_answers_through_barrier(
        fixture: &mut InputServiceFixture,
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
        use crate::runtime::runner_tests::control_service::observation_services::synthetic;

        let mut fixture = InputServiceFixture::configured(false, synthetic);
        fixture.start_running();
        let until = Instant::now() + Duration::from_secs(2);
        fixture.master.write_all(b"form-inspection\r").unwrap();
        fixture.read_until("Operation: write form-inspection.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, message_id) = loop {
            let row = with_database_budget(&mut fixture.fixture.conn, Deadline::until(until), |conn| {
                conn.query_row(
                    "SELECT event_id,interaction_id,message_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",
                    [fixture.fixture.run.run_id], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?,row.get::<_, i64>(2)?)),
                ).optional().map_err(db_err)
            }).unwrap();
            if let Some(row) = row {
                break row;
            }
            assert!(Instant::now() < until, "original stable-request deadline");
            std::thread::sleep(Duration::from_millis(2));
        };
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        let settings = Settings {
            database_url: format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
            max_text_len: 200,
            multiplexer: Some("tmux".into()),
        };
        let counts = |conn: &Connection| {
            [
                "member_input_operations",
                "member_permission_responses",
                "member_permission_response_claims",
            ]
            .map(|table| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            })
        };
        let before = counts(&fixture.fixture.conn);
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
        assert_eq!(counts(&fixture.fixture.conn), before);
        let notified: Option<String> = fixture
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(notified.is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-barrier");
        fixture.finish();
    }

    struct InspectionHost {
        socket: SocketHost,
        wrong_event: bool,
    }
    impl InputCommandHost for InspectionHost {
        fn lock(
            &mut self,
            settings: &Settings,
            run: &RunRecord,
            placement: &Placement,
            deadline: Deadline,
        ) -> std::result::Result<Option<PaneLock>, InputFailure> {
            self.socket.lock(settings, run, placement, deadline)
        }
        fn health(
            &mut self,
            run: &RunRecord,
            deadline: Instant,
        ) -> std::result::Result<HealthSnapshot, InputFailure> {
            self.socket.health(run, deadline)
        }
        fn exchange(
            &mut self,
            run: &RunRecord,
            request: &InputControlRequest,
            deadline: Instant,
        ) -> std::result::Result<InputControlReply, InputFailure> {
            assert!(matches!(
                request.operation,
                InputControlOperation::PermissionInspect(_)
            ));
            let mut reply = self.socket.exchange(run, request, deadline)?;
            if self.wrong_event {
                let InputControlReply::Inspection(value) = &mut reply else {
                    panic!("actual typed permission inspection")
                };
                value.event_id += 1;
            }
            Ok(reply)
        }
    }

    #[test]
    fn permission_inspection_client_uses_real_guard_original_sql_bound_and_validates_live_reply() {
        use crate::coding_agent::inspection_fixture_definition;
        use crate::runtime::runner_tests::control_service::observation_services::synthetic;

        let mut fixture = InputServiceFixture::configured(false, |fixture, spec| {
            synthetic(fixture, spec);
            let path = fixture._dir.path().join("observation-services.json");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            file.write_all(
                &serde_json::to_vec(
                    &serde_json::json!({"catalog":inspection_fixture_definition()}),
                )
                .unwrap(),
            )
            .unwrap();
            file.sync_all().unwrap();
        });
        fixture.start_running();
        let outer = Instant::now() + Duration::from_secs(2);
        fixture
            .master
            .write_all(b"form-executable-inspection\r")
            .unwrap();
        fixture.read_until("Operation: write form-executable-inspection.txt");
        let output_start = fixture.output.len();
        let (event_id, interaction_id, message_id) = loop {
            let ids = with_database_budget(&mut fixture.fixture.conn,Deadline::until(outer),|conn| {
                conn.query_row("SELECT event_id,interaction_id,message_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",[fixture.fixture.run.run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).optional().map_err(db_err)
            }).unwrap();
            if let Some(ids) = ids {
                break ids;
            }
            assert!(
                Instant::now() < outer,
                "original request stabilization deadline"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
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
        let mut host = InspectionHost {
            socket: SocketHost::new(&settings, &placement, inherited),
            wrong_event: false,
        };
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
        assert_eq!(host.socket.calls, [1, 1, 1]);
        assert!(host.socket.expiration.is_some());
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

        let mut wrong = InspectionHost {
            socket: SocketHost::new(&settings, &placement, outer),
            wrong_event: true,
        };
        let failure = match inspect_permission_with_host(
            &mut fixture.fixture.conn,
            &settings,
            &request,
            Deadline::until(outer),
            &mut wrong,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("a real typed reply still requires its pinned event identity"),
        };
        assert_eq!(wrong.socket.calls, [1, 1, 1]);
        assert_eq!(failure.run_id, Some(run.run_id));
        assert_eq!(failure.interaction_id, Some(interaction_id));
        assert!(
            !failure
                .source
                .message()
                .contains("form-executable-inspection")
        );
        assert!(
            !failure
                .source
                .message()
                .contains(value.request_token.as_ref().unwrap())
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
        for table in [
            "member_input_operations",
            "member_permission_responses",
            "member_permission_response_claims",
        ] {
            let count: i64 = fixture
                .fixture
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        let notified: Option<String> = fixture
            .fixture
            .conn
            .query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(notified.is_none());
        inspection_barrier(&mut fixture, output_start, "inspection-client-barrier");
        fixture.finish();
    }

    #[test]
    fn permission_response_history_honors_expired_inherited_budget_before_sql_or_live_calls() {
        use crate::runtime::runner_tests::permission_history::{Case, Retained};
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let mut case = Case::new(Retained::Accepted);
        let receipt = storage::get_permission_response(&case.fixture.conn, case.input_id).unwrap();
        let metadata = &receipt.response.metadata;
        let input = PermissionRespondInput {
            interaction_id: metadata.interaction_id,
            event_id: metadata.event_id,
            director_member_id: metadata.director_member_id,
            expected_run_id: receipt.input.run_id,
            expected_owner_epoch: metadata.expected_owner_epoch,
            request_token: metadata.request_token.clone(),
            decision: metadata.decision,
            decision_source: metadata.decision_source,
            director_mode: metadata.director_mode,
            mode_evidence_ref: metadata.mode_evidence_ref.clone(),
            decision_evidence_ref: metadata.decision_evidence_ref.clone(),
            operation_key: receipt.input.operation_key.clone(),
        };
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
                }))
                .map_err(db_err)?;
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
                    .map_err(db_err)
            });
        let expired = expired.expect("expired-frame invocation completed");
        let expired_calls = expired_calls.expect("SQL count sampled after expired invocation");
        eprintln!(
            "permission inherited expiry: result={:?}; SQL={expired_calls}; scope_cleanup={scoped:?}",
            expired.as_ref().map(|reply| (
                reply.submission.status,
                reply.submission.input_id,
                reply.submission.stage
            ))
        );
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
        crate::runtime::runner_tests::control_service::observation_services::synthetic(
            fixture, spec,
        );
        let mut catalog = crate::coding_agent::inspection_fixture_definition();
        catalog["profiles"][0]["capabilities"]["answers"][0]["stages"] = serde_json::json!([{
            "bytes_hex":"790d","required_rule_id":"synthetic-permission","required_choice_id":"allow-once"
        }]);
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
        fixture: &mut InputServiceFixture,
        until: Instant,
        key: &str,
    ) -> (Settings, RunRecord, Placement, PermissionRespondInput) {
        fixture.master.write_all(b"form-response\r").unwrap();
        fixture.read_until("Operation: write form-response.txt");
        let (event_id, interaction_id) = loop {
            let row=with_database_budget(&mut fixture.fixture.conn,Deadline::until(until),|conn| {
                conn.query_row("SELECT event_id,interaction_id FROM member_runtime_events WHERE run_id=?1 AND kind='interaction_opened'",[fixture.fixture.run.run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).optional().map_err(db_err)
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
        fixture: &InputServiceFixture,
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
        let mut fixture = InputServiceFixture::configured(false, response_services);
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
                rusqlite::params![crate::time::format_utc(crate::time::now_utc()), interaction_id],
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
        let master = fixture.master.try_clone().unwrap();
        let process = ProcessIdentity::read(run.owner_pid).unwrap().unwrap();
        assert_eq!(process.start_identity, run.owner_start_identity);
        let database = settings.database_url.clone();
        host.after_health = Some(Box::new(move || {
            assert!(process.matches().unwrap(), "generation-bound owned runner");
            rustix::termios::tcsetwinsize(
                &master,
                rustix::termios::Winsize {
                    ws_row: 24,
                    ws_col: 81,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                },
            )
            .unwrap();
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(process.pid).unwrap()),
                nix::sys::signal::Signal::SIGWINCH,
            )
            .unwrap();
            let mut conn = crate::db::connect(&database).unwrap();
            loop {
                let current = with_database_budget(&mut conn, Deadline::until(until), |conn| {
                    storage::get_event(conn, original.event.event_id)
                })
                .unwrap();
                let after = current.interaction.as_ref().unwrap();
                let sequence = after.evidence_json["screen_seq"].as_u64().unwrap();
                if sequence > previous {
                    assert_eq!(current.event, original.event);
                    let mut expected = original.interaction.as_ref().unwrap().clone();
                    expected.evidence_json = after.evidence_json.clone();
                    expected.evidence_digest = after.evidence_digest.clone();
                    assert_eq!(*after, expected);
                    assert_ne!(
                        after.evidence_digest,
                        original.interaction.as_ref().unwrap().evidence_digest
                    );
                    assert!(after.closed_at.is_none());
                    eprintln!(
                        "response compatible refresh: interaction_id={}; screen_seq={previous}->{sequence}; opening unchanged",
                        after.interaction_id
                    );
                    break;
                }
                assert!(
                    Instant::now() < until,
                    "actual writer commits resize evidence under the original command budget"
                );
                std::thread::sleep(
                    Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
                );
            }
        }));
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
            "compatible actual refresh permits one exchange: {:?}",
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
        let mut fixture = InputServiceFixture::configured(false, response_services);
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
        let mut fixture = InputServiceFixture::configured(false, response_services);
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
        use super::super::input_recovery::tests::TypedHost;
        use crate::runtime::runner_tests::permission_history::{Case, Retained};

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
        PaneLock::try_acquire(&case.settings.database_url, &placement)
            .unwrap()
            .expect("failed live invocation releases its genuine pane guard")
            .validate_binding(&case.settings.database_url, &placement)
            .unwrap();
    }

    #[test]
    fn exclusive_permission_receipt_client_preserves_uncertain_claim_and_continuous_guard() {
        use super::super::input_recovery::tests::TypedHost;
        use crate::runtime::runner_tests::permission_history::{Case, Retained};

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
                PaneLock::try_acquire(&case.settings.database_url, &placement)
                    .unwrap()
                    .unwrap()
                    .validate_binding(&case.settings.database_url, &placement)
                    .unwrap();
            }
            RunLock::acquire(&case.fixture.conn, &current.owner())
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
        use super::super::input_recovery::tests::{Termination, TypedHost};

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
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .expect("the acquired pane guard is released on every routed outcome")
                    .validate_binding(&settings.database_url, &placement)
                    .unwrap();
            }
            RunLock::acquire(&fixture.conn, &current.owner())
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

    fn digest(run_id: i64, request: &SubmitInput) -> String {
        let value = serde_json::json!({
            "kind":request.input_kind.as_str(), "literal":request.literal,
            "message_id":request.message_id, "run_id":run_id,
        });
        Sha256::digest(serde_json::to_vec(&value).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
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

    fn service_target(fixture: &InputServiceFixture) -> (Settings, RunRecord, Placement) {
        let conn = &fixture.fixture.conn;
        let run = storage::get_run(conn, fixture.fixture.run.run_id).unwrap();
        let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
        (
            Settings {
                database_url: format!("sqlite:///{}", conn.path().unwrap()),
                max_text_len: 200,
                multiplexer: Some("tmux".into()),
            },
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

        fn failure(source: CafleetError) -> InputFailure {
            InputFailure {
                source,
                run_id: None,
                interaction_id: None,
                input_id: None,
                operation_key: None,
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
                .map_err(Self::failure)
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
                None => super::super::probe_health(run, deadline).map_err(Self::failure)?,
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
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                    .unwrap()
                    .is_none(),
                "the actual guard spans every exchange"
            );
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
                }));
            }
            let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)
                .map_err(Self::failure)?;
            let mut client =
                super::super::control::ControlClient::connect(&paths.control_socket, deadline)
                    .map_err(Self::failure)?;
            if matches!(self.reply, ReplyMode::Disconnected) {
                client
                    .stream
                    .shutdown(Shutdown::Read)
                    .map_err(io_error)
                    .map_err(Self::failure)?;
                let bytes = super::super::input_protocol::encode_input_request(request)
                    .map_err(Self::failure)?;
                let mut offset = 0;
                while offset < bytes.len() {
                    check_deadline(Instant::now(), deadline).map_err(Self::failure)?;
                    match client.stream.write(&bytes[offset..]) {
                        Ok(0) => panic!("owned external request disconnected during send"),
                        Ok(count) => offset += count,
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            super::super::control::pause(deadline).map_err(Self::failure)?;
                        }
                        Err(cause) => return Err(Self::failure(io_error(cause))),
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
                        .map_err(Self::failure)?;
                    if retained.is_some_and(|input| input.state == InputState::Accepted) {
                        break;
                    }
                    super::super::control::pause(deadline).map_err(Self::failure)?;
                }
                drop(client);
                return Err(Self::failure(CafleetError::App(
                    "owned reply consumer closed".into(),
                )));
            }
            let mut reply = client
                .exchange_input(request, deadline)
                .map_err(Self::failure)?;
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

    fn health_records(conn: &Connection) -> [Vec<Vec<rusqlite::types::Value>>; 3] {
        ["member_interactions", "member_runtime_events", "messages"].map(|table| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
    }

    fn accepted_service_receipt(
        fixture: &mut InputServiceFixture,
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

        use crate::runtime::runner_tests::permission_history::{Case, Retained};
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
                    payload_digest: digest(owner.run_id, &request),
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
            let settings = Settings {
                database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
                max_text_len: 200,
                multiplexer: Some("tmux".into()),
            };
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
        eprintln!(
            "prepared history: expired={:?}; expired_reads={expired_reads}; active_sql_budgets_past_original={:?}; authorizer_cleanup={removed:?}",
            expired
                .as_ref()
                .map(|result| (result.status, result.input_id, result.stage)),
            budgets
                .lock()
                .unwrap()
                .iter()
                .map(|budget| budget.saturating_duration_since(original_active))
                .collect::<Vec<_>>()
        );
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
            let until = outer.min(Instant::now() + Duration::from_secs(2));
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
            assert_eq!(row.payload_digest, digest(run.run_id, &request));
            assert_eq!(row.state, InputState::Accepted);
            assert!(
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            drop(held);
            assert!(
                PaneLock::try_acquire(&settings.database_url, &placement)
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
            assert_eq!(records[0].len(), 1, "meaningful open interaction control");
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
                    payload_digest: digest(run.run_id, &request),
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
            assert_eq!(row.payload_digest, digest(run.run_id, &request));
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
}
