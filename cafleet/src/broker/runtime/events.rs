use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::*;

fn map_interaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionRecord> {
    Ok(InteractionRecord {
        interaction_id: row.get("interaction_id")?,
        run_id: row.get("run_id")?,
        episode_seq: row.get("episode_seq")?,
        classification: row.get("classification")?,
        evidence_digest: row.get("evidence_digest")?,
        opened_at: row.get("opened_at")?,
        evidence_json: json_column(row, "evidence_json", false)?,
        closed_at: row.get("closed_at")?,
        close_reason: row.get("close_reason")?,
    })
}

pub(super) fn interaction(conn: &Connection, id: i64) -> Result<InteractionRecord, CafleetError> {
    let interaction = conn
        .query_row(
            "SELECT * FROM member_interactions WHERE interaction_id=?1",
            [id],
            map_interaction,
        )
        .map_err(db_err)?;
    require(
        interaction.episode_seq > 0
            && interaction.closed_at.is_some() == interaction.close_reason.is_some(),
        "member_interactions",
        "invalid episode or closure",
    )?;
    evidence::validate(&interaction.evidence_json)?;
    require(
        interaction.evidence_json["evidence_digest"].as_str()
            == Some(interaction.evidence_digest.as_str()),
        "member_interactions.evidence_digest",
        "stored evidence digest mismatch",
    )?;
    Ok(interaction)
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeEventRecord> {
    Ok(RuntimeEventRecord {
        event_id: row.get("event_id")?,
        run_id: row.get("run_id")?,
        event_seq: row.get("event_seq")?,
        interaction_id: row.get("interaction_id")?,
        kind: row.get("kind")?,
        observed_at: row.get("observed_at")?,
        committed_at: row.get("committed_at")?,
        payload_json: json_column(row, "payload_json", false)?,
        notification_state: row.get("notification_state")?,
        message_id: row.get("message_id")?,
        archive_reason: row.get("archive_reason")?,
    })
}

fn source(kind: RuntimeEventKind) -> &'static str {
    match kind {
        RuntimeEventKind::InteractionOpened
        | RuntimeEventKind::InteractionObscured
        | RuntimeEventKind::InteractionClosed
        | RuntimeEventKind::PermissionDeniedObserved
        | RuntimeEventKind::AutomaticRefusalObserved
        | RuntimeEventKind::AutomaticReviewUnavailable => "runner_screen",
        _ => "runner_lifecycle",
    }
}

