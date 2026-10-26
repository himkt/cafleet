use super::*;
use crate::coding_agent::AnswerCapability;
use crate::runtime::runner::with_database_deadline;
use sha2::{Digest, Sha256};
use std::marker::PhantomData;
use std::time::Instant;

#[derive(Debug)]
pub struct ZeroByteRetirement<'guard> {
    owner: RunOwner,
    input_id: i64,
    interaction_id: i64,
    guard: PhantomData<&'guard mut ()>,
}
impl<'guard> ZeroByteRetirement<'guard> {
    #[cfg(test)]
    pub(crate) fn fixture(
        owner: &RunOwner,
        input_id: i64,
        interaction_id: i64,
        _: &'guard mut (),
    ) -> Result<Self, CafleetError> {
        require(
            input_id > 0 && interaction_id > 0,
            "retirement",
            "positive identities required",
        )?;
        Ok(Self {
            owner: owner.clone(),
            input_id,
            interaction_id,
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
    nonblank(&metadata.answer_profile_version, "answer_profile_version")?;
    nonblank(&metadata.answer_rule_id, "answer_rule_id")
}

pub fn permission_response_digest(
    run_id: i64,
    metadata: &PermissionResponseMetadata,
) -> Result<String, CafleetError> {
    require(run_id > 0, "run_id", "positive run required")?;
    validate_metadata(metadata)?;
    let object = json!({"run_id":run_id,"owner_epoch":metadata.expected_owner_epoch,
        "interaction_id":metadata.interaction_id,"event_id":metadata.event_id,"director_member_id":metadata.director_member_id,
        "request_token":metadata.request_token,"decision":metadata.decision,"director_mode":metadata.director_mode,
        "decision_source":metadata.decision_source,"mode_evidence_ref":metadata.mode_evidence_ref,
        "decision_evidence_ref":metadata.decision_evidence_ref,"answer_profile_version":metadata.answer_profile_version,"answer_rule_id":metadata.answer_rule_id});
    let sorted = object
        .as_object()
        .expect("digest object")
        .iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&sorted)
        .map_err(|error| invalid("permission_response", error.to_string()))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn map_response(row: &rusqlite::Row<'_>) -> rusqlite::Result<PermissionResponseRecord> {
    Ok(PermissionResponseRecord {
        input_id: row.get("input_id")?,
        metadata: PermissionResponseMetadata {
            event_id: row.get("event_id")?,
            interaction_id: row.get("interaction_id")?,
            expected_owner_epoch: row.get("expected_owner_epoch")?,
            request_token: row.get("request_token")?,
            decision: row.get("decision")?,
            director_member_id: row.get("director_member_id")?,
            director_mode: row.get("director_mode")?,
            decision_source: row.get("decision_source")?,
            mode_evidence_ref: row.get("mode_evidence_ref")?,
            decision_evidence_ref: row.get("decision_evidence_ref")?,
            answer_profile_version: row.get("answer_profile_version")?,
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
    let mut stmt = conn.prepare("SELECT interaction_id,input_id FROM member_permission_response_claims WHERE interaction_id=?1 OR input_id=?2").map_err(db_err)?;
    let claims = stmt
        .query_map(params![metadata.interaction_id, input.input_id], |row| {
            Ok(PermissionResponseClaim {
                interaction_id: row.get(0)?,
                input_id: row.get(1)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
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
        .optional()
        .map_err(db_err)?;
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
        event.event.run_id == input.run_id
            && event.event.interaction_id == Some(metadata.interaction_id)
            && event.event.kind == RuntimeEventKind::InteractionOpened
            && event.event.notification_state == NotificationState::Inbox
            && event.run.run.director_member_id == metadata.director_member_id
            && metadata.director_member_id == run.director_member_id
            && event.interaction.as_ref().is_some_and(|interaction| {
                interaction.classification == InteractionClass::PermissionRequest
            }),
        "permission_response",
        "recorded run/event/interaction/Director association mismatch",
    )?;
    read_claim(conn, input, metadata)?;
    Ok(Some(response))
}

pub fn get_permission_response(
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

pub fn lookup_permission_response(
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

pub fn reserve_permission_response(
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
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if let Some(receipt) = lookup_permission_response(&tx, &identity, request)? {
            tx.commit().map_err(db_err)?;
            return Ok(PermissionReservation::Retained(receipt));
        }
        let run = validate_owner(&tx, owner)?;
        require(
            run.owner_kind == OwnerKind::Runner
                && run.status == RunStatus::Running
                && historical_ownership(&tx, &run)?.is_none(),
            "member_runs",
            "new response requires active running runner ownership",
        )?;
        let metadata = &request.metadata;
        require(
            metadata.expected_owner_epoch == run.owner_epoch
                && metadata.director_member_id == run.director_member_id,
            "permission_response",
            "current epoch and pinned Director required",
        )?;
        let event = get_event(&tx, metadata.event_id)?;
        require(
            event.event.run_id == run.run_id
                && event.event.interaction_id == Some(metadata.interaction_id)
                && event.event.kind == RuntimeEventKind::InteractionOpened
                && event.event.notification_state == NotificationState::Inbox
                && event.interaction.as_ref().is_some_and(|interaction| {
                    interaction.classification == InteractionClass::PermissionRequest
                        && interaction.closed_at.is_none()
                }),
            "permission_response",
            "current open inbox permission event required",
        )?;
        let cap = capability
            .ok_or_else(|| invalid("answer_capability", "verified answer capability required"))?;
        require(
            cap.backend().name() == run.backend
                && Some(cap.native_version()) == run.native_version.as_deref()
                && cap.profile_version() == metadata.answer_profile_version
                && cap.answer_rule_id() == metadata.answer_rule_id
                && cap.request_rule_id()
                    == event.event.payload_json["evidence"]["rule_id"]
                        .as_str()
                        .ok_or_else(|| invalid("rule_id", "event request rule required"))?
                && cap.decision() == metadata.decision
                && !cap.fixture_ids().is_empty(),
            "answer_capability",
            "answer capability does not match this request",
        )?;
        inputs::require_capacity(&tx, run.run_id)?;
        let digest = permission_response_digest(run.run_id, metadata)?;
        let now = format_utc(now_utc());
        tx.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,?2,'permission_response',?3,'in_progress',?4,?4)",params![run.run_id,request.operation_key,digest,now]).map_err(db_err)?;
        let input_id = tx.last_insert_rowid();
        tx.execute("INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![input_id,metadata.event_id,metadata.interaction_id,metadata.expected_owner_epoch,metadata.request_token,metadata.decision.as_str(),metadata.director_member_id,metadata.director_mode.as_str(),metadata.decision_source.as_str(),metadata.mode_evidence_ref,metadata.decision_evidence_ref,metadata.answer_profile_version,metadata.answer_rule_id,now]).map_err(db_err)?;
        tx.execute(
            "INSERT INTO member_permission_response_claims(interaction_id,input_id) VALUES(?1,?2)",
            params![metadata.interaction_id, input_id],
        )
        .map_err(db_err)?;
        let receipt = get_permission_response(&tx, input_id)?;
        require(
            Instant::now() < deadline,
            "deadline",
            "response reservation deadline expired",
        )?;
        tx.commit().map_err(db_err)?;
        Ok(PermissionReservation::Reserved(receipt))
    })
}

pub fn release_permission_claim(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    proof: &ZeroByteRetirement<'_>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    with_database_deadline(conn, deadline, |conn| {
        owned_transaction(conn, owner, |conn, run| {
            let receipt = get_permission_response(conn, input_id)?;
            require(
                proof.owner == *owner
                    && proof.input_id == input_id
                    && proof.interaction_id == receipt.response.metadata.interaction_id
                    && receipt.input.run_id == run.run_id,
                "retirement",
                "retirement proof does not match current owner/input/interaction",
            )?;
            require(
                zero_byte_state(&receipt.input),
                "retirement",
                "serialized zero-byte state required",
            )?;
            if receipt.claim.is_some() {
                let changed = conn.execute("DELETE FROM member_permission_response_claims WHERE interaction_id=?1 AND input_id=?2",params![proof.interaction_id,input_id]).map_err(db_err)?;
                require(changed == 1, "permission_claim", "claim release lost CAS")?;
            }
            get_permission_response(conn, input_id)
        })
    })
}
