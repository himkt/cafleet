use super::*;
use crate::broker::members::db_err;
use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, *};
use crate::config::Settings;
use crate::input::{Deadline, PaneLock, with_database_budget};
use rusqlite::{Connection, OptionalExtension};
use std::time::Instant;

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
#[expect(
    dead_code,
    reason = "the staged history path intentionally invokes no live host method"
)]
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
