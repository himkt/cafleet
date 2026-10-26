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
fn runner_recovery_review_r3_machine_missing_paths_preserve_original_exited_and_failed_history() {
    original_history_cases(false);
}
#[test]
fn runner_recovery_review_r3_operator_missing_paths_preserve_original_exited_and_failed_history() {
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
