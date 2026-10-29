use super::*;

pub(super) fn reconcile(
    conn: &mut Connection,
    run: RunRecord,
    pinned: Option<Placement>,
    lock: Arc<RunLock>,
    host: &mut impl RecoveryHost,
    clock: ShutdownClock,
    until: Instant,
) -> Result<RecoveryCompletion> {
    let mut result = completion(run, RecoveryOutcome::Recovering)?;
    let attempt = perform(conn, &mut result, pinned.as_ref(), lock, host, clock, until);
    match attempt {
        Ok(()) => Ok(result),
        Err(cause)
            if matches!(cause, CafleetError::DeadlineExceeded) || Instant::now() >= until =>
        {
            result
                .report
                .cleanup_diagnostics
                .push(json!(cause.message()));
            Ok(result)
        }
        Err(cause) => Err(cause),
    }
}
fn refresh(conn: &mut Connection, result: &mut RecoveryCompletion, until: Instant) -> Result<()> {
    result.run = with_database_deadline(conn, until, |conn| {
        storage::get_run(conn, result.run.run_id)
    })?;
    result.report.status = result.run.status;
    Ok(())
}
fn count(
    result: &mut RecoveryCompletion,
    event: &RuntimeEventRecord,
    original: bool,
) -> Result<()> {
    fn increment(value: &mut u64) -> Result<()> {
        *value = value
            .checked_add(1)
            .ok_or_else(|| error("recovery commit count overflow"))?;
        Ok(())
    }
    if original {
        increment(&mut result.report.replayed_count)?;
    }
    match event.notification_state {
        NotificationState::Inbox => increment(&mut result.report.inbox_count),
        NotificationState::Archived => increment(&mut result.report.archived_count),
    }
}
fn persist(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    transition: &ObservedTransition,
    effective_policy: Option<&storage::EffectivePolicyObservation>,
    original: bool,
    host: &mut impl RecoveryHost,
    until: Instant,
) -> Result<()> {
    let (event, inserted) = with_database_deadline(conn, until, |conn| match effective_policy {
        Some(observation) => storage::persist_transition_with_effective_policy_counted(
            conn,
            &result.run.owner(),
            transition,
            observation,
            PersistenceMode::Recovery,
        ),
        None => storage::persist_transition_counted(
            conn,
            &result.run.owner(),
            transition,
            PersistenceMode::Recovery,
        ),
    })?;
    if inserted {
        count(result, &event, original)?;
    }
    refresh(conn, result, until)?;
    host.checkpoint(RecoveryBoundary::AfterCommit, &result.run, until)
}
pub(super) fn loss_transitions(
    conn: &Connection,
    run: &RunRecord,
    first: i64,
    missing: bool,
) -> Result<Vec<ObservedTransition>> {
    let snapshot = storage::runtime_snapshot(conn, run.run_id)?;
    let mut next = first;
    let now = crate::time::format_utc(crate::time::now_utc());
    let mut transitions = Vec::new();
    if let Some(interaction_id) = snapshot.current_interaction_id {
        let interaction = storage::get_event(conn, conn.query_row(
            "SELECT event_id FROM member_runtime_events WHERE interaction_id=?1 ORDER BY event_seq LIMIT 1",
            [interaction_id], |row| row.get::<_, i64>(0)).map_err(db_err)?)?.interaction
            .ok_or_else(|| error("pending recovery interaction missing"))?;
        transitions.push(ObservedTransition {
            run_id: run.run_id,
            event_seq: next,
            episode_seq: Some(interaction.episode_seq),
            kind: RuntimeEventKind::InteractionClosed,
            classification: Some(interaction.classification),
            observed_at: now.clone(),
            evidence_json: interaction.evidence_json,
            close_reason: Some(InteractionCloseReason::MonitorLost),
            cause_json: None,
        });
        next = next
            .checked_add(1)
            .ok_or_else(|| error("recovery sequence overflow"))?;
    }
    let evidence = storage::evidence::EvidencePolicy::metadata().apply(&json!({
        "backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,
        "rule_id":"runtime-recovery","screen_seq":0,"operation":null,"choices":null,"native_request_id":null,
        "excerpt":null,"truncated":false,"sensitive":false
    }))?;
    transitions.push(ObservedTransition {
        run_id: run.run_id,
        event_seq: next,
        episode_seq: None,
        kind: RuntimeEventKind::RunnerLost,
        classification: None,
        observed_at: now,
        evidence_json: evidence,
        close_reason: None,
        cause_json: Some(json!({"kind":if missing {"runtime_path_missing"} else {"monitor_lost"}})),
    });
    Ok(transitions)
}
pub(super) fn terminal_outcome(conn: &Connection, run: &RunRecord) -> Result<Option<RunStatus>> {
    let terminal: Option<String> = conn.query_row(
        "SELECT kind FROM member_runtime_events WHERE run_id=?1 AND kind IN ('runner_exited','runner_lost') ORDER BY event_seq DESC LIMIT 1",
        [run.run_id], |row| row.get(0)).optional().map_err(db_err)?;
    match terminal.as_deref() {
        Some("runner_exited") => Ok(Some(RunStatus::Exited)),
        Some("runner_lost") => Ok(Some(RunStatus::Lost)),
        Some(_) => Err(error("unexpected terminal recovery event")),
        None => Ok(run
            .recovery_origin_status
            .filter(|status| status.terminal())
            .or_else(|| run.status.terminal().then_some(run.status))),
    }
}
fn perform(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    pinned: Option<&Placement>,
    lock: Arc<RunLock>,
    host: &mut impl RecoveryHost,
    clock: ShutdownClock,
    until: Instant,
) -> Result<()> {
    host.checkpoint(RecoveryBoundary::AfterTakeover, &result.run, until)?;
    let mut descriptor = RecoveryDescriptor::pin(conn, &result.run, &lock, until)?;
    let mut outbox = Outbox::open_recovery(conn, &result.run, lock.clone(), until)?;
    let scan = outbox.scan_recovery(conn, until)?;
    if let Some(tail) = scan.incomplete_tail {
        result.report.cleanup_diagnostics.push(json!(tail));
    }
    for frame in scan.frames {
        with_database_deadline(conn, until, |conn| {
            check_snapshot(conn, &result.run, pinned)
        })?;
        host.checkpoint(RecoveryBoundary::BeforeReplay, &result.run, until)?;
        persist(
            conn,
            result,
            &frame.transition,
            frame.effective_policy.as_ref(),
            true,
            host,
            until,
        )?;
    }
    let existing = with_database_deadline(conn, until, |conn| terminal_outcome(conn, &result.run))?;
    let target = match existing {
        Some(status) => status,
        None => {
            let sequences = with_database_deadline(conn, until, |conn| {
                storage::reserve_event_sequences(conn, &result.run.owner())
            })?;
            refresh(conn, result, until)?;
            let transitions = with_database_deadline(conn, until, |conn| {
                loss_transitions(conn, &result.run, *sequences.start(), false)
            })?;
            for transition in transitions {
                outbox.append_recovery(conn, &transition, until)?;
                persist(conn, result, &transition, None, false, host, until)?;
            }
            RunStatus::Lost
        }
    };
    let cleanup = host.cleanup_owned(&result.run, clock)?;
    for diagnostic in &cleanup.diagnostics {
        result.report.cleanup_diagnostics.push(json!(diagnostic));
    }
    if cleanup.cleanup_state != CleanupState::Complete {
        result
            .report
            .cleanup_diagnostics
            .push(json!("owned recovery cleanup remains incomplete"));
        return Ok(());
    }
    host.checkpoint(RecoveryBoundary::BeforeCompact, &result.run, until)?;
    outbox.compact_committed(conn, &result.run.owner(), until)?;
    host.checkpoint(RecoveryBoundary::BeforeFinish, &result.run, until)?;
    with_database_deadline(conn, until, |conn| {
        check_snapshot(conn, &result.run, pinned)
    })?;
    descriptor.remove(conn, &result.run, &lock, until, |path, deadline| {
        host.unlink_descriptor(path, deadline)
    })?;
    result.run = with_database_deadline(conn, until, |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        check_snapshot(&tx, &result.run, pinned)?;
        lock.validate(&tx, &result.run.owner())?;
        storage::validate_owner(&tx, &result.run.owner())?;
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2),cleanup_diagnostics_json=?3 WHERE run_id=?4 AND owner_epoch=?5",
            params![target.as_str(),crate::time::format_utc(crate::time::now_utc()),serde_json::to_string(&result.report.cleanup_diagnostics).map_err(io_error)?,result.run.run_id,result.run.owner_epoch]).map_err(db_err)?;
        let current = storage::get_run(&tx, result.run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit().map_err(db_err)?;
        Ok(current)
    })?;
    result.report.status = result.run.status;
    result.report.outcome = RecoveryOutcome::Recovered;
    Ok(())
}

