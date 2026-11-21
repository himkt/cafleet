use crate::broker::runtime::*;
use crate::coding_agent::AnswerCapability;
use crate::coding_agent::catalog::valid_choice_id;
use crate::runtime::with_database_deadline;
use crate::time::now_stamp;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

fn map_input(row: &rusqlite::Row<'_>) -> rusqlite::Result<InputOperation> {
    Ok(InputOperation {
        input_id: row.get("input_id")?,
        run_id: row.get("run_id")?,
        operation_key: row.get("operation_key")?,
        kind: row.get("kind")?,
        payload_digest: row.get("payload_digest")?,
        state: row.get("state")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        disposition: row.get("disposition")?,
        evidence_ref: row.get("evidence_ref")?,
        permission_response: None,
        delivery_method: row.get("delivery_method")?,
        delivery_reason: row.get("delivery_reason")?,
        deferred_at: row.get("deferred_at")?,
        defer_reason: row.get("defer_reason")?,
        stop_cause: row.get("stop_cause")?,
    })
}

impl InputOperation {
    pub(crate) fn delivery(&self) -> Option<DeliveryProjection> {
        delivery_projection(self).expect("validated input receipts project their delivery")
    }
}

const FALLBACK_REASONS: [&str; 1] = ["submission_plan_unavailable"];

fn delivery_projection(input: &InputOperation) -> Result<Option<DeliveryProjection>, CafleetError> {
    let unknown = || {
        invalid(
            "member_input_operations.delivery_method",
            "unknown stored delivery combination",
        )
    };
    if input.kind == InputKind::PermissionResponse {
        require(
            input.delivery_method.is_none()
                && input.delivery_reason.is_none()
                && input.deferred_at.is_none(),
            "member_input_operations.delivery_method",
            "permission responses carry no ordinary delivery metadata",
        )?;
        return Ok(None);
    }
    let method = input.delivery_method;
    let recorded = || match method {
        Some(DeliveryMethod::Profile) if input.delivery_reason.is_none() => Ok(None),
        Some(DeliveryMethod::Fallback)
            if input
                .delivery_reason
                .as_deref()
                .is_some_and(|reason| FALLBACK_REASONS.contains(&reason)) =>
        {
            Ok(input.delivery_reason.clone())
        }
        _ => Err(unknown()),
    };
    let (status, reason) = match (input.state, input.disposition) {
        (InputState::Ready, _) if input.deferred_at.is_some() => {
            require(
                method.is_none() && input.delivery_reason.is_none(),
                "member_input_operations.delivery_method",
                "a queued input has no selected method",
            )?;
            (DeliveryStatus::Queued, input.defer_reason.clone())
        }
        (InputState::Ready, _) => {
            require(
                method.is_none() && input.delivery_reason.is_none(),
                "member_input_operations.delivery_method",
                "a ready input has no selected method",
            )?;
            (DeliveryStatus::Held, Some("zero_bytes_ready".to_owned()))
        }
        (InputState::InProgress | InputState::Uncertain, _)
        | (InputState::Settled, Some(InputDisposition::AbandonedUncertain)) => {
            (DeliveryStatus::Uncertain, recorded()?)
        }
        (InputState::Accepted, _) | (InputState::Settled, Some(InputDisposition::Submitted)) => {
            (DeliveryStatus::Submitted, recorded()?)
        }
        (InputState::Settled, Some(InputDisposition::NotSubmitted)) => match method {
            Some(_) => (DeliveryStatus::NotSubmitted, recorded()?),
            None => (
                DeliveryStatus::NotSubmitted,
                Some(
                    input
                        .delivery_reason
                        .clone()
                        .unwrap_or_else(|| "method_unrecorded".to_owned()),
                ),
            ),
        },
        (InputState::Settled, None) => return Err(unknown()),
    };
    Ok(Some(DeliveryProjection {
        status,
        method,
        reason,
    }))
}

fn validate_input(
    conn: &Connection,
    mut input: InputOperation,
) -> Result<InputOperation, CafleetError> {
    let run = get_run(conn, input.run_id)?;
    require(
        input.stop_cause.is_none()
            || matches!(input.state, InputState::Uncertain | InputState::Settled),
        "member_input_operations.stop_cause",
        "a stop cause requires an uncertain or settled input",
    )?;
    require(
        input.kind != InputKind::Preview,
        "member_input_operations.kind",
        "previews are delivered without a durable receipt",
    )?;
    input.permission_response = validate_input_response(conn, &run, &input)?;
    delivery_projection(&input)?;
    Ok(input)
}

pub(crate) fn input_payload_digest(run_id: i64, input: &crate::runtime::SubmitInput) -> String {
    let payload = json!({"run_id":run_id,"kind":input.input_kind.as_str(),"literal":input.literal});
    crate::digest::sha256_hex(crate::digest::canonical_json(&payload).as_bytes())
}

/// How a new ordinary receipt is reserved: the delivery selected before its first byte, or a
/// zero-byte queue admission with its defer reason.
#[derive(Clone, Copy)]
pub(crate) struct NewReservation<'a> {
    pub(crate) selection: Option<(DeliveryMethod, Option<&'a str>)>,
    pub(crate) defer_reason: Option<&'a str>,
}

#[cfg(test)]
impl NewReservation<'_> {
    pub(crate) const PLAIN: Self = Self {
        selection: Some((DeliveryMethod::Profile, None)),
        defer_reason: None,
    };
}

pub(crate) fn begin_deferred_attempt(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    method: DeliveryMethod,
    reason: Option<&str>,
) -> Result<InputOperation, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == InputState::Ready && input.deferred_at.is_some(),
            "member_input_operations.state",
            "a deferred attempt starts from its queued receipt",
        )?;
        update_input(
            conn,
            input_id,
            "UPDATE member_input_operations SET state='in_progress',updated_at=?1,delivery_method=?2,delivery_reason=?3 WHERE input_id=?4 AND state='ready'",
            params![now_stamp(), method.as_str(), reason, input_id],
            CAS_FAILED,
        )
    })
}

/// Whether an ordinary attempt of the run, deferred or immediate, may have written bytes and
/// awaits reconciliation, which stops automatic ordinary dispatch until it settles.
pub(crate) fn has_unresolved_ordinary_attempt(
    conn: &Connection,
    run_id: i64,
) -> Result<bool, CafleetError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM member_input_operations WHERE run_id=?1 AND kind IN ('prompt','exec') AND state IN ('in_progress','uncertain'))",
        [run_id],
        |row| row.get(0),
    )?)
}

/// Returns a reserved immediate request that wrote zero bytes to the queue, clearing its unexecuted
/// selection and recording why it was deferred.
pub(crate) fn requeue_zero_byte_immediate(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    defer_reason: &str,
) -> Result<InputOperation, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == InputState::InProgress
                && input.deferred_at.is_none()
                && input.kind != InputKind::PermissionResponse,
            "member_input_operations.state",
            "only an immediate ordinary attempt returns to the queue",
        )?;
        let now = now_stamp();
        update_input(
            conn,
            input_id,
            "UPDATE member_input_operations SET state='ready',updated_at=?1,delivery_method=NULL,delivery_reason=NULL,deferred_at=?1,defer_reason=?2 WHERE input_id=?3 AND state='in_progress'",
            params![now, defer_reason, input_id],
            CAS_FAILED,
        )
    })
}