pub fn get_event(conn: &Connection, event_id: i64) -> Result<RuntimeEventView, CafleetError> {
    let event = conn
        .query_row(
            "SELECT * FROM member_runtime_events WHERE event_id=?1",
            [event_id],
            map_event,
        )
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| CafleetError::Value(format!("Event {event_id} not found")))?;
    let run = runtime_snapshot(conn, event.run_id)?;
    historical_ownership(conn, &run.run)?;
    let interaction = event
        .interaction_id
        .map(|id| interaction(conn, id))
        .transpose()?;
    if let Some(interaction) = &interaction {
        require(
            interaction.run_id == event.run_id,
            "member_runtime_events.interaction_id",
            "cross-run interaction",
        )?;
    }
    let payload = &event.payload_json;
    let mut fields = vec![
        "schema_version",
        "event_id",
        "run_id",
        "event_seq",
        "member_id",
        "fleet_id",
        "director_member_id",
        "kind",
        "interaction_id",
        "classification",
        "observed_at",
        "source",
        "native_request_id",
        "decision_outcome",
        "evidence",
    ];
    let version = payload.get("schema_version").and_then(Value::as_u64);
    require(
        matches!(version, Some(1 | 2)),
        "schema_version",
        "unsupported event schema",
    )?;
    if event.kind == RuntimeEventKind::InteractionClosed {
        fields.push("close_reason");
    }
    if source(event.kind) == "runner_lifecycle" {
        fields.push("cause");
    }
    if version == Some(2) {
        fields.push("director_action");
    }
    require(
        payload
            .as_object()
            .expect("decoded event object")
            .keys()
            .all(|key| fields.contains(&key.as_str()))
            && payload.as_object().expect("event object").len() == fields.len(),
        "payload_json",
        "unknown public event field",
    )?;
    require(
        event.event_seq > 0,
        "payload_json",
        "invalid schema or sequence",
    )?;
    for (key, expected) in [
        ("event_id", event.event_id),
        ("run_id", event.run_id),
        ("event_seq", event.event_seq),
        ("member_id", run.run.member_id),
        ("fleet_id", run.run.fleet_id),
        ("director_member_id", run.run.director_member_id),
    ] {
        require(
            payload.get(key).and_then(Value::as_i64) == Some(expected),
            "payload_json",
            "public identity mismatch",
        )?;
    }
    require(
        payload["interaction_id"] == json!(event.interaction_id)
            && payload["classification"]
                == json!(
                    interaction
                        .as_ref()
                        .map(|episode| episode.classification.as_str())
                )
            && payload["kind"] == event.kind.as_str()
            && payload["observed_at"] == event.observed_at
            && payload["source"] == source(event.kind)
            && payload["decision_outcome"] == "unknown",
        "payload_json",
        "event/interaction identity mismatch",
    )?;
    evidence::validate(
        payload
            .get("evidence")
            .ok_or_else(|| invalid("payload_json.evidence", "evidence required"))?,
    )?;
    require(
        payload.get("native_request_id") == payload["evidence"].get("native_request_id"),
        "payload_json.native_request_id",
        "native request identity must match post-policy evidence",
    )?;
    if version == Some(2) {
        require(
            payload.get("director_action")
                == Some(&director_action(
                    &run.run,
                    event.kind,
                    interaction.as_ref().map(|episode| episode.classification),
                    event.event_id,
                    event.interaction_id,
                    &payload["evidence"],
                )?),
            "director_action",
            "invalid bound review action",
        )?;
    }
    let serialized_payload = payload.to_string();
    require(
        serialized_payload.len() <= 16 * 1024,
        "payload_json",
        "payload exceeds bound",
    )?;
    match event.notification_state {
        NotificationState::Inbox => {
            let message_id = event
                .message_id
                .ok_or_else(|| invalid("message_id", "inbox event requires message"))?;
            require(
                event.archive_reason.is_none(),
                "archive_reason",
                "inbox event cannot be archived",
            )?;
            let message = super::super::messaging::get_message(conn, message_id)?;
            require(
                message.from_member_id == run.run.member_id
                    && message.to_member_id == run.run.director_member_id
                    && message.text == serialized_payload,
                "message_id",
                "event/message association mismatch",
            )?;
        }
        NotificationState::Archived => require(
            event.message_id.is_none() && event.archive_reason.is_some(),
            "notification_state",
            "archived event requires only an archive reason",
        )?,
    }
    let view = RuntimeEventView {
        event,
        run,
        interaction,
    };
    validate_transition(&view.run.run, &original(&view)?)?;
    Ok(view)
}

pub(crate) fn original(view: &RuntimeEventView) -> Result<ObservedTransition, CafleetError> {
    let payload = &view.event.payload_json;
    let close_reason = payload
        .get("close_reason")
        .filter(|value| !value.is_null())
        .map(|value| {
            InteractionCloseReason::try_from(
                value
                    .as_str()
                    .ok_or_else(|| invalid("close_reason", "string required"))?,
            )
        })
        .transpose()?;
    Ok(ObservedTransition {
        run_id: view.event.run_id,
        event_seq: view.event.event_seq,
        episode_seq: view
            .interaction
            .as_ref()
            .map(|interaction| interaction.episode_seq),
        kind: view.event.kind,
        classification: view
            .interaction
            .as_ref()
            .map(|interaction| interaction.classification),
        observed_at: view.event.observed_at.clone(),
        evidence_json: payload["evidence"].clone(),
        close_reason,
        cause_json: payload
            .get("cause")
            .filter(|value| !value.is_null())
            .cloned(),
    })
}

