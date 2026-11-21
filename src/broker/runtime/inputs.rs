use crate::broker::runtime::*;
use crate::time::now_stamp;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;

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
        delivery_method: row.get("delivery_method")?,
        delivery_reason: row.get("delivery_reason")?,
        deferred_at: row.get("deferred_at")?,
        defer_reason: row.get("defer_reason")?,
        stop_cause: row.get("stop_cause")?,
    })
}

impl InputOperation {
    pub(crate) fn delivery(&self) -> DeliveryProjection {
        delivery_projection(self).expect("validated input receipts project their delivery")
    }
}

const FALLBACK_REASONS: [&str; 1] = ["submission_plan_unavailable"];

fn delivery_projection(input: &InputOperation) -> Result<DeliveryProjection, CafleetError> {
    let unknown = || {
        invalid(
            "member_input_operations.delivery_method",
            "unknown stored delivery combination",
        )
    };
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
    Ok(DeliveryProjection {
        status,
        method,
        reason,
    })
}

fn validate_input(
    conn: &Connection,
    input: InputOperation,
) -> Result<InputOperation, CafleetError> {
    get_run(conn, input.run_id)?;
    require(
        input.stop_cause.is_none()
            || matches!(input.state, InputState::Uncertain | InputState::Settled),
        "member_input_operations.stop_cause",
        "a stop cause requires an uncertain or settled input",
    )?;
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
            input.state == InputState::InProgress && input.deferred_at.is_none(),
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
        let delivery = input.delivery();
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

/// Settles `expected_input` at its current state inside the caller's ownership transaction.
pub(crate) fn settle_owned_input_in_transaction(
    conn: &Connection,
    owner: &RunOwner,
    expected_input: &InputOperation,
    disposition: InputDisposition,
    evidence_ref: &str,
) -> Result<InputOperation, CafleetError> {
    require(
        !conn.is_autocommit(),
        "transaction",
        "input settlement requires an active ownership transaction",
    )?;
    let run = validate_owner(conn, owner)?;
    settle_input_in_transaction(
        conn,
        &run,
        expected_input.input_id,
        expected_input.state,
        disposition,
        evidence_ref,
    )
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
        let input = fixture.input(&owner, "prompt:one", InputKind::Prompt);
        assert_eq!(input.state, InputState::InProgress);
        let request = NewInput {
            operation_key: "prompt:one".into(),
            kind: InputKind::Prompt,
            payload_digest: "d".repeat(64),
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
        fixture.input(&owner, "same-key", InputKind::Prompt);
        for changed in ["digest", "kind"] {
            let mut request = NewInput {
                operation_key: "same-key".into(),
                kind: InputKind::Prompt,
                payload_digest: "d".repeat(64),
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
        let input = fixture.input(&owner, "exec:lost-stdout", InputKind::Exec);
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
        let input = fixture.input(&owner, "prompt:uncertain", InputKind::Prompt);
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
        let delivery = read.delivery();
        assert_eq!(delivery.status, DeliveryStatus::NotSubmitted);
    }

    #[test]
    fn settled_receipt_history_can_exceed_the_unresolved_cache_bound() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.conn.execute("WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<4095) INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref) SELECT ?1,'settled:'||x,'prompt',?2,'settled',?3,?3,'not_submitted','audit:zero-byte' FROM n",params![owner.run_id,"d".repeat(64),NOW]).unwrap();
        let input = fixture.input(&owner, "settled:4096", InputKind::Prompt);
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
        let fault = crate::db::test_support::FaultPlan::Abort {
            trigger: "fail_input",
            on: "BEFORE INSERT ON member_input_operations",
            message: "injected input failure",
        };
        fault.arm(&mut fixture.conn);
        assert!(
            crate::broker::runtime::test_support::reserve_input(
                &mut fixture.conn,
                &owner,
                &NewInput {
                    operation_key: "reserved".into(),
                    kind: InputKind::Exec,
                    payload_digest: "d".repeat(64),
                }
            )
            .is_err()
        );
        assert!(rows(&fixture.reopen(), "member_input_operations").is_empty());
        fault.clear(&mut fixture.conn);
        let input = fixture.input(&owner, "reserved", InputKind::Exec);
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
            let input = fixture.input(owner, key, InputKind::Prompt);
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
        fn record_input_uncertain_persists_each_cause_across_reconnection() {
            const CAUSES: &[StopCause] = &[
                StopCause::InteractionCandidate,
                StopCause::InteractionOpen,
                StopCause::DeadlineExpired,
                StopCause::ShortWrite,
                StopCause::WriteFailed,
                StopCause::RunnerStopped,
            ];
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            for (index, &cause) in CAUSES.iter().enumerate() {
                let key = format!("exec:cause:{index}");
                let input = fixture.input(&owner, &key, InputKind::Exec);
                assert_eq!(input.stop_cause, None, "{cause:?}");
                let uncertain = storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    cause,
                )
                .unwrap_or_else(|error| panic!("{cause:?}: {error}"));
                assert_eq!(uncertain.state, InputState::Uncertain, "{cause:?}");
                assert_eq!(uncertain.stop_cause, Some(cause), "{cause:?}");
                assert_eq!(
                    uncertain.delivery().status,
                    DeliveryStatus::Uncertain,
                    "{cause:?}"
                );
                assert_eq!(
                    storage::get_input(&fixture.reopen(), input.input_id).unwrap(),
                    uncertain,
                    "{cause:?}"
                );
                let found = storage::find_input(&fixture.reopen(), owner.run_id, &key)
                    .unwrap()
                    .unwrap();
                assert_eq!(found.stop_cause, Some(cause), "{cause:?}");
            }
        }

        #[test]
        fn an_uncertain_transition_requires_an_owned_in_progress_receipt_and_changes_nothing() {
            #[derive(Clone, Copy, Debug)]
            enum Refused {
                PlainStateTransition,
                Ready,
                Accepted,
                AlreadyUncertain,
                SupersededOwner,
            }
            const CASES: &[(Refused, &str)] = &[
                (Refused::PlainStateTransition, "invalid input transition"),
                (Refused::Ready, "input state changed"),
                (Refused::Accepted, "input state changed"),
                (Refused::AlreadyUncertain, "input state changed"),
                (Refused::SupersededOwner, ""),
            ];
            for &(case, message) in CASES {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                let input = fixture.input(&owner, "prompt:refused", InputKind::Prompt);
                match case {
                    Refused::PlainStateTransition | Refused::SupersededOwner => {}
                    Refused::Ready => {
                        storage::requeue_zero_byte_immediate(
                            &mut fixture.conn,
                            &owner,
                            input.input_id,
                            "interaction_open",
                        )
                        .unwrap();
                    }
                    Refused::Accepted => {
                        storage::record_input_state(
                            &mut fixture.conn,
                            &owner,
                            input.input_id,
                            InputState::InProgress,
                            InputState::Accepted,
                        )
                        .unwrap();
                    }
                    Refused::AlreadyUncertain => {
                        storage::record_input_uncertain(
                            &mut fixture.conn,
                            &owner,
                            input.input_id,
                            StopCause::WriteFailed,
                        )
                        .unwrap();
                    }
                }
                if matches!(case, Refused::SupersededOwner) {
                    fixture.activate_next_epoch();
                }
                let before = durable_state(&fixture.conn);
                let result = if matches!(case, Refused::PlainStateTransition) {
                    storage::record_input_state(
                        &mut fixture.conn,
                        &owner,
                        input.input_id,
                        InputState::InProgress,
                        InputState::Uncertain,
                    )
                } else {
                    storage::record_input_uncertain(
                        &mut fixture.conn,
                        &owner,
                        input.input_id,
                        StopCause::ShortWrite,
                    )
                };
                let error = result.expect_err(&format!("{case:?}"));
                assert!(
                    error.message().contains(message),
                    "{case:?}: {}",
                    error.message()
                );
                assert_eq!(durable_state(&fixture.conn), before, "{case:?}");
                let stored = storage::get_input(&fixture.conn, input.input_id).unwrap();
                match case {
                    Refused::PlainStateTransition => {
                        assert_eq!(stored.state, InputState::InProgress, "{case:?}")
                    }
                    Refused::AlreadyUncertain => assert_eq!(
                        stored.stop_cause,
                        Some(StopCause::WriteFailed),
                        "a repeated transition keeps the first recorded cause"
                    ),
                    _ => {}
                }
            }
        }

        #[test]
        fn immediate_stopped_results_remain_unpublished_until_their_first_durable_event() {
            for kind in [InputKind::Prompt, InputKind::Exec] {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                let uncertain = fixture.input(&owner, "ordinary:immediate-uncertain", kind);
                storage::record_input_uncertain(
                    &mut fixture.conn,
                    &owner,
                    uncertain.input_id,
                    StopCause::WriteFailed,
                )
                .unwrap();
                let submitted = fixture.input(&owner, "ordinary:immediate-submitted", kind);
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
            let input = fixture.input(&owner, &format!("prompt:{index}"), InputKind::Prompt);
            fixture
                .conn
                .execute(
                    &format!("UPDATE member_input_operations SET {assignment} WHERE input_id=?1"),
                    [input.input_id],
                )
                .unwrap_or_else(|error| panic!("{assignment}: {error}"));
            let stored = storage::get_input(&fixture.conn, input.input_id)
                .unwrap_or_else(|error| panic!("{assignment}: {}", error.message()));
            let projected = stored.delivery();
            assert_eq!(projected.to_json(), delivery, "{assignment}");
            assert_eq!(stored.deferred_at.is_none(), !deferred, "{assignment}");
            assert_eq!(stored.defer_reason.is_none(), !deferred, "{assignment}");
        }
        let queued_with_method = fixture.input(&owner, "exec:invalid", InputKind::Exec);
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
    fn first_uncertain_ordinary_input_returns_the_runs_lowest_uncertain_prompt_or_exec() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        assert_eq!(
            storage::first_uncertain_ordinary_input(&fixture.conn, owner.run_id).unwrap(),
            None,
            "a run without input has no blocker"
        );
        let ready = fixture.input(&owner, "prompt:ready", InputKind::Prompt);
        storage::record_input_state(
            &mut fixture.conn,
            &owner,
            ready.input_id,
            InputState::InProgress,
            InputState::Ready,
        )
        .unwrap();
        let settled = fixture.input(&owner, "exec:settled", InputKind::Exec);
        storage::record_input_uncertain(
            &mut fixture.conn,
            &owner,
            settled.input_id,
            StopCause::WriteFailed,
        )
        .unwrap();
        storage::test_support::settle_input(
            &mut fixture.conn,
            &owner,
            settled.input_id,
            InputState::Uncertain,
            InputDisposition::NotSubmitted,
            "audit:pane-shows-no-input",
        )
        .unwrap();
        assert_eq!(
            storage::first_uncertain_ordinary_input(&fixture.conn, owner.run_id).unwrap(),
            None,
            "ready and settled inputs are no blocker"
        );
        let mut uncertain = Vec::new();
        for (key, kind) in [
            ("exec:first-uncertain", InputKind::Exec),
            ("prompt:second-uncertain", InputKind::Prompt),
        ] {
            let input = fixture.input(&owner, key, kind);
            storage::record_input_uncertain(
                &mut fixture.conn,
                &owner,
                input.input_id,
                StopCause::WriteFailed,
            )
            .unwrap();
            uncertain.push(input.input_id);
        }
        assert_eq!(
            storage::first_uncertain_ordinary_input(&fixture.conn, owner.run_id).unwrap(),
            Some(uncertain[0])
        );
        storage::test_support::settle_input(
            &mut fixture.conn,
            &owner,
            uncertain[0],
            InputState::Uncertain,
            InputDisposition::Submitted,
            "audit:pane-shows-the-input",
        )
        .unwrap();
        assert_eq!(
            storage::first_uncertain_ordinary_input(&fixture.conn, owner.run_id).unwrap(),
            Some(uncertain[1]),
            "settling the first blocker exposes the next"
        );
        assert_eq!(
            storage::first_uncertain_ordinary_input(&fixture.conn, owner.run_id + 1000).unwrap(),
            None,
            "another run's inputs are never its blocker"
        );
    }
}
