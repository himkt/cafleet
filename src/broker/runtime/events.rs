use crate::broker::runtime::*;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

const EVIDENCE_FIELDS: [&str; 4] = ["backend", "native_version", "rule_id", "screen_seq"];

/// The metadata evidence of an observation: its identity fields and their digest.
pub(crate) fn metadata_evidence(raw: &Value) -> Result<Value, CafleetError> {
    let raw = raw
        .as_object()
        .ok_or_else(|| invalid("evidence", "object required"))?;
    let mut fields = Map::new();
    for key in EVIDENCE_FIELDS {
        let value = raw
            .get(key)
            .ok_or_else(|| invalid(key, "required evidence field"))?;
        fields.insert(key.into(), value.clone());
    }
    let mut evidence = Value::Object(fields);
    evidence["evidence_digest"] = json!(evidence_digest(&evidence));
    validate_evidence(&evidence)?;
    Ok(evidence)
}

/// The metadata evidence of a run-scoped event that no screen form produced.
pub(crate) fn run_evidence(
    run: &RunRecord,
    rule_id: &str,
    screen_seq: u64,
) -> Result<Value, CafleetError> {
    metadata_evidence(&json!({
        "backend":run.backend,"native_version":run.native_version,
        "rule_id":rule_id,"screen_seq":screen_seq,
    }))
}

fn evidence_digest(value: &Value) -> String {
    let mut value = value.clone();
    value
        .as_object_mut()
        .expect("evidence is an object")
        .remove("evidence_digest");
    crate::digest::sha256_hex(crate::digest::canonical_json(&value).as_bytes())
}

pub(crate) fn validate_evidence(value: &Value) -> Result<(), CafleetError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("evidence_json", "object required"))?;
    require(
        object.len() == EVIDENCE_FIELDS.len() + 1
            && EVIDENCE_FIELDS
                .iter()
                .chain(["evidence_digest"].iter())
                .all(|key| object.contains_key(*key)),
        "evidence_json",
        "metadata evidence has exactly its identity fields and digest",
    )?;
    for field in ["backend", "rule_id"] {
        require(
            object[field]
                .as_str()
                .is_some_and(|text| !text.is_empty() && text.len() <= 256),
            field,
            "bounded evidence identity required",
        )?;
    }
    require(
        object["native_version"].is_null()
            || object["native_version"]
                .as_str()
                .is_some_and(|text| text.len() <= 256),
        "native_version",
        "nullable bounded string required",
    )?;
    require(
        object["screen_seq"].as_u64().is_some(),
        "screen_seq",
        "nonnegative sequence required",
    )?;
    require(
        object["evidence_digest"].as_str() == Some(evidence_digest(value).as_str()),
        "evidence_digest",
        "evidence digest mismatch",
    )
}

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
    let interaction = conn.query_row(
        "SELECT * FROM member_interactions WHERE interaction_id=?1",
        [id],
        map_interaction,
    )?;
    validate_evidence(&interaction.evidence_json)?;
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
        payload_json: json_column(row, "payload_json", false)?,
        notification_state: row.get("notification_state")?,
        message_id: row.get("message_id")?,
        archive_reason: row.get("archive_reason")?,
    })
}

const PAYLOAD_LIMIT: usize = 16 * 1024;

pub(crate) fn latest_event_seq(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<i64>, CafleetError> {
    conn.query_row(
        "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
        [run_id],
        |row| row.get(0),
    )
    .map_err(CafleetError::from)
}

fn interaction_id_for_episode(
    conn: &Connection,
    run_id: i64,
    episode_seq: i64,
) -> Result<i64, CafleetError> {
    conn.query_row(
        "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND episode_seq=?2",
        params![run_id, episode_seq],
        |row| row.get(0),
    )
    .map_err(CafleetError::from)
}

fn source(kind: RuntimeEventKind) -> &'static str {
    match kind {
        RuntimeEventKind::InteractionOpened
        | RuntimeEventKind::InteractionObscured
        | RuntimeEventKind::InteractionClosed => "runner_screen",
        _ => "runner_lifecycle",
    }
}

pub(crate) fn get_event(
    conn: &Connection,
    event_id: i64,
) -> Result<RuntimeEventView, CafleetError> {
    let event = conn
        .query_row(
            "SELECT * FROM member_runtime_events WHERE event_id=?1",
            [event_id],
            map_event,
        )
        .optional()?
        .ok_or_else(|| CafleetError::App(format!("Event {event_id} not found")))?;
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
    let view = RuntimeEventView {
        event,
        run,
        interaction,
    };
    let transition = original(&view)?;
    validate_transition(&view.run.run, &transition)?;
    let event = &view.event;
    let delivery = matches!(
        event.kind,
        RuntimeEventKind::InputDeliveryResult | RuntimeEventKind::InputDeliveryStalled
    )
    .then(|| published_delivery(conn, event))
    .transpose()?;
    require(
        payload(
            &view.run.run,
            &transition,
            event.event_id,
            event.interaction_id,
            delivery.as_ref(),
        )? == event.payload_json,
        "payload_json",
        "stored payload does not match its event",
    )?;
    let serialized_payload = event.payload_json.to_string();
    require(
        serialized_payload.len() <= PAYLOAD_LIMIT,
        "payload_json",
        "payload exceeds bound",
    )?;
    if event.notification_state == NotificationState::Inbox {
        let message_id = event
            .message_id
            .expect("the notification-state CHECK pairs an inbox event with its message");
        let message = crate::broker::messaging::get_message(conn, message_id)?;
        require(
            message.from_member_id == view.run.run.member_id
                && message.to_member_id == view.run.run.director_member_id
                && message.text == serialized_payload,
            "message_id",
            "event/message association mismatch",
        )?;
    }
    Ok(view)
}

const DELIVERY_RESULT_STAGES: [&str; 4] = ["zero_bytes", "partial", "submitted", "unknown"];

fn result_delivery(
    conn: &Connection,
    run_id: i64,
    input_id: i64,
    field: &str,
) -> Result<(InputOperation, DeliveryProjection), CafleetError> {
    let input = get_input(conn, input_id)?;
    require(
        input.run_id == run_id && (input.deferred_at.is_some() || input.stop_cause.is_some()),
        field,
        "delivery results describe this run's deferred or stopped ordinary inputs",
    )?;
    let mut delivery = input
        .delivery()
        .ok_or_else(|| invalid(field, "delivery results describe ordinary inputs"))?;
    if input.deferred_at.is_none() && input.stop_cause.is_some() {
        delivery.status = DeliveryStatus::Uncertain;
    }
    Ok((input, delivery))
}

fn validate_result_stage(stage: &Value, field: &str) -> Result<(), CafleetError> {
    require(
        stage
            .as_str()
            .is_some_and(|stage| DELIVERY_RESULT_STAGES.contains(&stage)),
        field,
        "stage outside its domain",
    )
}

fn result_stop_cause(input: &InputOperation, status: DeliveryStatus) -> Option<StopCause> {
    input
        .stop_cause
        .filter(|_| status == DeliveryStatus::Uncertain)
}

/// The delivery fields a stored result publishes, checked against its input receipt: an uncertain
/// result keeps the method it published even after the receipt settles.
fn published_delivery(
    conn: &Connection,
    event: &RuntimeEventRecord,
) -> Result<DeliveryFields, CafleetError> {
    let payload = &event.payload_json;
    if event.kind == RuntimeEventKind::InputDeliveryStalled {
        return stalled_delivery(conn, &get_run(conn, event.run_id)?, payload, true);
    }
    if payload["input_id"].is_null() {
        return preview_delivery(conn, &get_run(conn, event.run_id)?, payload);
    }
    let input_id = payload["input_id"]
        .as_i64()
        .ok_or_else(|| invalid("payload_json.input_id", "input identity required"))?;
    let (input, delivery) = result_delivery(conn, event.run_id, input_id, "payload_json.input_id")?;
    let published: DeliveryProjection = serde_json::from_value(payload["delivery"].clone())
        .map_err(|_| invalid("payload_json.delivery", "delivery projection required"))?;
    let consistent = if published.status == DeliveryStatus::Uncertain {
        delivery.status != DeliveryStatus::Queued
            && delivery.status != DeliveryStatus::Held
            && delivery.method == published.method
            && delivery.reason == published.reason
    } else {
        matches!(
            published.status,
            DeliveryStatus::Submitted | DeliveryStatus::NotSubmitted
        ) && published == delivery
    };
    require(
        consistent,
        "payload_json.delivery",
        "delivery must match the input receipt's settled result",
    )?;
    validate_result_stage(&payload["stage"], "payload_json.stage")?;
    Ok(DeliveryFields {
        input_id: Some(input_id),
        message_id: None,
        cause: result_stop_cause(&input, published.status),
        action: Value::Null,
        waiting_since: None,
        delivery: Some(published),
        stage: payload["stage"].clone(),
    })
}