pub(crate) fn validate_transition(
    run: &RunRecord,
    transition: &ObservedTransition,
) -> Result<(), CafleetError> {
    require(
        transition.run_id == run.run_id
            && transition.event_seq > 0
            && transition.event_seq <= run.event_seq_reserved_through,
        "event_seq",
        "run or reserved sequence mismatch",
    )?;
    chrono::DateTime::parse_from_rfc3339(&transition.observed_at)
        .map_err(|_| invalid("observed_at", "RFC 3339 timestamp required"))?;
    evidence::validate(&transition.evidence_json)?;
    let policy = &transition.evidence_json["evidence_policy"];
    require(
        run.event_evidence == EventEvidence::Redacted || policy == "metadata",
        "event_evidence",
        "run policy forbids screen evidence",
    )?;
    require(
        transition.evidence_json["backend"] == run.backend,
        "evidence.backend",
        "backend mismatch",
    )?;
    require(
        transition.episode_seq.is_some() == transition.classification.is_some(),
        "episode_seq",
        "episode and classification must be paired",
    )?;
    if let Some(sequence) = transition.episode_seq {
        require(sequence > 0, "episode_seq", "positive episode required")?;
    }
    let interactive = matches!(
        transition.kind,
        RuntimeEventKind::InteractionOpened
            | RuntimeEventKind::InteractionObscured
            | RuntimeEventKind::InteractionClosed
    );
    require(
        !interactive || transition.episode_seq.is_some(),
        "episode_seq",
        "interactive transition requires episode",
    )?;
    require(
        (transition.kind == RuntimeEventKind::InteractionClosed)
            == transition.close_reason.is_some(),
        "close_reason",
        "closure reason must match closure event",
    )?;
    if source(transition.kind) == "runner_lifecycle" {
        require(
            transition.episode_seq.is_none() && transition.classification.is_none(),
            "episode_seq",
            "run-level events have no interaction",
        )?;
        validate_cause(transition)?;
    } else {
        require(
            transition.cause_json.is_none(),
            "cause_json",
            "screen events carry only post-policy screen evidence",
        )?;
    }
    Ok(())
}

fn validate_cause(transition: &ObservedTransition) -> Result<(), CafleetError> {
    let cause = transition
        .cause_json
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("cause_json", "typed lifecycle cause required"))?;
    let kind = cause
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("cause_json.kind", "typed cause kind required"))?;
    let kind = RuntimeCauseKind::try_from(kind)?;
    let compatible = matches!(
        (transition.kind, kind),
        (
            RuntimeEventKind::CoverageUnknown,
            RuntimeCauseKind::CoverageUnknown
        ) | (RuntimeEventKind::RunnerExited, RuntimeCauseKind::NativeExit)
            | (
                RuntimeEventKind::RunnerLost,
                RuntimeCauseKind::MonitorLost | RuntimeCauseKind::RuntimePathMissing
            )
            | (
                RuntimeEventKind::RunnerDegraded,
                RuntimeCauseKind::StoragePressure
                    | RuntimeCauseKind::StorageRecovered
                    | RuntimeCauseKind::ObserverError
            )
    );
    require(
        compatible,
        "cause_json.kind",
        "cause does not match lifecycle event",
    )?;
    let allowed: &[&str] = match kind {
        RuntimeCauseKind::NativeExit => &["kind", "exit_code", "exit_signal"],
        RuntimeCauseKind::StoragePressure
        | RuntimeCauseKind::StorageRecovered
        | RuntimeCauseKind::ObserverError => {
            &["kind", "gap_count", "gap_started_at", "gap_ended_at"]
        }
        RuntimeCauseKind::CoverageUnknown
        | RuntimeCauseKind::MonitorLost
        | RuntimeCauseKind::RuntimePathMissing => &["kind"],
    };
    require(
        cause.keys().all(|key| allowed.contains(&key.as_str())),
        "cause_json",
        "unknown lifecycle cause field",
    )?;
    for key in ["exit_code", "exit_signal", "gap_count"] {
        if let Some(value) = cause.get(key) {
            require(
                value.is_null() || value.as_i64().is_some(),
                key,
                "nullable integer required",
            )?;
        }
    }
    for key in ["gap_started_at", "gap_ended_at"] {
        if let Some(value) = cause.get(key).filter(|value| !value.is_null()) {
            let value = value
                .as_str()
                .ok_or_else(|| invalid(key, "timestamp string required"))?;
            chrono::DateTime::parse_from_rfc3339(value)
                .map_err(|_| invalid(key, "RFC 3339 timestamp required"))?;
        }
    }
    if kind == RuntimeCauseKind::NativeExit {
        require(
            cause.get("exit_code").and_then(Value::as_i64).is_some()
                || cause.get("exit_signal").and_then(Value::as_i64).is_some(),
            "cause_json",
            "actual native exit status required",
        )?;
    }
    Ok(())
}