pub(crate) type UnpublishedDeliveryResult = (i64, &'static str, Option<StopCause>);

pub(crate) fn unpublished_delivery_results(
    conn: &Connection,
    run_id: i64,
) -> Result<Vec<UnpublishedDeliveryResult>, CafleetError> {
    let mut statement = conn.prepare(
        "SELECT input_id FROM member_input_operations WHERE run_id=?1 AND kind IN ('prompt','exec') AND (deferred_at IS NOT NULL OR stop_cause IS NOT NULL) AND input_id NOT IN (SELECT json_extract(payload_json,'$.input_id') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result') ORDER BY input_id",
    )?;
    let ids = statement
        .query_map([run_id], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut results = Vec::new();
    for input_id in ids {
        let input = get_input(conn, input_id)?;
        if input.deferred_at.is_none() && input.stop_cause.is_some() {
            results.push((input_id, "unknown", input.stop_cause));
            continue;
        }
        let delivery = input
            .delivery()
            .expect("a deferred input is an ordinary input");
        let stage = match delivery.status {
            DeliveryStatus::Submitted => "submitted",
            DeliveryStatus::NotSubmitted => "zero_bytes",
            DeliveryStatus::Uncertain => "unknown",
            DeliveryStatus::Queued | DeliveryStatus::Held | DeliveryStatus::NotApplicable => {
                continue;
            }
        };
        let stop_cause = input
            .stop_cause
            .filter(|_| delivery.status == DeliveryStatus::Uncertain);
        results.push((input_id, stage, stop_cause));
    }
    Ok(results)
}

/// Settles every queued receipt of a run whose runner was lost, after ownership fencing has
/// established that its in-memory payload and writer are gone.
pub(crate) fn settle_lost_deferred(
    conn: &mut Connection,
    owner: &RunOwner,
) -> Result<(), CafleetError> {
    owned_transaction(conn, owner, settle_lost_deferred_in_transaction)
}

/// `settle_lost_deferred` inside a caller's transaction that already holds the run's reconciler
/// ownership.
pub(crate) fn settle_lost_deferred_in_transaction(
    conn: &Connection,
    run: &RunRecord,
) -> Result<(), CafleetError> {
    require(
        run.owner_kind == OwnerKind::Reconciler,
        "owner_kind",
        "lost queued input settles under reconciler ownership",
    )?;
    conn.execute(
        "UPDATE member_input_operations SET state='settled',disposition='not_submitted',updated_at=?1,delivery_method=NULL,delivery_reason='runner_lost' WHERE run_id=?2 AND state='ready' AND deferred_at IS NOT NULL",
        params![now_stamp(), run.run_id],
    )?;
    Ok(())
}

/// Settles a queued input that wrote zero bytes as not submitted with its outcome reason.
pub(crate) fn settle_zero_byte_deferred(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    expected: InputState,
    reason: &str,
) -> Result<InputOperation, CafleetError> {
    require(
        matches!(expected, InputState::Ready | InputState::InProgress),
        "member_input_operations.state",
        "only a zero-byte deferred input is cancelled",
    )?;
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == expected && input.deferred_at.is_some(),
            "member_input_operations.state",
            "deferred input state changed",
        )?;
        update_input(
            conn,
            input_id,
            "UPDATE member_input_operations SET state='settled',disposition='not_submitted',updated_at=?1,delivery_method=NULL,delivery_reason=?2 WHERE input_id=?3 AND state=?4",
            params![now_stamp(), reason, input_id, expected.as_str()],
            CAS_FAILED,
        )
    })
}

pub(crate) fn get_input(conn: &Connection, input_id: i64) -> Result<InputOperation, CafleetError> {
    let input = conn
        .query_row(
            "SELECT * FROM member_input_operations WHERE input_id=?1",
            [input_id],
            map_input,
        )
        .optional()?
        .ok_or_else(|| CafleetError::App(format!("Input {input_id} not found")))?;
    validate_input(conn, input)
}

pub(crate) fn find_input(
    conn: &Connection,
    run_id: i64,
    key: &str,
) -> Result<Option<InputOperation>, CafleetError> {
    let input = conn
        .query_row(
            "SELECT * FROM member_input_operations WHERE run_id=?1 AND operation_key=?2",
            params![run_id, key],
            map_input,
        )
        .optional()?;
    input.map(|input| validate_input(conn, input)).transpose()
}

pub(crate) enum InputReservation {
    Inserted(InputOperation),
    Existing(InputOperation),
}

pub(crate) fn reserve_input_with_provenance(
    conn: &mut Connection,
    owner: &RunOwner,
    new: &NewInput,
    reservation: NewReservation<'_>,
) -> Result<InputReservation, CafleetError> {
    require(
        matches!(new.kind, InputKind::Prompt | InputKind::Exec),
        "kind",
        "only prompt and exec input reserves an ordinary receipt",
    )?;
    nonblank(&new.operation_key, "operation_key")?;
    hex(&new.payload_digest, 64, "payload_digest")?;
    owned_transaction(conn, owner, |conn, run| {
        if let Some(input) = find_input(conn, run.run_id, &new.operation_key)? {
            require(
                input.payload_digest == new.payload_digest && input.kind == new.kind,
                "operation_key",
                "conflicting operation key reuse",
            )?;
            return Ok(InputReservation::Existing(input));
        }
        require(
            run.status == RunStatus::Running && historical_ownership(conn, run)?.is_none(),
            "member_runs.status",
            "new input requires an active running member",
        )?;
        require_capacity(conn, run.run_id)?;
        let now = now_stamp();
        let selection = reservation.selection;
        let deferred = reservation
            .defer_reason
            .map(|reason| (now.as_str(), reason));
        require(
            selection.is_none() || deferred.is_none(),
            "delivery_method",
            "a queued input selects its method when its attempt starts",
        )?;
        conn.execute(
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,delivery_method,delivery_reason,deferred_at,defer_reason)
             VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10)",
            params![run.run_id,new.operation_key,new.kind.as_str(),new.payload_digest,
                if deferred.is_some() { "ready" } else { "in_progress" },now,
                selection.map(|(method, _)| method.as_str()),selection.and_then(|(_, reason)| reason),
                deferred.map(|(at, _)| at),deferred.map(|(_, reason)| reason)],
        )?;
        get_input(conn, conn.last_insert_rowid()).map(InputReservation::Inserted)
    })
}

fn owned_input(
    conn: &Connection,
    run: &RunRecord,
    input_id: i64,
) -> Result<InputOperation, CafleetError> {
    let input = get_input(conn, input_id)?;
    require(
        input.run_id == run.run_id,
        "input_id",
        "input belongs to another run",
    )?;
    Ok(input)
}

pub(crate) fn record_input_state(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    expected: InputState,
    next: InputState,
) -> Result<InputOperation, CafleetError> {
    require(
        matches!(
            (expected, next),
            (
                InputState::InProgress,
                InputState::Ready | InputState::Accepted
            )
        ),
        "member_input_operations.state",
        "invalid input transition",
    )?;
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == expected,
            "member_input_operations.state",
            "input state changed",
        )?;
        let sql = if next == InputState::Ready {
            "UPDATE member_input_operations SET state=?1,updated_at=?2,delivery_method=NULL,delivery_reason=NULL WHERE input_id=?3 AND state=?4"
        } else {
            "UPDATE member_input_operations SET state=?1,updated_at=?2 WHERE input_id=?3 AND state=?4"
        };
        update_input(
            conn,
            input_id,
            sql,
            params![next.as_str(), now_stamp(), input_id, expected.as_str()],
            CAS_FAILED,
        )
    })
}

