use rusqlite::{Connection, OptionalExtension, params};

use super::*;
use crate::time::now_stamp;

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
        message_id: row.get("message_id")?,
        disposition: row.get("disposition")?,
        evidence_ref: row.get("evidence_ref")?,
        permission_response: None,
        delivery_method: row.get("delivery_method")?,
        delivery_reason: row.get("delivery_reason")?,
        deferred_at: row.get("deferred_at")?,
        defer_reason: row.get("defer_reason")?,
        payload_digest_version: row.get("payload_digest_version")?,
    })
}

impl InputOperation {
    pub(crate) fn delivery(&self) -> Option<DeliveryProjection> {
        delivery_projection(self).expect("validated input receipts project their delivery")
    }
}

const FALLBACK_REASONS: [&str; 2] = ["submission_plan_unavailable", "idle_timeout"];

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
                && input.deferred_at.is_none()
                && input.payload_digest_version == 1,
            "member_input_operations.delivery_method",
            "permission responses carry no ordinary delivery metadata",
        )?;
        return Ok(None);
    }
    require(
        input.payload_digest_version == 1
            || (input.payload_digest_version == 2 && input.kind == InputKind::Preview),
        "member_input_operations.payload_digest_version",
        "unsupported digest version for this input kind",
    )?;
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
        None if input.delivery_reason.is_none() => Ok(Some("method_unrecorded".to_owned())),
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
        (InputState::Settled, Some(InputDisposition::NotSubmitted)) => {
            require(
                method.is_none(),
                "member_input_operations.delivery_method",
                "a zero-byte cancellation has no selected method",
            )?;
            (
                DeliveryStatus::NotSubmitted,
                Some(
                    input
                        .delivery_reason
                        .clone()
                        .unwrap_or_else(|| "method_unrecorded".to_owned()),
                ),
            )
        }
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
    nonblank(
        &input.operation_key,
        "member_input_operations.operation_key",
    )?;
    hex(
        &input.payload_digest,
        64,
        "member_input_operations.payload_digest",
    )?;
    require(
        (input.state == InputState::Settled) == input.disposition.is_some(),
        "member_input_operations.disposition",
        "settlement requires exactly one disposition",
    )?;
    let run = get_run(conn, input.run_id)?;
    if input.kind == InputKind::Preview {
        let message_id = input.message_id.ok_or_else(|| {
            invalid(
                "member_input_operations.message_id",
                "preview message required",
            )
        })?;
        let message = super::super::messaging::get_message(conn, message_id)?;
        require(
            message.to_member_id == run.member_id
                && input.operation_key == format!("preview:{}:{message_id}", run.run_id),
            "member_input_operations.message_id",
            "preview does not belong to this run",
        )?;
    } else {
        require(
            input.message_id.is_none(),
            "member_input_operations.message_id",
            "explicit input has no preview message",
        )?;
    }
    input.permission_response = super::permissions::validate_input_response(conn, &run, &input)?;
    delivery_projection(&input)?;
    Ok(input)
}

pub(crate) fn input_payload_digest(
    run_id: i64,
    input: &crate::runtime::runner::SubmitInput,
) -> String {
    let payload = json!({"run_id":run_id,"kind":input.input_kind.as_str(),
        "message_id":input.message_id,"literal":input.literal});
    crate::digest::sha256_hex(crate::digest::canonical_json(&payload).as_bytes())
}

pub(crate) fn preview_payload_digest_v2(
    run_id: i64,
    input: &crate::runtime::runner::SubmitInput,
) -> String {
    let payload = json!({"payload_digest_version":2,"run_id":run_id,"kind":input.input_kind.as_str(),
        "message_id":input.message_id,"literal":input.literal});
    crate::digest::sha256_hex(crate::digest::canonical_json(&payload).as_bytes())
}

/// The digest a stored receipt of `version` records for `input`.
pub(crate) fn versioned_payload_digest(
    version: i64,
    run_id: i64,
    input: &crate::runtime::runner::SubmitInput,
) -> Result<String, CafleetError> {
    match (version, input.input_kind) {
        (1, _) => Ok(input_payload_digest(run_id, input)),
        (2, InputKind::Preview) => Ok(preview_payload_digest_v2(run_id, input)),
        _ => Err(invalid(
            "member_input_operations.payload_digest_version",
            "unsupported digest version for this input kind",
        )),
    }
}

