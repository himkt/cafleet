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
    let changed = conn.execute(
        "UPDATE member_runs SET effective_policy_json=?1 WHERE run_id=?2 AND owner_epoch=?3",
        params![record.to_string(), run.run_id, run.owner_epoch],
    )?;
    require(
        changed == 1,
        "member_runs.owner",
        "effective policy write lost ownership",
    )?;
    Ok(true)
}

#[cfg(test)]
pub(crate) fn persist_event_with_effective_policy(
    conn: &mut Connection,
    owner: &RunOwner,
    transition: &ObservedTransition,
    observation: &EffectivePolicyObservation,
    mode: PersistenceMode,
) -> Result<RuntimeEventRecord, CafleetError> {
    persist_transition_with_effective_policy(conn, owner, transition, observation, mode)
        .map(|(event, _)| event)
}

pub(crate) fn persist_transition_with_effective_policy(
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

pub(crate) fn record_effective_policy(
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

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use serde_json::{Value, json};

    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::{self as storage, *};

    mod effective_policy_writes {
        use super::*;
        use crate::broker::runtime::test_support::automatic_review::*;
        use crate::error::CafleetError;
        use std::time::{Duration, Instant};

        const UNAVAILABLE_RULE: &str = "codex-auto-review-unavailable";
        const AVAILABLE_RULE: &str = "codex-auto-review-available";
        const LATER: &str = "2026-10-26T10:00:05.000000+00:00";

        fn observation(
            automatic_review: AutomaticReview,
            observed_mode: Option<&str>,
            rule_id: &str,
            observed_at: &str,
        ) -> EffectivePolicyObservation {
            EffectivePolicyObservation {
                automatic_review,
                observed_mode: observed_mode.map(str::to_owned),
                rule_id: rule_id.into(),
                observed_at: observed_at.into(),
            }
        }

        fn unavailable(observed_mode: Option<&str>) -> EffectivePolicyObservation {
            observation(
                AutomaticReview::Unavailable,
                observed_mode,
                UNAVAILABLE_RULE,
                NOW,
            )
        }

        fn available(observed_mode: Option<&str>, observed_at: &str) -> EffectivePolicyObservation {
            observation(
                AutomaticReview::Available,
                observed_mode,
                AVAILABLE_RULE,
                observed_at,
            )
        }

        fn record(observation: &EffectivePolicyObservation, event_id: Option<i64>) -> Value {
            json!({
                "schema_version":1,
                "automatic_review":observation.automatic_review.as_str(),
                "observed_mode":observation.observed_mode,
                "rule_id":observation.rule_id,
                "event_id":event_id,
                "observed_at":observation.observed_at,
            })
        }

        fn unavailable_evidence(fixture: &Fixture) -> Value {
            if fixture.run.event_evidence == EventEvidence::Redacted {
                redacted(&unavailable_raw())
            } else {
                evidence::EvidencePolicy::metadata()
                    .apply(&unavailable_raw())
                    .unwrap()
            }
        }

        fn unavailable_transition(fixture: &Fixture, event_seq: i64) -> ObservedTransition {
            annotation_event(
                fixture.run.run_id,
                event_seq,
                RuntimeEventKind::AutomaticReviewUnavailable,
                unavailable_evidence(fixture),
            )
        }

        fn redacted_running() -> (Fixture, RunOwner) {
            let mut fixture = redacted_fixture();
            let owner = fixture.activate();
            (fixture, owner)
        }

        fn stored(fixture: &Fixture) -> Option<Value> {
            storage::get_run(&fixture.conn, fixture.run.run_id)
                .unwrap()
                .effective_policy_json
        }

        fn record_available(
            fixture: &mut Fixture,
            owner: &RunOwner,
            observation: &EffectivePolicyObservation,
        ) -> Result<bool, CafleetError> {
            storage::record_effective_policy(
                &mut fixture.conn,
                owner,
                observation,
                Instant::now() + Duration::from_secs(5),
            )
        }

        fn assert_invariant(result: Result<impl std::fmt::Debug, CafleetError>, expected: &str) {
            match result {
                Err(CafleetError::InvalidStoredValue { field, .. }) => {
                    assert_eq!(field, expected)
                }
                other => panic!("expected an {expected} invariant violation, got {other:?}"),
            }
        }

        #[test]
        fn unavailable_event_records_the_policy_with_its_event_id_in_the_event_transaction() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let observation = unavailable(None);
            let transition = unavailable_transition(&fixture, 1);
            let event = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &owner,
                &transition,
                &observation,
                PersistenceMode::Live,
            )
            .unwrap();
            assert_eq!(event.kind, RuntimeEventKind::AutomaticReviewUnavailable);
            assert_eq!(
                stored(&fixture),
                Some(record(&observation, Some(event.event_id)))
            );
            assert_runner_screen_event_without_interaction(
                &fixture,
                &event,
                "automatic_review_unavailable",
            );
            assert_eq!(
                event.payload_json.get("effective_policy"),
                None,
                "the policy lives on the run, not in the payload"
            );
            let reopened = storage::get_run(&fixture.reopen(), owner.run_id).unwrap();
            assert_eq!(reopened.status, RunStatus::Running);
            assert_eq!(
                reopened.effective_policy_json,
                Some(record(&observation, Some(event.event_id)))
            );
        }

        #[test]
        fn unavailable_event_write_replays_idempotently_with_its_event() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let transition = unavailable_transition(&fixture, 1);
            let observation = unavailable(None);
            let first = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &owner,
                &transition,
                &observation,
                PersistenceMode::Live,
            )
            .unwrap();
            let committed = durable_state(&fixture.conn);
            let again = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &owner,
                &transition,
                &observation,
                PersistenceMode::Live,
            )
            .unwrap();
            assert_eq!(
                (again.event_id, again.message_id),
                (first.event_id, first.message_id)
            );
            assert_eq!(durable_state(&fixture.conn), committed);
            let recovery = fixture.recoverer();
            let replayed = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &recovery,
                &transition,
                &observation,
                PersistenceMode::Recovery,
            )
            .unwrap();
            assert_eq!(replayed.event_id, first.event_id);
            assert_eq!(replayed.payload_json, first.payload_json);
            assert_eq!(rows(&fixture.conn, "member_runtime_events").len(), 1);
            assert_eq!(rows(&fixture.conn, "messages").len(), 1);
            assert_eq!(
                stored(&fixture),
                Some(record(&observation, Some(first.event_id)))
            );
        }

        #[test]
        fn unavailable_event_first_committed_by_recovery_carries_its_policy() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let transition = unavailable_transition(&fixture, 1);
            let recovery = fixture.recoverer();
            let observation = unavailable(None);
            let event = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &recovery,
                &transition,
                &observation,
                PersistenceMode::Recovery,
            )
            .unwrap();
            assert_eq!(
                stored(&fixture),
                Some(record(&observation, Some(event.event_id)))
            );
            assert_eq!(rows(&fixture.conn, "messages").len(), 1);
        }

        #[test]
        fn failed_unavailable_event_transaction_writes_no_policy() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let before = durable_state(&fixture.conn);
            let rejected = ObservedTransition {
                cause_json: Some(json!({"kind":"coverage_unknown"})),
                ..unavailable_transition(&fixture, 1)
            };
            assert!(
                storage::persist_event_with_effective_policy(
                    &mut fixture.conn,
                    &owner,
                    &rejected,
                    &unavailable(None),
                    PersistenceMode::Live,
                )
                .is_err()
            );
            assert_eq!(durable_state(&fixture.conn), before);
            assert_eq!(stored(&fixture), None);
        }

        #[test]
        fn event_carried_write_accepts_only_an_unavailable_event_with_an_unavailable_observation() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let before = durable_state(&fixture.conn);
            let refusal = annotation_event(
                owner.run_id,
                1,
                RuntimeEventKind::AutomaticRefusalObserved,
                evidence::EvidencePolicy::metadata()
                    .apply(&refusal_raw("write outside.txt"))
                    .unwrap(),
            );
            for (label, transition, observation) in [
                ("refusal event", refusal, unavailable(None)),
                (
                    "interaction event",
                    opening(owner.run_id, 1, 1, metadata()),
                    unavailable(None),
                ),
                (
                    "available observation",
                    unavailable_transition(&fixture, 1),
                    available(None, NOW),
                ),
            ] {
                assert!(
                    storage::persist_event_with_effective_policy(
                        &mut fixture.conn,
                        &owner,
                        &transition,
                        &observation,
                        PersistenceMode::Live,
                    )
                    .is_err(),
                    "{label}"
                );
                assert_eq!(durable_state(&fixture.conn), before, "{label}");
            }
        }

        #[test]
        fn metadata_run_rejects_a_displayed_mode_on_either_write_before_any_write() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
            let before = durable_state(&fixture.conn);
            let transition = unavailable_transition(&fixture, 1);
            assert!(
                storage::persist_event_with_effective_policy(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    &unavailable(Some("auto")),
                    PersistenceMode::Live,
                )
                .is_err()
            );
            assert!(record_available(&mut fixture, &owner, &available(Some("auto"), NOW)).is_err());
            assert_eq!(durable_state(&fixture.conn), before);
            assert!(record_available(&mut fixture, &owner, &available(None, NOW)).unwrap());
            assert_eq!(stored(&fixture), Some(record(&available(None, NOW), None)));
        }

        #[test]
        fn redacted_run_records_the_post_policy_displayed_mode_or_null() {
            for observed_mode in [Some("auto"), None] {
                let (mut fixture, owner) = redacted_running();
                fixture.reserve(&owner);
                let observation = unavailable(observed_mode);
                let transition = unavailable_transition(&fixture, 1);
                let event = storage::persist_event_with_effective_policy(
                    &mut fixture.conn,
                    &owner,
                    &transition,
                    &observation,
                    PersistenceMode::Live,
                )
                .unwrap();
                assert_eq!(
                    stored(&fixture),
                    Some(record(&observation, Some(event.event_id)))
                );
                let shown = available(observed_mode, LATER);
                assert!(record_available(&mut fixture, &owner, &shown).unwrap());
                assert_eq!(stored(&fixture), Some(record(&shown, None)));
            }
        }

        #[test]
        fn available_update_writes_a_null_event_id_only_when_review_or_mode_changes() {
            let (mut fixture, owner) = redacted_running();
            let first = available(Some("auto"), NOW);
            assert!(record_available(&mut fixture, &owner, &first).unwrap());
            assert_eq!(stored(&fixture), Some(record(&first, None)));
            let committed = durable_state(&fixture.conn);
            for repeated in [
                available(Some("auto"), LATER),
                observation(
                    AutomaticReview::Available,
                    Some("auto"),
                    "codex-auto-review-badge",
                    LATER,
                ),
            ] {
                assert!(!record_available(&mut fixture, &owner, &repeated).unwrap());
                assert_eq!(durable_state(&fixture.conn), committed);
            }
            let changed_mode = available(Some("approve-for-me"), LATER);
            assert!(record_available(&mut fixture, &owner, &changed_mode).unwrap());
            assert_eq!(stored(&fixture), Some(record(&changed_mode, None)));
            let cleared_mode = available(None, LATER);
            assert!(record_available(&mut fixture, &owner, &cleared_mode).unwrap());
            assert_eq!(stored(&fixture), Some(record(&cleared_mode, None)));
            assert!(rows(&fixture.conn, "member_runtime_events").is_empty());
            assert!(rows(&fixture.conn, "messages").is_empty());
        }

        #[test]
        fn review_state_changes_alternate_between_event_carried_and_available_writes() {
            let (mut fixture, owner) = redacted_running();
            fixture.reserve(&owner);
            let first = available(Some("auto"), NOW);
            assert!(record_available(&mut fixture, &owner, &first).unwrap());
            let lost = unavailable(Some("auto"));
            let transition = unavailable_transition(&fixture, 1);
            let event = storage::persist_event_with_effective_policy(
                &mut fixture.conn,
                &owner,
                &transition,
                &lost,
                PersistenceMode::Live,
            )
            .unwrap();
            assert_eq!(stored(&fixture), Some(record(&lost, Some(event.event_id))));
            let regained = available(Some("auto"), LATER);
            assert!(record_available(&mut fixture, &owner, &regained).unwrap());
            assert_eq!(stored(&fixture), Some(record(&regained, None)));
        }

        #[test]
        fn available_update_rejects_an_unavailable_observation() {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            let before = durable_state(&fixture.conn);
            assert!(record_available(&mut fixture, &owner, &unavailable(None)).is_err());
            assert_eq!(durable_state(&fixture.conn), before);
        }

        #[test]
        fn available_update_commits_only_for_a_running_or_degraded_run() {
            for status in [
                RunStatus::Starting,
                RunStatus::Prepared,
                RunStatus::Running,
                RunStatus::Degraded,
                RunStatus::Stopping,
                RunStatus::Recovering,
                RunStatus::Exited,
                RunStatus::Failed,
                RunStatus::Lost,
            ] {
                let mut fixture = Fixture::new();
                let owner = fixture.activate();
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                        params![status.as_str(), owner.run_id],
                    )
                    .unwrap();
                let before = durable_state(&fixture.conn);
                let result = record_available(&mut fixture, &owner, &available(None, NOW));
                if matches!(status, RunStatus::Running | RunStatus::Degraded) {
                    assert!(result.unwrap(), "{status:?}");
                    assert_eq!(stored(&fixture), Some(record(&available(None, NOW), None)));
                } else {
                    assert_invariant(result, "member_runs.status");
                    assert_eq!(durable_state(&fixture.conn), before, "{status:?}");
                }
            }
        }

        #[test]
        fn superseded_runner_fails_with_the_stale_ownership_token_and_keeps_the_record() {
            let (mut fixture, stale) = redacted_running();
            let current = available(Some("auto"), NOW);
            assert!(record_available(&mut fixture, &stale, &current).unwrap());
            let fresh = fixture.activate_next_epoch();
            assert!(!record_available(&mut fixture, &fresh, &current).unwrap());
            let before = durable_state(&fixture.conn);
            let result = record_available(
                &mut fixture,
                &stale,
                &available(Some("approve-for-me"), LATER),
            );
            match result {
                Err(CafleetError::InvalidStoredValue { field, value }) => {
                    assert_eq!(field, "member_runs.owner");
                    assert_eq!(value, "stale ownership token");
                }
                other => panic!("expected the stale ownership token error, got {other:?}"),
            }
            assert_eq!(durable_state(&fixture.conn), before);
            assert_eq!(stored(&fixture), Some(record(&current, None)));
        }
    }
    #[test]
    fn effective_policy_reader_accepts_each_valid_shape() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let event = fixture.open(&owner);
        for record in [
            effective_policy("unavailable", None, Some(event.event_id)),
            effective_policy("unavailable", Some("auto"), Some(event.event_id)),
            effective_policy("available", None, None),
            effective_policy("available", Some("approve-for-me"), None),
        ] {
            store_effective_policy(&fixture, &record);
            assert_eq!(
                storage::get_run(&fixture.conn, owner.run_id)
                    .unwrap()
                    .effective_policy_json,
                Some(record)
            );
        }
    }

    #[test]
    fn effective_policy_reader_fails_on_a_malformed_record() {
        let fixture = Fixture::new();
        let valid = effective_policy("available", Some("auto"), None);
        let mut cases = Vec::new();
        for key in [
            "schema_version",
            "automatic_review",
            "observed_mode",
            "rule_id",
            "event_id",
            "observed_at",
        ] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            cases.push((format!("missing {key}"), missing));
        }
        for (label, key, value) in [
            ("schema version 2", "schema_version", json!(2)),
            ("schema version text", "schema_version", json!("1")),
            (
                "unknown review state",
                "automatic_review",
                json!("degraded"),
            ),
            ("review state null", "automatic_review", Value::Null),
            ("numeric mode", "observed_mode", json!(5)),
            ("empty rule", "rule_id", json!("")),
            ("null rule", "rule_id", Value::Null),
            ("text event", "event_id", json!("512")),
            ("non-timestamp", "observed_at", json!("yesterday")),
            ("null timestamp", "observed_at", Value::Null),
        ] {
            let mut changed = valid.clone();
            changed[key] = value;
            cases.push((label.to_owned(), changed));
        }
        let mut extra = valid.clone();
        extra["director_action"] = json!("untrusted");
        cases.push(("unknown key".to_owned(), extra));
        for (label, record) in cases {
            store_effective_policy(&fixture, &record);
            assert!(
                storage::get_run(&fixture.conn, fixture.run.run_id).is_err(),
                "{label}: {record}"
            );
        }
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET effective_policy_json=NULL WHERE run_id=?1",
                [fixture.run.run_id],
            )
            .unwrap();
        assert_eq!(
            storage::get_run(&fixture.conn, fixture.run.run_id)
                .unwrap()
                .effective_policy_json,
            None
        );
    }
}