const CAS_FAILED: &str = "input CAS failed";

/// Applies one compare-and-set `UPDATE` to an input row and returns the updated input.
fn update_input(
    conn: &Connection,
    input_id: i64,
    sql: &str,
    params: impl rusqlite::Params,
    failure: &str,
) -> Result<InputOperation, CafleetError> {
    let changed = conn.execute(sql, params)?;
    require(changed == 1, "member_input_operations.state", failure)?;
    get_input(conn, input_id)
}

pub(crate) fn record_input_uncertain(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    cause: StopCause,
) -> Result<InputOperation, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == InputState::InProgress,
            "member_input_operations.state",
            "input state changed",
        )?;
        update_input(
            conn,
            input_id,
            "UPDATE member_input_operations SET state='uncertain',updated_at=?1,stop_cause=?2 WHERE input_id=?3 AND state='in_progress'",
            params![now_stamp(), cause.as_str(), input_id],
            CAS_FAILED,
        )
    })
}

/// Settles `input_id` of `run`, whose ownership the caller's transaction has validated.
pub(super) fn settle_input_in_transaction(
    conn: &Connection,
    run: &RunRecord,
    input_id: i64,
    expected: InputState,
    disposition: InputDisposition,
    evidence_ref: &str,
) -> Result<InputOperation, CafleetError> {
    nonblank(evidence_ref, "evidence_ref")?;
    require(
        expected != InputState::Settled,
        "state",
        "settled operations retain their disposition",
    )?;
    let input = owned_input(conn, run, input_id)?;
    require(
        input.state == expected,
        "member_input_operations.state",
        "input state changed",
    )?;
    settle(conn, input_id, expected, disposition, Some(evidence_ref))
}

fn settle(
    conn: &Connection,
    input_id: i64,
    expected: InputState,
    disposition: InputDisposition,
    evidence_ref: Option<&str>,
) -> Result<InputOperation, CafleetError> {
    update_input(
        conn,
        input_id,
        "UPDATE member_input_operations SET state='settled',disposition=?1,evidence_ref=?2,updated_at=?3
         WHERE input_id=?4 AND state=?5",
        params![disposition.as_str(),evidence_ref,now_stamp(),input_id,expected.as_str()],
        "settlement CAS failed",
    )
}

/// Stamps the message of a preview the run's sweep delivered, under the run's ownership; an
/// already stamped message stays unchanged.
pub(crate) fn stamp_preview(
    conn: &mut Connection,
    owner: &RunOwner,
    message_id: i64,
) -> Result<(), CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let message = crate::broker::messaging::get_message(conn, message_id)?;
        require(
            message.to_member_id == run.member_id,
            "message_id",
            "preview recipient changed",
        )?;
        conn.execute(
            "UPDATE messages SET notified_at=?1 WHERE message_id=?2 AND notified_at IS NULL",
            params![now_stamp(), message_id],
        )?;
        Ok(())
    })
}

pub(crate) const UNRESOLVED_INPUT: &str = "(state IN ('in_progress','accepted','uncertain') OR (state='ready' AND deferred_at IS NOT NULL))";

