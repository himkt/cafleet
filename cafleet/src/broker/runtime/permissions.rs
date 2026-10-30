use super::*;
use crate::coding_agent::AnswerCapability;
use crate::runtime::runner::with_database_deadline;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct ZeroByteRetirement<'guard> {
    owner: RunOwner,
    input_id: i64,
    interaction_id: i64,
    input: Option<InputOperation>,
    rearm: Option<(NewPermissionResponse, AnswerCapability)>,
    guard: PhantomData<(&'guard mut (), Rc<()>)>,
}
impl<'guard> ZeroByteRetirement<'guard> {
    pub(crate) fn from_live_guard(
        guard: &'guard crate::runtime::runner::InputGuard<'_>,
        owner: &RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<Self, CafleetError> {
        let (receipt, capability) = guard.retirement_binding(owner, input_id, interaction_id)?;
        Ok(Self {
            owner: owner.clone(),
            input_id,
            interaction_id,
            input: Some(receipt.input.clone()),
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

    pub(crate) fn from_exclusive_guard(
        guard: &'guard crate::runtime::runner::ExclusiveReceiptGuard<'_>,
        owner: &RunOwner,
        input_id: i64,
        interaction_id: i64,
    ) -> Result<Self, CafleetError> {
        let input = guard.retirement_binding(owner, input_id, interaction_id)?;
        Ok(Self {
            owner: owner.clone(),
            input_id,
            interaction_id,
            input: Some(input.clone()),
            rearm: None,
            guard: PhantomData,
        })
    }

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
            input: None,
            rearm: None,
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

pub(crate) fn permission_response_digest(
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
    Ok(crate::digest::sha256_hex(
        crate::digest::canonical_json(&object).as_bytes(),
    ))
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
            interaction.classification == InteractionClass::PermissionRequest
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
            permission_event_matches(&event, run.run_id, metadata, true),
            "permission_response",
            "current open inbox permission event required",
        )?;
        validate_current_capability(&tx, &run, metadata, &event, capability)?;
        inputs::require_capacity(&tx, run.run_id)?;
        let digest = permission_response_digest(run.run_id, metadata)?;
        let now = now_stamp();
        tx.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,?2,'permission_response',?3,'in_progress',?4,?4)",params![run.run_id,request.operation_key,digest,now])?;
        let input_id = tx.last_insert_rowid();
        tx.execute("INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![input_id,metadata.event_id,metadata.interaction_id,metadata.expected_owner_epoch,metadata.request_token,metadata.decision.as_str(),metadata.director_member_id,metadata.director_mode.as_str(),metadata.decision_source.as_str(),metadata.mode_evidence_ref,metadata.decision_evidence_ref,metadata.answer_profile_version,metadata.answer_rule_id,now])?;
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

#[cfg(test)]
pub(crate) fn release_permission_claim(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    proof: &ZeroByteRetirement<'_>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    with_database_deadline(conn, deadline, |conn| {
        owned_transaction(conn, owner, |conn, _| {
            release_permission_claim_in_transaction(conn, owner, input_id, proof, deadline)
        })
    })
}

fn same_input_identity(a: &InputOperation, b: &InputOperation) -> bool {
    a.input_id == b.input_id
        && a.run_id == b.run_id
        && a.operation_key == b.operation_key
        && a.kind == b.kind
        && a.payload_digest == b.payload_digest
        && a.message_id == b.message_id
        && a.created_at == b.created_at
        && a.permission_response == b.permission_response
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
    if let Some(input) = &proof.input {
        require(
            same_input_identity(input, &receipt.input),
            "retirement",
            "retirement immutable receipt changed",
        )?;
    }
    Ok(())
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

pub(crate) fn release_permission_claim_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    input_id: i64,
    proof: &ZeroByteRetirement<'_>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    require(
        !conn.is_autocommit(),
        "transaction",
        "claim release requires an active ownership transaction",
    )?;
    validate_owner(conn, owner)?;
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

pub(crate) fn settle_permission_response_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    expected_input: &InputOperation,
    disposition: InputDisposition,
    evidence_ref: &str,
    proof: Option<&ZeroByteRetirement<'_>>,
    deadline: Instant,
) -> Result<PermissionReceipt, CafleetError> {
    require(
        !conn.is_autocommit(),
        "transaction",
        "permission settlement requires an active ownership transaction",
    )?;
    validate_owner(conn, owner)?;
    let receipt = get_permission_response(conn, expected_input.input_id)?;
    require(
        same_input_identity(&receipt.input, expected_input)
            && receipt.input.state == expected_input.state,
        "permission_response",
        "permission settlement snapshot changed",
    )?;
    if let Some(proof) = proof {
        validate_proof(owner, &receipt, proof)?;
    }
    inputs::settle_input_in_transaction(
        conn,
        owner,
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
    if expected_input.kind == InputKind::PermissionResponse {
        settle_permission_response_in_transaction(
            conn,
            owner,
            expected_input,
            disposition,
            evidence_ref,
            proof,
            deadline,
        )
        .map(|receipt| receipt.input)
    } else {
        inputs::settle_input_in_transaction(
            conn,
            owner,
            expected_input.input_id,
            expected_input.state,
            disposition,
            evidence_ref,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    use rusqlite::{Connection, params};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use crate::broker::members::test_support::register;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::coding_agent::{AnswerCapability, CodingAgent};

    fn state(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        let mut snapshot = durable_state(conn);
        snapshot.push(rows(conn, "member_permission_responses"));
        snapshot.push(rows(conn, "member_permission_response_claims"));
        snapshot.push(rows(conn, "sqlite_sequence"));
        snapshot
    }

    fn expected_digest(run: i64, metadata: &PermissionResponseMetadata) -> String {
        let value = json!({
            "run_id":run,"owner_epoch":metadata.expected_owner_epoch,"interaction_id":metadata.interaction_id,
            "event_id":metadata.event_id,"director_member_id":metadata.director_member_id,
            "request_token":metadata.request_token,"decision":metadata.decision,
            "director_mode":metadata.director_mode,"decision_source":metadata.decision_source,
            "mode_evidence_ref":metadata.mode_evidence_ref,"decision_evidence_ref":metadata.decision_evidence_ref,
            "answer_profile_version":metadata.answer_profile_version,"answer_rule_id":metadata.answer_rule_id,
        });
        let sorted = value
            .as_object()
            .unwrap()
            .iter()
            .collect::<BTreeMap<_, _>>();
        crate::digest::hex(&Sha256::digest(serde_json::to_vec(&sorted).unwrap()))
    }

    fn assert_rejected_unchanged(fixture: &mut ResponseFixture, request: &NewPermissionResponse) {
        let before = state(&fixture.base.conn);
        assert!(
            storage::reserve_permission_response(
                &mut fixture.base.conn,
                &fixture.owner,
                request,
                Some(&fixture.capability),
                deadline()
            )
            .is_err()
        );
        assert_eq!(state(&fixture.base.reopen()), before);
    }

    #[test]
    fn runtime_permission_response_rejects_an_automatic_review_event_before_any_write() {
        for kind in [
            RuntimeEventKind::AutomaticRefusalObserved,
            RuntimeEventKind::AutomaticReviewUnavailable,
        ] {
            let mut fixture = ResponseFixture::new();
            let refusal = storage::persist_event(
                &mut fixture.base.conn,
                &fixture.owner,
                &ObservedTransition {
                    episode_seq: None,
                    kind,
                    classification: None,
                    ..opening(fixture.owner.run_id, 2, 1, metadata())
                },
                PersistenceMode::Live,
            )
            .unwrap();
            assert_eq!(refusal.interaction_id, None);
            let mut request = fixture.request.clone();
            request.metadata.event_id = refusal.event_id;
            assert_rejected_unchanged(&mut fixture, &request);
            assert!(rows(&fixture.base.conn, "member_input_operations").is_empty());
            assert!(rows(&fixture.base.conn, "member_permission_responses").is_empty());
        }
    }

    fn another_run(fixture: &mut ResponseFixture) -> (RunOwner, NewPermissionResponse) {
        let member = register(
            &mut fixture.base.conn,
            fixture.base.fleet,
            "another-response-worker",
            Some("%3"),
        );
        let spec = Fixture::spec(
            &fixture.base.conn,
            fixture.base.fleet,
            fixture.base.director,
            member,
        );
        let run = storage::create_run(&mut fixture.base.conn, &spec).unwrap();
        fixture.base.conn.execute("UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2,coverage='tested',profile_version='fixture-v1' WHERE run_id=?1",[run.run_id]).unwrap();
        let owner = storage::get_run(&fixture.base.conn, run.run_id)
            .unwrap()
            .owner();
        storage::reserve_event_sequences(&mut fixture.base.conn, &owner).unwrap();
        let event = storage::persist_event(
            &mut fixture.base.conn,
            &owner,
            &opening(owner.run_id, 1, 1, metadata()),
            PersistenceMode::Live,
        )
        .unwrap();
        let mut request = fixture.request.clone();
        request.metadata.event_id = event.event_id;
        request.metadata.interaction_id = event.interaction_id.unwrap();
        request.metadata.expected_owner_epoch = owner.owner_epoch;
        (owner, request)
    }

    #[test]
    fn runtime_permission_claims_and_equal_operation_keys_remain_independent_across_runs() {
        let mut fixture = ResponseFixture::new();
        let first = fixture.reserve();
        let (owner, request) = another_run(&mut fixture);
        let second = match storage::reserve_permission_response(
            &mut fixture.base.conn,
            &owner,
            &request,
            Some(&fixture.capability),
            deadline(),
        )
        .unwrap()
        {
            PermissionReservation::Reserved(receipt) => receipt,
            PermissionReservation::Retained(_) => {
                panic!("another run inherited a retained key")
            }
        };
        assert_ne!(first.input.input_id, second.input.input_id);
        assert_ne!(
            first.claim.as_ref().unwrap().interaction_id,
            second.claim.as_ref().unwrap().interaction_id
        );
        assert_eq!(first.input.operation_key, second.input.operation_key);
        assert_eq!(
            rows(&fixture.base.conn, "member_permission_response_claims").len(),
            2
        );
    }

    #[test]
    fn runtime_permission_readers_reject_a_foreign_run_event_and_claim_input() {
        for changed in ["event", "claim"] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            if changed == "event" {
                let (_, request) = another_run(&mut fixture);
                fixture
                    .base
                    .conn
                    .execute(
                        "UPDATE member_permission_responses SET event_id=?1 WHERE input_id=?2",
                        params![request.metadata.event_id, receipt.input.input_id],
                    )
                    .unwrap();
            } else {
                let ordinary = fixture.base.input(
                    &fixture.owner,
                    "foreign-claim-input",
                    InputKind::Prompt,
                    None,
                );
                fixture
                    .base
                    .conn
                    .execute_batch("PRAGMA foreign_keys=OFF")
                    .unwrap();
                fixture.base.conn.execute("UPDATE member_permission_response_claims SET input_id=?1 WHERE interaction_id=?2",params![ordinary.input_id,fixture.request.metadata.interaction_id]).unwrap();
                fixture
                    .base
                    .conn
                    .execute_batch("PRAGMA foreign_keys=ON")
                    .unwrap();
            }
            assert!(
                storage::get_input(&fixture.base.conn, receipt.input.input_id).is_err(),
                "{changed}"
            );
            assert!(
                storage::get_permission_response(&fixture.base.conn, receipt.input.input_id)
                    .is_err(),
                "{changed}"
            );
            assert!(
                storage::lookup_permission_response(
                    &fixture.base.conn,
                    &fixture.identity(),
                    &fixture.request
                )
                .is_err(),
                "{changed}"
            );
        }
    }

    #[test]
    fn runtime_permission_historical_associations_are_checked_with_a_recomputed_valid_digest() {
        for changed in ["run", "event", "interaction", "director"] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            assert_eq!(
                storage::get_permission_response(&fixture.base.conn, receipt.input.input_id)
                    .unwrap(),
                receipt
            );
            let (other_owner, _) = another_run(&mut fixture);
            let alternate_episode = if matches!(changed, "event" | "interaction") {
                storage::persist_event(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    &closure(fixture.owner.run_id, 2, 1),
                    PersistenceMode::Live,
                )
                .unwrap();
                let event = storage::persist_event(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    &opening(fixture.owner.run_id, 3, 2, metadata()),
                    PersistenceMode::Live,
                )
                .unwrap();
                assert_eq!(
                    storage::get_permission_response(&fixture.base.conn, receipt.input.input_id)
                        .unwrap(),
                    receipt
                );
                Some(event)
            } else {
                None
            };
            let mut request = fixture.request.clone();
            let mut identity = fixture.identity();
            match changed {
                "run" => identity.run_id = other_owner.run_id,
                "event" => request.metadata.event_id = alternate_episode.as_ref().unwrap().event_id,
                "interaction" => {
                    request.metadata.interaction_id =
                        alternate_episode.as_ref().unwrap().interaction_id.unwrap()
                }
                "director" => request.metadata.director_member_id = fixture.base.member,
                _ => unreachable!(),
            }
            let digest = expected_digest(identity.run_id, &request.metadata);
            let tx = fixture.base.conn.transaction().unwrap();
            tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
            tx.execute("UPDATE member_permission_responses SET event_id=?1,interaction_id=?2,director_member_id=?3 WHERE input_id=?4",params![request.metadata.event_id,request.metadata.interaction_id,request.metadata.director_member_id,receipt.input.input_id]).unwrap();
            tx.execute(
                "UPDATE member_permission_response_claims SET interaction_id=?1 WHERE input_id=?2",
                params![request.metadata.interaction_id, receipt.input.input_id],
            )
            .unwrap();
            tx.execute(
                "UPDATE member_input_operations SET run_id=?1,payload_digest=?2 WHERE input_id=?3",
                params![identity.run_id, digest, receipt.input.input_id],
            )
            .unwrap();
            tx.commit().unwrap();
            assert!(
                fixture
                    .base
                    .conn
                    .prepare("PRAGMA foreign_key_check")
                    .unwrap()
                    .query([])
                    .unwrap()
                    .next()
                    .unwrap()
                    .is_none(),
                "{changed}"
            );
            assert_eq!(
                fixture
                    .base
                    .conn
                    .query_row(
                        "SELECT payload_digest FROM member_input_operations WHERE input_id=?1",
                        [receipt.input.input_id],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                expected_digest(identity.run_id, &request.metadata)
            );
            assert!(
                storage::get_input(&fixture.base.conn, receipt.input.input_id).is_err(),
                "{changed}"
            );
            assert!(
                storage::find_input(&fixture.base.conn, identity.run_id, &request.operation_key)
                    .is_err(),
                "{changed}"
            );
            assert!(
                storage::get_permission_response(&fixture.base.conn, receipt.input.input_id)
                    .is_err(),
                "{changed}"
            );
            assert!(
                storage::lookup_permission_response(&fixture.base.conn, &identity, &request)
                    .is_err(),
                "{changed}"
            );
        }
    }

    #[test]
    fn runtime_permission_foreign_claim_is_rejected_even_with_a_valid_composite_foreign_key() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        assert_eq!(
            storage::get_permission_response(&fixture.base.conn, receipt.input.input_id).unwrap(),
            receipt
        );
        let digest = expected_digest(fixture.owner.run_id, &fixture.request.metadata);
        fixture.base.conn.execute("INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) VALUES(?1,'foreign-valid-audit','permission_response',?2,'in_progress',?3,?3)",params![fixture.owner.run_id,digest,NOW]).unwrap();
        let foreign_input = fixture.base.conn.last_insert_rowid();
        fixture.base.conn.execute("INSERT INTO member_permission_responses(input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at) SELECT ?1,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at FROM member_permission_responses WHERE input_id=?2",params![foreign_input,receipt.input.input_id]).unwrap();
        fixture
            .base
            .conn
            .execute(
                "UPDATE member_permission_response_claims SET input_id=?1 WHERE interaction_id=?2",
                params![foreign_input, fixture.request.metadata.interaction_id],
            )
            .unwrap();
        assert!(
            fixture
                .base
                .conn
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            storage::get_permission_response(&fixture.base.conn, foreign_input)
                .unwrap()
                .claim
                .unwrap()
                .input_id,
            foreign_input
        );
        assert!(storage::get_input(&fixture.base.conn, receipt.input.input_id).is_err());
        assert!(
            storage::get_permission_response(&fixture.base.conn, receipt.input.input_id).is_err()
        );
        assert!(
            storage::lookup_permission_response(
                &fixture.base.conn,
                &fixture.identity(),
                &fixture.request
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_permission_expired_and_blocked_reservations_share_the_caller_deadline() {
        for blocked in [false, true] {
            let mut fixture = ResponseFixture::new();
            let blocker = fixture.base.reopen();
            if blocked {
                blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
            }
            let before = state(&fixture.base.conn);
            let started = Instant::now();
            let until = if blocked {
                started + Duration::from_millis(120)
            } else {
                started - Duration::from_millis(1)
            };
            let result = storage::reserve_permission_response(
                &mut fixture.base.conn,
                &fixture.owner,
                &fixture.request,
                Some(&fixture.capability),
                until,
            );
            assert!(result.is_err(), "{result:?}");
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(state(&fixture.base.conn), before);
            if blocked {
                blocker.execute_batch("ROLLBACK").unwrap();
            }
        }
    }

    #[test]
    fn runtime_permission_retained_reservation_returns_while_another_writer_holds_sqlite() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        let blocker = fixture.base.reopen();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let result = storage::reserve_permission_response(
            &mut fixture.base.conn,
            &fixture.owner,
            &fixture.request,
            None,
            Instant::now() + Duration::from_millis(120),
        );
        blocker.execute_batch("ROLLBACK").unwrap();
        match result.unwrap() {
            PermissionReservation::Retained(found) => assert_eq!(found, receipt),
            PermissionReservation::Reserved(_) => {
                panic!("retained lookup entered a new write transaction")
            }
        }
    }

    #[test]
    fn runtime_permission_recovery_proof_uses_current_owner_while_audit_retains_decision_epoch() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        storage::record_input_state(
            &mut fixture.base.conn,
            &fixture.owner,
            receipt.input.input_id,
            InputState::InProgress,
            InputState::Ready,
        )
        .unwrap();
        let recovery = fixture.base.recoverer();
        let mut guard = ();
        let proof = ZeroByteRetirement::fixture(
            &recovery,
            receipt.input.input_id,
            fixture.request.metadata.interaction_id,
            &mut guard,
        )
        .unwrap();
        let released = storage::release_permission_claim(
            &mut fixture.base.conn,
            &recovery,
            receipt.input.input_id,
            &proof,
            deadline(),
        )
        .unwrap();
        assert_eq!(released.claim, None);
        assert_eq!(released.response, receipt.response);
        assert_eq!(
            released.response.metadata.expected_owner_epoch,
            fixture.owner.owner_epoch
        );
        assert!(recovery.owner_epoch > fixture.owner.owner_epoch);
    }

    #[test]
    fn runtime_permission_reservation_commits_inprogress_input_audit_and_claim_together() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        assert_eq!(receipt.input.kind, InputKind::PermissionResponse);
        assert_eq!(receipt.input.state, InputState::InProgress);
        assert_eq!(receipt.input.message_id, None);
        assert_eq!(
            receipt.input.payload_digest,
            expected_digest(fixture.owner.run_id, &fixture.request.metadata)
        );
        assert_eq!(receipt.response.metadata, fixture.request.metadata);
        assert_eq!(receipt.response.input_id, receipt.input.input_id);
        assert_eq!(
            receipt.claim,
            Some(PermissionResponseClaim {
                interaction_id: fixture.request.metadata.interaction_id,
                input_id: receipt.input.input_id
            })
        );
        assert_eq!(
            receipt.input.permission_response,
            Some(receipt.response.clone())
        );
        let time = chrono::DateTime::parse_from_rfc3339(&receipt.response.decided_at).unwrap();
        assert_eq!(time.offset().local_minus_utc(), 0);
        assert_eq!(
            storage::get_permission_response(&fixture.base.reopen(), receipt.input.input_id)
                .unwrap(),
            receipt
        );
        assert_eq!(
            storage::get_input(&fixture.base.conn, receipt.input.input_id).unwrap(),
            receipt.input
        );
        assert_eq!(
            rows(&fixture.base.conn, "member_permission_responses").len(),
            1
        );
        assert_eq!(
            rows(&fixture.base.conn, "member_permission_response_claims").len(),
            1
        );
    }

    #[test]
    fn runtime_permission_user_decisions_roundtrip_each_attributed_mode_and_decision() {
        for mode in [
            DirectorMode::Auto,
            DirectorMode::Manual,
            DirectorMode::Unknown,
        ] {
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                let mut fixture = ResponseFixture::new();
                fixture.request.metadata.director_mode = mode;
                fixture.request.metadata.decision_source = DecisionSource::User;
                fixture.request.metadata.decision = decision;
                fixture.capability = capability(decision);
                let receipt = fixture.reserve();
                assert_eq!(receipt.response.metadata.director_mode, mode);
                assert_eq!(receipt.response.metadata.decision, decision);
                assert_eq!(
                    receipt.response.metadata.decision_source,
                    DecisionSource::User
                );
            }
        }
    }

    #[test]
    fn runtime_permission_digest_matches_independent_sorted_metadata_and_changes_per_field() {
        let fixture = ResponseFixture::new();
        let mut metadata = fixture.request.metadata.clone();
        metadata.decision_source = DecisionSource::User;
        let original =
            storage::permission_response_digest(fixture.owner.run_id, &metadata).unwrap();
        assert_eq!(original, expected_digest(fixture.owner.run_id, &metadata));
        assert_ne!(
            storage::permission_response_digest(fixture.owner.run_id + 1, &metadata).unwrap(),
            original
        );
        for field in [
            "epoch",
            "interaction",
            "event",
            "director",
            "token",
            "decision",
            "mode",
            "source",
            "mode-ref",
            "decision-ref",
            "profile",
            "rule",
        ] {
            let mut changed = metadata.clone();
            match field {
                "epoch" => changed.expected_owner_epoch += 1,
                "interaction" => changed.interaction_id += 1,
                "event" => changed.event_id += 1,
                "director" => changed.director_member_id += 1,
                "token" => changed.request_token = "b".repeat(64),
                "decision" => changed.decision = PermissionDecision::Deny,
                "mode" => changed.director_mode = DirectorMode::Manual,
                "source" => changed.decision_source = DecisionSource::DirectorAuto,
                "mode-ref" => changed.mode_evidence_ref = "fixture:other-mode".into(),
                "decision-ref" => changed.decision_evidence_ref = "fixture:other-decision".into(),
                "profile" => changed.answer_profile_version = "answer-fixture-v2".into(),
                "rule" => changed.answer_rule_id = "different-choice".into(),
                _ => unreachable!(),
            }
            let digest =
                storage::permission_response_digest(fixture.owner.run_id, &changed).unwrap();
            assert_eq!(
                digest,
                expected_digest(fixture.owner.run_id, &changed),
                "{field}"
            );
            assert_ne!(digest, original, "{field}");
        }
        let mut other_key = fixture.request.clone();
        other_key.metadata = metadata;
        other_key.operation_key = "a separate operation key".into();
        assert_eq!(
            storage::permission_response_digest(fixture.owner.run_id, &other_key.metadata).unwrap(),
            original
        );
    }

    #[test]
    fn runtime_permission_invalid_identity_token_mode_and_required_metadata_roll_back() {
        for field in [
            "key",
            "event",
            "interaction",
            "epoch",
            "director",
            "short-token",
            "uppercase-token",
            "nonhex-token",
            "profile",
            "rule",
            "auto-source-manual",
            "auto-source-unknown",
        ] {
            let mut fixture = ResponseFixture::new();
            let mut request = fixture.request.clone();
            match field {
                "key" => request.operation_key = " ".into(),
                "event" => request.metadata.event_id = 0,
                "interaction" => request.metadata.interaction_id = -1,
                "epoch" => request.metadata.expected_owner_epoch = 0,
                "director" => request.metadata.director_member_id = 0,
                "short-token" => request.metadata.request_token = "a".repeat(63),
                "uppercase-token" => request.metadata.request_token = "A".repeat(64),
                "nonhex-token" => request.metadata.request_token = "g".repeat(64),
                "profile" => request.metadata.answer_profile_version = " ".into(),
                "rule" => request.metadata.answer_rule_id = "".into(),
                "auto-source-manual" => request.metadata.director_mode = DirectorMode::Manual,
                "auto-source-unknown" => request.metadata.director_mode = DirectorMode::Unknown,
                _ => unreachable!(),
            }
            assert_rejected_unchanged(&mut fixture, &request);
        }
    }

    #[test]
    fn runtime_permission_reference_validation_uses_utf8_bytes_and_rejects_controls() {
        for reference in [
            "".into(),
            " \u{2003} ".into(),
            "x".repeat(2049),
            "界".repeat(683),
            "line\nnext".into(),
            "x\u{0}y".into(),
            "x\u{1f}y".into(),
            "x\u{7f}y".into(),
        ] {
            for mode_reference in [false, true] {
                let mut fixture = ResponseFixture::new();
                let mut request = fixture.request.clone();
                if mode_reference {
                    request.metadata.mode_evidence_ref = reference.clone();
                } else {
                    request.metadata.decision_evidence_ref = reference.clone();
                }
                assert_rejected_unchanged(&mut fixture, &request);
            }
        }
        let mut fixture = ResponseFixture::new();
        fixture.request.metadata.mode_evidence_ref = format!("{}ab", "界".repeat(682));
        fixture.request.metadata.decision_evidence_ref = "x".repeat(2048);
        let receipt = fixture.reserve();
        assert_eq!(receipt.response.metadata.mode_evidence_ref.len(), 2048);
        assert_eq!(receipt.response.metadata.decision_evidence_ref.len(), 2048);
    }

    #[test]
    fn runtime_permission_new_reservation_requires_matching_owner_and_live_relationships() {
        for changed in [
            "epoch",
            "kind",
            "database",
            "member",
            "director",
            "fleet",
            "status",
            "expected-epoch",
            "pinned-director",
        ] {
            let mut fixture = ResponseFixture::new();
            match changed {
                "epoch" => fixture.owner.owner_epoch += 1,
                "kind" => fixture.owner.owner_kind = OwnerKind::Creator,
                "database" => {
                    fixture.owner.database_identity_json["instance_id"] = json!("0".repeat(32))
                }
                "member" => {
                    crate::broker::deregister_member(&mut fixture.base.conn, fixture.base.member)
                        .unwrap();
                }
                "director" => {
                    fixture.base.conn.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",params![NOW,fixture.base.director]).unwrap();
                }
                "fleet" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                            params![NOW, fixture.base.fleet],
                        )
                        .unwrap();
                }
                "status" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_runs SET status='degraded' WHERE run_id=?1",
                            [fixture.owner.run_id],
                        )
                        .unwrap();
                }
                "expected-epoch" => fixture.request.metadata.expected_owner_epoch += 1,
                "pinned-director" => {
                    fixture.request.metadata.director_member_id = fixture.base.member
                }
                _ => unreachable!(),
            }
            let request = fixture.request.clone();
            assert_rejected_unchanged(&mut fixture, &request);
        }
    }

    #[test]
    fn runtime_permission_new_reservation_requires_the_open_inbox_permission_event() {
        for changed in [
            "closed",
            "wrong-event-kind",
            "wrong-class",
            "message",
            "archived",
        ] {
            let mut fixture = ResponseFixture::new();
            match changed {
                "closed" => {
                    fixture.base.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",params![NOW,fixture.request.metadata.interaction_id]).unwrap();
                }
                "wrong-event-kind" => {
                    let next = storage::persist_event(
                        &mut fixture.base.conn,
                        &fixture.owner,
                        &ObservedTransition {
                            kind: RuntimeEventKind::InteractionObscured,
                            ..opening(fixture.owner.run_id, 2, 1, metadata())
                        },
                        PersistenceMode::Live,
                    )
                    .unwrap();
                    fixture.request.metadata.event_id = next.event_id;
                }
                "wrong-class" => {
                    let mut payload = fixture.event.payload_json.clone();
                    payload["classification"] = json!("question");
                    payload["director_action"] = Value::Null;
                    fixture.base.conn.execute("UPDATE member_interactions SET classification='question' WHERE interaction_id=?1",[fixture.request.metadata.interaction_id]).unwrap();
                    replace_public_payload(&mut fixture.base.conn, &fixture.event, payload);
                }
                "message" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE messages SET text='forged message' WHERE message_id=?1",
                            [fixture.event.message_id.unwrap()],
                        )
                        .unwrap();
                }
                "archived" => {
                    fixture.base.conn.execute("UPDATE member_runtime_events SET notification_state='archived',message_id=NULL,archive_reason='member_inactive' WHERE event_id=?1",[fixture.event.event_id]).unwrap();
                }
                _ => unreachable!(),
            }
            let request = fixture.request.clone();
            assert_rejected_unchanged(&mut fixture, &request);
        }
    }

    #[test]
    fn runtime_permission_capability_binding_is_required_only_for_a_fresh_attempt() {
        for changed in [
            "absent",
            "backend",
            "version",
            "profile",
            "request-rule",
            "answer-rule",
            "decision",
        ] {
            let mut fixture = ResponseFixture::new();
            let (backend, version, profile, request_rule, answer_rule, decision) = (
                if changed == "backend" {
                    CodingAgent::Claude
                } else {
                    CodingAgent::Codex
                },
                if changed == "version" {
                    "other-version"
                } else {
                    "0.163.0"
                },
                if changed == "profile" {
                    "different-profile"
                } else {
                    "answer-fixture-v1"
                },
                if changed == "request-rule" {
                    "different-request"
                } else {
                    "native-tool-permission"
                },
                if changed == "answer-rule" {
                    "different-answer"
                } else {
                    "single-request-choice"
                },
                if changed == "decision" {
                    PermissionDecision::Deny
                } else {
                    PermissionDecision::Allow
                },
            );
            let capability = AnswerCapability::fixture(
                backend,
                version,
                profile,
                request_rule,
                answer_rule,
                decision,
                &["synthetic-fixture".into()],
            )
            .unwrap();
            let before = state(&fixture.base.conn);
            assert!(
                storage::reserve_permission_response(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    &fixture.request,
                    if changed == "absent" {
                        None
                    } else {
                        Some(&capability)
                    },
                    deadline()
                )
                .is_err(),
                "{changed}"
            );
            assert_eq!(state(&fixture.base.conn), before);
        }
    }

    #[test]
    fn runtime_permission_reservation_rolls_back_each_insert_and_commit_failure() {
        for target in [
            "BEFORE INSERT ON member_input_operations",
            "BEFORE INSERT ON member_permission_responses",
            "BEFORE INSERT ON member_permission_response_claims",
            "commit",
        ] {
            let mut fixture = ResponseFixture::new();
            if target == "commit" {
                fixture
                    .base
                    .conn
                    .authorizer(Some(|context: AuthContext<'_>| {
                        if matches!(
                            context.action,
                            AuthAction::Transaction {
                                operation: TransactionOperation::Unknown
                            }
                        ) {
                            Authorization::Deny
                        } else {
                            Authorization::Allow
                        }
                    }))
                    .unwrap();
            } else {
                fixture.base.conn.execute_batch(&format!("CREATE TRIGGER response_failure {target} BEGIN SELECT RAISE(ABORT,'fixture response reservation failure'); END")).unwrap();
            }
            let request = fixture.request.clone();
            assert_rejected_unchanged(&mut fixture, &request);
        }
    }

    #[test]
    fn runtime_permission_retained_lookup_precedes_changed_live_owner_and_capability() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        fixture.base.conn.execute("UPDATE member_runs SET owner_epoch=owner_epoch+1,owner_kind='reconciler',status='exited',ended_at=?1 WHERE run_id=?2",params![NOW,fixture.owner.run_id]).unwrap();
        let before = state(&fixture.base.conn);
        let found = storage::lookup_permission_response(
            &fixture.base.conn,
            &fixture.identity(),
            &fixture.request,
        )
        .unwrap()
        .unwrap();
        assert_eq!(found, receipt);
        match storage::reserve_permission_response(
            &mut fixture.base.conn,
            &fixture.owner,
            &fixture.request,
            None,
            deadline(),
        )
        .unwrap()
        {
            PermissionReservation::Retained(found) => assert_eq!(found, receipt),
            PermissionReservation::Reserved(_) => {
                panic!("retained key started a second attempt")
            }
        }
        assert_eq!(state(&fixture.base.conn), before);
    }

    #[test]
    fn runtime_permission_historical_receipts_survive_closed_and_inactive_relationships() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        fixture.base.conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason='not_visible' WHERE interaction_id=?2",params![NOW,fixture.request.metadata.interaction_id]).unwrap();
        crate::broker::deregister_member(&mut fixture.base.conn, fixture.base.member).unwrap();
        fixture
            .base
            .conn
            .execute(
                "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                params![NOW, fixture.base.director],
            )
            .unwrap();
        fixture
            .base
            .conn
            .execute(
                "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                params![NOW, fixture.base.fleet],
            )
            .unwrap();
        fixture
            .base
            .conn
            .execute(
                "UPDATE member_runs SET status='lost',owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [fixture.owner.run_id],
            )
            .unwrap();
        assert_eq!(
            storage::get_permission_response(&fixture.base.conn, receipt.input.input_id).unwrap(),
            receipt
        );
        assert_eq!(
            storage::lookup_permission_response(
                &fixture.base.conn,
                &fixture.identity(),
                &fixture.request
            )
            .unwrap(),
            Some(receipt)
        );
    }

    #[test]
    fn runtime_permission_retained_key_conflicts_on_changed_identity_or_digest() {
        let mut fixture = ResponseFixture::new();
        fixture.reserve();
        for changed in [
            "decision",
            "token",
            "epoch",
            "event",
            "interaction",
            "reference",
            "database",
        ] {
            let mut request = fixture.request.clone();
            let mut identity = fixture.identity();
            match changed {
                "decision" => request.metadata.decision = PermissionDecision::Deny,
                "token" => request.metadata.request_token = "b".repeat(64),
                "epoch" => request.metadata.expected_owner_epoch += 1,
                "event" => request.metadata.event_id += 1,
                "interaction" => request.metadata.interaction_id += 1,
                "reference" => request.metadata.decision_evidence_ref = "fixture:different".into(),
                "database" => {
                    identity.database_identity_json["instance_id"] = json!("0".repeat(32))
                }
                _ => unreachable!(),
            }
            let before = state(&fixture.base.conn);
            assert!(
                storage::lookup_permission_response(&fixture.base.conn, &identity, &request)
                    .is_err(),
                "{changed}"
            );
            assert_eq!(state(&fixture.base.conn), before);
        }
    }

    #[test]
    fn runtime_permission_absent_key_is_distinct_from_an_existing_other_input_kind() {
        let mut fixture = ResponseFixture::new();
        assert_eq!(
            storage::lookup_permission_response(
                &fixture.base.conn,
                &fixture.identity(),
                &fixture.request
            )
            .unwrap(),
            None
        );
        fixture.base.input(
            &fixture.owner,
            &fixture.request.operation_key,
            InputKind::Prompt,
            None,
        );
        assert!(
            storage::lookup_permission_response(
                &fixture.base.conn,
                &fixture.identity(),
                &fixture.request
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_permission_generic_reservation_cannot_create_an_unaudited_response() {
        let mut fixture = ResponseFixture::new();
        let before = state(&fixture.base.conn);
        assert!(
            storage::reserve_input(
                &mut fixture.base.conn,
                &fixture.owner,
                &NewInput {
                    operation_key: "generic-response".into(),
                    kind: InputKind::PermissionResponse,
                    payload_digest: "d".repeat(64),
                    message_id: None
                }
            )
            .is_err()
        );
        assert_eq!(state(&fixture.base.conn), before);
    }

    #[test]
    fn runtime_permission_ready_row_cannot_be_rearmed_by_generic_state_transition() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        storage::record_input_state(
            &mut fixture.base.conn,
            &fixture.owner,
            receipt.input.input_id,
            InputState::InProgress,
            InputState::Ready,
        )
        .unwrap();
        let before = state(&fixture.base.conn);
        assert!(
            storage::record_input_state(
                &mut fixture.base.conn,
                &fixture.owner,
                receipt.input.input_id,
                InputState::Ready,
                InputState::InProgress
            )
            .is_err()
        );
        assert_eq!(state(&fixture.base.conn), before);
        match storage::reserve_permission_response(
            &mut fixture.base.conn,
            &fixture.owner,
            &fixture.request,
            None,
            deadline(),
        )
        .unwrap()
        {
            PermissionReservation::Retained(found) => {
                assert_eq!(found.input.state, InputState::Ready)
            }
            PermissionReservation::Reserved(_) => {
                panic!("a retained Ready row was silently rearmed")
            }
        }
    }

    #[test]
    fn runtime_permission_episode_claim_blocks_a_different_key_in_every_possible_write_state() {
        for outcome in [
            "in-progress",
            "accepted",
            "uncertain",
            "submitted",
            "abandoned-uncertain",
            "ready",
            "not-submitted",
        ] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            match outcome {
                "accepted" | "uncertain" | "ready" => {
                    let next = match outcome {
                        "accepted" => InputState::Accepted,
                        "uncertain" => InputState::Uncertain,
                        "ready" => InputState::Ready,
                        _ => unreachable!(),
                    };
                    storage::record_input_state(
                        &mut fixture.base.conn,
                        &fixture.owner,
                        receipt.input.input_id,
                        InputState::InProgress,
                        next,
                    )
                    .unwrap();
                }
                "submitted" | "abandoned-uncertain" | "not-submitted" => {
                    let disposition = match outcome {
                        "submitted" => InputDisposition::Submitted,
                        "abandoned-uncertain" => InputDisposition::AbandonedUncertain,
                        "not-submitted" => InputDisposition::NotSubmitted,
                        _ => unreachable!(),
                    };
                    storage::settle_input(
                        &mut fixture.base.conn,
                        &fixture.owner,
                        receipt.input.input_id,
                        InputState::InProgress,
                        disposition,
                        "fixture:settlement",
                    )
                    .unwrap();
                }
                "in-progress" => {}
                _ => unreachable!(),
            }
            let mut second = fixture.request.clone();
            second.operation_key = "permission:different-key".into();
            second.metadata.decision = PermissionDecision::Deny;
            let before = state(&fixture.base.conn);
            let denied_capability = capability(PermissionDecision::Deny);
            assert!(
                storage::reserve_permission_response(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    &second,
                    Some(&denied_capability),
                    deadline()
                )
                .is_err(),
                "{outcome}"
            );
            assert_eq!(state(&fixture.base.conn), before);
            assert_eq!(
                rows(&fixture.base.conn, "member_permission_response_claims").len(),
                1
            );
        }
    }

    #[test]
    fn runtime_permission_concurrent_distinct_keys_commit_one_episode_claim() {
        let fixture = ResponseFixture::new();
        let barrier = Arc::new(Barrier::new(2));
        let url = format!("sqlite:///{}", fixture.base.conn.path().unwrap());
        let results = std::thread::scope(|scope| {
            let handles = (0..2)
                .map(|index| {
                    let barrier = barrier.clone();
                    let url = url.clone();
                    let owner = fixture.owner.clone();
                    let mut request = fixture.request.clone();
                    request.operation_key = format!("permission:racing-{index}");
                    scope.spawn(move || {
                        let mut conn = crate::db::connect(&url).unwrap();
                        let cap = capability(PermissionDecision::Allow);
                        barrier.wait();
                        storage::reserve_permission_response(
                            &mut conn,
                            &owner,
                            &request,
                            Some(&cap),
                            deadline(),
                        )
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert_eq!(rows(&fixture.base.conn, "member_input_operations").len(), 1);
        assert_eq!(
            rows(&fixture.base.conn, "member_permission_responses").len(),
            1
        );
        assert_eq!(
            rows(&fixture.base.conn, "member_permission_response_claims").len(),
            1
        );
    }

    #[test]
    fn runtime_permission_shared_unresolved_limit_preserves_retained_receipts() {
        let mut fixture = ResponseFixture::new();
        fixture.base.conn.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at) SELECT ?1,'capacity:'||x,'exec',?2,'in_progress',?3,?3 FROM n",params![fixture.owner.run_id,"d".repeat(64),NOW]).unwrap();
        let request = fixture.request.clone();
        assert_rejected_unchanged(&mut fixture, &request);
        fixture.base.conn.execute("UPDATE member_input_operations SET state='settled',disposition='not_submitted',evidence_ref='fixture:zero-byte' WHERE operation_key='capacity:1'",[]).unwrap();
        let receipt = fixture.reserve();
        match storage::reserve_permission_response(
            &mut fixture.base.conn,
            &fixture.owner,
            &fixture.request,
            None,
            deadline(),
        )
        .unwrap()
        {
            PermissionReservation::Retained(found) => assert_eq!(found, receipt),
            PermissionReservation::Reserved(_) => {
                panic!("capacity retry duplicated a retained operation")
            }
        }
        assert_eq!(
            rows(&fixture.base.conn, "member_input_operations").len(),
            4097
        );
    }

    #[test]
    fn runtime_permission_zero_byte_release_retains_audit_key_and_allows_a_new_key() {
        for settled in [false, true] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            if settled {
                storage::settle_input(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    receipt.input.input_id,
                    InputState::InProgress,
                    InputDisposition::NotSubmitted,
                    "fixture:zero-byte",
                )
                .unwrap();
            } else {
                storage::record_input_state(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    receipt.input.input_id,
                    InputState::InProgress,
                    InputState::Ready,
                )
                .unwrap();
            }
            let audit = rows(&fixture.base.conn, "member_permission_responses");
            let mut guard = ();
            let proof = ZeroByteRetirement::fixture(
                &fixture.owner,
                receipt.input.input_id,
                fixture.request.metadata.interaction_id,
                &mut guard,
            )
            .unwrap();
            let released = storage::release_permission_claim(
                &mut fixture.base.conn,
                &fixture.owner,
                receipt.input.input_id,
                &proof,
                deadline(),
            )
            .unwrap();
            assert_eq!(released.claim, None);
            assert_eq!(
                rows(&fixture.base.conn, "member_permission_responses"),
                audit
            );
            assert!(
                storage::lookup_permission_response(
                    &fixture.base.conn,
                    &fixture.identity(),
                    &fixture.request
                )
                .unwrap()
                .is_some()
            );
            let mut fresh = fixture.request.clone();
            fresh.operation_key = "permission:deliberate-fresh-attempt".into();
            fresh.metadata.request_token = "b".repeat(64);
            assert!(matches!(
                storage::reserve_permission_response(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    &fresh,
                    Some(&fixture.capability),
                    deadline()
                )
                .unwrap(),
                PermissionReservation::Reserved(_)
            ));
            assert_eq!(
                rows(&fixture.base.conn, "member_permission_responses").len(),
                2
            );
        }
    }

    #[test]
    fn runtime_permission_possible_write_states_keep_claim_even_with_matching_proof() {
        for outcome in [
            "in-progress",
            "accepted",
            "uncertain",
            "submitted",
            "abandoned-uncertain",
        ] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            match outcome {
                "accepted" | "uncertain" => {
                    storage::record_input_state(
                        &mut fixture.base.conn,
                        &fixture.owner,
                        receipt.input.input_id,
                        InputState::InProgress,
                        if outcome == "accepted" {
                            InputState::Accepted
                        } else {
                            InputState::Uncertain
                        },
                    )
                    .unwrap();
                }
                "submitted" | "abandoned-uncertain" => {
                    storage::settle_input(
                        &mut fixture.base.conn,
                        &fixture.owner,
                        receipt.input.input_id,
                        InputState::InProgress,
                        if outcome == "submitted" {
                            InputDisposition::Submitted
                        } else {
                            InputDisposition::AbandonedUncertain
                        },
                        "fixture:possible-write",
                    )
                    .unwrap();
                }
                "in-progress" => {}
                _ => unreachable!(),
            }
            let mut guard = ();
            let proof = ZeroByteRetirement::fixture(
                &fixture.owner,
                receipt.input.input_id,
                fixture.request.metadata.interaction_id,
                &mut guard,
            )
            .unwrap();
            let before = state(&fixture.base.conn);
            assert!(
                storage::release_permission_claim(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    receipt.input.input_id,
                    &proof,
                    deadline()
                )
                .is_err(),
                "{outcome}"
            );
            assert_eq!(state(&fixture.base.conn), before);
        }
    }

    #[test]
    fn runtime_permission_release_rejects_foreign_and_stale_borrowed_proofs() {
        for changed in ["input", "interaction", "epoch"] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            storage::record_input_state(
                &mut fixture.base.conn,
                &fixture.owner,
                receipt.input.input_id,
                InputState::InProgress,
                InputState::Ready,
            )
            .unwrap();
            let mut owner = fixture.owner.clone();
            let mut input = receipt.input.input_id;
            let mut interaction = fixture.request.metadata.interaction_id;
            match changed {
                "input" => input += 1,
                "interaction" => interaction += 1,
                "epoch" => owner.owner_epoch += 1,
                _ => unreachable!(),
            }
            let mut guard = ();
            let proof =
                ZeroByteRetirement::fixture(&owner, input, interaction, &mut guard).unwrap();
            let before = state(&fixture.base.conn);
            assert!(
                storage::release_permission_claim(
                    &mut fixture.base.conn,
                    &fixture.owner,
                    receipt.input.input_id,
                    &proof,
                    deadline()
                )
                .is_err(),
                "{changed}"
            );
            assert_eq!(state(&fixture.base.conn), before);
        }
    }

    #[test]
    fn runtime_permission_claim_release_failure_preserves_all_history_for_retry() {
        let mut fixture = ResponseFixture::new();
        let receipt = fixture.reserve();
        storage::record_input_state(
            &mut fixture.base.conn,
            &fixture.owner,
            receipt.input.input_id,
            InputState::InProgress,
            InputState::Ready,
        )
        .unwrap();
        fixture.base.conn.execute_batch("CREATE TRIGGER reject_claim_release BEFORE DELETE ON member_permission_response_claims BEGIN SELECT RAISE(ABORT,'fixture release failure'); END").unwrap();
        let mut guard = ();
        let proof = ZeroByteRetirement::fixture(
            &fixture.owner,
            receipt.input.input_id,
            fixture.request.metadata.interaction_id,
            &mut guard,
        )
        .unwrap();
        let before = state(&fixture.base.conn);
        assert!(
            storage::release_permission_claim(
                &mut fixture.base.conn,
                &fixture.owner,
                receipt.input.input_id,
                &proof,
                deadline()
            )
            .is_err()
        );
        assert_eq!(state(&fixture.base.conn), before);
        fixture
            .base
            .conn
            .execute_batch("DROP TRIGGER reject_claim_release")
            .unwrap();
        assert_eq!(
            storage::release_permission_claim(
                &mut fixture.base.conn,
                &fixture.owner,
                receipt.input.input_id,
                &proof,
                deadline()
            )
            .unwrap()
            .claim,
            None
        );
    }

    #[test]
    fn runtime_permission_readers_reject_missing_foreign_or_corrupted_audit_claim_data() {
        for changed in [
            "audit-missing",
            "claim-missing",
            "kind",
            "director",
            "epoch",
            "token",
            "reference",
            "time",
            "digest",
        ] {
            let mut fixture = ResponseFixture::new();
            let receipt = fixture.reserve();
            match changed {
                "audit-missing" => {
                    fixture
                        .base
                        .conn
                        .execute("DELETE FROM member_permission_response_claims", [])
                        .unwrap();
                    fixture
                        .base
                        .conn
                        .execute("DELETE FROM member_permission_responses", [])
                        .unwrap();
                }
                "claim-missing" => {
                    fixture
                        .base
                        .conn
                        .execute("DELETE FROM member_permission_response_claims", [])
                        .unwrap();
                }
                "kind" => {
                    fixture
                        .base
                        .conn
                        .execute("UPDATE member_input_operations SET kind='prompt'", [])
                        .unwrap();
                }
                "director" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_permission_responses SET director_member_id=?1",
                            [fixture.base.member],
                        )
                        .unwrap();
                }
                "epoch" => {
                    fixture.base.conn.execute("UPDATE member_permission_responses SET expected_owner_epoch=expected_owner_epoch+1",[]).unwrap();
                }
                "token" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_permission_responses SET request_token=?1",
                            ["b".repeat(64)],
                        )
                        .unwrap();
                }
                "reference" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_permission_responses SET decision_evidence_ref='\n'",
                            [],
                        )
                        .unwrap();
                }
                "time" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_permission_responses SET decided_at='not-a-time'",
                            [],
                        )
                        .unwrap();
                }
                "digest" => {
                    fixture
                        .base
                        .conn
                        .execute(
                            "UPDATE member_input_operations SET payload_digest=?1",
                            ["0".repeat(64)],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                storage::get_input(&fixture.base.conn, receipt.input.input_id).is_err(),
                "{changed}"
            );
            assert!(
                storage::find_input(
                    &fixture.base.conn,
                    fixture.owner.run_id,
                    &fixture.request.operation_key
                )
                .is_err(),
                "{changed}"
            );
            assert!(
                storage::get_permission_response(&fixture.base.conn, receipt.input.input_id)
                    .is_err(),
                "{changed}"
            );
            assert!(
                storage::lookup_permission_response(
                    &fixture.base.conn,
                    &fixture.identity(),
                    &fixture.request
                )
                .is_err(),
                "{changed}"
            );
        }
    }
}