/// The fields of an uncertain preview's result, which has no input receipt: the message must be
/// addressed to the run's member, and the published delivery is uncertain with its method.
fn preview_delivery(
    conn: &Connection,
    run: &RunRecord,
    record: &Value,
) -> Result<DeliveryFields, CafleetError> {
    let message_id = record["message_id"]
        .as_i64()
        .ok_or_else(|| invalid("message_id", "an uncertain preview names its message"))?;
    let message = crate::broker::messaging::get_message(conn, message_id)?;
    require(
        message.to_member_id == run.member_id,
        "message_id",
        "an uncertain preview addresses the run's member",
    )?;
    let method = match record.get("delivery") {
        Some(delivery) => delivery["method"].clone(),
        None => record["method"].clone(),
    };
    let method = method
        .as_str()
        .and_then(|method| DeliveryMethod::try_from(method).ok())
        .ok_or_else(|| invalid("delivery.method", "an uncertain preview records its method"))?;
    validate_result_stage(&record["stage"], "stage")?;
    let cause = record["cause"]["kind"]
        .as_str()
        .and_then(|kind| StopCause::try_from(kind).ok())
        .ok_or_else(|| invalid("cause", "an uncertain preview records its stop cause"))?;
    Ok(DeliveryFields {
        input_id: None,
        message_id: Some(message_id),
        delivery: Some(DeliveryProjection {
            status: DeliveryStatus::Uncertain,
            method: Some(method),
            reason: None,
        }),
        stage: record["stage"].clone(),
        cause: Some(cause),
        action: Value::Null,
        waiting_since: None,
    })
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
        cause_json: if view.event.kind == RuntimeEventKind::InputDeliveryStalled {
            Some(
                json!({"kind":"input_delivery_stalled","input_id":payload["input_id"],
                "message_id":payload["message_id"],"waiting_since":payload["waiting_since"]}),
            )
        } else if view.event.kind == RuntimeEventKind::InputDeliveryResult {
            let mut cause = json!({"kind":"input_delivery_result",
                "input_id":payload["input_id"],"stage":payload["stage"]});
            if payload["input_id"].is_null() {
                cause["message_id"] = payload["message_id"].clone();
                cause["method"] = payload["delivery"]["method"].clone();
            }
            if let Some(diagnostic) = payload.get("cause") {
                cause["cause"] = diagnostic.clone();
            }
            Some(cause)
        } else {
            payload
                .get("cause")
                .filter(|value| !value.is_null())
                .cloned()
        },
    })
}

/// The runner's internal record of a deferred input's first result: its input, its stage and
/// the receipt's recorded uncertain cause, carried through the outbox as the transition's cause.
pub(crate) fn input_delivery_result_cause(
    input_id: i64,
    stage: &str,
    stop_cause: Option<StopCause>,
) -> Value {
    let mut cause = json!({"kind":"input_delivery_result","input_id":input_id,"stage":stage});
    if let Some(stop_cause) = stop_cause {
        cause["cause"] = stop_cause_payload(stop_cause);
    }
    cause
}

/// The runner's internal record of an uncertain preview's result, which has no input receipt:
/// its message, the delivery method it was typed with, its written stage and its stop cause.
pub(crate) fn preview_delivery_result_cause(
    message_id: i64,
    method: DeliveryMethod,
    stage: &str,
    stop_cause: StopCause,
) -> Value {
    json!({"kind":"input_delivery_result","input_id":null,"message_id":message_id,
        "method":method.as_str(),"stage":stage,"cause":stop_cause_payload(stop_cause)})
}

fn stop_cause_payload(cause: StopCause) -> Value {
    json!({"kind":cause.as_str()})
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
    rfc3339(&transition.observed_at, "observed_at")?;
    validate_evidence(&transition.evidence_json)?;
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
        if transition.kind == RuntimeEventKind::InputDeliveryResult {
            validate_delivery_cause(transition)?;
        } else if transition.kind == RuntimeEventKind::InputDeliveryStalled {
            validate_stalled_record(transition)?;
        } else {
            validate_cause(transition)?;
        }
    } else {
        require(
            transition.cause_json.is_none(),
            "cause_json",
            "screen events carry only post-policy screen evidence",
        )?;
    }
    Ok(())
}

fn validate_stalled_record(transition: &ObservedTransition) -> Result<(), CafleetError> {
    let record = transition
        .cause_json
        .as_ref()
        .filter(|record| record.as_object().is_some_and(|record| record.len() == 4))
        .ok_or_else(|| invalid("cause_json", "stalled delivery record required"))?;
    let named = |id: &Value| id.as_i64().is_some_and(|id| id > 0);
    require(
        record["kind"] == "input_delivery_stalled"
            && ((named(&record["input_id"]) && record["message_id"].is_null())
                || (record["input_id"].is_null() && named(&record["message_id"]))),
        "cause_json",
        "a stalled record names exactly one waiting input or message and its wait start",
    )?;
    rfc3339(
        record["waiting_since"]
            .as_str()
            .ok_or_else(|| invalid("waiting_since", "timestamp required"))?,
        "waiting_since",
    )
}

fn validate_delivery_cause(transition: &ObservedTransition) -> Result<(), CafleetError> {
    let cause = transition
        .cause_json
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("cause_json", "delivery result record required"))?;
    let preview = cause.get("input_id").is_some_and(Value::is_null);
    let keys: &[&str] = if preview {
        &["kind", "input_id", "message_id", "method", "stage", "cause"]
    } else {
        &["kind", "input_id", "stage", "cause"]
    };
    let identified = if preview {
        cause
            .get("message_id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id > 0)
            && cause
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|method| DeliveryMethod::try_from(method).is_ok())
            && cause.contains_key("cause")
    } else {
        cause
            .get("input_id")
            .and_then(Value::as_i64)
            .is_some_and(|id| id > 0)
    };
    require(
        cause.keys().all(|key| keys.contains(&key.as_str()))
            && cause.get("kind").and_then(Value::as_str) == Some("input_delivery_result")
            && identified
            && cause
                .get("stage")
                .and_then(Value::as_str)
                .is_some_and(|stage| DELIVERY_RESULT_STAGES.contains(&stage)),
        "cause_json",
        "a delivery result names its input or preview message and its stage",
    )?;
    if let Some(uncertain) = cause.get("cause") {
        let kind = uncertain
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.get("kind"))
            .and_then(Value::as_str);
        require(
            kind.is_some_and(|kind| StopCause::try_from(kind).is_ok()),
            "cause_json.cause.kind",
            "a delivery result cause is a recorded uncertain cause",
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
            rfc3339(value, key)?;
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
        let latest: Option<i64> = conn.query_row(
            "SELECT MAX(episode_seq) FROM member_interactions WHERE run_id=?1",
            [transition.run_id],
            |row| row.get(0),
        )?;
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
        )?;
        return Ok(Some(conn.last_insert_rowid()));
    }
    let id = interaction_id_for_episode(conn, transition.run_id, sequence)?;
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
        )?;
    }
    Ok(Some(id))
}

/// The public fields an input delivery result adds from its input receipt.
struct DeliveryFields {
    input_id: Option<i64>,
    message_id: Option<i64>,
    delivery: Option<DeliveryProjection>,
    stage: Value,
    cause: Option<StopCause>,
    action: Value,
    waiting_since: Option<String>,
}

fn action_database_args(conn: &Connection) -> Result<Vec<String>, CafleetError> {
    let path = conn.path().ok_or_else(|| {
        invalid(
            "database_url",
            "a delivery action requires its database path",
        )
    })?;
    let url = format!("sqlite:///{path}");
    Ok(if url == crate::config::default_database_url()? {
        Vec::new()
    } else {
        vec!["--database-url".into(), url]
    })
}

