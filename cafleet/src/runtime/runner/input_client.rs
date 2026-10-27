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

struct NativeInputHost;
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

fn validate_submission_target(
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
fn submit_wrapped_input_under_pane_lock_with_host(
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
fn validate_respond(input: &PermissionRespondInput) -> Result<()> {
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
fn projection(run: &RunRecord, receipt: PermissionReceipt) -> Result<PermissionResponseResult> {
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
        InputState::Ready => return Err(error("live permission coordination is not implemented")),
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
fn respond(
    conn: &mut Connection,
    input: &PermissionRespondInput,
    deadline: Deadline,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    let mut context = InputContext {
        run_id: Some(input.expected_run_id),
        interaction_id: Some(input.interaction_id),
        input_id: None,
        operation_key: Some(input.operation_key.clone()),
    };
    let result = (|| {
        validate_respond(input)?;
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            let run = selected_run(&tx, input.expected_run_id)?;
            let id = tx.query_row("SELECT input_id FROM member_input_operations WHERE run_id=?1 AND operation_key=?2", rusqlite::params![run.run_id, input.operation_key], |row| row.get::<_, i64>(0)).optional().map_err(db_err)?;
            context.input_id = id;
            let result = if let Some(id) = id {
                let retained = storage::get_permission_response(&tx, id)?;
                let request = NewPermissionResponse {
                    operation_key: input.operation_key.clone(),
                    metadata: metadata(input, &retained.response),
                };
                let identity = PermissionReceiptIdentity {
                    database_identity_json: run.database_identity_json.clone(),
                    run_id: run.run_id,
                };
                let receipt = storage::lookup_permission_response(&tx, &identity, &request)?
                    .expect("the same read snapshot retains the selected input");
                projection(&run, receipt)?
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
                return Err(error("live permission coordination is not implemented"));
            };
            deadline.remaining()?;
            tx.commit().map_err(db_err)?;
            Ok(result)
        })
    })();
    result.map_err(|source| context.failure(source))
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the actual live host adapter follows the retained-history checkpoint"
    )
)]
pub(crate) fn respond_permission_with_host(
    conn: &mut Connection,
    _settings: &Settings,
    input: &PermissionRespondInput,
    deadline: Deadline,
    _host: &mut impl InputCommandHost,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    respond(conn, input, deadline)
}
pub(crate) fn respond_permission(
    conn: &mut Connection,
    _settings: &Settings,
    input: &PermissionRespondInput,
    deadline: Deadline,
) -> std::result::Result<PermissionResponseResult, InputFailure> {
    respond(conn, input, deadline)
}
pub(crate) fn inspect_permission(
    conn: &mut Connection,
    _settings: &Settings,
    input: &PermissionInspectInput,
    deadline: Deadline,
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
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction().map_err(db_err)?;
            let run_id = tx
                .query_row(
                    "SELECT run_id FROM member_runtime_events WHERE event_id=?1",
                    [input.event_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db_err)?
                .ok_or_else(|| {
                    CafleetError::Value(format!("Event {} not found", input.event_id))
                })?;
            context.run_id = Some(run_id);
            let run = selected_run(&tx, run_id)?;
            association(
                &tx,
                &run,
                input.interaction_id,
                input.event_id,
                input.director_member_id,
            )?;
            Err(error("live permission coordination is not implemented"))
        })
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
            let health = super::super::probe_health(run, deadline).map_err(Self::failure)?;
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
            assert_eq!(self.calls[2], 1, "an explicit attempt exchanges once");
            assert_eq!(deadline, self.until);
            assert_eq!(
                serde_json::to_value(&request.identity).unwrap(),
                serde_json::to_value(ControlIdentity::from_run(run)).unwrap()
            );
            if let Some(expiration) = self.expiration {
                assert_eq!(request.expires_at_mono_ns, expiration);
            }
            self.requests.push(
                serde_json::from_slice(
                    &super::super::input_protocol::encode_input_request(request).unwrap()[4..],
                )
                .unwrap(),
            );
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
                    _ => panic!("ordinary fixture sends an ordinary request"),
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
            client
                .exchange_input(request, deadline)
                .map_err(Self::failure)
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
        with_input_service(|fixture, outer| {
            let (settings, run, placement) = service_target(fixture);
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
