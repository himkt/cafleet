use rusqlite::{Connection, OptionalExtension, params};

use super::*;
use crate::time::{format_utc, now_utc};

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
    })
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
    Ok(input)
}

pub fn get_input(conn: &Connection, input_id: i64) -> Result<InputOperation, CafleetError> {
    let input = conn
        .query_row(
            "SELECT * FROM member_input_operations WHERE input_id=?1",
            [input_id],
            map_input,
        )
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| CafleetError::Value(format!("Input {input_id} not found")))?;
    validate_input(conn, input)
}

pub fn find_input(
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
        .optional()
        .map_err(db_err)?;
    input.map(|input| validate_input(conn, input)).transpose()
}

pub(crate) enum InputReservation {
    Inserted(InputOperation),
    Existing(InputOperation),
}

pub fn reserve_input(
    conn: &mut Connection,
    owner: &RunOwner,
    new: &NewInput,
) -> Result<InputOperation, CafleetError> {
    reserve_input_with_provenance(conn, owner, new).map(|reservation| match reservation {
        InputReservation::Inserted(operation) | InputReservation::Existing(operation) => operation,
    })
}

pub(crate) fn reserve_input_with_provenance(
    conn: &mut Connection,
    owner: &RunOwner,
    new: &NewInput,
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
        let now = format_utc(now_utc());
        conn.execute(
            "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id)
             VALUES(?1,?2,?3,?4,'in_progress',?5,?5,?6)",
            params![run.run_id,new.operation_key,new.kind.as_str(),new.payload_digest,now,new.message_id],
        ).map_err(db_err)?;
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

pub fn record_input_state(
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
        let changed = conn.execute(
            "UPDATE member_input_operations SET state=?1,updated_at=?2 WHERE input_id=?3 AND state=?4",
            params![next.as_str(),format_utc(now_utc()),input_id,expected.as_str()],
        ).map_err(db_err)?;
        require(
            changed == 1,
            "member_input_operations.state",
            "input CAS failed",
        )?;
        get_input(conn, input_id)
    })
}

pub fn settle_input(
    conn: &mut Connection,
    owner: &RunOwner,
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
    owned_transaction(conn, owner, |conn, run| {
        let input = owned_input(conn, run, input_id)?;
        require(
            input.state == expected,
            "member_input_operations.state",
            "input state changed",
        )?;
        if input.kind == InputKind::Preview && disposition == InputDisposition::Submitted {
            stamp_preview(conn, run, &input)?;
        }
        settle(conn, input_id, expected, disposition, Some(evidence_ref))
    })
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
        params![disposition.as_str(),evidence_ref,format_utc(now_utc()),input_id,expected.as_str()],
    ).map_err(db_err)?;
    require(
        changed == 1,
        "member_input_operations.state",
        "settlement CAS failed",
    )?;
    get_input(conn, input_id)
}

pub fn settle_preview(
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
        params![format_utc(now_utc()), message_id],
    )
    .map_err(db_err)?;
    Ok(())
}

pub(super) fn require_capacity(conn: &Connection, run_id: i64) -> Result<(), CafleetError> {
    let unresolved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM member_input_operations WHERE run_id=?1 AND state IN ('in_progress','accepted','uncertain')",
        [run_id], |row| row.get(0),
    ).map_err(db_err)?;
    require(
        unresolved < 4096,
        "member_input_operations",
        "unresolved input limit reached",
    )?;
    Ok(())
}