/// The fields of a stalled event: a queued ordinary head of this run, ready when first
/// published, or a message addressed to the run's member.
fn stalled_delivery(
    conn: &Connection,
    run: &RunRecord,
    record: &Value,
    published: bool,
) -> Result<DeliveryFields, CafleetError> {
    let input_id = record["input_id"].as_i64();
    let message_id = record["message_id"].as_i64();
    match (input_id, message_id) {
        (Some(input_id), None) => {
            let input = get_input(conn, input_id)?;
            require(
                input.run_id == run.run_id
                    && input.kind != InputKind::PermissionResponse
                    && input.deferred_at.is_some(),
                "input_id",
                "a stalled head is a queued ordinary input of this run",
            )?;
            require(
                published || input.state == InputState::Ready,
                "input_id",
                "a new stalled head must be ready",
            )?;
        }
        (None, Some(message_id)) => {
            let message = crate::broker::messaging::get_message(conn, message_id)?;
            require(
                message.to_member_id == run.member_id,
                "message_id",
                "a stalled preview addresses the run's member",
            )?;
        }
        _ => {
            return Err(invalid(
                "input_id",
                "a stalled event names exactly one waiting input or message",
            ));
        }
    }
    let waiting_since = record["waiting_since"]
        .as_str()
        .ok_or_else(|| invalid("waiting_since", "timestamp required"))?
        .to_owned();
    rfc3339(&waiting_since, "waiting_since")?;
    let action = json!({"kind":"review_stalled_delivery",
        "instruction":REVIEW_STALLED_DELIVERY_INSTRUCTION,
        "inspect_argv":member_capture_argv(conn, run.member_id)?});
    Ok(DeliveryFields {
        input_id,
        message_id,
        delivery: None,
        stage: Value::Null,
        cause: None,
        waiting_since: Some(waiting_since),
        action,
    })
}

fn member_capture_argv(conn: &Connection, member_id: i64) -> Result<Value, CafleetError> {
    let mut argv = vec![
        "cafleet".to_owned(),
        "member".into(),
        "capture".into(),
        member_id.to_string(),
        "--json".into(),
    ];
    argv.extend(action_database_args(conn)?);
    Ok(json!(argv))
}

const REVIEW_STALLED_DELIVERY_INSTRUCTION: &str = "Capture the member's pane. When the composer holds text, submit or clear it with member press keys the capture establishes, or ask the user to clear or submit it on the member's native surface.";

fn payload(
    run: &RunRecord,
    transition: &ObservedTransition,
    event_id: i64,
    interaction_id: Option<i64>,
    delivery: Option<&DeliveryFields>,
) -> Result<Value, CafleetError> {
    let mut payload = json!({
        "event_id":event_id,"run_id":run.run_id,"event_seq":transition.event_seq,
        "member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,
        "kind":transition.kind.as_str(),"interaction_id":interaction_id,
        "classification":transition.classification.map(|class| class.as_str()),
        "observed_at":transition.observed_at,"source":source(transition.kind),
        "decision_outcome":"unknown","evidence":transition.evidence_json,
    });
    if let Some(reason) = transition.close_reason {
        payload["close_reason"] = json!(reason.as_str());
    }
    if let Some(cause) = &transition.cause_json
        && !matches!(
            transition.kind,
            RuntimeEventKind::InputDeliveryResult | RuntimeEventKind::InputDeliveryStalled
        )
    {
        payload["cause"] = cause.clone();
    }
    if let Some(fields) = delivery {
        if let Some(cause) = fields.cause {
            payload["cause"] = stop_cause_payload(cause);
        }
        payload["input_id"] = json!(fields.input_id);
        payload["message_id"] = json!(fields.message_id);
        match (&fields.waiting_since, &fields.delivery) {
            (Some(waiting_since), None) => payload["waiting_since"] = json!(waiting_since),
            (None, Some(delivery)) => {
                payload["delivery"] = delivery.to_json();
                payload["stage"] = fields.stage.clone();
            }
            _ => {
                unreachable!("a delivery result has a projection and a stalled event a wait start")
            }
        }
    }
    payload["director_action"] = if let Some(fields) = delivery {
        fields.action.clone()
    } else {
        director_action(
            run,
            transition.kind,
            transition.classification,
            event_id,
            interaction_id,
        )?
    };
    Ok(payload)
}

pub(crate) fn persist_transition(
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
        .optional()?;
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
        crate::broker::fleets::require_live_fleet(conn, run.fleet_id)?;
    }
    let delivery_fields = if matches!(
        transition.kind,
        RuntimeEventKind::InputDeliveryResult | RuntimeEventKind::InputDeliveryStalled
    ) {
        let record = transition
            .cause_json
            .as_ref()
            .expect("validated delivery results carry their record");
        let published: Option<i64> = match record["input_id"].as_i64() {
            Some(input_id) => conn
                .query_row(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND kind=?2 AND json_extract(payload_json,'$.input_id')=?3",
                    params![run.run_id, transition.kind.as_str(), input_id],
                    |row| row.get(0),
                )
                .optional()?,
            None => conn
                .query_row(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND kind=?2 AND json_type(payload_json,'$.input_id')='null' AND json_extract(payload_json,'$.message_id')=?3",
                    params![run.run_id, transition.kind.as_str(), record["message_id"].as_i64()],
                    |row| row.get(0),
                )
                .optional()?,
        };
        if let Some(id) = published {
            return Ok((get_event(conn, id)?.event, false));
        }
        if transition.kind == RuntimeEventKind::InputDeliveryStalled {
            Some(stalled_delivery(conn, run, record, false)?)
        } else if let Some(input_id) = record["input_id"].as_i64() {
            let (input, delivery) = result_delivery(conn, run.run_id, input_id, "input_id")?;
            validate_result_stage(&record["stage"], "cause_json.stage")?;
            Some(DeliveryFields {
                input_id: Some(input_id),
                message_id: None,
                cause: result_stop_cause(&input, delivery.status),
                action: Value::Null,
                waiting_since: None,
                delivery: Some(delivery),
                stage: record["stage"].clone(),
            })
        } else {
            Some(preview_delivery(conn, run, record)?)
        }
    } else {
        None
    };
    let latest = latest_event_seq(conn, run.run_id)?;
    require(
        latest.is_none_or(|latest| transition.event_seq > latest),
        "event_seq",
        "event sequence is not increasing",
    )?;
    let interaction_id = apply_interaction(conn, transition)?;
    let (notification_state, message_id) = match archive_reason {
        Some(_) => (NotificationState::Archived, None),
        None => (
            NotificationState::Inbox,
            Some(
                crate::broker::messaging::insert_message_validated(
                    conn,
                    run.member_id,
                    run.director_member_id,
                    "{}",
                )?
                .message_id,
            ),
        ),
    };
    conn.execute(
            "INSERT INTO member_runtime_events(run_id,event_seq,interaction_id,kind,observed_at,payload_json,notification_state,message_id,archive_reason)
             VALUES(?1,?2,?3,?4,?5,'{}',?6,?7,?8)",
            params![run.run_id,transition.event_seq,interaction_id,transition.kind.as_str(),transition.observed_at,
                notification_state.as_str(),message_id,archive_reason.map(|reason| reason.as_str())],
        )?;
    let event_id = conn.last_insert_rowid();
    let payload = payload(
        run,
        transition,
        event_id,
        interaction_id,
        delivery_fields.as_ref(),
    )?
    .to_string();
    require(
        payload.len() <= PAYLOAD_LIMIT,
        "payload_json",
        "complete payload exceeds bound",
    )?;
    conn.execute(
        "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
        params![payload, event_id],
    )?;
    if let Some(message_id) = message_id {
        conn.execute(
            "UPDATE messages SET text=?1 WHERE message_id=?2",
            params![payload, message_id],
        )?;
    }
    if transition.kind == RuntimeEventKind::RunnerExited {
        let cause = transition
            .cause_json
            .as_ref()
            .expect("validate_cause requires a native exit cause for RunnerExited");
        let exit_code = cause.get("exit_code").and_then(Value::as_i64);
        conn.execute(
            "UPDATE member_runs SET status='exited',ended_at=?1,exit_code=?2 WHERE run_id=?3",
            params![transition.observed_at, exit_code, run.run_id],
        )?;
    }
    get_event(conn, event_id).map(|view| (view.event, true))
}