pub(super) fn require_capacity(conn: &Connection, run_id: i64) -> Result<(), CafleetError> {
    let unresolved: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND {UNRESOLVED_INPUT}"
        ),
        [run_id],
        |row| row.get(0),
    )?;
    require(
        unresolved < 4096,
        "member_input_operations",
        "unresolved input limit reached",
    )?;
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ZeroByteRetirement<'guard> {
    owner: RunOwner,
    input_id: i64,
    interaction_id: i64,
    input: InputOperation,
    rearm: Option<(NewPermissionResponse, AnswerCapability)>,
    guard: PhantomData<(&'guard mut (), Rc<()>)>,
}
impl<'guard> ZeroByteRetirement<'guard> {
    pub(crate) fn from_live_guard(
        guard: &'guard crate::runtime::InputGuard<'_>,
        owner: &RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<Self, CafleetError> {
        let (receipt, capability) = guard.retirement_binding(owner, input_id, interaction_id)?;
        Ok(Self {
            owner: owner.clone(),
            input_id,
            interaction_id,
            input: receipt.input.clone(),
            rearm: capability.map(|cap| {
                (
                    NewPermissionResponse {
                        operation_key: receipt.input.operation_key.clone(),
                        metadata: receipt.response.metadata.clone(),
                    },
                    cap.clone(),
                )
            }),
            guard: PhantomData,
        })
    }
}

fn validate_metadata(metadata: &PermissionResponseMetadata) -> Result<(), CafleetError> {
    require(
        metadata.event_id > 0
            && metadata.interaction_id > 0
            && metadata.expected_owner_epoch > 0
            && metadata.director_member_id > 0,
        "permission_response",
        "positive identities required",
    )?;
    require(
        metadata.request_token.len() == 64
            && metadata
                .request_token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "request_token",
        "lowercase 64-hex token required",
    )?;
    require(
        metadata.decision_source != DecisionSource::DirectorAuto
            || metadata.director_mode == DirectorMode::Auto,
        "decision_source",
        "director-auto requires auto mode",
    )?;
    for (field, value) in [
        ("mode_evidence_ref", &metadata.mode_evidence_ref),
        ("decision_evidence_ref", &metadata.decision_evidence_ref),
    ] {
        require(
            !value.trim().is_empty()
                && value.len() <= 2048
                && !value.chars().any(|ch| ch <= '\u{1f}' || ch == '\u{7f}'),
            field,
            "nonblank reference of 1-2048 UTF-8 bytes without controls required",
        )?;
    }
    if let AnswerKey::Choice(choice) = &metadata.key {
        require(
            valid_choice_id(choice),
            "choice_id",
            "lowercase hyphenated choice id of 1-64 bytes required",
        )?;
    }
    nonblank(&metadata.answer_rule_id, "answer_rule_id")
}

pub(crate) fn permission_response_digest(
    run_id: i64,
    metadata: &PermissionResponseMetadata,
) -> Result<String, CafleetError> {
    require(run_id > 0, "run_id", "positive run required")?;
    validate_metadata(metadata)?;
    let object = json!({"run_id":run_id,"owner_epoch":metadata.expected_owner_epoch,
        "interaction_id":metadata.interaction_id,"event_id":metadata.event_id,"director_member_id":metadata.director_member_id,
        "request_token":metadata.request_token,"decision":metadata.key.decision(),"choice_id":metadata.key.choice(),
        "director_mode":metadata.director_mode,
        "decision_source":metadata.decision_source,"mode_evidence_ref":metadata.mode_evidence_ref,
        "decision_evidence_ref":metadata.decision_evidence_ref,"answer_rule_id":metadata.answer_rule_id});
    Ok(crate::digest::sha256_hex(
        crate::digest::canonical_json(&object).as_bytes(),
    ))
}

fn stored_key(
    decision: Option<PermissionDecision>,
    choice_id: Option<String>,
) -> rusqlite::Result<AnswerKey> {
    match (decision, choice_id) {
        (Some(decision), None) => Ok(AnswerKey::Decision(decision)),
        (None, Some(choice)) => Ok(AnswerKey::Choice(choice)),
        _ => Err(rusqlite::Error::InvalidColumnName(
            "member_permission_responses requires exactly one of decision and choice_id".into(),
        )),
    }
}

fn map_response(row: &rusqlite::Row<'_>) -> rusqlite::Result<PermissionResponseRecord> {
    Ok(PermissionResponseRecord {
        input_id: row.get("input_id")?,
        metadata: PermissionResponseMetadata {
            event_id: row.get("event_id")?,
            interaction_id: row.get("interaction_id")?,
            expected_owner_epoch: row.get("expected_owner_epoch")?,
            request_token: row.get("request_token")?,
            key: stored_key(row.get("decision")?, row.get("choice_id")?)?,
            director_member_id: row.get("director_member_id")?,
            director_mode: row.get("director_mode")?,
            decision_source: row.get("decision_source")?,
            mode_evidence_ref: row.get("mode_evidence_ref")?,
            decision_evidence_ref: row.get("decision_evidence_ref")?,
            answer_rule_id: row.get("answer_rule_id")?,
        },
        decided_at: row.get("decided_at")?,
    })
}

fn zero_byte_state(input: &InputOperation) -> bool {
    input.state == InputState::Ready
        || (input.state == InputState::Settled
            && input.disposition == Some(InputDisposition::NotSubmitted))
}

fn read_claim(
    conn: &Connection,
    input: &InputOperation,
    metadata: &PermissionResponseMetadata,
) -> Result<Option<PermissionResponseClaim>, CafleetError> {
    let mut stmt = conn.prepare("SELECT interaction_id,input_id FROM member_permission_response_claims WHERE interaction_id=?1 OR input_id=?2")?;
    let claims = stmt
        .query_map(params![metadata.interaction_id, input.input_id], |row| {
            Ok(PermissionResponseClaim {
                interaction_id: row.get(0)?,
                input_id: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let matching = claims
        .iter()
        .find(|claim| {
            claim.input_id == input.input_id && claim.interaction_id == metadata.interaction_id
        })
        .cloned();
    require(
        claims.iter().all(|claim| {
            claim.input_id != input.input_id || claim.interaction_id == metadata.interaction_id
        }),
        "permission_claim",
        "claim belongs to another interaction",
    )?;
    require(
        matching.is_some() || zero_byte_state(input),
        "permission_claim",
        "possibly transmitted response requires its own claim",
    )?;
    Ok(matching)
}

pub(super) fn validate_input_response(
    conn: &Connection,
    run: &RunRecord,
    input: &InputOperation,
) -> Result<Option<PermissionResponseRecord>, CafleetError> {
    let response = conn
        .query_row(
            "SELECT * FROM member_permission_responses WHERE input_id=?1",
            [input.input_id],
            map_response,
        )
        .optional()?;
    if input.kind != InputKind::PermissionResponse {
        require(
            response.is_none(),
            "permission_response",
            "ordinary input has no response audit",
        )?;
        return Ok(None);
    }
    let response =
        response.ok_or_else(|| invalid("permission_response", "response audit required"))?;
    let metadata = &response.metadata;
    validate_metadata(metadata)?;
    require(
        metadata.expected_owner_epoch <= run.owner_epoch,
        "expected_owner_epoch",
        "audit epoch exceeds recorded ownership",
    )?;
    let timestamp = chrono::DateTime::parse_from_rfc3339(&response.decided_at)
        .map_err(|_| invalid("decided_at", "UTC RFC3339 timestamp required"))?;
    require(
        timestamp.offset().local_minus_utc() == 0,
        "decided_at",
        "UTC timestamp required",
    )?;
    require(
        permission_response_digest(input.run_id, metadata)? == input.payload_digest,
        "payload_digest",
        "response audit digest mismatch",
    )?;
    let event = get_event(conn, metadata.event_id)?;
    require(
        permission_event_matches(&event, input.run_id, metadata, false)
            && event.run.run.director_member_id == metadata.director_member_id
            && metadata.director_member_id == run.director_member_id,
        "permission_response",
        "recorded run/event/interaction/Director association mismatch",
    )?;
    read_claim(conn, input, metadata)?;
    Ok(Some(response))
}

fn answered_class(key: &AnswerKey) -> InteractionClass {
    match key {
        AnswerKey::Decision(_) => InteractionClass::PermissionRequest,
        AnswerKey::Choice(_) => InteractionClass::Question,
    }
}

fn permission_event_matches(
    event: &RuntimeEventView,
    run_id: i64,
    metadata: &PermissionResponseMetadata,
    require_open: bool,
) -> bool {
    event.event.run_id == run_id
        && event.event.interaction_id == Some(metadata.interaction_id)
        && event.event.kind == RuntimeEventKind::InteractionOpened
        && event.event.notification_state == NotificationState::Inbox
        && event.interaction.as_ref().is_some_and(|interaction| {
            interaction.classification == answered_class(&metadata.key)
                && (!require_open || interaction.closed_at.is_none())
        })
}

pub(crate) fn get_permission_response(
    conn: &Connection,
    input_id: i64,
) -> Result<PermissionReceipt, CafleetError> {
    let input = get_input(conn, input_id)?;
    let response = input
        .permission_response
        .clone()
        .ok_or_else(|| invalid("input_id", "permission response required"))?;
    let claim = read_claim(conn, &input, &response.metadata)?;
    Ok(PermissionReceipt {
        input,
        response,
        claim,
    })
}

pub(crate) fn lookup_permission_response(
    conn: &Connection,
    identity: &PermissionReceiptIdentity,
    request: &NewPermissionResponse,
) -> Result<Option<PermissionReceipt>, CafleetError> {
    nonblank(&request.operation_key, "operation_key")?;
    require(
        identity.run_id > 0 && database_identity(conn)? == identity.database_identity_json,
        "database_identity",
        "selected database identity mismatch",
    )?;
    let run = get_run(conn, identity.run_id)?;
    require(
        run.database_identity_json == identity.database_identity_json,
        "database_identity",
        "recorded database identity mismatch",
    )?;
    let digest = permission_response_digest(identity.run_id, &request.metadata)?;
    let input = find_input(conn, identity.run_id, &request.operation_key)?;
    input
        .map(|input| {
            require(
                input.kind == InputKind::PermissionResponse && input.payload_digest == digest,
                "operation_key",
                "conflicting retained operation key",
            )?;
            get_permission_response(conn, input.input_id)
        })
        .transpose()
}

pub(crate) fn reserve_permission_response(
    conn: &mut Connection,
    owner: &RunOwner,
    request: &NewPermissionResponse,
    capability: Option<&AnswerCapability>,
    deadline: Instant,
) -> Result<PermissionReservation, CafleetError> {
    with_database_deadline(conn, deadline, |conn| {
        let identity = PermissionReceiptIdentity {
            database_identity_json: owner.database_identity_json.clone(),
            run_id: owner.run_id,
        };
        if let Some(receipt) = lookup_permission_response(conn, &identity, request)? {
            return Ok(PermissionReservation::Retained(receipt));
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = lookup_permission_response(&tx, &identity, request)? {
            tx.commit()?;
            return Ok(PermissionReservation::Retained(receipt));
        }
        let run = validate_owner(&tx, owner)?;
        let metadata = &request.metadata;
        let event = get_event(&tx, metadata.event_id)?;
        validate_current_capability(&tx, &run, metadata, &event, capability)?;
        require_capacity(&tx, run.run_id)?;
        let digest = permission_response_digest(run.run_id, metadata)?;
        let now = now_stamp();
        tx.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,?2,'permission_response',?3,'in_progress',?4,?4)",params![run.run_id,request.operation_key,digest,now])?;
        let input_id = tx.last_insert_rowid();
        tx.execute("INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,choice_id,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![input_id,metadata.event_id,metadata.interaction_id,metadata.expected_owner_epoch,metadata.request_token,metadata.key.decision().map(PermissionDecision::as_str),metadata.key.choice(),metadata.director_member_id,metadata.director_mode.as_str(),metadata.decision_source.as_str(),metadata.mode_evidence_ref,metadata.decision_evidence_ref,metadata.answer_rule_id,now])?;
        tx.execute(
            "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES(?1,?2)",
            params![metadata.interaction_id, input_id],
        )?;
        let receipt = get_permission_response(&tx, input_id)?;
        require(
            Instant::now() < deadline,
            "deadline",
            "response reservation deadline expired",
        )?;
        tx.commit()?;
        Ok(PermissionReservation::Reserved(receipt))
    })
}

fn validate_proof(
    owner: &RunOwner,
    receipt: &PermissionReceipt,
    proof: &ZeroByteRetirement<'_>,
) -> Result<(), CafleetError> {
    require(
        proof.owner == *owner
            && proof.input_id == receipt.input.input_id
            && proof.interaction_id == receipt.response.metadata.interaction_id
            && receipt.input.run_id == owner.run_id,
        "retirement",
        "retirement proof does not match current owner/input/interaction",
    )?;
    require(
        proof.input.same_identity(&receipt.input),
        "retirement",
        "retirement immutable receipt changed",
    )
}

fn validate_current_capability(
    conn: &Connection,
    run: &RunRecord,
    metadata: &PermissionResponseMetadata,
    event: &RuntimeEventView,
    capability: Option<&AnswerCapability>,
) -> Result<(), CafleetError> {
    require(
        run.owner_kind == OwnerKind::Runner
            && run.status == RunStatus::Running
            && historical_ownership(conn, run)?.is_none()
            && metadata.expected_owner_epoch == run.owner_epoch
            && metadata.director_member_id == run.director_member_id
            && permission_event_matches(event, run.run_id, metadata, true),
        "permission_response",
        "current open runner permission association required",
    )?;
    let cap = capability
        .ok_or_else(|| invalid("answer_capability", "verified answer capability required"))?;
    require(
        cap.backend().name() == run.backend
            && Some(cap.native_version()) == run.native_version.as_deref()
            && cap.answer_rule_id() == metadata.answer_rule_id
            && cap.request_rule_id()
                == event.event.payload_json["evidence"]["rule_id"]
                    .as_str()
                    .ok_or_else(|| invalid("rule_id", "event request rule required"))?
            && cap.key() == &metadata.key,
        "answer_capability",
        "answer capability does not match this request",
    )?;
    Ok(())
}

pub(crate) fn rearm_permission_response(
    conn: &mut Connection,
    owner: &RunOwner,
    request: &NewPermissionResponse,
    proof: &ZeroByteRetirement<'_>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    let (bound, capability) = proof
        .rearm
        .as_ref()
        .ok_or_else(|| invalid("retirement", "current-plan rearm authority required"))?;
    require(
        bound == request,
        "permission_response",
        "rearm request changed",
    )?;
    require(
        conn.is_autocommit(),
        "transaction",
        "rearm requires its own immediate transaction",
    )?;
    with_database_deadline(conn, deadline, |conn| {
        conn.busy_handler(Some(|_| false))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = validate_owner(&tx, owner)?;
        let receipt = get_permission_response(&tx, proof.input_id)?;
        validate_proof(owner, &receipt, proof)?;
        require(
            receipt.input.state == InputState::Ready
                && receipt.input.operation_key == request.operation_key
                && receipt.response.metadata == request.metadata,
            "permission_response",
            "exact ready response required",
        )?;
        let event = get_event(&tx, request.metadata.event_id)?;
        validate_current_capability(&tx, &run, &request.metadata, &event, Some(capability))?;
        if receipt.claim.is_none() {
            tx.execute("INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES(?1,?2)", params![request.metadata.interaction_id, receipt.input.input_id])?;
        }
        let changed = tx.execute("UPDATE member_input_operations SET state='in_progress',updated_at=?1 WHERE input_id=?2 AND state='ready'", params![now_stamp(), receipt.input.input_id])?;
        require(changed == 1, "state", "permission rearm lost CAS")?;
        let rearmed = get_permission_response(&tx, receipt.input.input_id)?;
        require(
            Instant::now() < deadline,
            "deadline",
            "permission rearm deadline expired",
        )?;
        tx.commit()?;
        Ok(rearmed)
    })
}

pub(super) fn release_permission_claim_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    input_id: i64,
    proof: &ZeroByteRetirement<'_>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    let receipt = get_permission_response(conn, input_id)?;
    validate_proof(owner, &receipt, proof)?;
    require(
        zero_byte_state(&receipt.input),
        "retirement",
        "serialized zero-byte state required",
    )?;
    if receipt.claim.is_some() {
        let changed = conn.execute(
            "DELETE FROM member_permission_response_claims WHERE interaction_id=?1 AND input_id=?2",
            params![proof.interaction_id, input_id],
        )?;
        require(changed == 1, "permission_claim", "claim release lost CAS")?;
    }
    require(
        Instant::now() < deadline,
        "deadline",
        "claim release deadline expired",
    )?;
    get_permission_response(conn, input_id)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the settlement entry passes its validated owner, run and retirement proof"
)]
fn settle_permission_response_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    run: &RunRecord,
    expected_input: &InputOperation,
    disposition: InputDisposition,
    evidence_ref: &str,
    proof: Option<&ZeroByteRetirement<'_>>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    let receipt = get_permission_response(conn, expected_input.input_id)?;
    require(
        receipt.input.same_identity(expected_input) && receipt.input.state == expected_input.state,
        "permission_response",
        "permission settlement snapshot changed",
    )?;
    if let Some(proof) = proof {
        validate_proof(owner, &receipt, proof)?;
    }
    settle_input_in_transaction(
        conn,
        run,
        expected_input.input_id,
        expected_input.state,
        disposition,
        evidence_ref,
    )?;
    if disposition == InputDisposition::NotSubmitted
        && let Some(proof) = proof
    {
        return release_permission_claim_in_transaction(
            conn,
            owner,
            expected_input.input_id,
            proof,
            deadline,
        );
    }
    require(
        Instant::now() < deadline,
        "deadline",
        "permission settlement deadline expired",
    )?;
    get_permission_response(conn, expected_input.input_id)
}

/// Settles `expected_input` at its current state. A `PermissionResponse` goes through the
/// permission settlement, which consumes `proof`; every other kind ignores `proof` and `deadline`.
pub(crate) fn settle_any_input_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    expected_input: &InputOperation,
    disposition: InputDisposition,
    evidence_ref: &str,
    proof: Option<&ZeroByteRetirement<'_>>,
    deadline: Instant,
) -> Result<InputOperation, CafleetError> {
    require(
        !conn.is_autocommit(),
        "transaction",
        "input settlement requires an active ownership transaction",
    )?;
    let run = validate_owner(conn, owner)?;
    if expected_input.kind == InputKind::PermissionResponse {
        settle_permission_response_in_transaction(
            conn,
            owner,
            &run,
            expected_input,
            disposition,
            evidence_ref,
            proof,
            deadline,
        )
        .map(|receipt| receipt.input)
    } else {
        settle_input_in_transaction(
            conn,
            &run,
            expected_input.input_id,
            expected_input.state,
            disposition,
            evidence_ref,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime as storage;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::*;

    #[test]
    fn input_payload_digest_hashes_the_key_sorted_payload() {
        use crate::runtime::SubmitInput;
        let literal = "say \"hi\" λ";
        let prompt = SubmitInput {
            operation_key: "prompt:7".into(),
            input_kind: InputKind::Prompt,
            literal: literal.into(),
        };
        let exec = SubmitInput {
            operation_key: "exec:7".into(),
            input_kind: InputKind::Exec,
            literal: literal.into(),
        };
        let quoted = serde_json::to_string(literal).unwrap();
        let expected = |kind: &str| {
            crate::digest::sha256_hex(
                format!("{{\"kind\":\"{kind}\",\"literal\":{quoted},\"run_id\":7}}").as_bytes(),
            )
        };
        assert_eq!(input_payload_digest(7, &prompt), expected("prompt"));
        assert_eq!(input_payload_digest(7, &exec), expected("exec"));
    }

    #[test]
    fn receipt_key_digest_and_settled_disposition_survive_reconnection() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let input = fixture.input(&owner, "prompt:one", InputKind::Prompt, None);
        assert_eq!(input.state, InputState::InProgress);
        let request = NewInput {
            operation_key: "prompt:one".into(),
            kind: InputKind::Prompt,
            payload_digest: "d".repeat(64),
            message_id: None,
        };
        let mut other = fixture.reopen();
        assert_eq!(
            crate::broker::runtime::test_support::reserve_input(&mut other, &owner, &request)
                .unwrap()
                .input_id,
            input.input_id
        );
        let settled = crate::broker::runtime::test_support::settle_input(
            &mut other,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputDisposition::NotSubmitted,
            "audit:zero-byte-confirmation",
        )
        .unwrap();
        assert_eq!(settled.state, InputState::Settled);
        let retry = crate::broker::runtime::test_support::reserve_input(
            &mut fixture.conn,
            &owner,
            &request,
        )
        .unwrap();
        assert_eq!(retry.input_id, input.input_id);
        assert_eq!(retry.disposition, Some(InputDisposition::NotSubmitted));
        assert_eq!(rows(&fixture.conn, "member_input_operations").len(), 1);
    }

    #[test]
    fn operation_key_rejects_a_changed_digest_or_kind() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.input(&owner, "same-key", InputKind::Prompt, None);
        for changed in ["digest", "kind"] {
            let mut request = NewInput {
                operation_key: "same-key".into(),
                kind: InputKind::Prompt,
                payload_digest: "d".repeat(64),
                message_id: None,
            };
            match changed {
                "digest" => request.payload_digest = "e".repeat(64),
                "kind" => request.kind = InputKind::Exec,
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                crate::broker::runtime::test_support::reserve_input(
                    &mut fixture.conn,
                    &owner,
                    &request
                )
                .is_err(),
                "{changed}"
            );
            assert_eq!(durable_state(&fixture.conn), before);
        }
    }

    #[test]
    fn settlement_cas_rejects_stale_state_and_preserves_recorded_evidence() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let input = fixture.input(&owner, "exec:lost-stdout", InputKind::Exec, None);
        storage::record_input_uncertain(
            &mut fixture.conn,
            &owner,
            input.input_id,
            StopCause::WriteFailed,
        )
        .unwrap();
        assert!(
            crate::broker::runtime::test_support::settle_input(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputDisposition::Submitted,
                "audit:stale"
            )
            .is_err()
        );
        assert!(
            crate::broker::runtime::test_support::settle_input(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::Uncertain,
                InputDisposition::Submitted,
                " "
            )
            .is_err()
        );
        let settled = crate::broker::runtime::test_support::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::Uncertain,
            InputDisposition::AbandonedUncertain,
            "audit:inspected-output",
        )
        .unwrap();
        assert_eq!(
            settled.disposition,
            Some(InputDisposition::AbandonedUncertain)
        );
        assert_eq!(
            settled.evidence_ref.as_deref(),
            Some("audit:inspected-output")
        );
        let read = storage::get_input(&fixture.reopen(), input.input_id).unwrap();
        assert_eq!(read.state, InputState::Settled);
        assert!(
            crate::broker::runtime::test_support::settle_input(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::Uncertain,
                InputDisposition::NotSubmitted,
                "audit:changed"
            )
            .is_err()
        );
        assert!(rows(&fixture.conn, "member_runtime_events").is_empty());
    }

