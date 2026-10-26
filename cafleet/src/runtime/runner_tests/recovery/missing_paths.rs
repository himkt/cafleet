use super::*;

fn persist_opening(case: &mut Case) -> RuntimeEventRecord {
    let transition = opening(&case.fixture.run, 1, 1);
    storage::persist_transition(
        &mut case.fixture.conn,
        &case.fixture.run.owner(),
        &transition,
        PersistenceMode::Live,
    )
    .unwrap()
}
fn missing_lock(case: &mut Case) {
    case.release();
    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
}
fn claim(run: &RunRecord) -> OwnerTerminationAttestation {
    OwnerTerminationAttestation {
        expected_run_id: run.run_id,
        expected_owner_epoch: run.owner_epoch,
        evidence_ref: "audit:owned-fixture-termination".into(),
    }
}
fn recover_claim(
    case: &mut Case,
    host: &mut Host,
    claim: &OwnerTerminationAttestation,
) -> TestResult<RecoveryReport> {
    case.release();
    recover_member_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.member_id,
        Some(claim),
        host.deadline,
        host,
    )
}
fn assert_loss(case: &Case, report: &RecoveryReport, original: &RunRecord) {
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(report.status, RunStatus::Lost);
    assert_eq!(counts(report), (0, 2, 0));
    let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
    assert_eq!(current.owner_epoch, original.owner_epoch + 1);
    assert_eq!(current.owner_kind, OwnerKind::Reconciler);
    assert_eq!(
        current.observation_gap_count,
        original.observation_gap_count + 1
    );
    let events = events(case);
    assert_eq!(
        events.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
        vec![
            RuntimeEventKind::InteractionOpened,
            RuntimeEventKind::InteractionClosed,
            RuntimeEventKind::RunnerLost
        ]
    );
    assert_eq!(
        events[1].interaction.as_ref().unwrap().close_reason,
        Some(InteractionCloseReason::MonitorLost)
    );
    assert!(
        events[1..]
            .iter()
            .all(|row| row.event.event_seq > original.event_seq_reserved_through)
    );
    assert!(
        current
            .cleanup_diagnostics_json
            .as_ref()
            .unwrap()
            .to_string()
            .contains("runtime_path_missing")
    );
}

#[test]
fn runner_recovery_missing_path_unknown_identity_preserves_open_episode_and_requires_operator() {
    let mut case = Case::new();
    persist_opening(&mut case);
    missing_lock(&mut case);
    let before = rows(&case.fixture.conn);
    let mut host = new_host();
    host.process = Evidence::Unknown;
    let report = case.recover(&mut host).unwrap();
    assert!(matches!(
        report.outcome,
        RecoveryOutcome::OperatorReconciliationRequired
    ));
    assert_eq!(counts(&report), (0, 0, 0));
    assert_eq!(rows(&case.fixture.conn), before);
    assert!(
        events(&case)[0]
            .interaction
            .as_ref()
            .unwrap()
            .closed_at
            .is_none()
    );
    assert!(!case.artifacts.paths.owner_lock.exists());
    assert!(
        !host
            .calls
            .iter()
            .any(|name| name == "before_replay" || name == "cleanup")
    );
}

#[test]
fn runner_recovery_missing_path_rejects_wrong_snapshot_and_blank_operator_evidence_without_effects()
{
    for invalid in ["run", "epoch", "zero_run", "zero_epoch", "reference"] {
        let mut case = Case::new();
        persist_opening(&mut case);
        missing_lock(&mut case);
        let mut attestation = claim(&case.fixture.run);
        match invalid {
            "run" => attestation.expected_run_id += 1,
            "epoch" => attestation.expected_owner_epoch += 1,
            "zero_run" => attestation.expected_run_id = 0,
            "zero_epoch" => attestation.expected_owner_epoch = 0,
            "reference" => attestation.evidence_ref = " \t".into(),
            _ => unreachable!(),
        }
        let before = rows(&case.fixture.conn);
        let mut host = new_host();
        host.process = Evidence::Unknown;
        assert!(
            recover_claim(&mut case, &mut host, &attestation).is_err(),
            "{invalid}"
        );
        assert_eq!(rows(&case.fixture.conn), before, "{invalid}");
    }
}

#[test]
fn runner_recovery_live_unlinked_original_lock_is_not_replaced_or_treated_as_termination() {
    use std::os::unix::fs::PermissionsExt;
    let mut case = Case::new();
    persist_opening(&mut case);
    let original_guard = case.held.as_ref().unwrap().clone();
    case.release();
    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    std::fs::write(
        &case.artifacts.paths.owner_lock,
        b"foreign replacement inode",
    )
    .unwrap();
    std::fs::set_permissions(
        &case.artifacts.paths.owner_lock,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let before = rows(&case.fixture.conn);
    let mut host = new_host();
    host.process = Evidence::Alive;
    let report = case.recover(&mut host).unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
    assert_eq!(rows(&case.fixture.conn), before);
    assert_eq!(
        std::fs::read(&case.artifacts.paths.owner_lock).unwrap(),
        b"foreign replacement inode"
    );
    assert!(
        original_guard
            .validate(&case.fixture.conn, &case.fixture.run.owner())
            .is_err()
    );
}

#[test]
fn runner_recovery_missing_path_machine_termination_commits_one_atomic_database_only_loss() {
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
fn runner_recovery_missing_path_uses_current_lifecycle_for_atomic_archived_closure_and_loss() {
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
fn runner_recovery_positive_pid_lock_or_strict_health_independently_rejects_termination_claim() {
    for witness in ["pid", "lock", "health"] {
        let mut case = Case::new();
        persist_opening(&mut case);
        let attestation = claim(&case.fixture.run);
        if witness != "lock" {
            missing_lock(&mut case);
        }
        let before = rows(&case.fixture.conn);
        let mut host = new_host();
        host.process = if witness == "pid" {
            Evidence::Alive
        } else {
            Evidence::Unknown
        };
        host.matching_health = witness == "health";
        let result = recover_member_with_host(
            &mut case.fixture.conn,
            &case.settings,
            case.fixture.run.member_id,
            Some(&attestation),
            host.deadline,
            &mut host,
        );
        if let Ok(report) = result {
            assert!(
                matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable),
                "{witness}"
            );
        }
        assert_eq!(rows(&case.fixture.conn), before, "{witness}");
        assert!(
            !host
                .calls
                .iter()
                .any(|name| name == "after_takeover" || name == "cleanup")
        );
    }
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
            case.fixture.conn.commit_hook(Some(|| true));
        }
        let mut host = new_host();
        host.process = Evidence::Unknown;
        let attestation = claim(&case.fixture.run);
        let result = recover_claim(&mut case, &mut host, &attestation);
        case.fixture.conn.commit_hook(None::<fn() -> bool>);
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