fn apply_interaction(
    conn: &Connection,
    transition: &ObservedTransition,
) -> Result<Option<i64>, CafleetError> {
    let Some(sequence) = transition.episode_seq else {
        return Ok(None);
    };
    let class = transition
        .classification
        .expect("validated episode has classification");
    if transition.kind == RuntimeEventKind::InteractionOpened {
        let latest: Option<i64> = conn
            .query_row(
                "SELECT MAX(episode_seq) FROM member_interactions WHERE run_id=?1",
                [transition.run_id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        require(
            latest.is_none_or(|latest| sequence > latest),
            "episode_seq",
            "episode sequence is not increasing",
        )?;
        conn.execute(
            "INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![transition.run_id,sequence,class.as_str(),transition.evidence_json["evidence_digest"].as_str().expect("validated digest"),
                transition.observed_at,transition.evidence_json.to_string()],
        ).map_err(db_err)?;
        return Ok(Some(conn.last_insert_rowid()));
    }
    let id: i64 = conn
        .query_row(
            "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND episode_seq=?2",
            params![transition.run_id, sequence],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    let current = interaction(conn, id)?;
    require(
        current.classification == class && current.closed_at.is_none(),
        "interaction_id",
        "episode is closed or classification changed",
    )?;
    if let Some(reason) = transition.close_reason {
        conn.execute(
            "UPDATE member_interactions SET closed_at=?1,close_reason=?2 WHERE interaction_id=?3",
            params![transition.observed_at, reason.as_str(), id],
        )
        .map_err(db_err)?;
    }
    Ok(Some(id))
}

fn payload(
    run: &RunRecord,
    transition: &ObservedTransition,
    event_id: i64,
    interaction_id: Option<i64>,
) -> Result<Value, CafleetError> {
    let mut payload = json!({
        "schema_version":2,"event_id":event_id,"run_id":run.run_id,"event_seq":transition.event_seq,
        "member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,
        "kind":transition.kind.as_str(),"interaction_id":interaction_id,
        "classification":transition.classification.map(|class| class.as_str()),
        "observed_at":transition.observed_at,"source":source(transition.kind),
        "native_request_id":transition.evidence_json["native_request_id"],
        "decision_outcome":"unknown","evidence":transition.evidence_json,
    });
    if let Some(reason) = transition.close_reason {
        payload["close_reason"] = json!(reason.as_str());
    }
    if let Some(cause) = &transition.cause_json {
        payload["cause"] = cause.clone();
    }
    payload["director_action"] = director_action(
        run,
        transition.kind,
        transition.classification,
        event_id,
        interaction_id,
        &transition.evidence_json,
    )?;
    Ok(payload)
}

pub fn persist_transition(
    conn: &mut Connection,
    owner: &RunOwner,
    transition: &ObservedTransition,
    mode: PersistenceMode,
) -> Result<RuntimeEventRecord, CafleetError> {
    persist_transition_counted(conn, owner, transition, mode).map(|(event, _)| event)
}

pub(crate) fn persist_transition_counted(
    conn: &mut Connection,
    owner: &RunOwner,
    transition: &ObservedTransition,
    mode: PersistenceMode,
) -> Result<(RuntimeEventRecord, bool), CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        persist_transition_in_transaction(conn, run, transition, mode)
    })
}