/// How a new ordinary receipt is reserved: the delivery selected before its first byte, or a
/// zero-byte queue admission with its defer reason, and its digest version.
#[derive(Clone, Copy)]
pub(crate) struct NewReservation<'a> {
    pub(crate) selection: Option<(DeliveryMethod, Option<&'a str>)>,
    pub(crate) defer_reason: Option<&'a str>,
    pub(crate) payload_digest_version: i64,
}

#[cfg(test)]
impl NewReservation<'_> {
    pub(crate) const PLAIN: Self = Self {
        selection: None,
        defer_reason: None,
        payload_digest_version: 1,
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
        let changed = conn.execute(
            "UPDATE member_input_operations SET state='in_progress',updated_at=?1,delivery_method=?2,delivery_reason=?3 WHERE input_id=?4 AND state='ready'",
            params![now_stamp(), method.as_str(), reason, input_id],
        )?;
        require(
            changed == 1,
            "member_input_operations.state",
            "input CAS failed",
        )?;
        get_input(conn, input_id)
    })
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
        let changed = conn.execute(
            "UPDATE member_input_operations SET state='ready',updated_at=?1,delivery_method=NULL,delivery_reason=NULL,deferred_at=?1,defer_reason=?2 WHERE input_id=?3 AND state='in_progress'",
            params![now, defer_reason, input_id],
        )?;
        require(
            changed == 1,
            "member_input_operations.state",
            "input CAS failed",
        )?;
        get_input(conn, input_id)
    })
}

/// The deferred inputs of a run whose first result is durable but whose result event is not, with
/// the stage that event reports. Recovery publishes these after the live writer is gone.
pub(crate) fn unpublished_delivery_results(
    conn: &Connection,
    run_id: i64,
) -> Result<Vec<(i64, &'static str)>, CafleetError> {
    let mut statement = conn.prepare(
        "SELECT input_id FROM member_input_operations WHERE run_id=?1 AND deferred_at IS NOT NULL AND input_id NOT IN (SELECT json_extract(payload_json,'$.input_id') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result') ORDER BY input_id",
    )?;
    let ids = statement
        .query_map([run_id], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut results = Vec::new();
    for input_id in ids {
        let delivery = get_input(conn, input_id)?
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
        results.push((input_id, stage));
    }
    Ok(results)
}

/// Settles every queued receipt of a run whose runner was lost, after ownership fencing has
/// established that its in-memory payload and writer are gone.
pub(crate) fn settle_lost_deferred(
    conn: &mut Connection,
    owner: &RunOwner,
) -> Result<(), CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
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
    })
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
        let changed = conn.execute(
            "UPDATE member_input_operations SET state='settled',disposition='not_submitted',updated_at=?1,delivery_method=NULL,delivery_reason=?2 WHERE input_id=?3 AND state=?4",
            params![now_stamp(), reason, input_id, expected.as_str()],
        )?;
        require(
            changed == 1,
            "member_input_operations.state",
            "input CAS failed",
        )?;
        get_input(conn, input_id)
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
        .ok_or_else(|| CafleetError::Value(format!("Input {input_id} not found")))?;
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

#[cfg(test)]
pub(crate) fn reserve_input(
    conn: &mut Connection,
    owner: &RunOwner,
    new: &NewInput,
) -> Result<InputOperation, CafleetError> {
    reserve_input_with_provenance(conn, owner, new, NewReservation::PLAIN).map(|reservation| {
        match reservation {
            InputReservation::Inserted(operation) | InputReservation::Existing(operation) => {
                operation
            }
        }
    })
}