const REVIEW_INSTRUCTION: &str = "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward.";
const QUESTION_INSTRUCTION: &str = "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward.";
fn director_action(
    run: &RunRecord,
    kind: RuntimeEventKind,
    classification: Option<InteractionClass>,
    event_id: i64,
    interaction_id: Option<i64>,
) -> Result<Value, CafleetError> {
    let (action, instruction) = match (kind, classification) {
        (RuntimeEventKind::InteractionOpened, Some(InteractionClass::PermissionRequest)) => {
            ("review_permission_request", REVIEW_INSTRUCTION)
        }
        (RuntimeEventKind::InteractionOpened, Some(InteractionClass::Question)) => {
            ("review_question", QUESTION_INSTRUCTION)
        }
        _ => return Ok(Value::Null),
    };
    let interaction_id = interaction_id.ok_or_else(|| {
        invalid(
            "interaction_id",
            "an answerable opening requires an interaction",
        )
    })?;
    require(
        event_id > 0 && interaction_id > 0 && run.director_member_id > 0,
        "director_action",
        "allocated positive action identities required",
    )?;
    Ok(json!({"kind":action,"instruction":instruction,
        "inspect_argv":["cafleet","permission","inspect",interaction_id.to_string(),"--event-id",event_id.to_string(),"--from-member-id",run.director_member_id.to_string(),"--json"]}))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use serde_json::Value;
    use serde_json::json;

    use crate::broker::runtime as storage;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::*;

    use crate::broker::members::test_support::register;

    const REVIEW_INSTRUCTION: &str = "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward.";
    const MULTI_ROW_SENTENCE: &str = "The operation is shown as its displayed rows, where a row break may be a wrap or a line break in the command, so confirm the command against the excerpt before deciding.";
    mod versioned_events {
        use crate::broker::runtime::events::tests::*;

        fn action(fixture: &Fixture, event: &RuntimeEventRecord) -> Value {
            let instruction = match event.payload_json["evidence"]["operation_rows"].as_u64() {
                Some(rows) if rows > 1 => format!("{REVIEW_INSTRUCTION} {MULTI_ROW_SENTENCE}"),
                _ => REVIEW_INSTRUCTION.to_owned(),
            };
            json!({
                "kind":"review_permission_request",
                "instruction":instruction,
                "inspect_argv":["cafleet","permission","inspect",event.interaction_id.unwrap().to_string(),"--event-id",event.event_id.to_string(),"--from-member-id",fixture.director.to_string(),"--json"],
            })
        }

        fn assert_v2(event: &RuntimeEventRecord) {
            assert!(event.payload_json.get("director_action").is_some());
        }

        fn independent_v2(fixture: &mut Fixture, event: &RuntimeEventRecord) -> Value {
            let mut payload = event.payload_json.clone();
            payload["director_action"] = action(fixture, event);
            replace_public_payload(&mut fixture.conn, event, payload.clone());
            storage::get_event(&fixture.conn, event.event_id)
                .expect("valid independently constructed v2 action must be readable");
            payload
        }

        #[test]
        fn runtime_payload_v2_permission_opening_has_the_constant_bound_review_action() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            assert_v2(&event);
            assert_eq!(
                event.payload_json["director_action"],
                action(&fixture, &event)
            );
            let message =
                crate::broker::get_message(&fixture.conn, event.message_id.unwrap()).unwrap();
            assert_eq!(message.text, event.payload_json.to_string());
            assert_eq!(event.payload_json["decision_outcome"], "unknown");
            assert_eq!(event.payload_json["evidence"]["operation"], Value::Null);
        }

        #[test]
        fn runtime_payload_v2_other_interaction_classes_have_explicit_null_action() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open_as(&owner, InteractionClass::UnknownInteraction);
            assert_v2(&event);
            assert_eq!(event.payload_json["director_action"], Value::Null);
        }

        const QUESTION_INSTRUCTION: &str = "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward.";

        fn question_action(fixture: &Fixture, event: &RuntimeEventRecord) -> Value {
            json!({
                "kind":"review_question",
                "instruction":QUESTION_INSTRUCTION,
                "inspect_argv":["cafleet","permission","inspect",event.interaction_id.unwrap().to_string(),"--event-id",event.event_id.to_string(),"--from-member-id",fixture.director.to_string(),"--json"],
            })
        }

        #[test]
        fn runtime_payload_v2_question_opening_has_the_constant_bound_review_question_action() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open_as(&owner, InteractionClass::Question);
            assert_v2(&event);
            assert_eq!(
                event.payload_json["director_action"],
                question_action(&fixture, &event)
            );
            let message =
                crate::broker::get_message(&fixture.conn, event.message_id.unwrap()).unwrap();
            assert_eq!(message.text, event.payload_json.to_string());
            assert_eq!(
                storage::get_event(&fixture.conn, event.event_id)
                    .unwrap()
                    .event
                    .payload_json["director_action"],
                question_action(&fixture, &event)
            );
        }

        #[test]
        fn runtime_payload_v2_reader_rejects_a_forged_question_action() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open_as(&owner, InteractionClass::Question);
            let mut payload = event.payload_json.clone();
            payload["director_action"]["instruction"] = json!(REVIEW_INSTRUCTION);
            replace_public_payload(&mut fixture.conn, &event, payload);
            assert!(
                storage::get_event(&fixture.conn, event.event_id).is_err(),
                "a question action carrying the permission instruction is forged"
            );
        }

        #[test]
        fn runtime_payload_v2_reader_rejects_a_question_action_on_a_permission_opening() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            let mut payload = event.payload_json.clone();
            payload["director_action"] = question_action(&fixture, &event);
            replace_public_payload(&mut fixture.conn, &event, payload);
            assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
        }

        #[test]
        fn runtime_payload_v2_closure_has_null_action() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.open(&owner);
            let transitions = [closure(owner.run_id, 2, 1)];
            let events = transitions
                .iter()
                .map(|transition| {
                    storage::persist_transition(
                        &mut fixture.conn,
                        &owner,
                        transition,
                        PersistenceMode::Live,
                    )
                    .unwrap()
                    .0
                })
                .collect::<Vec<_>>();
            for event in events {
                assert_v2(&event);
                assert_eq!(event.payload_json["director_action"], Value::Null);
            }
        }

        #[test]
        fn runtime_payload_v2_lifecycle_events_have_null_action_and_preserve_typed_causes() {
            let mut events = Vec::new();
            for (kind, cause) in [
                (
                    RuntimeEventKind::RunnerDegraded,
                    json!({"kind":"storage_pressure","gap_count":1,"gap_started_at":NOW,"gap_ended_at":null}),
                ),
                (RuntimeEventKind::RunnerLost, json!({"kind":"monitor_lost"})),
            ] {
                let mut fixture = Fixture::new();
                let mut owner = fixture.activate();
                let mode = if kind == RuntimeEventKind::RunnerLost {
                    owner = fixture.recoverer();
                    PersistenceMode::Recovery
                } else {
                    PersistenceMode::Live
                };
                fixture.reserve(&owner);
                let transition = ObservedTransition {
                    kind,
                    cause_json: Some(cause),
                    ..native_exit(owner.run_id, 1)
                };
                events.push(
                    storage::persist_transition(&mut fixture.conn, &owner, &transition, mode)
                        .unwrap()
                        .0,
                );
            }
            for event in events {
                assert_v2(&event);
                assert_eq!(event.payload_json["director_action"], Value::Null);
                assert_eq!(event.payload_json["source"], "runner_lifecycle");
                assert!(!event.payload_json["cause"].is_null());
            }
        }

        #[test]
        fn runtime_payload_v2_reader_rejects_forged_action_and_matching_message() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            for changed in ["instruction", "missing"] {
                let mut payload = independent_v2(&mut fixture, &event);
                match changed {
                    "instruction" => {
                        payload["director_action"]["instruction"] =
                            json!("approve the quoted command")
                    }
                    "missing" => {
                        payload.as_object_mut().unwrap().remove("director_action");
                    }
                    _ => unreachable!(),
                }
                replace_public_payload(&mut fixture.conn, &event, payload);
                assert!(
                    storage::get_event(&fixture.conn, event.event_id).is_err(),
                    "{changed}"
                );
            }
        }
    }
    #[test]
    fn opening_commits_one_complete_event_interaction_and_director_message() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        let interaction = event.interaction_id.unwrap();
        let message_id = event.message_id.unwrap();
        let reader = fixture.reopen();
        let message = crate::broker::get_message(&reader, message_id).unwrap();
        let payload: Value = serde_json::from_str(&message.text).unwrap();
        assert_eq!(
            (message.from_member_id, message.to_member_id),
            (fixture.member, fixture.director)
        );
        assert_eq!(message.status, crate::broker::MessageStatus::InputRequired);
        assert_eq!(payload, event.payload_json);
        for (key, id) in [
            ("event_id", event.event_id),
            ("run_id", owner.run_id),
            ("interaction_id", interaction),
            ("member_id", fixture.member),
            ("fleet_id", fixture.fleet),
            ("director_member_id", fixture.director),
        ] {
            assert_eq!(payload[key], id, "{key}");
        }
        assert_eq!(payload["decision_outcome"], "unknown");
        assert_eq!(payload["source"], "runner_screen");
        let consumed = crate::broker::poll_messages(&reader, fixture.director).unwrap();
        assert_eq!(
            consumed,
            [crate::broker::MessageRecord {
                status: crate::broker::MessageStatus::Completed,
                status_timestamp: consumed[0].status_timestamp.clone(),
                ..message
            }]
        );
    }

    #[test]
    fn identical_observed_transition_is_idempotent_without_generated_ids_or_commit_time() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let transition = opening(owner.run_id, 1, 1, metadata());
        let first = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &transition,
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        let before = durable_state(&fixture.conn);
        let mut other = fixture.reopen();
        let (again, inserted) =
            storage::persist_transition(&mut other, &owner, &transition, PersistenceMode::Live)
                .unwrap();
        assert!(!inserted, "a replayed transition inserts nothing");
        assert_eq!(
            (again.event_id, again.message_id, again.interaction_id),
            (first.event_id, first.message_id, first.interaction_id)
        );
        assert_eq!(again.payload_json, first.payload_json);
        assert_eq!(durable_state(&other), before);
    }

    #[test]
    fn each_conflicting_original_transition_field_rejects_sequence_reuse() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let original = opening(owner.run_id, 1, 1, metadata());
        storage::persist_transition(&mut fixture.conn, &owner, &original, PersistenceMode::Live)
            .unwrap();
        for changed in ["time", "episode", "kind", "classification"] {
            let mut conflict = original.clone();
            match changed {
                "time" => conflict.observed_at = "2026-10-26T10:00:01.000000+00:00".into(),
                "episode" => conflict.episode_seq = Some(2),
                "kind" => conflict.kind = RuntimeEventKind::InteractionClosed,
                "classification" => conflict.classification = Some(InteractionClass::Question),
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &conflict,
                    PersistenceMode::Live
                )
                .is_err(),
                "{changed}"
            );
            assert_eq!(durable_state(&fixture.conn), before, "{changed}");
        }
    }

    #[test]
    fn failure_at_each_transaction_stage_rolls_back_opening_and_inbox() {
        for target in [
            "BEFORE INSERT ON member_interactions",
            "BEFORE UPDATE OF text ON messages",
            "commit",
        ] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            if target == "commit" {
                use rusqlite::hooks::AuthAction;
                use rusqlite::hooks::AuthContext;
                use rusqlite::hooks::Authorization;
                use rusqlite::hooks::TransactionOperation;
                fixture
                    .conn
                    .authorizer(Some(|context: AuthContext<'_>| {
                        if matches!(
                            context.action,
                            AuthAction::Transaction {
                                // rusqlite 0.39 maps SQLite's COMMIT operation to Unknown.
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
                fixture.conn.execute_batch(&format!("CREATE TRIGGER injected_failure {target} BEGIN SELECT RAISE(ABORT,'injected event failure'); END")).unwrap();
            }
            let before = durable_state(&fixture.conn);
            let transition = opening(owner.run_id, 1, 1, metadata());
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live
                )
                .is_err(),
                "{target}"
            );
            assert_eq!(durable_state(&fixture.reopen()), before, "{target}");
            if target == "commit" {
                fixture
                    .conn
                    .authorizer(
                        None::<
                            fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization,
                        >,
                    )
                    .unwrap();
            } else {
                fixture
                    .conn
                    .execute_batch("DROP TRIGGER injected_failure")
                    .unwrap();
            }
            let (_, inserted) = storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live,
            )
            .unwrap();
            assert!(
                inserted,
                "{target}: the retry commits the removed failure's write"
            );
        }
    }

    #[test]
    fn closure_and_its_notification_roll_back_together() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.open(&owner);
        fixture.conn.execute_batch("CREATE TRIGGER fail_closure BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'closure inbox failure'); END").unwrap();
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &closure(owner.run_id, 2, 1),
                PersistenceMode::Live
            )
            .is_err()
        );
        assert_eq!(durable_state(&fixture.reopen()), before);
        fixture
            .conn
            .execute_batch("DROP TRIGGER fail_closure")
            .unwrap();
        let closed = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &closure(owner.run_id, 2, 1),
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        let view = storage::get_event(&fixture.conn, closed.event_id).unwrap();
        assert_eq!(
            view.interaction.unwrap().close_reason,
            Some(InteractionCloseReason::NotVisible)
        );
        assert_eq!(closed.payload_json["decision_outcome"], "unknown");
    }

    #[test]
    fn closing_transition_keeps_the_request_evidence_and_reports_the_closing_frame_in_its_event() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let opened = fixture.open(&owner);
        let request = storage::get_event(&fixture.conn, opened.event_id)
            .unwrap()
            .interaction
            .unwrap();
        let mut raw = raw_evidence(SECRET);
        raw["screen_seq"] = json!(121);
        let closing_frame = crate::broker::runtime::metadata_evidence(&raw).unwrap();
        assert_ne!(closing_frame, request.evidence_json);
        let closed = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &ObservedTransition {
                evidence_json: closing_frame.clone(),
                ..closure(owner.run_id, 2, 1)
            },
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        assert_eq!(closed.payload_json["evidence"], closing_frame);
        let stored = storage::get_event(&fixture.conn, closed.event_id)
            .unwrap()
            .interaction
            .unwrap();
        assert_eq!(
            stored.close_reason,
            Some(InteractionCloseReason::NotVisible)
        );
        assert!(stored.closed_at.is_some());
        assert_eq!(
            stored.evidence_json, request.evidence_json,
            "the stored interaction keeps the evidence of the request"
        );
        assert_eq!(stored.evidence_digest, request.evidence_digest);
    }

    #[test]
    fn archived_recovery_selects_the_lifecycle_reason_without_fabricating_an_inbox() {
        for (damage, reason) in [
            ("member", "member_inactive"),
            ("director", "director_inactive"),
            ("fleet", "fleet_deleted"),
        ] {
            let mut fixture = Fixture::new();
            fixture.activate();
            let owner = fixture.recoverer();
            fixture.reserve(&owner);
            match damage {
                "member" => {
                    crate::broker::members::test_support::deregister_member(
                        &mut fixture.conn,
                        fixture.member,
                    )
                    .unwrap();
                }
                "director" => {
                    fixture
                        .conn
                        .execute(
                            "UPDATE members SET status='deregistered' WHERE member_id=?1",
                            [fixture.director],
                        )
                        .unwrap();
                }
                "fleet" => {
                    fixture
                        .conn
                        .execute(
                            "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                            params![NOW, fixture.fleet],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let transition = opening(owner.run_id, 1, 1, metadata());
            let before = durable_state(&fixture.conn);
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live
                )
                .is_err()
            );
            assert_eq!(durable_state(&fixture.conn), before);
            let event = storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Recovery,
            )
            .unwrap()
            .0;
            let output = crate::output::json::runtime_event(
                &storage::get_event(&fixture.conn, event.event_id).unwrap(),
            );
            assert_eq!(output["notification_state"], "archived");
            assert_eq!(output["archive_reason"], reason);
            assert_eq!(event.message_id, None);
            assert!(rows(&fixture.conn, "messages").is_empty());
        }
    }

    #[test]
    fn committed_inbox_retry_after_deregistration_retains_the_original_association() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let first = fixture.open(&owner);
        crate::broker::members::test_support::deregister_member(&mut fixture.conn, fixture.member)
            .unwrap();
        let recovery = fixture.recoverer();
        let retry = storage::persist_transition(
            &mut fixture.conn,
            &recovery,
            &opening(owner.run_id, 1, 1, metadata()),
            PersistenceMode::Recovery,
        )
        .unwrap()
        .0;
        assert_eq!(
            (retry.event_id, retry.message_id),
            (first.event_id, first.message_id)
        );
        assert_eq!(retry.notification_state, NotificationState::Inbox);
        assert_eq!(rows(&fixture.conn, "messages").len(), 1);
    }

    #[test]
    fn poll_consumes_the_message_without_closing_the_observed_interaction() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        let consumed = crate::broker::poll_messages(&fixture.conn, fixture.director).unwrap();
        assert_eq!(
            consumed
                .iter()
                .map(|m| Some(m.message_id))
                .collect::<Vec<_>>(),
            [event.message_id]
        );
        let view = storage::get_event(&fixture.reopen(), event.event_id).unwrap();
        assert_eq!(view.interaction.unwrap().closed_at, None);
        assert_eq!(view.event.payload_json["decision_outcome"], "unknown");
        assert!(
            crate::broker::poll_messages(&fixture.conn, fixture.director)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn nonexistent_episode_closure_and_corrupt_cross_run_links_fail_explicitly() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &closure(owner.run_id, 1, 404),
                PersistenceMode::Live
            )
            .is_err()
        );
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &opening(owner.run_id, 1, 1, metadata()),
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        let other_member = register(&mut fixture.conn, fixture.fleet, "other-worker", None);
        let spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, other_member);
        let other =
            crate::broker::runtime::test_support::create_run(&mut fixture.conn, &spec).unwrap();
        fixture
            .conn
            .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON")
            .unwrap();
        fixture
            .conn
            .execute(
                "UPDATE member_runtime_events SET run_id=?1 WHERE event_id=?2",
                params![other.run_id, event.event_id],
            )
            .unwrap();
        assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
    }

    #[test]
    fn event_read_rejects_a_tampered_message_body_or_public_identity() {
        for damage in [
            "UPDATE messages SET text='{}'",
            "UPDATE member_runtime_events SET payload_json='{\"run_id\":404}'",
        ] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let event = fixture.open(&owner);
            fixture.conn.execute_batch(damage).unwrap();
            assert!(
                storage::get_event(&fixture.conn, event.event_id).is_err(),
                "{damage}"
            );
        }
    }
    #[test]
    fn transition_outside_reserved_sequences_or_with_invalid_payload_is_rejected_atomically() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        for invalid in ["unreserved", "missing-evidence"] {
            let mut transition = opening(owner.run_id, 1, 1, metadata());
            match invalid {
                "unreserved" => transition.event_seq = 257,
                "missing-evidence" => transition.evidence_json = json!({}),
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live
                )
                .is_err(),
                "{invalid}"
            );
            assert_eq!(durable_state(&fixture.conn), before);
        }
    }
    fn native_exit(run_id: i64, event_seq: i64) -> ObservedTransition {
        ObservedTransition {
            run_id,
            event_seq,
            episode_seq: None,
            kind: RuntimeEventKind::RunnerExited,
            classification: None,
            observed_at: NOW.into(),
            evidence_json: metadata(),
            close_reason: None,
            cause_json: Some(json!({"kind":"native_exit","exit_code":7,"exit_signal":null})),
        }
    }
    #[test]
    fn terminal_live_write_regression_keeps_committed_retries_and_rejects_new_observations() {
        let status = "exited";
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let exit = native_exit(owner.run_id, 1);
        let first =
            storage::persist_transition(&mut fixture.conn, &owner, &exit, PersistenceMode::Live)
                .unwrap()
                .0;
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .status,
            RunStatus::Exited
        );
        let retry =
            storage::persist_transition(&mut fixture.conn, &owner, &exit, PersistenceMode::Live)
                .unwrap()
                .0;
        assert_eq!(
            (retry.event_id, retry.message_id),
            (first.event_id, first.message_id)
        );
        let next = ObservedTransition {
            event_seq: 2,
            kind: RuntimeEventKind::CoverageUnknown,
            cause_json: Some(json!({"kind":"coverage_unknown"})),
            ..exit
        };
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(&mut fixture.conn, &owner, &next, PersistenceMode::Live)
                .is_err(),
            "{status}"
        );
        assert_eq!(durable_state(&fixture.reopen()), before, "{status}");
    }

    #[test]
    fn terminal_live_write_regression_preserves_explicitly_owned_historical_recovery() {
        let mut fixture = Fixture::new();
        fixture.activate();
        let owner = fixture.recoverer();
        fixture.reserve(&owner);
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET recovery_origin_status='exited' WHERE run_id=?1",
                [owner.run_id],
            )
            .unwrap();
        crate::broker::members::test_support::deregister_member(&mut fixture.conn, fixture.member)
            .unwrap();
        let opened = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &opening(owner.run_id, 1, 1, metadata()),
            PersistenceMode::Recovery,
        )
        .unwrap()
        .0;
        let mut closed = closure(owner.run_id, 2, 1);
        closed.close_reason = Some(InteractionCloseReason::ProcessExited);
        storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &closed,
            PersistenceMode::Recovery,
        )
        .unwrap();
        let exited = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &native_exit(owner.run_id, 3),
            PersistenceMode::Recovery,
        )
        .unwrap()
        .0;
        assert_eq!(opened.notification_state, NotificationState::Archived);
        assert_eq!(exited.notification_state, NotificationState::Archived);
        assert_eq!(exited.message_id, None);
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .status,
            RunStatus::Exited
        );
        assert!(rows(&fixture.conn, "messages").is_empty());
    }

    #[test]
    fn event_shape_regression_rejects_lifecycle_transitions_linked_to_an_episode() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.open(&owner);
        let transition = ObservedTransition {
            episode_seq: Some(1),
            classification: Some(InteractionClass::PermissionRequest),
            ..native_exit(owner.run_id, 2)
        };
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live
            )
            .is_err()
        );
        assert_eq!(durable_state(&fixture.reopen()), before);
    }

    #[test]
    fn event_shape_regression_requires_a_typed_lifecycle_cause_before_commit() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let kind = RuntimeEventKind::CoverageUnknown;
        for cause in [
            None,
            Some(json!({})),
            Some(json!({"kind":"invented-cause"})),
        ] {
            let transition = ObservedTransition {
                kind,
                cause_json: cause,
                ..native_exit(owner.run_id, 1)
            };
            let before = durable_state(&fixture.conn);
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live
                )
                .is_err(),
                "{kind:?}: {:?}",
                transition.cause_json
            );
            assert_eq!(durable_state(&fixture.reopen()), before);
        }
    }

    #[test]
    fn event_shape_regression_rejects_lifecycle_causes_on_screen_transitions() {
        let kind = RuntimeEventKind::InteractionOpened;
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let mut transition = opening(owner.run_id, 1, 1, metadata());
        transition.cause_json =
            Some(json!({"kind":"native_exit","exit_code":7,"exit_signal":null}));
        let before = durable_state(&fixture.conn);
        assert!(
            storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live
            )
            .is_err(),
            "{kind:?}"
        );
        assert_eq!(durable_state(&fixture.reopen()), before);
    }

    #[test]
    fn event_shape_regression_rejects_a_persisted_lifecycle_episode_even_with_a_matching_message() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let opened = fixture.open(&owner);
        storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &closure(owner.run_id, 2, 1),
            PersistenceMode::Live,
        )
        .unwrap();
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &native_exit(owner.run_id, 3),
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        let mut payload = event.payload_json.clone();
        payload["interaction_id"] = json!(opened.interaction_id.unwrap());
        payload["classification"] = json!("permission_request");
        fixture
            .conn
            .execute(
                "UPDATE member_runtime_events SET interaction_id=?1 WHERE event_id=?2",
                params![opened.interaction_id, event.event_id],
            )
            .unwrap();
        replace_public_payload(&mut fixture.conn, &event, payload);
        assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
    }

    #[test]
    fn event_shape_regression_rejects_missing_or_malformed_persisted_exit_causes() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        let event = storage::persist_transition(
            &mut fixture.conn,
            &owner,
            &native_exit(owner.run_id, 1),
            PersistenceMode::Live,
        )
        .unwrap()
        .0;
        for cause in [
            None,
            Some(json!({"kind":"native_exit","exit_code":"7","exit_signal":null})),
        ] {
            let mut payload = event.payload_json.clone();
            match cause {
                Some(cause) => {
                    payload["cause"] = cause;
                }
                None => {
                    payload.as_object_mut().unwrap().remove("cause");
                }
            }
            replace_public_payload(&mut fixture.conn, &event, payload);
            assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
        }
    }

    #[test]
    fn event_shape_regression_rejects_a_persisted_screen_cause_even_with_a_matching_message() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        let mut payload = event.payload_json.clone();
        payload["cause"] = json!({"kind":"native_exit","exit_code":7,"exit_signal":null});
        replace_public_payload(&mut fixture.conn, &event, payload);
        assert!(storage::get_event(&fixture.conn, event.event_id).is_err());
    }

    mod composer_reliability {
        use crate::broker::runtime::events::tests::*;

        fn stalled_fixture() -> (Fixture, RunOwner, InputOperation) {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let head = fixture.input(&owner, "prompt:stalled-head", InputKind::Prompt, None);
            let head = storage::requeue_zero_byte_immediate(
                &mut fixture.conn,
                &owner,
                head.input_id,
                "earlier_input",
            )
            .unwrap();
            (fixture, owner, head)
        }

        fn stalled_transition(owner: &RunOwner, head: i64) -> ObservedTransition {
            ObservedTransition {
                kind: RuntimeEventKind::InputDeliveryStalled,
                cause_json: Some(json!({
                    "kind":"input_delivery_stalled", "input_id":head,
                    "message_id":null, "waiting_since":NOW,
                })),
                ..native_exit(owner.run_id, 1)
            }
        }

        #[test]
        fn stalled_events_remain_readable_and_deduplicated_after_head_settlement() {
            for submitted in [false, true] {
                let (mut fixture, owner, head) = stalled_fixture();
                let transition = stalled_transition(&owner, head.input_id);
                let (event, inserted) = storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert!(inserted);
                if submitted {
                    storage::begin_deferred_attempt(
                        &mut fixture.conn,
                        &owner,
                        head.input_id,
                        DeliveryMethod::Profile,
                        None,
                    )
                    .unwrap();
                    storage::test_support::settle_input(
                        &mut fixture.conn,
                        &owner,
                        head.input_id,
                        InputState::InProgress,
                        InputDisposition::Submitted,
                        "fixture:verified-submit",
                    )
                    .unwrap();
                } else {
                    storage::settle_zero_byte_deferred(
                        &mut fixture.conn,
                        &owner,
                        head.input_id,
                        InputState::Ready,
                        "target_inactive",
                    )
                    .unwrap();
                }
                let before = durable_state(&fixture.reopen());
                let historical = storage::get_event(&fixture.reopen(), event.event_id).unwrap();
                assert_eq!(historical.event.payload_json, event.payload_json);
                let (same, inserted) = storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert!(!inserted);
                assert_eq!(same.event_id, event.event_id);
                assert_eq!(same.payload_json, event.payload_json);
                assert_eq!(durable_state(&fixture.reopen()), before);
            }
        }

        #[test]
        fn a_new_stalled_event_requires_its_head_to_be_ready_and_deferred() {
            for state in [
                InputState::InProgress,
                InputState::Accepted,
                InputState::Settled,
            ] {
                let (mut fixture, owner, head) = stalled_fixture();
                storage::begin_deferred_attempt(
                    &mut fixture.conn,
                    &owner,
                    head.input_id,
                    DeliveryMethod::Profile,
                    None,
                )
                .unwrap();
                if state == InputState::Accepted {
                    storage::record_input_state(
                        &mut fixture.conn,
                        &owner,
                        head.input_id,
                        InputState::InProgress,
                        InputState::Accepted,
                    )
                    .unwrap();
                } else if state == InputState::Settled {
                    storage::settle_zero_byte_deferred(
                        &mut fixture.conn,
                        &owner,
                        head.input_id,
                        InputState::InProgress,
                        "target_inactive",
                    )
                    .unwrap();
                }
                let transition = stalled_transition(&owner, head.input_id);
                let before = durable_state(&fixture.reopen());
                assert!(
                    storage::persist_transition(
                        &mut fixture.conn,
                        &owner,
                        &transition,
                        PersistenceMode::Live
                    )
                    .is_err(),
                    "{state:?}"
                );
                assert_eq!(durable_state(&fixture.reopen()), before);
            }
            let (mut fixture, owner, _) = stalled_fixture();
            let head = fixture.input(&owner, "prompt:unselected-head", InputKind::Prompt, None);
            storage::record_input_state(
                &mut fixture.conn,
                &owner,
                head.input_id,
                InputState::InProgress,
                InputState::Ready,
            )
            .unwrap();
            let before = durable_state(&fixture.reopen());
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &stalled_transition(&owner, head.input_id),
                    PersistenceMode::Live
                )
                .is_err()
            );
            assert_eq!(durable_state(&fixture.reopen()), before);
        }

        fn immediate_uncertain(fixture: &mut Fixture, owner: &RunOwner) -> InputOperation {
            let input = fixture.input(owner, "prompt:immediate-history", InputKind::Prompt, None);
            storage::record_input_uncertain(
                &mut fixture.conn,
                owner,
                input.input_id,
                StopCause::WriteFailed,
            )
            .unwrap()
        }

        fn stopped_result(owner: &RunOwner, input_id: i64) -> ObservedTransition {
            ObservedTransition {
                kind: RuntimeEventKind::InputDeliveryResult,
                cause_json: Some(storage::input_delivery_result_cause(
                    input_id,
                    "unknown",
                    Some(StopCause::WriteFailed),
                )),
                ..native_exit(owner.run_id, 1)
            }
        }

        #[test]
        fn immediate_uncertain_result_history_survives_reconciliation_and_deduplicates() {
            for disposition in [
                InputDisposition::Submitted,
                InputDisposition::NotSubmitted,
                InputDisposition::AbandonedUncertain,
            ] {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                fixture.reserve(&owner);
                let input = immediate_uncertain(&mut fixture, &owner);
                let transition = stopped_result(&owner, input.input_id);
                let (event, inserted) = storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert!(inserted);
                storage::test_support::settle_input(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    InputState::Uncertain,
                    disposition,
                    "fixture:observed-native-result",
                )
                .unwrap();
                let before = durable_state(&fixture.reopen());
                let historical = storage::get_event(&fixture.reopen(), event.event_id).unwrap();
                assert_eq!(historical.event.payload_json, event.payload_json);
                assert_eq!(
                    historical.event.payload_json["delivery"],
                    json!({
                        "status":"uncertain", "method":"profile", "reason":null,
                    })
                );
                assert_eq!(
                    historical.event.payload_json["cause"],
                    json!({"kind":"write_failed"})
                );
                assert_eq!(historical.event.payload_json["stage"], "unknown");
                let (same, inserted) = storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert!(!inserted);
                assert_eq!(same.event_id, event.event_id);
                assert_eq!(same.payload_json, event.payload_json);
                assert_eq!(durable_state(&fixture.reopen()), before);
            }
        }

        #[test]
        fn immediate_stop_reconciled_before_first_publication_recovers_its_original_uncertainty() {
            for disposition in [
                InputDisposition::Submitted,
                InputDisposition::NotSubmitted,
                InputDisposition::AbandonedUncertain,
            ] {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                fixture.reserve(&owner);
                let input = immediate_uncertain(&mut fixture, &owner);
                storage::test_support::settle_input(
                    &mut fixture.conn,
                    &owner,
                    input.input_id,
                    InputState::Uncertain,
                    disposition,
                    "fixture:reconciled-before-publication",
                )
                .unwrap();
                assert_eq!(
                    storage::unpublished_delivery_results(&fixture.conn, owner.run_id).unwrap(),
                    [(input.input_id, "unknown", Some(StopCause::WriteFailed))],
                );
                let transition = stopped_result(&owner, input.input_id);
                let (event, inserted) = storage::persist_transition(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert!(inserted);
                let payload = storage::get_event(&fixture.reopen(), event.event_id)
                    .unwrap()
                    .event
                    .payload_json;
                assert_eq!(
                    payload["delivery"],
                    json!({
                        "status":"uncertain", "method":"profile", "reason":null,
                    })
                );
                assert_eq!(payload["cause"], json!({"kind":"write_failed"}));
                assert_eq!(payload["stage"], "unknown");
                assert!(
                    storage::unpublished_delivery_results(&fixture.conn, owner.run_id)
                        .unwrap()
                        .is_empty()
                );
            }
        }

        #[test]
        fn immediate_submitted_inputs_publish_no_delivery_result() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let input = fixture.input(
                &owner,
                "prompt:immediate-submitted",
                InputKind::Prompt,
                None,
            );
            storage::record_input_state(
                &mut fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputState::Accepted,
            )
            .unwrap();
            assert!(
                storage::unpublished_delivery_results(&fixture.conn, owner.run_id)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    mod input_delivery_result_events {
        use crate::broker::runtime::events::tests::*;

        const SUBMITTED: &str = r#"{"status":"submitted","method":"profile","reason":null}"#;

        fn envelope(
            base: &Value,
            input_id: i64,
            message_id: Value,
            delivery: Value,
            stage: &str,
        ) -> Value {
            let mut object = base.as_object().unwrap().clone();
            object.remove("cause");
            object.remove("director_action");
            object.insert("kind".into(), json!("input_delivery_result"));
            object.insert("input_id".into(), json!(input_id));
            object.insert("message_id".into(), message_id);
            object.insert("delivery".into(), delivery);
            object.insert("stage".into(), json!(stage));
            object.insert("director_action".into(), Value::Null);
            Value::Object(object)
        }

        fn deferred_result(fixture: &mut Fixture) -> (RuntimeEventRecord, i64, Value) {
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let input = fixture.input(&owner, "prompt:deferred", InputKind::Prompt, None);
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET state='settled',disposition='submitted',delivery_method='profile',deferred_at=?1,defer_reason='interaction_open' WHERE input_id=?2",
                    params![NOW, input.input_id],
                )
                .unwrap();
            let transition = ObservedTransition {
                kind: RuntimeEventKind::CoverageUnknown,
                cause_json: Some(json!({"kind":"coverage_unknown"})),
                ..native_exit(owner.run_id, 1)
            };
            let event = storage::persist_transition(
                &mut fixture.conn,
                &owner,
                &transition,
                PersistenceMode::Live,
            )
            .unwrap()
            .0;
            fixture
                .conn
                .execute(
                    "UPDATE member_runtime_events SET kind='input_delivery_result' WHERE event_id=?1",
                    [event.event_id],
                )
                .unwrap();
            let payload = envelope(
                &event.payload_json,
                input.input_id,
                Value::Null,
                serde_json::from_str(SUBMITTED).unwrap(),
                "submitted",
            );
            replace_public_payload(&mut fixture.conn, &event, payload.clone());
            (event, input.input_id, payload)
        }

        #[test]
        fn input_delivery_result_envelope_appends_its_receipt_fields_before_a_null_action() {
            let mut fixture = Fixture::new();
            let (event, input_id, payload) = deferred_result(&mut fixture);
            let view = storage::get_event(&fixture.conn, event.event_id)
                .unwrap_or_else(|error| panic!("{}", error.message()));
            assert_eq!(view.event.kind, RuntimeEventKind::InputDeliveryResult);
            assert!(view.interaction.is_none());
            let read = &view.event.payload_json;
            assert_eq!(*read, payload);
            assert_eq!(read["interaction_id"], Value::Null);
            assert_eq!(read["classification"], Value::Null);
            assert_eq!(read["source"], "runner_lifecycle");
            assert_eq!(read["input_id"], input_id);
            let keys = read
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(
                keys[keys.len() - 5..],
                [
                    "input_id",
                    "message_id",
                    "delivery",
                    "stage",
                    "director_action"
                ]
            );
            assert_key_order(&read["delivery"], &["status", "method", "reason"]);
        }

        #[test]
        fn input_delivery_result_reader_rejects_an_envelope_that_disagrees_with_its_receipt() {
            let mut fixture = Fixture::new();
            let (event, input_id, payload) = deferred_result(&mut fixture);
            for (label, forged) in [
                (
                    "delivery differs from the stored receipt",
                    envelope(
                        &payload,
                        input_id,
                        Value::Null,
                        json!({"status":"not_submitted","method":null,"reason":"runner_stopped"}),
                        "submitted",
                    ),
                ),
                (
                    "unknown input",
                    envelope(
                        &payload,
                        input_id + 1000,
                        Value::Null,
                        serde_json::from_str(SUBMITTED).unwrap(),
                        "submitted",
                    ),
                ),
                (
                    "explicit input names a message",
                    envelope(
                        &payload,
                        input_id,
                        json!(event.message_id.unwrap()),
                        serde_json::from_str(SUBMITTED).unwrap(),
                        "submitted",
                    ),
                ),
                (
                    "stage outside its domain",
                    envelope(
                        &payload,
                        input_id,
                        Value::Null,
                        serde_json::from_str(SUBMITTED).unwrap(),
                        "done",
                    ),
                ),
            ] {
                replace_public_payload(&mut fixture.conn, &event, forged);
                assert!(
                    storage::get_event(&fixture.conn, event.event_id).is_err(),
                    "{label}"
                );
            }
            for (label, change) in [
                (
                    "non-null action",
                    (
                        "director_action",
                        json!({"kind":"review_permission_request"}),
                    ),
                ),
                ("literal payload", ("literal", json!("private prompt"))),
                (
                    "cause on a submitted result",
                    ("cause", json!({"kind":"coverage_unknown"})),
                ),
            ] {
                let mut forged = payload.clone();
                forged[change.0] = change.1;
                replace_public_payload(&mut fixture.conn, &event, forged);
                assert!(
                    storage::get_event(&fixture.conn, event.event_id).is_err(),
                    "{label}"
                );
            }
            replace_public_payload(&mut fixture.conn, &event, payload);
            storage::get_event(&fixture.conn, event.event_id).unwrap();
        }

        const UNCERTAIN: &str = r#"{"status":"uncertain","method":"profile","reason":null}"#;

        fn uncertain_result(
            fixture: &mut Fixture,
            cause: Option<&str>,
        ) -> (RuntimeEventRecord, i64, Value) {
            let (event, input_id, base) = deferred_result(fixture);
            fixture
                .conn
                .execute(
                    "UPDATE member_input_operations SET state='uncertain',disposition=NULL,stop_cause=?1 WHERE input_id=?2",
                    params![cause, input_id],
                )
                .unwrap();
            let mut payload = envelope(
                &base,
                input_id,
                Value::Null,
                serde_json::from_str(UNCERTAIN).unwrap(),
                "unknown",
            );
            if let Some(cause) = cause {
                payload["cause"] = json!({ "kind": cause });
            }
            replace_public_payload(&mut fixture.conn, &event, payload.clone());
            (event, input_id, payload)
        }

        #[test]
        fn input_delivery_result_cause_carries_the_receipt_cause_only_when_recorded() {
            assert_eq!(
                storage::input_delivery_result_cause(7, "unknown", Some(StopCause::WriteFailed)),
                json!({"kind":"input_delivery_result","input_id":7,"stage":"unknown","cause":{"kind":"write_failed"}})
            );
            let plain = storage::input_delivery_result_cause(7, "submitted", None);
            assert_eq!(
                plain,
                json!({"kind":"input_delivery_result","input_id":7,"stage":"submitted"})
            );
            assert!(plain.get("cause").is_none());
        }

        #[test]
        fn input_delivery_result_transition_admits_only_a_recorded_cause_kind() {
            let transition = |cause_json: Value| ObservedTransition {
                kind: RuntimeEventKind::InputDeliveryResult,
                cause_json: Some(cause_json),
                ..native_exit(1, 1)
            };
            for cause in [
                storage::input_delivery_result_cause(3, "unknown", Some(StopCause::RunnerStopped)),
                storage::input_delivery_result_cause(3, "submitted", None),
            ] {
                crate::broker::runtime::events::validate_delivery_cause(&transition(cause.clone()))
                    .unwrap_or_else(|error| panic!("{cause}: {}", error.message()));
            }
            for forged in [
                json!({"kind":"input_delivery_result","input_id":3,"stage":"unknown","cause":{"kind":"terminal_reply"}}),
                json!({"kind":"input_delivery_result","input_id":3,"stage":"unknown","cause":{"kind":"write_failed","detail":"x"}}),
                json!({"kind":"input_delivery_result","input_id":3,"stage":"unknown","cause":"write_failed"}),
            ] {
                assert!(
                    crate::broker::runtime::events::validate_delivery_cause(&transition(
                        forged.clone()
                    ))
                    .is_err(),
                    "{forged}"
                );
            }
        }

        #[test]
        fn input_delivery_result_reader_accepts_the_receipt_cause_on_an_uncertain_result() {
            let mut fixture = Fixture::new();
            let (event, _, payload) = uncertain_result(&mut fixture, Some("manual_input"));
            let view = storage::get_event(&fixture.conn, event.event_id)
                .unwrap_or_else(|error| panic!("{}", error.message()));
            assert_eq!(view.event.payload_json, payload);
            assert_eq!(
                view.event.payload_json["cause"],
                json!({"kind":"manual_input"})
            );
            assert_eq!(
                crate::broker::runtime::events::original(&view)
                    .unwrap()
                    .cause_json,
                Some(
                    json!({"kind":"input_delivery_result","input_id":payload["input_id"],"stage":"unknown","cause":{"kind":"manual_input"}})
                )
            );
        }

        #[test]
        fn input_delivery_result_reader_accepts_an_absent_cause_for_a_causeless_receipt() {
            let mut fixture = Fixture::new();
            let (event, _, payload) = uncertain_result(&mut fixture, None);
            let view = storage::get_event(&fixture.conn, event.event_id)
                .unwrap_or_else(|error| panic!("{}", error.message()));
            assert!(view.event.payload_json.get("cause").is_none());
            let mut forged = payload;
            forged["cause"] = json!({"kind":"write_failed"});
            replace_public_payload(&mut fixture.conn, &event, forged);
            assert!(
                storage::get_event(&fixture.conn, event.event_id).is_err(),
                "a cause the receipt never recorded is rejected"
            );
        }

        #[test]
        fn input_delivery_result_reader_rejects_a_cause_that_disagrees_with_the_receipt() {
            let mut fixture = Fixture::new();
            let (event, _, payload) = uncertain_result(&mut fixture, Some("manual_input"));
            let mut mismatched = payload.clone();
            mismatched["cause"] = json!({"kind":"write_failed"});
            let mut missing = payload.clone();
            missing.as_object_mut().unwrap().remove("cause");
            for (label, forged) in [("mismatched cause", mismatched), ("missing cause", missing)] {
                replace_public_payload(&mut fixture.conn, &event, forged);
                assert!(
                    storage::get_event(&fixture.conn, event.event_id).is_err(),
                    "{label}"
                );
            }
            replace_public_payload(&mut fixture.conn, &event, payload);
            storage::get_event(&fixture.conn, event.event_id).unwrap();
        }
    }
}