pub(super) fn count_exceptional(
    report: &mut RecoveryReport,
    event: &RuntimeEventRecord,
) -> Result<()> {
    let counter = match event.notification_state {
        NotificationState::Inbox => &mut report.inbox_count,
        NotificationState::Archived => &mut report.archived_count,
    };
    *counter = counter
        .checked_add(1)
        .ok_or_else(|| error("recovery commit count overflow"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::io::Write;

    use super::super::test_support::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::error::CafleetError;
    use crate::runtime::runner::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    mod history {
        use super::*;

        #[test]
        fn runner_recovery_archives_each_original_and_synthesized_event_by_current_lifecycle() {
            for (lifecycle, reason) in [
                ("member", "member_inactive"),
                ("director", "director_inactive"),
                ("fleet", "fleet_deleted"),
            ] {
                let mut case = Case::new();
                case.seed(&[
                    opening(&case.fixture.run, 1, 1),
                    obscured(&case.fixture.run, 2, 1),
                ]);
                match lifecycle {
                    "member" => {
                        crate::broker::deregister_member(
                            &mut case.fixture.conn,
                            case.fixture.run.member_id,
                        )
                        .unwrap();
                    }
                    "director" => {
                        case.fixture.conn.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",rusqlite::params![NOW,case.fixture.run.director_member_id]).unwrap();
                    }
                    "fleet" => {
                        case.fixture
                            .conn
                            .execute(
                                "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                                rusqlite::params![NOW, case.fixture.run.fleet_id],
                            )
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(counts(&report), (2, 0, 4), "{lifecycle}");
                let events = events(&case);
                assert_eq!(events.len(), 4);
                assert!(events.iter().all(|row| row.event.notification_state
                    == NotificationState::Archived
                    && row.event.message_id.is_none()
                    && row.event.archive_reason.unwrap().as_str() == reason));
                assert_eq!(
            case.fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE from_member_id=?1 AND to_member_id=?2",
                    rusqlite::params![
                        case.fixture.run.member_id,
                        case.fixture.run.director_member_id
                    ],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
            }
        }

        #[test]
        fn runner_recovery_lifecycle_change_between_actual_commits_selects_each_disposition_atomically()
         {
            let mut case = Case::new();
            case.seed(&[
                opening(&case.fixture.run, 1, 1),
                obscured(&case.fixture.run, 2, 1),
            ]);
            let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
            let member = case.fixture.run.member_id;
            let mut changed = false;
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, _, _| {
                if boundary == "after_commit" && !changed {
                    mutation.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",rusqlite::params![NOW,member]).unwrap();
                    changed = true;
                }
                Ok(())
            }));
            let report = case.recover(&mut host).unwrap();
            assert_eq!(counts(&report), (2, 1, 3));
            let rows = events(&case);
            assert_eq!(rows[0].event.notification_state, NotificationState::Inbox);
            assert!(rows[0].event.message_id.is_some());
            assert!(rows[1..].iter().all(|row| row.event.notification_state
                == NotificationState::Archived
                && row.event.message_id.is_none()
                && row.event.archive_reason == Some(ArchiveReason::MemberInactive)));
        }

        #[test]
        fn runner_recovery_preserves_committed_v1_bytes_ids_and_disposition_while_new_records_use_v2()
         {
            for archived in [false, true] {
                let mut case = Case::new();
                let original = opening(&case.fixture.run, 1, 1);
                let event = storage::persist_transition(
                    &mut case.fixture.conn,
                    &case.fixture.run.owner(),
                    &original,
                    PersistenceMode::Live,
                )
                .unwrap();
                let mut legacy = event.payload_json.clone();
                legacy["schema_version"] = json!(1);
                assert!(
                    legacy
                        .as_object_mut()
                        .unwrap()
                        .remove("director_action")
                        .is_some()
                );
                let payload = legacy.to_string();
                case.fixture
                    .conn
                    .execute(
                        "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
                        rusqlite::params![payload, event.event_id],
                    )
                    .unwrap();
                case.fixture
                    .conn
                    .execute(
                        "UPDATE messages SET text=?1 WHERE message_id=?2",
                        rusqlite::params![payload, event.message_id.unwrap()],
                    )
                    .unwrap();
                let before = storage::get_event(&case.fixture.conn, event.event_id)
                    .unwrap()
                    .event;
                let message =
                    crate::broker::get_message(&case.fixture.conn, before.message_id.unwrap())
                        .unwrap();
                let second = obscured(&case.fixture.run, 2, 1);
                case.seed(&[original, second]);
                if archived {
                    crate::broker::deregister_member(
                        &mut case.fixture.conn,
                        case.fixture.run.member_id,
                    )
                    .unwrap();
                }
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(
                    counts(&report),
                    if archived { (1, 0, 3) } else { (1, 3, 0) }
                );
                let rows = events(&case);
                assert_eq!(rows[0].event, before);
                assert_eq!(
                    crate::broker::get_message(&case.fixture.conn, before.message_id.unwrap())
                        .unwrap()
                        .text,
                    message.text
                );
                assert!(
                    rows[1..]
                        .iter()
                        .all(|row| row.event.payload_json["schema_version"] == 2)
                );
            }
        }

        #[test]
        fn runner_recovery_preserves_latest_evidence_receipts_response_audits_and_episode_claims() {
            use crate::coding_agent::{AnswerCapability, CodingAgent};
            let mut case = Case::new();
            case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET native_version='0.162.0',coverage='tested',profile_version='fixture-v1',profile_match='exact' WHERE run_id=?1",
            [case.fixture.run.run_id],
        )
        .unwrap();
            case.refresh();
            let owner = case.fixture.run.owner();
            let original = opening(&case.fixture.run, 1, 1);
            let event = storage::persist_transition(
                &mut case.fixture.conn,
                &owner,
                &original,
                PersistenceMode::Live,
            )
            .unwrap();
            let latest = evidence(&case.fixture.run, 77);
            storage::update_interaction_evidence(
                &mut case.fixture.conn,
                &owner,
                &InteractionEvidenceUpdate {
                    run_id: owner.run_id,
                    episode_seq: 1,
                    output_seq: 77,
                    evidence_json: latest.clone(),
                },
                1,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
            let input = storage::reserve_input(
                &mut case.fixture.conn,
                &owner,
                &NewInput {
                    operation_key: "prompt:retained-uncertainty".into(),
                    kind: InputKind::Prompt,
                    payload_digest: "d".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            assert_eq!(input.state, InputState::InProgress);
            storage::record_input_state(
                &mut case.fixture.conn,
                &owner,
                input.input_id,
                InputState::InProgress,
                InputState::Uncertain,
            )
            .unwrap();
            let capability = AnswerCapability::fixture(
                CodingAgent::Codex,
                case.fixture
                    .run
                    .native_version
                    .as_deref()
                    .expect("fixture run records its native version"),
                "answer-fixture-v1",
                "recovery-fixture",
                "single-request-choice",
                PermissionDecision::Allow,
                &["synthetic-storage-fixture".into()],
            )
            .unwrap();
            let request = NewPermissionResponse {
                operation_key: "permission:retained-claim".into(),
                metadata: PermissionResponseMetadata {
                    event_id: event.event_id,
                    interaction_id: event.interaction_id.unwrap(),
                    expected_owner_epoch: owner.owner_epoch,
                    request_token: "a".repeat(64),
                    decision: PermissionDecision::Allow,
                    director_member_id: case.fixture.run.director_member_id,
                    director_mode: DirectorMode::Auto,
                    decision_source: DecisionSource::DirectorAuto,
                    mode_evidence_ref: "fixture:mode".into(),
                    decision_evidence_ref: "fixture:decision".into(),
                    answer_profile_version: "answer-fixture-v1".into(),
                    answer_rule_id: "single-request-choice".into(),
                },
            };
            assert!(matches!(
                storage::reserve_permission_response(
                    &mut case.fixture.conn,
                    &owner,
                    &request,
                    Some(&capability),
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap(),
                PermissionReservation::Reserved(_)
            ));
            let snapshot = || {
                rows(&case.fixture.conn)
                    .into_iter()
                    .filter(|table| {
                        table.name == "member_input_operations"
                            || table.name.starts_with("member_permission_")
                    })
                    .collect::<Vec<_>>()
            };
            let before = snapshot();
            assert!(before.iter().all(|table| !table.rows.is_empty()));
            case.recover(&mut new_host()).unwrap();
            let after = rows(&case.fixture.conn)
                .into_iter()
                .filter(|table| {
                    table.name == "member_input_operations"
                        || table.name.starts_with("member_permission_")
                })
                .collect::<Vec<_>>();
            assert_eq!(after, before);
            let interaction = events(&case)[0].interaction.clone().unwrap();
            assert_eq!(interaction.evidence_json, latest);
            assert_eq!(
                interaction.close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            assert_eq!(
                storage::get_input(&case.fixture.conn, input.input_id)
                    .unwrap()
                    .state,
                InputState::Uncertain
            );
            assert_eq!(events(&case)[0].event.payload_json, event.payload_json);
        }
    }

    mod replay {
        use super::*;

        fn unavailable(run: &RunRecord, sequence: i64) -> ObservedTransition {
            ObservedTransition {
        run_id: run.run_id,
        event_seq: sequence,
        episode_seq: None,
        kind: RuntimeEventKind::AutomaticReviewUnavailable,
        classification: None,
        observed_at: NOW.into(),
        evidence_json: storage::evidence::EvidencePolicy::metadata().apply(&json!({"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"rule_id":"codex-auto-review-unavailable","screen_seq":sequence,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap(),
        close_reason: None,
        cause_json: None,
    }
        }

        #[test]
        fn runner_recovery_replays_an_unavailable_frame_with_its_effective_policy_record() {
            let mut case = Case::new();
            let value = unavailable(&case.fixture.run, 1);
            let lost = storage::EffectivePolicyObservation {
                automatic_review: storage::AutomaticReview::Unavailable,
                observed_mode: None,
                rule_id: "codex-auto-review-unavailable".into(),
                observed_at: NOW.into(),
            };
            {
                let deadline = Instant::now() + Duration::from_secs(1);
                let mut spool = Outbox::open_owned(
                    &case.fixture.conn,
                    &case.fixture.run,
                    case.held.as_ref().unwrap().clone(),
                    deadline,
                )
                .unwrap();
                spool
                    .append_synced_with_policy(&value, Some(&lost), deadline)
                    .unwrap();
            }
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            let rows = events(&case);
            assert_eq!(
                rows[0].event.kind,
                RuntimeEventKind::AutomaticReviewUnavailable
            );
            assert_eq!(rows[0].event.event_seq, value.event_seq);
            assert_eq!(rows[0].event.notification_state, NotificationState::Inbox);
            assert_eq!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .effective_policy_json,
                Some(json!({
                    "schema_version":1,"automatic_review":"unavailable","observed_mode":null,
                    "rule_id":"codex-auto-review-unavailable","event_id":rows[0].event.event_id,
                    "observed_at":NOW,
                }))
            );
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
        }

        #[test]
        fn runner_recovery_replays_a_committed_unavailable_frame_idempotently() {
            let mut case = Case::new();
            let value = unavailable(&case.fixture.run, 1);
            let lost = storage::EffectivePolicyObservation {
                automatic_review: storage::AutomaticReview::Unavailable,
                observed_mode: None,
                rule_id: "codex-auto-review-unavailable".into(),
                observed_at: NOW.into(),
            };
            {
                let deadline = Instant::now() + Duration::from_secs(1);
                let mut spool = Outbox::open_owned(
                    &case.fixture.conn,
                    &case.fixture.run,
                    case.held.as_ref().unwrap().clone(),
                    deadline,
                )
                .unwrap();
                spool
                    .append_synced_with_policy(&value, Some(&lost), deadline)
                    .unwrap();
            }
            let committed = storage::persist_transition_with_effective_policy(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &value,
                &lost,
                PersistenceMode::Live,
            )
            .unwrap();
            let record = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                .unwrap()
                .effective_policy_json;
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            let rows = events(&case);
            assert_eq!(rows[0].event.event_id, committed.event_id);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::AutomaticReviewUnavailable)
                    .count(),
                1
            );
            assert_eq!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .effective_policy_json,
                record
            );
        }

        #[test]
        fn runner_recovery_replays_opening_and_obscurity_before_one_monitor_closure_and_loss() {
            let mut case = Case::new();
            let opening = opening(&case.fixture.run, 1, 1);
            let obscured = obscured(&case.fixture.run, 2, 1);
            case.seed(&[opening.clone(), obscured.clone()]);
            let high_water = case.fixture.run.event_seq_reserved_through;
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            assert_eq!(counts(&report), (2, 4, 0));
            let rows = events(&case);
            assert_eq!(
                rows.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
                vec![
                    RuntimeEventKind::InteractionOpened,
                    RuntimeEventKind::InteractionObscured,
                    RuntimeEventKind::InteractionClosed,
                    RuntimeEventKind::RunnerLost
                ]
            );
            assert_eq!(rows[0].event.event_seq, opening.event_seq);
            assert_eq!(rows[1].event.event_seq, obscured.event_seq);
            assert!(rows[2].event.event_seq > high_water);
            assert!(rows[3].event.event_seq > rows[2].event.event_seq);
            assert_eq!(
                rows[2].interaction.as_ref().unwrap().close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            assert!(rows.iter().all(|row| row.event.notification_state
                == NotificationState::Inbox
                && row.event.message_id.is_some()));
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
            assert!(host.calls.iter().any(|name| name == "before_compact"));
        }

        #[test]
        fn runner_recovery_counts_only_new_commits_across_prefix_and_identical_duplicate_frames() {
            let mut case = Case::new();
            let prefix = opening(&case.fixture.run, 1, 1);
            storage::persist_transition(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &prefix,
                PersistenceMode::Live,
            )
            .unwrap();
            let second = obscured(&case.fixture.run, 2, 1);
            let third = obscured(&case.fixture.run, 3, 1);
            let mut bytes = frame(&prefix);
            bytes.extend(frame(&second));
            bytes.extend(frame(&second));
            bytes.extend(frame(&third));
            case.replace_owned_bytes(&bytes);
            let prefix_view = events(&case)[0].clone();
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (2, 4, 0));
            let rows = events(&case);
            assert_eq!(rows.len(), 5);
            assert_eq!(rows[0].event, prefix_view.event);
            assert_eq!(
                rows.iter().filter(|row| row.event.event_seq == 2).count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                    .count(),
                1
            );
            let again = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&again), (0, 0, 0));
            assert_eq!(events(&case), rows);
        }

        #[test]
        fn runner_recovery_validates_all_complete_frames_before_any_replay_and_preserves_corruption()
         {
            for damage in ["checksum", "conflict", "regression", "foreign_run"] {
                let mut case = Case::new();
                let first = opening(&case.fixture.run, 1, 1);
                let second = obscured(&case.fixture.run, 2, 1);
                let mut bytes = frame(&first);
                match damage {
                    "checksum" => {
                        let mut corrupt = frame(&second);
                        corrupt[4] ^= 1;
                        bytes.extend(corrupt);
                    }
                    "conflict" => {
                        let mut conflict = first.clone();
                        conflict.observed_at = "2026-10-26T20:00:01.000000+00:00".into();
                        bytes.extend(frame(&conflict));
                    }
                    "regression" => {
                        bytes = frame(&second);
                        bytes.extend(frame(&first));
                    }
                    "foreign_run" => {
                        let mut foreign = second.clone();
                        foreign.run_id += 1;
                        bytes.extend(frame(&foreign));
                    }
                    _ => unreachable!(),
                }
                case.replace_owned_bytes(&bytes);
                assert!(case.recover(&mut new_host()).is_err(), "{damage}");
                assert!(
                    events(&case).is_empty(),
                    "complete validation precedes replay: {damage}"
                );
                assert_eq!(case.spool_bytes().unwrap(), bytes, "{damage}");
            }
        }

        #[test]
        fn runner_recovery_incomplete_final_tail_replays_complete_originals_and_retains_a_diagnostic()
         {
            let mut case = Case::new();
            let first = opening(&case.fixture.run, 1, 1);
            let mut bytes = frame(&first);
            bytes.extend_from_slice(&[0, 0, 0, 40, 1, 2, 3]);
            case.replace_owned_bytes(&bytes);
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (1, 3, 0));
            assert_eq!(
                events(&case)[0].event.kind,
                RuntimeEventKind::InteractionOpened
            );
            assert!(!report.cleanup_diagnostics.is_empty());
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
        }

        #[test]
        fn runner_recovery_replayed_native_exit_preserves_exact_code_or_signal_without_fabricated_loss()
         {
            for (code, signal) in [(Some(7), None), (None, Some(15))] {
                let mut case = Case::new();
                case.seed(&[exited(&case.fixture.run, 1, code, signal)]);
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(report.status, RunStatus::Exited);
                assert_eq!(counts(&report), (1, 1, 0));
                case.refresh();
                assert_eq!(case.fixture.run.exit_code, code);
                assert_eq!(case.fixture.run.exit_signal, signal);
                assert_eq!(
                    events(&case)
                        .iter()
                        .map(|row| row.event.kind)
                        .collect::<Vec<_>>(),
                    vec![RuntimeEventKind::RunnerExited]
                );
            }
        }

        #[test]
        fn runner_recovery_preserves_recorded_terminal_outcomes_through_first_reconciliation() {
            for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
                let mut case = Case::new();
                let original_epoch = case.fixture.run.owner_epoch;
                case.fixture
            .conn
            .execute(
                "UPDATE member_runs SET status=?1,cleanup_state='incomplete' WHERE run_id=?2",
                rusqlite::params![status.as_str(), case.fixture.run.run_id],
            )
            .unwrap();
                case.refresh();
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(report.status, status);
                assert!(
                    events(&case)
                        .iter()
                        .all(|row| row.event.kind != RuntimeEventKind::RunnerLost)
                );
                case.refresh();
                if case.fixture.run.owner_epoch > original_epoch {
                    assert_eq!(case.fixture.run.recovery_origin_status, Some(status));
                } else {
                    assert_eq!(counts(&report), (0, 0, 0));
                }
            }
        }

        #[test]
        fn runner_recovery_sequence_allocation_uses_current_reserved_ceiling_and_complete_spool_maximum()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            storage::reserve_event_sequences(&mut case.fixture.conn, &case.fixture.run.owner())
                .unwrap();
            case.refresh();
            let ceiling = case.fixture.run.event_seq_reserved_through;
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (1, 3, 0));
            let rows = events(&case);
            assert!(rows[1..].iter().all(|row| row.event.event_seq > ceiling));
            assert!(
                rows.windows(2)
                    .all(|pair| pair[0].event.event_seq < pair[1].event.event_seq)
            );
        }

        #[test]
        fn runner_recovery_expiry_after_a_commit_retains_counts_and_next_pass_counts_only_new_work()
        {
            let mut case = Case::new();
            case.seed(&[
                opening(&case.fixture.run, 1, 1),
                obscured(&case.fixture.run, 2, 1),
            ]);
            let mut host = Host::new(Instant::now() + Duration::from_millis(300));
            host.action = Some(Box::new(|boundary, _, deadline| {
                if boundary == "after_commit" {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    return Err(CafleetError::DeadlineExceeded);
                }
                Ok(())
            }));
            let first = case.recover(&mut host).unwrap();
            assert!(Instant::now() >= host.deadline);
            assert!(matches!(first.outcome, RecoveryOutcome::Recovering));
            assert_eq!(counts(&first), (1, 1, 0));
            assert_eq!(events(&case).len(), 1);
            assert!(case.spool_bytes().is_some_and(|bytes| !bytes.is_empty()));
            let epoch = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                .unwrap()
                .owner_epoch;
            let second = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&second), (1, 3, 0));
            assert_eq!(events(&case).len(), 4);
            assert!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .owner_epoch
                    > epoch
            );
        }

        #[test]
        fn runner_recovery_interruption_boundaries_preserve_origin_and_idempotent_event_associations()
         {
            for stop in [
                "after_takeover",
                "before_replay",
                "after_commit",
                "before_compact",
                "before_finish",
            ] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let original = case.fixture.run.status;
                let mut host = new_host();
                let mut fired = false;
                host.action = Some(Box::new(move |boundary, _, _| {
                    if boundary == stop && !fired {
                        fired = true;
                        return Err(CafleetError::App("injected bounded interruption".into()));
                    }
                    Ok(())
                }));
                let _interrupted = case.recover(&mut host);
                assert!(host.calls.iter().any(|name| name == stop), "{stop}");
                let prior = events(&case);
                let result = case.recover(&mut new_host()).unwrap();
                assert_eq!(result.status, RunStatus::Lost, "{stop}");
                let final_rows = events(&case);
                assert_eq!(final_rows.len(), 3, "{stop}");
                for earlier in prior {
                    assert!(final_rows.iter().any(|row| row.event == earlier.event));
                }
                case.refresh();
                assert_eq!(case.fixture.run.recovery_origin_status, Some(original));
                assert_eq!(
                    final_rows
                        .iter()
                        .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                        .count(),
                    1
                );
            }
        }

        #[test]
        fn runner_recovery_interruption_before_compaction_retains_originals_and_committed_delivery()
        {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let original = case.spool_bytes().unwrap();
            let mut host = new_host();
            host.action = Some(Box::new(|boundary, _, _| {
                if boundary == "before_compact" {
                    Err(CafleetError::App("injected compaction refusal".into()))
                } else {
                    Ok(())
                }
            }));
            let result = case.recover(&mut host);
            assert!(
                result.is_err()
                    || result
                        .is_ok_and(|report| matches!(report.outcome, RecoveryOutcome::Recovering))
            );
            let retained = case.spool_bytes().unwrap();
            assert!(retained.starts_with(&original));
            let committed = events(&case);
            assert!(!committed.is_empty());
            case.recover(&mut new_host()).unwrap();
            assert_eq!(
                events(&case)
                    .into_iter()
                    .map(|row| row.event)
                    .collect::<Vec<_>>(),
                committed
                    .into_iter()
                    .map(|row| row.event)
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn runner_recovery_fsynced_synthesis_from_an_interrupted_pass_counts_as_next_pass_input() {
            for failed_kind in ["interaction_closed", "runner_lost"] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                case.fixture.conn.execute_batch(&format!("CREATE TRIGGER refuse_synthesis BEFORE INSERT ON member_runtime_events WHEN NEW.kind='{failed_kind}' BEGIN SELECT RAISE(ABORT,'injected synthesis persistence refusal'); END")).unwrap();
                let first = case.recover(&mut new_host());
                let committed = events(&case);
                let expected_commits = if failed_kind == "interaction_closed" {
                    1
                } else {
                    2
                };
                assert_eq!(committed.len(), expected_commits);
                if let Ok(report) = first {
                    assert!(matches!(report.outcome, RecoveryOutcome::Recovering));
                    assert_eq!(
                        counts(&report),
                        (1, u64::try_from(expected_commits).unwrap(), 0)
                    );
                }
                let bytes = case.spool_bytes().unwrap();
                let mut remaining = bytes.as_slice();
                let mut originals = Vec::<ObservedTransition>::new();
                while !remaining.is_empty() {
                    assert!(remaining.len() >= 36);
                    let size =
                        usize::try_from(u32::from_be_bytes(remaining[..4].try_into().unwrap()))
                            .unwrap();
                    assert!(remaining.len() >= 36 + size);
                    let body = &remaining[36..36 + size];
                    assert_eq!(&remaining[4..36], &Sha256::digest(body)[..]);
                    originals.push(serde_json::from_slice(body).unwrap());
                    remaining = &remaining[36 + size..];
                }
                assert_eq!(originals.last().unwrap().kind.as_str(), failed_kind);
                assert!(originals.last().unwrap().event_seq > 256);
                case.fixture
                    .conn
                    .execute_batch("DROP TRIGGER refuse_synthesis")
                    .unwrap();
                let next = case.recover(&mut new_host()).unwrap();
                assert_eq!(
                    counts(&next),
                    if failed_kind == "interaction_closed" {
                        (1, 2, 0)
                    } else {
                        (1, 1, 0)
                    }
                );
                let final_rows = events(&case);
                assert_eq!(final_rows.len(), 3);
                for earlier in committed {
                    assert!(final_rows.iter().any(|row| row.event == earlier.event));
                }
                assert_eq!(
                    final_rows
                        .iter()
                        .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                        .count(),
                    1
                );
            }
        }
    }

    mod terminal_tail {
        use super::*;
        use crate::input::PaneLock;

        fn terminal_case(status: RunStatus) -> Case {
            let mut case = Case::new();
            case.fixture.conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='incomplete',exit_code=?2,ended_at=?3 WHERE run_id=?4",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, case.fixture.run.run_id],
    ).unwrap();
            case.refresh();
            case
        }

        fn exclusive_host(case: &Case) -> Host {
            let mut host = new_host();
            let database_url = case.settings.database_url.clone();
            let path = case.fixture.conn.path().unwrap().to_owned();
            let placement = case.placement.clone();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary == "before_compact" {
                    let conn = Connection::open(&path).unwrap();
                    assert!(RunLock::acquire(&conn, &run.owner()).unwrap().is_none());
                    assert!(
                        PaneLock::try_acquire(&database_url, &placement)
                            .unwrap()
                            .is_none()
                    );
                }
                Ok(())
            }));
            host
        }

        fn assert_diagnosed_tail(report: &RecoveryReport) {
            assert!(report.cleanup_diagnostics.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.contains("incomplete outbox"))
            }));
        }

        #[test]
        fn runner_recovery_terminal_empty_incomplete_suffix_is_retired_without_new_events() {
            for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
                for suffix in [vec![0, 0], vec![0, 0, 0, 40, 1, 2, 3]] {
                    let mut case = terminal_case(status);
                    let original = case.fixture.run.clone();
                    case.replace_owned_bytes(&suffix);
                    let mut host = exclusive_host(&case);
                    let report = case.recover(&mut host).unwrap();
                    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
                    assert_eq!(report.status, status);
                    assert_eq!(counts(&report), (0, 0, 0));
                    assert_diagnosed_tail(&report);
                    assert!(events(&case).is_empty());
                    case.refresh();
                    assert_eq!(case.fixture.run.status, status);
                    assert_eq!(case.fixture.run.recovery_origin_status, Some(status));
                    assert_eq!(case.fixture.run.exit_code, original.exit_code);
                    assert_eq!(case.fixture.run.exit_signal, original.exit_signal);
                    assert_eq!(case.fixture.run.cleanup_state, CleanupState::Complete);
                    assert!(host.calls.iter().any(|name| name == "before_compact"));
                    assert!(
                        case.spool_bytes().is_none_or(|bytes| bytes.is_empty()),
                        "terminal {status:?} suffix remains after reported completion"
                    );
                    let committed = rows(&case.fixture.conn);
                    assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
                    assert_eq!(rows(&case.fixture.conn), committed);
                }
            }
        }

        #[test]
        fn runner_recovery_terminal_committed_originals_before_tail_keep_zero_new_counts_and_truthful_exit()
         {
            for (code, signal) in [(Some(7), None), (None, Some(15))] {
                let mut case = Case::new();
                let original = exited(&case.fixture.run, 1, code, signal);
                let event = storage::persist_transition(
                    &mut case.fixture.conn,
                    &case.fixture.run.owner(),
                    &original,
                    PersistenceMode::Live,
                )
                .unwrap();
                let message =
                    crate::broker::get_message(&case.fixture.conn, event.message_id.unwrap())
                        .unwrap();
                case.fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
                        [case.fixture.run.run_id],
                    )
                    .unwrap();
                case.refresh();
                let mut bytes = frame(&original);
                bytes.extend(frame(&original));
                bytes.extend([0, 0, 0, 40, 1, 2, 3]);
                case.replace_owned_bytes(&bytes);
                let mut host = exclusive_host(&case);
                let report = case.recover(&mut host).unwrap();
                assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
                assert_eq!(report.status, RunStatus::Exited);
                assert_eq!(counts(&report), (0, 0, 0));
                assert_diagnosed_tail(&report);
                let committed = events(&case);
                assert_eq!(committed.len(), 1);
                assert_eq!(committed[0].event, event);
                assert_eq!(
                    crate::broker::get_message(&case.fixture.conn, event.message_id.unwrap())
                        .unwrap()
                        .text,
                    message.text
                );
                case.refresh();
                assert_eq!(case.fixture.run.exit_code, code);
                assert_eq!(case.fixture.run.exit_signal, signal);
                assert_eq!(case.fixture.run.cleanup_state, CleanupState::Complete);
                assert!(host.calls.iter().any(|name| name == "before_compact"));
                assert!(
                    case.spool_bytes().is_none_or(|bytes| bytes.is_empty()),
                    "committed originals and incomplete suffix remain after reported completion"
                );
            }
        }

        #[test]
        fn runner_recovery_terminal_complete_corruption_before_tail_preserves_every_byte_and_commit()
         {
            let mut case = Case::new();
            let original = exited(&case.fixture.run, 1, Some(7), None);
            let event = storage::persist_transition(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &original,
                PersistenceMode::Live,
            )
            .unwrap();
            case.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
                    [case.fixture.run.run_id],
                )
                .unwrap();
            case.refresh();
            let mut bytes = frame(&original);
            let mut damaged = frame(&original);
            damaged[4] ^= 1;
            bytes.extend(damaged);
            bytes.extend([0, 0]);
            case.replace_owned_bytes(&bytes);
            assert!(case.recover(&mut new_host()).is_err());
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            let committed = events(&case);
            assert_eq!(committed.len(), 1);
            assert_eq!(committed[0].event, event);
        }
    }

    mod descriptor {
        use super::*;
        use crate::runtime::runner::recovery::test_support::descriptor::*;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        };
        #[test]
        fn runner_recovery_review_r4_verified_creator_recovery_removes_its_private_descriptor() {
            let mut case = described_creator();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            assert_eq!(counts(&report), (0, 1, 0));
            secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
            assert!(
                !case.artifacts.paths.descriptor.exists(),
                "successful creator cleanup removes its launch descriptor"
            );
        }

        #[test]
        fn runner_recovery_review_r4_historical_descriptor_survives_reconciler_retries_until_verified_removal()
         {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let original_epoch = case.fixture.run.owner_epoch;
            for increment in 1..=2 {
                let mut host = new_host();
                host.action = Some(Box::new(|boundary, _, _| {
                    if boundary == "before_finish" {
                        return Err(CafleetError::App(
                            "owned descriptor completion interruption".into(),
                        ));
                    }
                    Ok(())
                }));
                assert!(case.recover(&mut host).is_err());
                assert!(host.calls.iter().any(|name| name == "before_finish"));
                case.refresh();
                assert_eq!(case.fixture.run.owner_epoch, original_epoch + increment);
                assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
                assert!(
                    owned_bytes(&case) == bytes,
                    "interrupted cleanup retains its original descriptor"
                );
                assert_eq!(events(&case).len(), 1);
            }
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(counts(&report), (0, 0, 0));
            assert_eq!(events(&case).len(), 1);
            assert!(
                !case.artifacts.paths.descriptor.exists(),
                "a positive historical epoch remains removable after legitimate takeovers"
            );
        }

        #[test]
        fn runner_recovery_review_r4_absent_descriptor_and_completed_retry_are_idempotent() {
            let mut case = Case::creator();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert!(!case.artifacts.paths.descriptor.exists());
            let before = rows(&case.fixture.conn);
            assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), before);
        }

        #[derive(Clone, Copy)]
        enum Mutation {
            Replacement,
            Symlink,
            Content,
            Appeared,
            Disappeared,
            ForeignOwner,
        }
        fn changed_after_pin(mutation: Mutation) {
            let mut case = described_creator();
            let original = owned_bytes(&case);
            if matches!(mutation, Mutation::Appeared) {
                case.artifacts.remove_descriptor().unwrap();
            }
            let path = case.artifacts.paths.descriptor.clone();
            let retained = case.artifacts.paths.run_dir.join("retained-launch.json");
            let changed = Arc::new(Mutex::new(None));
            let published = changed.clone();
            let database = case.fixture.conn.path().unwrap().to_owned();
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary != "before_finish" {
                    return Ok(());
                }
                let conn = Connection::open(&database).unwrap();
                assert!(RunLock::acquire(&conn, &run.owner()).unwrap().is_none());
                match mutation {
                    Mutation::Replacement => {
                        std::fs::rename(&path, &retained).unwrap();
                        write_private(&path, &original);
                        assert_ne!(identity(&path).1, identity(&retained).1);
                        *published.lock().unwrap() = Some(original.clone());
                    }
                    Mutation::Symlink => {
                        std::fs::rename(&path, &retained).unwrap();
                        std::os::unix::fs::symlink(&retained, &path).unwrap();
                        *published.lock().unwrap() = Some(original.clone());
                    }
                    Mutation::Content => {
                        let before = identity(&path);
                        let mut value: Value = serde_json::from_slice(&original).unwrap();
                        value["spawn"]["argv"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!("owned-in-place-change"));
                        let bytes = serde_json::to_vec(&value).unwrap();
                        let mut file = std::fs::OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&path)
                            .unwrap();
                        file.write_all(&bytes).unwrap();
                        file.sync_all().unwrap();
                        assert_eq!((identity(&path).0, identity(&path).1), (before.0, before.1));
                        *published.lock().unwrap() = Some(bytes);
                    }
                    Mutation::Appeared => {
                        write_private(&path, &original);
                        *published.lock().unwrap() = Some(original.clone());
                    }
                    Mutation::Disappeared => {
                        std::fs::remove_file(&path).unwrap();
                    }
                    Mutation::ForeignOwner => {
                        conn.execute(
                            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                            [run.run_id],
                        )
                        .unwrap();
                        *published.lock().unwrap() = Some(original.clone());
                    }
                }
                Ok(())
            }));
            let result = case.recover(&mut host);
            assert!(host.calls.iter().any(|name| name == "before_finish"));
            if matches!(mutation, Mutation::Disappeared) {
                assert!(matches!(
                    result.unwrap().outcome,
                    RecoveryOutcome::Recovered
                ));
                assert!(!case.artifacts.paths.descriptor.exists());
                return;
            }
            incomplete(&case, result);
            if matches!(mutation, Mutation::Symlink) {
                assert!(
                    std::fs::symlink_metadata(&case.artifacts.paths.descriptor)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
                let lock = RunLock::acquire_released(&case.fixture.conn, &run.owner())
                    .unwrap()
                    .unwrap();
                lock.validate(&case.fixture.conn, &run.owner()).unwrap();
                assert!(
                    std::fs::read(case.artifacts.paths.run_dir.join("retained-launch.json"))
                        .unwrap()
                        == *changed.lock().unwrap().as_ref().unwrap(),
                    "the symlink target remains unchanged"
                );
            } else {
                assert!(
                    owned_bytes(&case) == *changed.lock().unwrap().as_ref().unwrap(),
                    "the refused descriptor remains unchanged"
                );
            }
        }
        #[test]
        fn runner_recovery_review_r4_replaced_descriptor_after_pin_is_preserved_with_incomplete_cleanup()
         {
            changed_after_pin(Mutation::Replacement);
        }
        #[test]
        fn runner_recovery_review_r4_symlink_after_pin_preserves_link_and_target_with_incomplete_cleanup()
         {
            changed_after_pin(Mutation::Symlink);
        }
        #[test]
        fn runner_recovery_review_r4_in_place_descriptor_change_after_pin_is_preserved() {
            changed_after_pin(Mutation::Content);
        }
        #[test]
        fn runner_recovery_review_r4_descriptor_appearing_after_absence_pin_is_preserved() {
            changed_after_pin(Mutation::Appeared);
        }
        #[test]
        fn runner_recovery_review_r4_descriptor_disappearing_after_pin_is_safe_absence() {
            changed_after_pin(Mutation::Disappeared);
        }
        #[test]
        fn runner_recovery_review_r4_foreign_epoch_before_unlink_preserves_descriptor() {
            changed_after_pin(Mutation::ForeignOwner);
        }

        #[test]
        fn runner_recovery_review_r4_invalid_descriptor_identity_epoch_shape_or_permissions_fails_privately()
         {
            for damage in [
                "schema",
                "database",
                "fleet",
                "member",
                "director",
                "run",
                "nonce",
                "zero_epoch",
                "future_epoch",
                "unknown_field",
                "json",
                "size",
                "permissions",
                "symlink",
            ] {
                let mut case = described_creator();
                let mut value: Value = serde_json::from_slice(&owned_bytes(&case)).unwrap();
                match damage {
                    "schema" => value["schema_version"] = json!(2),
                    "database" => {
                        value["database_identity_json"]["instance_id"] = json!("0".repeat(32))
                    }
                    "fleet" => value["fleet_id"] = json!(case.fixture.run.fleet_id + 1),
                    "member" => value["member_id"] = json!(case.fixture.run.member_id + 1),
                    "director" => {
                        value["director_member_id"] = json!(case.fixture.run.director_member_id + 1)
                    }
                    "run" => value["run_id"] = json!(case.fixture.run.run_id + 1),
                    "nonce" => value["launch_nonce"] = json!("b".repeat(64)),
                    "zero_epoch" => value["owner_epoch"] = json!(0),
                    "future_epoch" => {
                        value["owner_epoch"] = json!(case.fixture.run.owner_epoch + 10)
                    }
                    "unknown_field" => value["unexpected"] = json!(true),
                    "json" | "size" | "permissions" | "symlink" => {}
                    _ => unreachable!(),
                }
                let bytes = match damage {
                    "json" => b"{".to_vec(),
                    "size" => vec![b' '; 1024 * 1024 + 1],
                    _ => serde_json::to_vec(&value).unwrap(),
                };
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&case.artifacts.paths.descriptor)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                if damage == "permissions" {
                    std::fs::set_permissions(
                        &case.artifacts.paths.descriptor,
                        std::fs::Permissions::from_mode(0o644),
                    )
                    .unwrap();
                }
                if damage == "symlink" {
                    let retained = case.artifacts.paths.run_dir.join("invalid-target.json");
                    std::fs::rename(&case.artifacts.paths.descriptor, &retained).unwrap();
                    std::os::unix::fs::symlink(&retained, &case.artifacts.paths.descriptor)
                        .unwrap();
                }
                let before = identity(&case.artifacts.paths.descriptor);
                let result = case.recover(&mut new_host());
                incomplete(&case, result);
                assert_eq!(identity(&case.artifacts.paths.descriptor), before);
            }
        }

        #[test]
        fn runner_recovery_review_r4_original_deadline_after_before_finish_keeps_descriptor_retryable()
         {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let mut host = Host::new(Instant::now() + Duration::from_millis(500));
            let reached = Arc::new(AtomicBool::new(false));
            let observed = reached.clone();
            host.action = Some(Box::new(move |boundary, _, deadline| {
                if boundary == "before_finish" {
                    observed.store(true, Ordering::Release);
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                Ok(())
            }));
            let result = case.recover(&mut host);
            assert!(reached.load(Ordering::Acquire));
            assert!(Instant::now() >= host.deadline);
            incomplete(&case, result);
            assert!(
                owned_bytes(&case) == bytes,
                "expiry leaves the descriptor retryable"
            );
        }

        #[test]
        fn runner_recovery_review_r4_exceptional_database_only_reconciliation_preserves_suspect_descriptor()
         {
            let mut case = described_creator();
            let before = identity(&case.artifacts.paths.descriptor);
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(identity(&case.artifacts.paths.descriptor), before);
            secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
        }
    }

    mod descriptor_cleanup {
        use super::*;
        use crate::runtime::runner::recovery::test_support::descriptor::recover;
        use crate::runtime::runner::recovery::test_support::descriptor::*;
        #[test]
        fn runner_recovery_review_r4_coordinator_propagates_actual_unlink_failure_and_retries_historical_epoch()
         {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let mut failed =
                Host::with_unlink(&case, Instant::now() + Duration::from_secs(2), true);
            let result = recover(&mut case, &mut failed);
            assert_eq!(
                failed.unlink.as_ref().unwrap().calls,
                1,
                "the real coordinator reaches the approved unlink operation"
            );
            incomplete(&case, result);
            assert!(
                owned_bytes(&case) == bytes,
                "the failed dispatch preserves its selected descriptor"
            );
            let first = events(&case)
                .into_iter()
                .map(|view| view.event)
                .collect::<Vec<_>>();
            assert_eq!(first.len(), 1);
            let epoch = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                .unwrap()
                .owner_epoch;
            let mut retry =
                Host::with_unlink(&case, Instant::now() + Duration::from_secs(2), false);
            let report = recover(&mut case, &mut retry).unwrap();
            assert_eq!(retry.unlink.as_ref().unwrap().calls, 1);
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(counts(&report), (0, 0, 0));
            assert_eq!(
                events(&case)
                    .into_iter()
                    .map(|view| view.event)
                    .collect::<Vec<_>>(),
                first
            );
            assert_eq!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .owner_epoch,
                epoch + 1
            );
            assert!(!case.artifacts.paths.descriptor.exists());
        }

        #[test]
        fn runner_recovery_review_r4_expired_original_budget_prevents_actual_unlink_dispatch() {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let mut host =
                Host::with_unlink(&case, Instant::now() + Duration::from_millis(500), false);
            host.action = Some(Box::new(|boundary, _, deadline| {
                if boundary == "before_finish" {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                Ok(())
            }));
            let result = recover(&mut case, &mut host);
            assert!(host.calls.iter().any(|name| name == "before_finish"));
            assert!(Instant::now() >= host.deadline);
            assert_eq!(
                host.unlink.as_ref().unwrap().calls,
                0,
                "the expired deadline is checked before forwarding the unlink seam"
            );
            incomplete(&case, result);
            assert!(
                owned_bytes(&case) == bytes,
                "expiry preserves the selected descriptor"
            );
        }
    }

    mod relationships {
        use super::*;
        use crate::runtime::runner::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_review_r5_finalization_after_replay_refuses_actual_member_fleet_change_atomically()
         {
            refusal(Stage::Finalization, Relationship::MemberFleet);
        }
        #[test]
        fn runner_recovery_review_r5_finalization_after_replay_refuses_actual_director_fleet_change_atomically()
         {
            refusal(Stage::Finalization, Relationship::DirectorFleet);
        }
        #[test]
        fn runner_recovery_review_r5_finalization_after_replay_refuses_actual_fleet_director_change_atomically()
         {
            refusal(Stage::Finalization, Relationship::FleetDirector);
        }
        #[test]
        fn runner_recovery_review_r5_valid_historical_lifecycle_change_after_replay_allows_finalization()
         {
            historical_control(Stage::Finalization);
        }

        #[test]
        fn runner_recovery_review_r5_valid_historical_replay_selects_current_archive_disposition() {
            for lifecycle in [
                Lifecycle::MemberInactive,
                Lifecycle::DirectorInactive,
                Lifecycle::FleetDeleted,
            ] {
                let mut case = prepare(Stage::Finalization);
                let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
                let mut changed = false;
                let mut host = new_host();
                host.action = Some(Box::new(move |boundary, run, _| {
                    if boundary == "before_takeover" {
                        assert!(!changed);
                        lifecycle.apply(&mutation, run);
                        changed = true;
                    }
                    Ok(())
                }));
                let report = invoke(&mut case, &mut host).unwrap();
                assert!(host.calls.iter().any(|call| call == "before_replay"));
                assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
                assert_eq!(report.status, RunStatus::Exited);
                assert_eq!(counts(&report), (1, 0, 1));
                let recorded = events(&case);
                assert_eq!(recorded.len(), 1);
                assert_eq!(recorded[0].event.kind, RuntimeEventKind::RunnerExited);
                assert_eq!(
                    recorded[0].event.notification_state,
                    NotificationState::Archived
                );
                assert_eq!(
                    recorded[0].event.archive_reason,
                    Some(lifecycle.archive_reason())
                );
                assert_eq!(recorded[0].event.message_id, None);
                assert!(
                    rows(&case.fixture.conn)
                        .into_iter()
                        .find(|table| table.name == "messages")
                        .unwrap()
                        .rows
                        .is_empty()
                );
            }
        }
    }
}