pub(crate) fn reserve_input_with_provenance(
    conn: &mut Connection,
    owner: &RunOwner,
    new: &NewInput,
    reservation: NewReservation<'_>,
) -> Result<InputReservation, CafleetError> {
    require(
        new.kind != InputKind::PermissionResponse,
        "kind",
        "permission responses require typed audit reservation",
    )?;
    nonblank(&new.operation_key, "operation_key")?;
    hex(&new.payload_digest, 64, "payload_digest")?;
    owned_transaction(conn, owner, |conn, run| {
        if let Some(input) = find_input(conn, run.run_id, &new.operation_key)? {
            require(
                input.payload_digest == new.payload_digest
                    && input.kind == new.kind
                    && input.message_id == new.message_id,
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
        if new.kind == InputKind::Preview {
            let message_id = new
                .message_id
                .ok_or_else(|| invalid("message_id", "preview message required"))?;
            let message = super::super::messaging::get_message(conn, message_id)?;
            require(
                message.to_member_id == run.member_id
                    && message.from_member_id != run.member_id
                    && new.operation_key == format!("preview:{}:{message_id}", run.run_id),
                "message_id",
                "preview must address its run member",
            )?;
        } else {
            require(
                new.message_id.is_none(),
                "message_id",
                "explicit input has no message",
            )?;
        }
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
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,delivery_method,delivery_reason,deferred_at,defer_reason,payload_digest_version)
             VALUES(?1,?2,?3,?4,?5,?6,?6,?7,?8,?9,?10,?11,?12)",
            params![run.run_id,new.operation_key,new.kind.as_str(),new.payload_digest,
                if deferred.is_some() { "ready" } else { "in_progress" },now,new.message_id,
                selection.map(|(method, _)| method.as_str()),selection.and_then(|(_, reason)| reason),
                deferred.map(|(at, _)| at),deferred.map(|(_, reason)| reason),reservation.payload_digest_version],
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
            (InputState::Ready, InputState::InProgress)
                | (
                    InputState::InProgress,
                    InputState::Ready | InputState::Accepted | InputState::Uncertain
                )
        ),
        "member_input_operations.state",
        "invalid input transition",
    )?;
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.kind != InputKind::PermissionResponse || expected != InputState::Ready,
            "state",
            "permission response rearming requires serialized live validation",
        )?;
        require(
            input.state == expected,
            "member_input_operations.state",
            "input state changed",
        )?;
        let changed = if next == InputState::Ready {
            conn.execute(
                "UPDATE member_input_operations SET state=?1,updated_at=?2,delivery_method=NULL,delivery_reason=NULL WHERE input_id=?3 AND state=?4",
                params![next.as_str(),now_stamp(),input_id,expected.as_str()],
            )?
        } else {
            conn.execute(
                "UPDATE member_input_operations SET state=?1,updated_at=?2 WHERE input_id=?3 AND state=?4",
                params![next.as_str(),now_stamp(),input_id,expected.as_str()],
            )?
        };
        require(
            changed == 1,
            "member_input_operations.state",
            "input CAS failed",
        )?;
        get_input(conn, input_id)
    })
}

#[cfg(test)]
pub(crate) fn settle_input(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
    expected: InputState,
    disposition: InputDisposition,
    evidence_ref: &str,
) -> Result<InputOperation, CafleetError> {
    owned_transaction(conn, owner, |conn, _run| {
        settle_input_in_transaction(conn, owner, input_id, expected, disposition, evidence_ref)
    })
}

pub(crate) fn settle_input_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    input_id: i64,
    expected: InputState,
    disposition: InputDisposition,
    evidence_ref: &str,
) -> Result<InputOperation, CafleetError> {
    require(
        !conn.is_autocommit(),
        "transaction",
        "input settlement requires an active ownership transaction",
    )?;
    nonblank(evidence_ref, "evidence_ref")?;
    require(
        expected != InputState::Settled,
        "state",
        "settled operations retain their disposition",
    )?;
    let run = validate_owner(conn, owner)?;
    let input = owned_input(conn, &run, input_id)?;
    require(
        input.state == expected,
        "member_input_operations.state",
        "input state changed",
    )?;
    if input.kind == InputKind::Preview && disposition == InputDisposition::Submitted {
        stamp_preview(conn, &run, &input)?;
    }
    settle(conn, input_id, expected, disposition, Some(evidence_ref))
}