pub(crate) fn persist_transition_in_transaction(
    conn: &Connection,
    run: &RunRecord,
    transition: &ObservedTransition,
    mode: PersistenceMode,
) -> Result<(RuntimeEventRecord, bool), CafleetError> {
    validate_transition(run, transition)?;
    require(
        mode != PersistenceMode::Recovery || run.owner_kind == OwnerKind::Reconciler,
        "owner_kind",
        "recovery requires reconciler ownership",
    )?;
    let previous: Option<i64> = conn
        .query_row(
            "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2",
            params![run.run_id, transition.event_seq],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if let Some(id) = previous {
        let existing = get_event(conn, id)?;
        require(
            original(&existing)? == *transition,
            "event_seq",
            "conflicting original transition",
        )?;
        return Ok((existing.event, false));
    }
    require(
        mode == PersistenceMode::Recovery || !run.status.terminal(),
        "member_runs.status",
        "terminal runs accept only committed retries or owned recovery",
    )?;
    require(
        transition.kind != RuntimeEventKind::RunnerLost || mode == PersistenceMode::Recovery,
        "kind",
        "observer loss requires explicit recovery ownership",
    )?;
    let archive_reason = historical_ownership(conn, run)?;
    require(
        mode == PersistenceMode::Recovery
            || (archive_reason.is_none() && run.owner_kind == OwnerKind::Runner),
        "notification_state",
        "live notification requires active runner ownership",
    )?;
    if archive_reason.is_none() {
        super::super::fleets::require_live_fleet(conn, run.fleet_id)?;
    }
    let latest: Option<i64> = conn
        .query_row(
            "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
            [run.run_id],
            |row| row.get(0),
        )
        .map_err(db_err)?;
    require(
        latest.is_none_or(|latest| transition.event_seq > latest),
        "event_seq",
        "event sequence is not increasing",
    )?;
    let interaction_id = apply_interaction(conn, transition)?;
    let message_id = if archive_reason.is_none() {
        Some(
            super::super::messaging::insert_message_validated(
                conn,
                run.member_id,
                run.director_member_id,
                "{}",
            )?
            .message_id,
        )
    } else {
        None
    };
    let notification_state = if archive_reason.is_some() {
        NotificationState::Archived
    } else {
        NotificationState::Inbox
    };
    conn.execute(
            "INSERT INTO member_runtime_events(run_id,event_seq,interaction_id,kind,observed_at,committed_at,payload_json,notification_state,message_id,archive_reason)
             VALUES(?1,?2,?3,?4,?5,?6,'{}',?7,?8,?9)",
            params![run.run_id,transition.event_seq,interaction_id,transition.kind.as_str(),transition.observed_at,
                format_utc(now_utc()),notification_state.as_str(),message_id,archive_reason.map(|reason| reason.as_str())],
        ).map_err(db_err)?;
    let event_id = conn.last_insert_rowid();
    let payload = payload(run, transition, event_id, interaction_id)?.to_string();
    require(
        payload.len() <= 16 * 1024,
        "payload_json",
        "complete payload exceeds bound",
    )?;
    conn.execute(
        "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
        params![payload, event_id],
    )
    .map_err(db_err)?;
    if let Some(message_id) = message_id {
        conn.execute(
            "UPDATE messages SET text=?1 WHERE message_id=?2",
            params![payload, message_id],
        )
        .map_err(db_err)?;
    }
    if transition.kind == RuntimeEventKind::RunnerExited {
        let cause = transition
            .cause_json
            .as_ref()
            .ok_or_else(|| invalid("cause_json", "native exit evidence required"))?;
        let exit_code = cause
            .get("exit_code")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| invalid("exit_code", "integer required"))
            })
            .transpose()?;
        let exit_signal = cause
            .get("exit_signal")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_i64()
                    .ok_or_else(|| invalid("exit_signal", "integer required"))
            })
            .transpose()?;
        require(
            exit_code.is_some() || exit_signal.is_some(),
            "cause_json",
            "actual native status required",
        )?;
        conn.execute("UPDATE member_runs SET status='exited',ended_at=?1,exit_code=?2,exit_signal=?3 WHERE run_id=?4",
                params![transition.observed_at,exit_code,exit_signal,run.run_id]).map_err(db_err)?;
    }
    get_event(conn, event_id).map(|view| (view.event, true))
}

