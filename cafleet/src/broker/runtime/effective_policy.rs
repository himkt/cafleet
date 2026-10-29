use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::*;

const FIELDS: [&str; 6] = [
    "schema_version",
    "automatic_review",
    "observed_mode",
    "rule_id",
    "event_id",
    "observed_at",
];

pub(super) fn validate(run: &RunRecord) -> Result<(), CafleetError> {
    let Some(record) = &run.effective_policy_json else {
        return Ok(());
    };
    let field = "member_runs.effective_policy_json";
    let object = record.as_object().expect("decoded effective policy object");
    require(
        object.len() == FIELDS.len() && FIELDS.iter().all(|key| object.contains_key(*key)),
        field,
        "exact effective policy fields required",
    )?;
    require(
        record["schema_version"].as_u64() == Some(1),
        field,
        "unsupported effective policy schema",
    )?;
    let review = AutomaticReview::try_from(
        record["automatic_review"]
            .as_str()
            .ok_or_else(|| invalid(field, "automatic review state required"))?,
    )?;
    let mode = &record["observed_mode"];
    require(
        mode.is_null() || mode.is_string(),
        field,
        "nullable observed mode text required",
    )?;
    require(
        run.event_evidence == EventEvidence::Redacted || mode.is_null(),
        field,
        "metadata runs record no observed mode",
    )?;
    nonblank(
        record["rule_id"]
            .as_str()
            .ok_or_else(|| invalid(field, "rule id required"))?,
        field,
    )?;
    let event = &record["event_id"];
    require(
        event.is_null() || event.as_i64().is_some_and(|id| id > 0),
        field,
        "nullable positive event id required",
    )?;
    require(
        event.is_null() == (review == AutomaticReview::Available),
        field,
        "only an unavailable record names its event",
    )?;
    chrono::DateTime::parse_from_rfc3339(
        record["observed_at"]
            .as_str()
            .ok_or_else(|| invalid(field, "observation time required"))?,
    )
    .map_err(|_| invalid(field, "RFC 3339 observation time required"))?;
    Ok(())
}

fn check_observation(
    run: &RunRecord,
    observation: &EffectivePolicyObservation,
    expected: AutomaticReview,
) -> Result<(), CafleetError> {
    require(
        observation.automatic_review == expected,
        "effective_policy.automatic_review",
        "observation does not match its write path",
    )?;
    require(
        run.event_evidence == EventEvidence::Redacted || observation.observed_mode.is_none(),
        "effective_policy.observed_mode",
        "metadata runs record no observed mode",
    )?;
    nonblank(&observation.rule_id, "effective_policy.rule_id")?;
    chrono::DateTime::parse_from_rfc3339(&observation.observed_at).map_err(|_| {
        invalid(
            "effective_policy.observed_at",
            "RFC 3339 timestamp required",
        )
    })?;
    Ok(())
}

fn write_if_changed(
    conn: &Connection,
    run: &RunRecord,
    observation: &EffectivePolicyObservation,
    event_id: Option<i64>,
) -> Result<bool, CafleetError> {
    let unchanged = run.effective_policy_json.as_ref().is_some_and(|stored| {
        stored["automatic_review"] == observation.automatic_review.as_str()
            && stored["observed_mode"] == json!(observation.observed_mode)
    });
    if unchanged {
        return Ok(false);
    }
    let record: Value = json!({
        "schema_version":1,
        "automatic_review":observation.automatic_review.as_str(),
        "observed_mode":observation.observed_mode,
        "rule_id":observation.rule_id,
        "event_id":event_id,
        "observed_at":observation.observed_at,
    });
    let changed = conn
        .execute(
            "UPDATE member_runs SET effective_policy_json=?1 WHERE run_id=?2 AND owner_epoch=?3",
            params![record.to_string(), run.run_id, run.owner_epoch],
        )
        .map_err(db_err)?;
    require(
        changed == 1,
        "member_runs.owner",
        "effective policy write lost ownership",
    )?;
    Ok(true)
}

pub fn persist_transition_with_effective_policy(
    conn: &mut Connection,
    owner: &RunOwner,
    transition: &ObservedTransition,
    observation: &EffectivePolicyObservation,
    mode: PersistenceMode,
) -> Result<RuntimeEventRecord, CafleetError> {
    persist_transition_with_effective_policy_counted(conn, owner, transition, observation, mode)
        .map(|(event, _)| event)
}

pub(crate) fn persist_transition_with_effective_policy_counted(
    conn: &mut Connection,
    owner: &RunOwner,
    transition: &ObservedTransition,
    observation: &EffectivePolicyObservation,
    mode: PersistenceMode,
) -> Result<(RuntimeEventRecord, bool), CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        require(
            transition.kind == RuntimeEventKind::AutomaticReviewUnavailable,
            "kind",
            "only an unavailable review event carries an effective policy",
        )?;
        check_observation(run, observation, AutomaticReview::Unavailable)?;
        let (event, inserted) = persist_transition_in_transaction(conn, run, transition, mode)?;
        if inserted {
            write_if_changed(conn, run, observation, Some(event.event_id))?;
        }
        Ok((event, inserted))
    })
}

pub fn record_effective_policy(
    conn: &mut Connection,
    owner: &RunOwner,
    observation: &EffectivePolicyObservation,
    deadline: std::time::Instant,
) -> Result<bool, CafleetError> {
    crate::runtime::runner::with_database_deadline(conn, deadline, |conn| {
        owned_transaction(conn, owner, |conn, run| {
            check_observation(run, observation, AutomaticReview::Available)?;
            require(
                matches!(run.status, RunStatus::Running | RunStatus::Degraded),
                "member_runs.status",
                "effective policy updates require a live run",
            )?;
            write_if_changed(conn, run, observation, None)
        })
    })
}