fn settle(
    conn: &Connection,
    input_id: i64,
    expected: InputState,
    disposition: InputDisposition,
    evidence_ref: Option<&str>,
) -> Result<InputOperation, CafleetError> {
    let changed = conn.execute(
        "UPDATE member_input_operations SET state='settled',disposition=?1,evidence_ref=?2,updated_at=?3
         WHERE input_id=?4 AND state=?5",
        params![disposition.as_str(),evidence_ref,now_stamp(),input_id,expected.as_str()],
    )?;
    require(
        changed == 1,
        "member_input_operations.state",
        "settlement CAS failed",
    )?;
    get_input(conn, input_id)
}

pub(crate) fn settle_preview(
    conn: &mut Connection,
    owner: &RunOwner,
    input_id: i64,
) -> Result<InputOperation, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.kind == InputKind::Preview,
            "kind",
            "preview receipt required",
        )?;
        if input.state == InputState::Settled {
            require(
                input.disposition == Some(InputDisposition::Submitted),
                "disposition",
                "preview was settled with another disposition",
            )?;
            return Ok(input);
        }
        require(
            input.state == InputState::Accepted,
            "state",
            "accepted receipt required before stamping",
        )?;
        stamp_preview(conn, run, &input)?;
        settle(
            conn,
            input_id,
            InputState::Accepted,
            InputDisposition::Submitted,
            None,
        )
    })
}