    #[test]
    fn reconciling_an_uncertain_receipt_with_a_recorded_method_not_submitted_reads_back() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let input = fixture.input(&owner, "prompt:uncertain", InputKind::Prompt, None);
        storage::requeue_zero_byte_immediate(
            &mut fixture.conn,
            &owner,
            input.input_id,
            "interaction_open",
        )
        .unwrap();
        storage::begin_deferred_attempt(
            &mut fixture.conn,
            &owner,
            input.input_id,
            DeliveryMethod::Fallback,
            Some("submission_plan_unavailable"),
        )
        .unwrap();
        storage::record_input_uncertain(
            &mut fixture.conn,
            &owner,
            input.input_id,
            StopCause::ShortWrite,
        )
        .unwrap();
        let settled = crate::broker::runtime::test_support::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::Uncertain,
            InputDisposition::NotSubmitted,
            "audit:no-bytes-observed",
        )
        .unwrap();
        let read = storage::get_input(&fixture.reopen(), input.input_id).unwrap();
        assert_eq!(read, settled);
        assert_eq!(read.state, InputState::Settled);
        assert_eq!(read.disposition, Some(InputDisposition::NotSubmitted));
        assert_eq!(read.delivery_method, Some(DeliveryMethod::Fallback));
        assert_eq!(
            read.delivery_reason.as_deref(),
            Some("submission_plan_unavailable")
        );
        let delivery = read
            .delivery()
            .expect("a settled receipt projects its delivery");
        assert_eq!(delivery.status, DeliveryStatus::NotSubmitted);
    }

    #[test]
    fn settled_receipt_history_can_exceed_the_unresolved_cache_bound() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.conn.execute("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref) SELECT ?1,'settled:'||x,'prompt',?2,'settled',?3,?3,'not_submitted','audit:zero-byte' FROM n",params![owner.run_id,"d".repeat(64),NOW]).unwrap();
        let input = fixture.input(&owner, "settled:4096", InputKind::Prompt, None);
        crate::broker::runtime::test_support::settle_input(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputDisposition::NotSubmitted,
            "audit:zero-byte",
        )
        .unwrap();
        assert_eq!(rows(&fixture.conn, "member_input_operations").len(), 4097);
        assert!(
            storage::find_input(&fixture.reopen(), owner.run_id, "settled:0")
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn failed_input_reservation_and_corrupt_receipt_reads_surface_the_original_state() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.conn.execute_batch("CREATE TRIGGER fail_input BEFORE INSERT ON member_input_operations BEGIN SELECT RAISE(ABORT,'injected input failure'); END").unwrap();
        assert!(
            crate::broker::runtime::test_support::reserve_input(
                &mut fixture.conn,
                &owner,
                &NewInput {
                    operation_key: "reserved".into(),
                    kind: InputKind::Exec,
                    payload_digest: "d".repeat(64),
                    message_id: None
                }
            )
            .is_err()
        );
        assert!(rows(&fixture.reopen(), "member_input_operations").is_empty());
        fixture
            .conn
            .execute_batch("DROP TRIGGER fail_input")
            .unwrap();
        let input = fixture.input(&owner, "reserved", InputKind::Exec, None);
        fixture
            .conn
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        fixture
            .conn
            .execute(
                "UPDATE member_input_operations SET state='corrupt' WHERE input_id=?1",
                [input.input_id],
            )
            .unwrap();
        let error = storage::get_input(&fixture.conn, input.input_id).unwrap_err();
        assert!(error.message().contains("state"));
        assert!(error.message().contains("corrupt"));
    }

    mod stop_cause {
        use crate::broker::runtime::inputs::tests::*;

        fn deferred_attempt(fixture: &mut Fixture, owner: &RunOwner, key: &str) -> InputOperation {
            let input = fixture.input(owner, key, InputKind::Prompt, None);
            storage::requeue_zero_byte_immediate(
                &mut fixture.conn,
                owner,
                input.input_id,
                "interaction_open",
            )
            .unwrap();
            storage::begin_deferred_attempt(
                &mut fixture.conn,
                owner,
                input.input_id,
                DeliveryMethod::Profile,
                None,
            )
            .unwrap()
        }

        #[test]
        fn record_input_uncertain_persists_its_cause_across_reconnection() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let input = fixture.input(&owner, "prompt:caused", InputKind::Prompt, None);
            assert_eq!(input.stop_cause, None);
            let uncertain = storage::record_input_uncertain(
                &mut fixture.conn,
                &owner,
                input.input_id,
                StopCause::ManualInput,
            )
            .unwrap();
            assert_eq!(uncertain.state, InputState::Uncertain);
            assert_eq!(uncertain.stop_cause, Some(StopCause::ManualInput));
            assert_eq!(
                uncertain.delivery().unwrap().status,
                DeliveryStatus::Uncertain
            );
            let read = storage::get_input(&fixture.reopen(), input.input_id).unwrap();
            assert_eq!(read, uncertain);
            let found = storage::find_input(&fixture.reopen(), owner.run_id, "prompt:caused")
                .unwrap()
                .unwrap();
            assert_eq!(found.stop_cause, Some(StopCause::ManualInput));
        }

        #[test]
        fn record_input_uncertain_stores_every_cause_value() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            for (index, cause) in [
                StopCause::ManualInput,
                StopCause::InteractionCandidate,
                StopCause::InteractionOpen,
                StopCause::DeadlineExpired,
                StopCause::ShortWrite,
                StopCause::WriteFailed,
                StopCause::RunnerStopped,
            ]
            .into_iter()
            .enumerate()
            {
                let input = fixture.input(
                    &owner,
                    &format!("exec:cause:{index}"),
                    InputKind::Exec,
                    None,
                );
                let uncertain = storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    cause,
                )
                .unwrap_or_else(|error| panic!("{cause:?}: {error}"));
                assert_eq!(uncertain.stop_cause, Some(cause));
                assert_eq!(
                    storage::get_input(&fixture.reopen(), input.input_id)
                        .unwrap()
                        .stop_cause,
                    Some(cause)
                );
            }
        }

        #[test]
        fn record_input_state_rejects_in_progress_to_uncertain() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let input = fixture.input(&owner, "prompt:no-cause", InputKind::Prompt, None);
            let before = durable_state(&fixture.conn);
            let error = storage::record_input_state(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputState::Uncertain,
            )
            .unwrap_err();
            assert!(
                error.message().contains("invalid input transition"),
                "{}",
                error.message()
            );
            assert_eq!(durable_state(&fixture.conn), before);
            assert_eq!(
                storage::get_input(&fixture.conn, input.input_id)
                    .unwrap()
                    .state,
                InputState::InProgress
            );
        }

        #[test]
        fn record_input_uncertain_requires_an_in_progress_receipt() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let queued = fixture.input(&owner, "prompt:queued", InputKind::Prompt, None);
            storage::requeue_zero_byte_immediate(
                &mut fixture.conn,
                &owner,
                queued.input_id,
                "interaction_open",
            )
            .unwrap();
            let accepted = fixture.input(&owner, "prompt:accepted", InputKind::Prompt, None);
            storage::record_input_state(
                &mut fixture.conn,
                &owner,
                accepted.input_id,
                InputState::InProgress,
                InputState::Accepted,
            )
            .unwrap();
            let uncertain = fixture.input(&owner, "prompt:uncertain", InputKind::Prompt, None);
            storage::record_input_uncertain(
                &mut fixture.conn,
                &owner,
                uncertain.input_id,
                StopCause::WriteFailed,
            )
            .unwrap();
            for (input, state) in [
                (queued.input_id, "ready"),
                (accepted.input_id, "accepted"),
                (uncertain.input_id, "uncertain"),
            ] {
                let before = durable_state(&fixture.conn);
                let error = storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input,
                    StopCause::ManualInput,
                )
                .unwrap_err();
                assert!(
                    error.message().contains("input state changed"),
                    "{state}: {}",
                    error.message()
                );
                assert_eq!(durable_state(&fixture.conn), before, "{state}");
            }
            assert_eq!(
                storage::get_input(&fixture.conn, uncertain.input_id)
                    .unwrap()
                    .stop_cause,
                Some(StopCause::WriteFailed),
                "a repeated transition keeps the first recorded cause"
            );
        }

        #[test]
        fn record_input_uncertain_checks_run_ownership() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let input = fixture.input(&owner, "prompt:owned", InputKind::Prompt, None);
            fixture.activate_next_epoch();
            let before = durable_state(&fixture.conn);
            assert!(
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    StopCause::ManualInput,
                )
                .is_err(),
                "a superseded owner records no cause"
            );
            assert_eq!(durable_state(&fixture.conn), before);
        }

        #[test]
        fn immediate_stopped_results_remain_unpublished_until_their_first_durable_event() {
            for kind in [InputKind::Prompt, InputKind::Exec] {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                fixture.reserve(&owner);
                let uncertain = fixture.input(&owner, "ordinary:immediate-uncertain", kind, None);
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    uncertain.input_id,
                    StopCause::WriteFailed,
                )
                .unwrap();
                let submitted = fixture.input(&owner, "ordinary:immediate-submitted", kind, None);
                storage::record_input_state(
                    &mut fixture.conn,
                    &owner,
                    submitted.input_id,
                    InputState::InProgress,
                    InputState::Accepted,
                )
                .unwrap();
                let mut expected =
                    vec![(uncertain.input_id, "unknown", Some(StopCause::WriteFailed))];
                assert_eq!(
                    storage::unpublished_delivery_results(&fixture.conn, owner.run_id).unwrap(),
                    expected
                );
                for (index, (input_id, stage, cause)) in expected.clone().into_iter().enumerate() {
                    let transition = ObservedTransition {
                        run_id: owner.run_id,
                        event_seq: i64::try_from(index).unwrap() + 1,
                        episode_seq: None,
                        kind: RuntimeEventKind::InputDeliveryResult,
                        classification: None,
                        observed_at: NOW.into(),
                        evidence_json: storage::test_support::metadata(),
                        close_reason: None,
                        cause_json: Some(storage::input_delivery_result_cause(
                            input_id, stage, cause,
                        )),
                    };
                    let (event, inserted) = storage::persist_transition(
                        &mut fixture.conn,
                        &owner,
                        &transition,
                        PersistenceMode::Live,
                    )
                    .unwrap();
                    assert!(inserted);
                    assert_eq!(
                        storage::get_event(&fixture.conn, event.event_id)
                            .unwrap()
                            .event
                            .payload_json["input_id"],
                        input_id
                    );
                    expected.remove(0);
                    assert_eq!(
                        storage::unpublished_delivery_results(&fixture.conn, owner.run_id).unwrap(),
                        expected
                    );
                }
            }
        }

        #[test]
        fn unpublished_delivery_results_return_the_receipt_cause() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let uncertain = deferred_attempt(&mut fixture, &owner, "prompt:deferred-uncertain");
            storage::record_input_uncertain(
                &mut fixture.conn,
                &owner,
                uncertain.input_id,
                StopCause::WriteFailed,
            )
            .unwrap();
            let submitted = deferred_attempt(&mut fixture, &owner, "prompt:deferred-submitted");
            storage::record_input_state(
                &mut fixture.conn,
                &owner,
                submitted.input_id,
                InputState::InProgress,
                InputState::Accepted,
            )
            .unwrap();
            let abandoned = deferred_attempt(&mut fixture, &owner, "prompt:deferred-abandoned");
            storage::record_input_uncertain(
                &mut fixture.conn,
                &owner,
                abandoned.input_id,
                StopCause::RunnerStopped,
            )
            .unwrap();
            crate::broker::runtime::test_support::settle_input(
                &mut fixture.conn,
                &owner,
                abandoned.input_id,
                InputState::Uncertain,
                InputDisposition::AbandonedUncertain,
                "audit:abandoned",
            )
            .unwrap();
            let mut expected = vec![
                (uncertain.input_id, "unknown", Some(StopCause::WriteFailed)),
                (submitted.input_id, "submitted", None),
                (
                    abandoned.input_id,
                    "unknown",
                    Some(StopCause::RunnerStopped),
                ),
            ];
            for (key, disposition, stage) in [
                (
                    "prompt:reconciled-submitted",
                    InputDisposition::Submitted,
                    "submitted",
                ),
                (
                    "prompt:reconciled-not-submitted",
                    InputDisposition::NotSubmitted,
                    "zero_bytes",
                ),
            ] {
                let input = deferred_attempt(&mut fixture, &owner, key);
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    StopCause::WriteFailed,
                )
                .unwrap();
                let settled = crate::broker::runtime::test_support::settle_input(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    InputState::Uncertain,
                    disposition,
                    "audit:inspected",
                )
                .unwrap();
                assert_eq!(settled.stop_cause, Some(StopCause::WriteFailed));
                expected.push((input.input_id, stage, None));
            }
            let lost = deferred_attempt(&mut fixture, &owner, "prompt:deferred-lost");
            expected.push((lost.input_id, "unknown", None));
            assert_eq!(
                storage::unpublished_delivery_results(&fixture.conn, owner.run_id).unwrap(),
                expected,
                "only an uncertain result carries the receipt's cause"
            );
        }
    }

    use rusqlite::params;

    #[test]
    fn input_presentation_projects_delivery_from_the_stored_receipt_state() {
        const NOW: &str = "2026-10-30T10:00:00.000000+00:00";
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        for (index, (assignment, delivery, deferred)) in [
            (
                format!("state='ready',delivery_method=NULL,deferred_at='{NOW}',defer_reason='interaction_open'"),
                serde_json::json!({"status":"queued","method":null,"reason":"interaction_open"}),
                true,
            ),
            (
                "state='ready',delivery_method=NULL".to_owned(),
                serde_json::json!({"status":"held","method":null,"reason":"zero_bytes_ready"}),
                false,
            ),
            (
                "delivery_method='fallback',delivery_reason='submission_plan_unavailable'".to_owned(),
                serde_json::json!({"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}),
                false,
            ),
            (
                "state='accepted',delivery_method='profile'".to_owned(),
                serde_json::json!({"status":"submitted","method":"profile","reason":null}),
                false,
            ),
            (
                "state='settled',disposition='submitted',delivery_method='fallback',delivery_reason='submission_plan_unavailable'".to_owned(),
                serde_json::json!({"status":"submitted","method":"fallback","reason":"submission_plan_unavailable"}),
                false,
            ),
            (
                "state='settled',disposition='not_submitted',delivery_method=NULL,delivery_reason='runner_stopped'".to_owned(),
                serde_json::json!({"status":"not_submitted","method":null,"reason":"runner_stopped"}),
                false,
            ),
            (
                "state='settled',disposition='not_submitted',delivery_method='profile'".to_owned(),
                serde_json::json!({"status":"not_submitted","method":"profile","reason":null}),
                false,
            ),
            (
                "state='settled',disposition='abandoned_uncertain',delivery_method='profile'".to_owned(),
                serde_json::json!({"status":"uncertain","method":"profile","reason":null}),
                false,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let input = fixture.input(&owner, &format!("prompt:{index}"), InputKind::Prompt, None);
            fixture
                .conn
                .execute(
                    &format!("UPDATE member_input_operations SET {assignment} WHERE input_id=?1"),
                    [input.input_id],
                )
                .unwrap_or_else(|error| panic!("{assignment}: {error}"));
            let stored = storage::get_input(&fixture.conn, input.input_id)
                .unwrap_or_else(|error| panic!("{assignment}: {}", error.message()));
            let projected = stored.delivery().expect("an unsettled or delivered receipt projects");
            assert_eq!(projected.to_json(), delivery, "{assignment}");
            assert_eq!(stored.deferred_at.is_none(), !deferred, "{assignment}");
            assert_eq!(stored.defer_reason.is_none(), !deferred, "{assignment}");
        }
        let queued_with_method = fixture.input(&owner, "exec:invalid", InputKind::Exec, None);
        fixture
            .conn
            .execute(
                &format!(
                    "UPDATE member_input_operations SET state='ready',deferred_at='{NOW}',defer_reason='interaction_open',delivery_method='profile' WHERE input_id=?1"
                ),
                [queued_with_method.input_id],
            )
            .unwrap();
        assert!(
            storage::get_input(&fixture.conn, queued_with_method.input_id).is_err(),
            "a queued receipt with a selected method fails validation"
        );
    }

    #[test]
    fn zero_byte_retirement_proof_is_neither_send_clone_nor_serialize() {
        assert_not_impl!(storage::ZeroByteRetirement<'static>: Send, Clone, serde::Serialize);
    }
}
