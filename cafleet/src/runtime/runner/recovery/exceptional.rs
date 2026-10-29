use super::*;

pub(super) fn reconcile(
    conn: &mut Connection,
    expected: &RunRecord,
    pinned: Option<&Placement>,
    process: &ProcessIdentity,
    attestation: Option<&OwnerTerminationAttestation>,
    until: Instant,
    check: impl FnOnce(&Connection) -> Result<()>,
) -> Result<RecoveryCompletion> {
    with_database_deadline(conn, until, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check(&tx)?;
        let current = check_snapshot(&tx, expected, pinned)?;
        validate_attestation(&current, attestation)?;
        if current.status.terminal() && current.cleanup_state == CleanupState::Complete {
            check_deadline(Instant::now(), until)?;
            tx.commit()?;
            return completion(current, RecoveryOutcome::Recovered);
        }
        let established = super::replay::terminal_outcome(&tx, &current)?;
        let mut run = advance_recovery_owner(&tx, &current, process)?;
        let mut report = completion(run.clone(), RecoveryOutcome::Recovering)?.report;
        report
            .cleanup_diagnostics
            .push(json!("runtime_path_missing"));
        if let Some(claim) = attestation {
            report.cleanup_diagnostics.push(json!({"kind":"owner_termination_attestation",
                "run_id":current.run_id,"owner_epoch":current.owner_epoch,"owner_pid":current.owner_pid,
                "owner_start_identity":current.owner_start_identity,"evidence_ref":claim.evidence_ref,
                "recorded_at":crate::time::now_stamp(),"source":"operator_via_director"}));
        }
        tx.execute("UPDATE member_runs SET observation_gap_count=observation_gap_count+1,cleanup_diagnostics_json=?1 WHERE run_id=?2",
            params![serde_json::to_string(&report.cleanup_diagnostics).map_err(io_error)?,run.run_id])?;
        let target = match established {
            Some(status) => status,
            None => {
                let maximum: Option<i64> = tx.query_row(
                    "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
                    [run.run_id],
                    |row| row.get(0),
                )?;
                let ceiling = maximum.map_or(run.event_seq_reserved_through, |value| {
                    value.max(run.event_seq_reserved_through)
                });
                let first = ceiling
                    .checked_add(1)
                    .ok_or_else(|| error("recovery sequence overflow"))?;
                let reserved = ceiling
                    .checked_add(256)
                    .ok_or_else(|| error("recovery sequence overflow"))?;
                tx.execute(
                    "UPDATE member_runs SET event_seq_reserved_through=?1 WHERE run_id=?2",
                    params![reserved, run.run_id],
                )?;
                run = storage::get_run(&tx, run.run_id)?;
                for transition in super::replay::loss_transitions(&tx, &run, first, true)? {
                    let (event, inserted) = storage::persist_transition_in_transaction(
                        &tx,
                        &run,
                        &transition,
                        PersistenceMode::Recovery,
                    )?;
                    ensure(
                        inserted,
                        "exceptional recovery requires fresh reserved transitions",
                    )?;
                    super::replay::count_exceptional(&mut report, &event)?;
                }
                RunStatus::Lost
            }
        };
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2) WHERE run_id=?3",
            params![target.as_str(),crate::time::now_stamp(),run.run_id])?;
        let run = storage::get_run(&tx, run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit()?;
        report.outcome = RecoveryOutcome::Recovered;
        report.status = run.status;
        Ok(RecoveryCompletion { report, run })
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::{Value, json};

    use super::super::test_support::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::db::test_support::TableSnapshot;
    use crate::error::CafleetError;
    use crate::runtime::runner::*;
    use std::time::{Duration, Instant};

    mod history {
        use super::*;
        use std::os::unix::fs::MetadataExt;
        use std::path::PathBuf;

        struct ArtifactWitness {
            path: PathBuf,
            identity: (u64, u64, u64, i64, i64),
        }
        fn metadata(path: &std::path::Path) -> (u64, u64, u64, i64, i64) {
            let value = std::fs::symlink_metadata(path).unwrap();
            (
                value.dev(),
                value.ino(),
                value.len(),
                value.mtime(),
                value.mtime_nsec(),
            )
        }
        fn lose_artifacts(case: &mut Case, directory: bool) -> ArtifactWitness {
            let identity = metadata(&case.artifacts.paths.outbox);
            case.release();
            let path = if directory {
                let retained = case.fixture._dir.path().join("retained-runtime");
                std::fs::rename(&case.artifacts.paths.run_dir, &retained).unwrap();
                retained.join("events.outbox")
            } else {
                std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
                case.artifacts.paths.outbox.clone()
            };
            ArtifactWitness { path, identity }
        }
        fn history(case: &Case) -> Vec<TableSnapshot> {
            rows(&case.fixture.conn)
                .into_iter()
                .filter(|table| {
                    matches!(
                        table.name.as_str(),
                        "messages"
                            | "member_runtime_events"
                            | "member_interactions"
                            | "member_input_operations"
                    ) || table.name.starts_with("member_permission_")
                })
                .collect()
        }
        fn original_terminal(status: RunStatus) -> Case {
            let mut case = Case::new();
            let event = ObservedTransition {
                run_id: case.fixture.run.run_id,
                event_seq: 1,
                episode_seq: None,
                kind: RuntimeEventKind::CoverageUnknown,
                classification: None,
                observed_at: NOW.into(),
                evidence_json: evidence(&case.fixture.run, 1),
                close_reason: None,
                cause_json: Some(json!({"kind":"coverage_unknown"})),
            };
            storage::persist_transition(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &event,
                PersistenceMode::Live,
            )
            .unwrap();
            case.fixture.conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='incomplete',exit_code=?2,ended_at=?3,cleanup_diagnostics_json=?4 WHERE run_id=?5",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, json!([{"kind":"retained_terminal_history"}]).to_string(), case.fixture.run.run_id],
    ).unwrap();
            case.refresh();
            case
        }
        fn recover_missing(case: &mut Case, operator: bool) -> TestResult<RecoveryReport> {
            let claim = operator.then(|| OwnerTerminationAttestation {
                expected_run_id: case.fixture.run.run_id,
                expected_owner_epoch: case.fixture.run.owner_epoch,
                evidence_ref: "audit:established-terminal-history".into(),
            });
            let mut host = new_host();
            if operator {
                host.process = Evidence::Unknown;
            }
            let result = recover_member_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                claim.as_ref(),
                host.deadline,
                &mut host,
            );
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|name| matches!(name.as_str(), "before_replay" | "cleanup"))
            );
            result
        }
        fn assert_terminal_bookkeeping(
            case: &Case,
            report: &RecoveryReport,
            before: &RunRecord,
            status: RunStatus,
            prior: &[TableSnapshot],
            artifact: &ArtifactWitness,
            operator: bool,
        ) {
            let current = storage::get_run(&case.fixture.conn, before.run_id).unwrap();
            eprintln!(
                "exceptional terminal history: expected={status:?}, actual={:?}, counts={:?}, epoch={} -> {}, gap={} -> {}, events={}",
                report.status,
                counts(report),
                before.owner_epoch,
                current.owner_epoch,
                before.observation_gap_count,
                current.observation_gap_count,
                events(case).len()
            );
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, status);
            assert_eq!(current.status, status);
            assert_eq!(counts(report), (0, 0, 0));
            assert_eq!(current.owner_epoch, before.owner_epoch + 1);
            assert_eq!(current.owner_kind, OwnerKind::Reconciler);
            let process = ProcessIdentity::current().unwrap();
            assert_eq!(current.owner_pid, process.pid);
            assert_eq!(current.owner_start_identity, process.start_identity);
            assert_eq!(
                current.observation_gap_count,
                before.observation_gap_count + 1
            );
            assert_eq!(
                current.event_seq_reserved_through,
                before.event_seq_reserved_through
            );
            assert_eq!(current.exit_code, before.exit_code);
            assert_eq!(current.exit_signal, before.exit_signal);
            assert_eq!(current.cleanup_state, CleanupState::Complete);
            assert_eq!(
                current.recovery_origin_status,
                before.recovery_origin_status.or(Some(before.status))
            );
            assert_eq!(history(case), prior);
            assert_eq!(
                metadata(&artifact.path),
                artifact.identity,
                "database-only bookkeeping preserves surviving spool identity and metadata"
            );
            let diagnostics = current
                .cleanup_diagnostics_json
                .as_ref()
                .unwrap()
                .to_string();
            assert!(diagnostics.contains("runtime_path_missing"));
            assert_eq!(diagnostics.contains("operator_via_director"), operator);
            if operator {
                assert!(diagnostics.contains("audit:established-terminal-history"));
            }
        }

        fn original_history_cases(operator: bool) {
            for status in [RunStatus::Exited, RunStatus::Failed] {
                for directory in [false, true] {
                    let mut case = original_terminal(status);
                    let before = case.fixture.run.clone();
                    let prior = history(&case);
                    let artifact = lose_artifacts(&mut case, directory);
                    let report = recover_missing(&mut case, operator).unwrap();
                    assert_terminal_bookkeeping(
                        &case, &report, &before, status, &prior, &artifact, operator,
                    );
                    assert!(
                        storage::get_run(&case.fixture.conn, before.run_id)
                            .unwrap()
                            .cleanup_diagnostics_json
                            .unwrap()
                            .to_string()
                            .contains("retained_terminal_history")
                    );
                }
            }
        }
        #[test]
        fn runner_recovery_review_r3_machine_missing_paths_preserve_original_exited_and_failed_history()
         {
            original_history_cases(false);
        }
        #[test]
        fn runner_recovery_review_r3_operator_missing_paths_preserve_original_exited_and_failed_history()
         {
            original_history_cases(true);
        }

        fn committed_exit_cases(operator: bool) {
            for (code, signal) in [(Some(7), None), (None, Some(15))] {
                for directory in [false, true] {
                    let mut case = Case::new();
                    let transition = exited(&case.fixture.run, 1, code, signal);
                    storage::persist_transition(
                        &mut case.fixture.conn,
                        &case.fixture.run.owner(),
                        &transition,
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
                    let before = case.fixture.run.clone();
                    let prior = history(&case);
                    let artifact = lose_artifacts(&mut case, directory);
                    let report = recover_missing(&mut case, operator).unwrap();
                    assert_terminal_bookkeeping(
                        &case,
                        &report,
                        &before,
                        RunStatus::Exited,
                        &prior,
                        &artifact,
                        operator,
                    );
                    assert_eq!(events(&case).len(), 1);
                }
            }
        }
        #[test]
        fn runner_recovery_review_r3_machine_missing_paths_keep_committed_native_exit_code_signal_and_message()
         {
            committed_exit_cases(false);
        }
        #[test]
        fn runner_recovery_review_r3_operator_missing_paths_keep_committed_native_exit_code_signal_and_message()
         {
            committed_exit_cases(true);
        }

        fn interrupted_loss_cases(operator: bool) {
            for directory in [false, true] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let mut first = new_host();
                first.action = Some(Box::new(|boundary, _, _| {
                    if boundary == "before_finish" {
                        return Err(CafleetError::App(
                            "owned interruption after committed terminal history".into(),
                        ));
                    }
                    Ok(())
                }));
                assert!(case.recover(&mut first).is_err());
                assert!(first.calls.iter().any(|name| name == "before_finish"));
                let committed = events(&case);
                assert_eq!(
                    committed
                        .iter()
                        .map(|view| view.event.kind)
                        .collect::<Vec<_>>(),
                    vec![
                        RuntimeEventKind::InteractionOpened,
                        RuntimeEventKind::InteractionClosed,
                        RuntimeEventKind::RunnerLost
                    ]
                );
                case.refresh();
                assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
                let before = case.fixture.run.clone();
                let prior = history(&case);
                let artifact = lose_artifacts(&mut case, directory);
                let report = recover_missing(&mut case, operator).unwrap();
                assert_terminal_bookkeeping(
                    &case,
                    &report,
                    &before,
                    RunStatus::Lost,
                    &prior,
                    &artifact,
                    operator,
                );
                assert_eq!(events(&case).len(), 3);
                let complete = rows(&case.fixture.conn);
                assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
                assert_eq!(rows(&case.fixture.conn), complete);
            }
        }
        #[test]
        fn runner_recovery_review_r3_machine_artifact_loss_after_committed_runner_lost_creates_no_duplicate()
         {
            interrupted_loss_cases(false);
        }
        #[test]
        fn runner_recovery_review_r3_operator_artifact_loss_after_committed_runner_lost_creates_no_duplicate()
         {
            interrupted_loss_cases(true);
        }

        #[test]
        fn runner_recovery_review_r3_terminal_bookkeeping_faults_rollback_atomically_then_retry_without_events()
         {
            for fault in ["owner", "diagnostic", "gap", "finish", "commit"] {
                let mut case = original_terminal(RunStatus::Failed);
                let before_run = case.fixture.run.clone();
                let prior = history(&case);
                let artifact = lose_artifacts(&mut case, false);
                let trigger = match fault {
                    "owner" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF owner_epoch ON member_runs BEGIN SELECT RAISE(ABORT,'owned history owner fault'); END",
                    ),
                    "diagnostic" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs BEGIN SELECT RAISE(ABORT,'owned history diagnostic fault'); END",
                    ),
                    "gap" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF observation_gap_count ON member_runs BEGIN SELECT RAISE(ABORT,'owned history gap fault'); END",
                    ),
                    "finish" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF cleanup_state ON member_runs WHEN NEW.cleanup_state='complete' BEGIN SELECT RAISE(ABORT,'owned history finish fault'); END",
                    ),
                    "commit" => None,
                    _ => unreachable!(),
                };
                if let Some(trigger) = trigger {
                    case.fixture.conn.execute_batch(trigger).unwrap();
                }
                let before = rows(&case.fixture.conn);
                if fault == "commit" {
                    case.fixture.conn.commit_hook(Some(|| true)).unwrap();
                }
                let result = recover_missing(&mut case, true);
                case.fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
                assert!(result.is_err(), "{fault}");
                assert_eq!(rows(&case.fixture.conn), before, "{fault}");
                assert_eq!(metadata(&artifact.path), artifact.identity);
                if trigger.is_some() {
                    case.fixture
                        .conn
                        .execute_batch("DROP TRIGGER fail_history")
                        .unwrap();
                }
                if fault == "commit" {
                    let report = recover_missing(&mut case, true).unwrap();
                    assert_terminal_bookkeeping(
                        &case,
                        &report,
                        &before_run,
                        RunStatus::Failed,
                        &prior,
                        &artifact,
                        true,
                    );
                }
            }
        }
    }

    mod missing_paths {
        use super::*;
        use crate::runtime::runner::recovery::test_support::missing_paths::*;
        #[test]
        fn runner_recovery_missing_path_machine_termination_commits_one_atomic_database_only_loss()
        {
            let mut case = Case::new();
            persist_opening(&mut case);
            let original = case.fixture.run.clone();
            missing_lock(&mut case);
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert_loss(&case, &report, &original);
            assert!(!host.calls.iter().any(|name| name == "before_replay"));
            assert!(!case.artifacts.paths.owner_lock.exists());
            let diagnostics = storage::get_run(&case.fixture.conn, original.run_id)
                .unwrap()
                .cleanup_diagnostics_json
                .unwrap()
                .to_string();
            assert!(!diagnostics.contains("operator_via_director"));
        }

        #[test]
        fn runner_recovery_missing_path_uses_current_lifecycle_for_atomic_archived_closure_and_loss()
         {
            for (lifecycle, reason) in [
                ("member", "member_inactive"),
                ("director", "director_inactive"),
                ("fleet", "fleet_deleted"),
            ] {
                let mut case = Case::new();
                let opening = persist_opening(&mut case);
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
                missing_lock(&mut case);
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(counts(&report), (0, 0, 2));
                let committed = events(&case);
                assert_eq!(committed[0].event, opening);
                assert_eq!(committed.len(), 3);
                assert!(committed[1..].iter().all(|row| row.event.notification_state
                    == NotificationState::Archived
                    && row.event.message_id.is_none()
                    && row.event.archive_reason.unwrap().as_str() == reason));
            }
        }

        #[test]
        fn runner_recovery_missing_directory_keeps_surviving_suspect_artifacts_untouched() {
            use std::os::unix::fs::MetadataExt;
            let mut case = Case::new();
            persist_opening(&mut case);
            case.seed(&[obscured(&case.fixture.run, 2, 1)]);
            let original = case.fixture.run.clone();
            let snapshot = case.spool_bytes().unwrap();
            let spool_meta = std::fs::metadata(&case.artifacts.paths.outbox).unwrap();
            let sentinel = case
                .artifacts
                .paths
                .run_dir
                .join("unrelated-suspect-artifact");
            std::fs::write(&sentinel, b"surviving owned fixture artifact").unwrap();
            case.release();
            let retained = case.artifacts.paths.run_dir.with_extension("retained");
            std::fs::rename(&case.artifacts.paths.run_dir, &retained).unwrap();
            let report = case.recover(&mut new_host()).unwrap();
            assert_loss(&case, &report, &original);
            assert!(!case.artifacts.paths.run_dir.exists());
            assert_eq!(
                std::fs::read(retained.join("unrelated-suspect-artifact")).unwrap(),
                b"surviving owned fixture artifact"
            );
            let after = std::fs::metadata(retained.join("events.outbox")).unwrap();
            assert_eq!(
                (
                    after.dev(),
                    after.ino(),
                    after.len(),
                    after.modified().unwrap()
                ),
                (
                    spool_meta.dev(),
                    spool_meta.ino(),
                    u64::try_from(snapshot.len()).unwrap(),
                    spool_meta.modified().unwrap()
                )
            );
            assert_eq!(
                events(&case).len(),
                3,
                "database-only recovery does not replay the surviving spool"
            );
        }

        #[test]
        fn runner_recovery_attestation_records_attributed_original_provenance_and_current_terminal_retries()
         {
            let mut case = Case::new();
            persist_opening(&mut case);
            let original = case.fixture.run.clone();
            let original_claim = claim(&original);
            missing_lock(&mut case);
            let earliest = chrono::Utc::now();
            let mut host = new_host();
            host.process = Evidence::Unknown;
            let report = recover_claim(&mut case, &mut host, &original_claim).unwrap();
            let latest = chrono::Utc::now();
            assert_loss(&case, &report, &original);
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            let diagnostics = current.cleanup_diagnostics_json.as_ref().unwrap();
            let text = diagnostics.to_string();
            for value in [
                "owner_termination_attestation",
                "operator_via_director",
                original_claim.evidence_ref.as_str(),
                original.owner_start_identity.as_str(),
            ] {
                assert!(text.contains(value), "{text}");
            }
            let mut leaves = Vec::new();
            fn values<'a>(value: &'a Value, leaves: &mut Vec<&'a Value>) {
                match value {
                    Value::Object(map) => {
                        for value in map.values() {
                            values(value, leaves)
                        }
                    }
                    Value::Array(array) => {
                        for value in array {
                            values(value, leaves)
                        }
                    }
                    _ => leaves.push(value),
                }
            }
            values(diagnostics, &mut leaves);
            for expected in [original.run_id, original.owner_epoch, original.owner_pid] {
                assert!(leaves.iter().any(|value| value.as_i64() == Some(expected)));
            }
            assert!(
                leaves
                    .iter()
                    .filter_map(|value| value.as_str())
                    .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .any(|value| value >= earliest && value <= latest)
            );
            let committed = rows(&case.fixture.conn);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            assert!(recover_claim(&mut case, &mut retry, &original_claim).is_err());
            assert_eq!(rows(&case.fixture.conn), committed);
            let matching = claim(&current);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            let no_work = recover_claim(&mut case, &mut retry, &matching).unwrap();
            assert_eq!(counts(&no_work), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), committed);
            case.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [original.run_id],
                )
                .unwrap();
            let changed = rows(&case.fixture.conn);
            assert!(recover_claim(&mut case, &mut new_host(), &matching).is_err());
            assert_eq!(rows(&case.fixture.conn), changed);
            case.refresh();
            let latest_claim = claim(&case.fixture.run);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            assert_eq!(
                counts(&recover_claim(&mut case, &mut retry, &latest_claim).unwrap()),
                (0, 0, 0)
            );
            assert_eq!(rows(&case.fixture.conn), changed);
            assert_eq!(
                storage::get_run(&case.fixture.conn, original.run_id)
                    .unwrap()
                    .cleanup_diagnostics_json,
                current.cleanup_diagnostics_json
            );
        }
        #[test]
        fn runner_recovery_missing_path_faults_rollback_owner_provenance_counters_episode_and_events_together()
         {
            for fault in [
                "owner",
                "provenance",
                "reservation",
                "closure",
                "loss",
                "terminal",
                "commit",
            ] {
                let mut case = Case::new();
                persist_opening(&mut case);
                missing_lock(&mut case);
                let statement = match fault {
                    "owner" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF owner_epoch ON member_runs BEGIN SELECT RAISE(ABORT,'injected owner failure'); END",
                    ),
                    "provenance" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs BEGIN SELECT RAISE(ABORT,'injected provenance failure'); END",
                    ),
                    "reservation" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF event_seq_reserved_through ON member_runs BEGIN SELECT RAISE(ABORT,'injected reservation failure'); END",
                    ),
                    "closure" => Some(
                        "CREATE TRIGGER fault BEFORE INSERT ON member_runtime_events WHEN NEW.kind='interaction_closed' BEGIN SELECT RAISE(ABORT,'injected closure failure'); END",
                    ),
                    "loss" => Some(
                        "CREATE TRIGGER fault BEFORE INSERT ON member_runtime_events WHEN NEW.kind='runner_lost' BEGIN SELECT RAISE(ABORT,'injected loss failure'); END",
                    ),
                    "terminal" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF status ON member_runs WHEN NEW.status='lost' BEGIN SELECT RAISE(ABORT,'injected terminal failure'); END",
                    ),
                    "commit" => None,
                    _ => unreachable!(),
                };
                if let Some(statement) = statement {
                    case.fixture.conn.execute_batch(statement).unwrap();
                }
                let before = rows(&case.fixture.conn);
                if fault == "commit" {
                    case.fixture.conn.commit_hook(Some(|| true)).unwrap();
                }
                let mut host = new_host();
                host.process = Evidence::Unknown;
                let attestation = claim(&case.fixture.run);
                let result = recover_claim(&mut case, &mut host, &attestation);
                case.fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
                assert!(result.is_err(), "{fault}");
                assert_eq!(rows(&case.fixture.conn), before, "{fault}");
                assert!(!host.calls.iter().any(|name| name == "before_replay"));
            }
        }

        #[test]
        fn runner_recovery_attestation_revalidates_epoch_after_obtaining_the_actual_write_lock() {
            use std::sync::mpsc;
            let mut case = Case::new();
            persist_opening(&mut case);
            missing_lock(&mut case);
            let original = case.fixture.run.clone();
            let attestation = claim(&original);
            let run_id = original.run_id;
            let path = case.fixture.conn.path().unwrap().to_owned();
            let (start_tx, start_rx) = mpsc::sync_channel(1);
            let (held_tx, held_rx) = mpsc::sync_channel(1);
            let mut start_tx = Some(start_tx);
            let mut host = new_host();
            host.process = Evidence::Unknown;
            host.action = Some(Box::new(move |boundary, _, deadline| {
                if boundary == "before_takeover"
                    && let Some(start) = start_tx.take()
                {
                    start.send(()).unwrap();
                    held_rx
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .unwrap();
                }
                Ok(())
            }));
            std::thread::scope(|scope| {
                let writer = scope.spawn(move || {
                    start_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                    let writer = Connection::open(path).unwrap();
                    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                    writer
                        .execute(
                            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                            [run_id],
                        )
                        .unwrap();
                    held_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(80));
                    writer.execute_batch("COMMIT").unwrap();
                });
                let result = recover_claim(&mut case, &mut host, &attestation);
                writer.join().unwrap();
                assert!(result.is_err());
            });
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            assert_eq!(current.owner_epoch, original.owner_epoch + 1);
            assert_eq!(current.owner_kind, original.owner_kind);
            assert_eq!(current.status, original.status);
            assert_eq!(
                current.cleanup_diagnostics_json,
                original.cleanup_diagnostics_json
            );
            assert_eq!(events(&case).len(), 1);
        }
    }

    mod relationships {
        use crate::runtime::runner::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_review_r5_exceptional_zero_event_takeover_refuses_actual_member_fleet_change_atomically()
         {
            refusal(Stage::ExceptionalTakeover, Relationship::MemberFleet);
        }
        #[test]
        fn runner_recovery_review_r5_exceptional_zero_event_takeover_refuses_actual_director_fleet_change_atomically()
         {
            refusal(Stage::ExceptionalTakeover, Relationship::DirectorFleet);
        }
        #[test]
        fn runner_recovery_review_r5_exceptional_zero_event_takeover_refuses_actual_fleet_director_change_atomically()
         {
            refusal(Stage::ExceptionalTakeover, Relationship::FleetDirector);
        }
        #[test]
        fn runner_recovery_review_r5_valid_historical_exceptional_zero_event_bookkeeping_keeps_history()
         {
            historical_control(Stage::ExceptionalTakeover);
        }
    }
}