fn stamp_preview(
    conn: &Connection,
    run: &RunRecord,
    input: &InputOperation,
) -> Result<(), CafleetError> {
    let message_id = input
        .message_id
        .expect("a validated preview has its message");
    let message = super::super::messaging::get_message(conn, message_id)?;
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

#[cfg(test)]
mod tests {
    use crate::broker::members::test_support::register;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::{self as storage, *};

    #[test]
    fn input_payload_digest_hashes_the_key_sorted_payload() {
        use crate::runtime::runner::SubmitInput;
        let literal = "say \"hi\" λ";
        let preview = SubmitInput {
            operation_key: "preview:7:9".into(),
            input_kind: InputKind::Preview,
            message_id: Some(9),
            literal: literal.into(),
        };
        let prompt = SubmitInput {
            operation_key: "prompt:7".into(),
            input_kind: InputKind::Prompt,
            message_id: None,
            literal: literal.into(),
        };
        let quoted = serde_json::to_string(literal).unwrap();
        let expected = |kind: &str, message_id: &str| {
            crate::digest::sha256_hex(
                format!(
                    "{{\"kind\":\"{kind}\",\"literal\":{quoted},\"message_id\":{message_id},\"run_id\":7}}"
                )
                .as_bytes(),
            )
        };
        assert_eq!(input_payload_digest(7, &preview), expected("preview", "9"));
        assert_eq!(input_payload_digest(7, &prompt), expected("prompt", "null"));
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
            storage::reserve_input(&mut other, &owner, &request)
                .unwrap()
                .input_id,
            input.input_id
        );
        let settled = storage::settle_input(
            &mut other,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputDisposition::NotSubmitted,
            "audit:zero-byte-confirmation",
        )
        .unwrap();
        assert_eq!(settled.state, InputState::Settled);
        let retry = storage::reserve_input(&mut fixture.conn, &owner, &request).unwrap();
        assert_eq!(retry.input_id, input.input_id);
        assert_eq!(retry.disposition, Some(InputDisposition::NotSubmitted));
        assert_eq!(rows(&fixture.conn, "member_input_operations").len(), 1);
    }

    #[test]
    fn operation_key_rejects_a_changed_digest_kind_or_message() {
        for changed in ["digest", "kind", "message"] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.input(&owner, "same-key", InputKind::Prompt, None);
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.director,
                fixture.member,
                "preview",
            )
            .unwrap();
            let mut request = NewInput {
                operation_key: "same-key".into(),
                kind: InputKind::Prompt,
                payload_digest: "d".repeat(64),
                message_id: None,
            };
            match changed {
                "digest" => request.payload_digest = "e".repeat(64),
                "kind" => request.kind = InputKind::Exec,
                "message" => {
                    request.kind = InputKind::Preview;
                    request.message_id = Some(message.message_id);
                }
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                storage::reserve_input(&mut fixture.conn, &owner, &request).is_err(),
                "{changed}"
            );
            assert_eq!(durable_state(&fixture.conn), before);
        }
    }

    #[test]
    fn preview_receipt_requires_a_message_addressed_to_its_run_member() {
        for destination in ["missing", "director", "foreign"] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let recipient = if destination == "director" {
                fixture.director
            } else {
                register(&mut fixture.conn, fixture.fleet, "other", None)
            };
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.director,
                recipient,
                "other-message",
            )
            .unwrap();
            let request = NewInput {
                operation_key: format!("preview:{}:{}", owner.run_id, message.message_id),
                kind: InputKind::Preview,
                payload_digest: "d".repeat(64),
                message_id: if destination == "missing" {
                    None
                } else {
                    Some(message.message_id)
                },
            };
            assert!(
                storage::reserve_input(&mut fixture.conn, &owner, &request).is_err(),
                "{destination}"
            );
            assert!(rows(&fixture.conn, "member_input_operations").is_empty());
        }
    }

    #[test]
    fn accepted_preview_settlement_and_message_stamp_are_atomic() {
        for already_acked in [false, true] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let message = crate::broker::send_message(
                &mut fixture.conn,
                fixture.director,
                fixture.member,
                "held preview",
            )
            .unwrap();
            let input = fixture.input(
                &owner,
                &format!("preview:{}:{}", owner.run_id, message.message_id),
                InputKind::Preview,
                Some(message.message_id),
            );
            storage::record_input_state(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputState::Accepted,
            )
            .unwrap();
            if already_acked {
                crate::broker::ack_message(&mut fixture.conn, message.message_id).unwrap();
            }
            let settled =
                storage::settle_preview(&mut fixture.conn, &owner, input.input_id).unwrap();
            assert_eq!(settled.state, InputState::Settled);
            assert_eq!(settled.disposition, Some(InputDisposition::Submitted));
            let stamp: Option<String> = fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(stamp.is_some());
        }
    }

    #[test]
    fn failed_preview_stamp_retains_the_durable_accepted_receipt_for_a_later_invocation() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let message = crate::broker::send_message(
            &mut fixture.conn,
            fixture.director,
            fixture.member,
            "preview",
        )
        .unwrap();
        let input = fixture.input(
            &owner,
            &format!("preview:{}:{}", owner.run_id, message.message_id),
            InputKind::Preview,
            Some(message.message_id),
        );
        storage::record_input_state(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputState::Accepted,
        )
        .unwrap();
        fixture.conn.execute_batch("CREATE TRIGGER fail_stamp BEFORE UPDATE OF notified_at ON messages BEGIN SELECT RAISE(ABORT,'stamp failure'); END").unwrap();
        assert!(storage::settle_preview(&mut fixture.conn, &owner, input.input_id).is_err());
        let mut other = fixture.reopen();
        let durable = storage::find_input(&other, owner.run_id, &input.operation_key)
            .unwrap()
            .unwrap();
        assert_eq!(durable.state, InputState::Accepted);
        assert_eq!(
            other
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message.message_id],
                    |row| row.get::<_, Option<String>>(0)
                )
                .unwrap(),
            None
        );
        other.execute_batch("DROP TRIGGER fail_stamp").unwrap();
        storage::settle_preview(&mut other, &owner, input.input_id).unwrap();
        assert_eq!(rows(&other, "member_input_operations").len(), 1);
    }

    #[test]
    fn settlement_cas_rejects_stale_state_and_preserves_recorded_evidence() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let input = fixture.input(&owner, "exec:lost-stdout", InputKind::Exec, None);
        storage::record_input_state(
            &mut fixture.conn,
            &owner,
            input.input_id,
            InputState::InProgress,
            InputState::Uncertain,
        )
        .unwrap();
        assert!(
            storage::settle_input(
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
            storage::settle_input(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::Uncertain,
                InputDisposition::Submitted,
                " "
            )
            .is_err()
        );
        let settled = storage::settle_input(
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
            storage::settle_input(
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
    fn settled_receipt_history_can_exceed_the_unresolved_cache_bound() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        for index in 0..4097 {
            let input = fixture.input(&owner, &format!("settled:{index}"), InputKind::Prompt, None);
            storage::settle_input(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputDisposition::NotSubmitted,
                "audit:zero-byte",
            )
            .unwrap();
        }
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
            storage::reserve_input(
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
}