const REVIEW_INSTRUCTION: &str = "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward.";
const MULTI_ROW_INSTRUCTION: &str = "The operation is shown as its displayed rows, where a row break may be a wrap or a line break in the command, so confirm the command against the excerpt before deciding.";
fn review_instruction(evidence: &Value) -> String {
    match evidence["operation_rows"].as_u64() {
        Some(rows) if rows > 1 => format!("{REVIEW_INSTRUCTION} {MULTI_ROW_INSTRUCTION}"),
        _ => REVIEW_INSTRUCTION.to_owned(),
    }
}
fn director_action(
    run: &RunRecord,
    kind: RuntimeEventKind,
    classification: Option<InteractionClass>,
    event_id: i64,
    interaction_id: Option<i64>,
    evidence: &Value,
) -> Result<Value, CafleetError> {
    if kind != RuntimeEventKind::InteractionOpened
        || classification != Some(InteractionClass::PermissionRequest)
    {
        return Ok(Value::Null);
    }
    let interaction_id = interaction_id.ok_or_else(|| {
        invalid(
            "interaction_id",
            "permission opening requires an interaction",
        )
    })?;
    require(
        event_id > 0 && interaction_id > 0 && run.director_member_id > 0,
        "director_action",
        "allocated positive action identities required",
    )?;
    Ok(
        json!({"kind":"review_permission_request","instruction":review_instruction(evidence),
        "inspect_argv":["cafleet","permission","inspect",interaction_id.to_string(),"--event-id",event_id.to_string(),"--from-member-id",run.director_member_id.to_string(),"--json"]}),
    )
}

pub fn update_interaction_evidence(
    conn: &mut Connection,
    owner: &RunOwner,
    update: &InteractionEvidenceUpdate,
    after_event_seq: i64,
    deadline: std::time::Instant,
) -> Result<InteractionRecord, CafleetError> {
    crate::runtime::runner::with_database_deadline(conn, deadline, |conn| {
        owned_transaction(conn, owner, |conn, run| {
            require(
                update.run_id == run.run_id && update.episode_seq > 0,
                "episode_seq",
                "matching run and positive episode required",
            )?;
            evidence::validate(&update.evidence_json)?;
            let value = &update.evidence_json;
            require(
                value["screen_seq"].as_u64() == Some(update.output_seq)
                    && value["backend"] == run.backend
                    && value["profile_version"] == json!(run.profile_version)
                    && value["native_version"] == json!(run.native_version)
                    && (run.event_evidence == EventEvidence::Redacted
                        || value["evidence_policy"] == "metadata"),
                "evidence_json",
                "refresh evidence does not match current run/profile/policy/sequence",
            )?;
            let maximum: Option<i64> = conn
                .query_row(
                    "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
                    [run.run_id],
                    |row| row.get(0),
                )
                .map_err(db_err)?;
            require(
                after_event_seq > 0 && maximum == Some(after_event_seq),
                "after_event_seq",
                "refresh barrier must match committed run maximum",
            )?;
            let id: i64 = conn.query_row("SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND episode_seq=?2",params![run.run_id,update.episode_seq],|row| row.get(0)).map_err(db_err)?;
            let current = interaction(conn, id)?;
            require(
                current.closed_at.is_none(),
                "interaction_id",
                "refresh requires an open current episode",
            )?;
            let opening_id:i64 = conn.query_row("SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND interaction_id=?2 AND kind='interaction_opened' AND event_seq<=?3",params![run.run_id,id,after_event_seq],|row| row.get(0)).map_err(db_err)?;
            get_event(conn, opening_id)?;
            let previous = current.evidence_json["screen_seq"]
                .as_u64()
                .expect("validated interaction sequence");
            require(
                update.output_seq >= previous,
                "screen_seq",
                "refresh output sequence regressed",
            )?;
            if update.output_seq == previous {
                require(
                    value == &current.evidence_json,
                    "screen_seq",
                    "conflicting refresh at the same output sequence",
                )?;
                return Ok(current);
            }
            let changed = conn.execute("UPDATE member_interactions SET evidence_json=?1,evidence_digest=?2 WHERE interaction_id=?3 AND closed_at IS NULL",params![value.to_string(),value["evidence_digest"].as_str().expect("validated digest"),id]).map_err(db_err)?;
            require(
                changed == 1,
                "interaction_id",
                "refresh lost its active episode",
            )?;
            interaction(conn, id)
        })
    })
}
