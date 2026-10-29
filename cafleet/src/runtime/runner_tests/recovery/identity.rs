use super::*;
use crate::input::PaneLock;

#[test]
fn runner_recovery_matching_held_original_lock_defers_without_mutating_rows_or_spool() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let before = rows(&case.fixture.conn);
    let bytes = case.spool_bytes().unwrap();
    let mut host = new_host();
    let report = recover_member_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.member_id,
        None,
        host.deadline,
        &mut host,
    )
    .unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
    assert_eq!(counts(&report), (0, 0, 0));
    assert_eq!(rows(&case.fixture.conn), before);
    assert_eq!(case.spool_bytes().unwrap(), bytes);
    assert!(
        !host
            .calls
            .iter()
            .any(|name| name == "after_takeover" || name == "cleanup" || name == "before_replay")
    );
}

#[test]
fn runner_recovery_copied_registry_identity_fails_before_touching_the_original_owner_namespace() {
    let case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let original_rows = rows(&case.fixture.conn);
    let original_spool = case.spool_bytes().unwrap();
    let copy = case.fixture._dir.path().join("copied.db");
    std::fs::copy(case.fixture.conn.path().unwrap(), &copy).unwrap();
    let mut conn = Connection::open(&copy).unwrap();
    let copied_before = rows(&conn);
    let settings = Settings::for_database(&copy, None);
    let mut host = new_host();
    assert!(
        recover_member_with_host(
            &mut conn,
            &settings,
            case.fixture.run.member_id,
            None,
            host.deadline,
            &mut host
        )
        .is_err()
    );
    assert_eq!(rows(&conn), copied_before);
    assert_eq!(rows(&case.fixture.conn), original_rows);
    assert_eq!(case.spool_bytes().unwrap(), original_spool);
}

#[test]
fn runner_recovery_creator_free_precreated_lock_requires_complete_startup_evidence() {
    for evidence in [Evidence::Alive, Evidence::Unknown] {
        let mut case = Case::creator();
        let before = rows(&case.fixture.conn);
        let mut host = new_host();
        host.startup = evidence;
        let report = case.recover(&mut host).unwrap();
        match evidence {
            Evidence::Alive => {
                assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable))
            }
            Evidence::Unknown => assert!(matches!(
                report.outcome,
                RecoveryOutcome::OperatorReconciliationRequired
            )),
            Evidence::Terminated => unreachable!(),
        }
        assert!(host.calls.iter().any(|name| name == "startup"));
        assert_eq!(counts(&report), (0, 0, 0));
        assert_eq!(rows(&case.fixture.conn), before);
    }
}

#[test]
fn runner_recovery_creator_positive_complete_startup_termination_allows_one_bookkeeping_loss() {
    let mut case = Case::creator();
    let epoch = case.fixture.run.owner_epoch;
    let mut host = new_host();
    let report = case.recover(&mut host).unwrap();
    assert!(host.calls.iter().any(|name| name == "startup"));
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(report.status, RunStatus::Lost);
    assert_eq!(
        events(&case)
            .iter()
            .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
            .count(),
        1
    );
    case.refresh();
    assert_eq!(case.fixture.run.owner_epoch, epoch + 1);
}

#[test]
fn runner_recovery_borrowed_original_guard_reaches_actual_replay_without_reacquiring_pane() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let expected = case.fixture.run.clone();
    let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
        .unwrap()
        .unwrap();
    case.release();
    let mut host = new_host();
    let completion = recover_member_with_held_pane_lock(
        &mut case.fixture.conn,
        &case.settings,
        expected.member_id,
        &expected,
        &case.placement,
        None,
        host.deadline,
        &held,
        &mut host,
    )
    .unwrap();
    assert_eq!(counts(&completion.report), (1, 3, 0));
    assert_eq!(completion.run.owner_epoch, expected.owner_epoch + 1);
    assert_eq!(
        completion.run,
        storage::get_run(&case.fixture.conn, expected.run_id).unwrap()
    );
    assert!(
        PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .is_none()
    );
    assert!(host.calls.iter().any(|name| name == "before_replay"));
}

#[test]
fn runner_recovery_borrowed_call_rejects_pre_entry_foreign_epoch_instead_of_recapturing_it() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let expected = case.fixture.run.clone();
    let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
        .unwrap()
        .unwrap();
    case.release();
    case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
            [expected.run_id],
        )
        .unwrap();
    let before = rows(&case.fixture.conn);
    let bytes = case.spool_bytes().unwrap();
    let mut host = new_host();
    assert!(
        recover_member_with_held_pane_lock(
            &mut case.fixture.conn,
            &case.settings,
            expected.member_id,
            &expected,
            &case.placement,
            None,
            host.deadline,
            &held,
            &mut host
        )
        .is_err()
    );
    assert_eq!(rows(&case.fixture.conn), before);
    assert_eq!(case.spool_bytes().unwrap(), bytes);
    assert!(
        !host
            .calls
            .iter()
            .any(|name| name == "after_takeover" || name == "cleanup")
    );
}

#[test]
fn runner_recovery_borrowed_terminal_no_work_validates_fixed_starting_identity() {
    let mut case = Case::new();
    case.fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at=?1 WHERE run_id=?2",rusqlite::params![NOW,case.fixture.run.run_id]).unwrap();
    case.refresh();
    let expected = case.fixture.run.clone();
    let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
        .unwrap()
        .unwrap();
    case.release();
    case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
            [expected.run_id],
        )
        .unwrap();
    let before = rows(&case.fixture.conn);
    let mut host = new_host();
    assert!(
        recover_member_with_held_pane_lock(
            &mut case.fixture.conn,
            &case.settings,
            expected.member_id,
            &expected,
            &case.placement,
            None,
            host.deadline,
            &held,
            &mut host
        )
        .is_err()
    );
    assert_eq!(rows(&case.fixture.conn), before);
    case.refresh();
    let matching = case.fixture.run.clone();
    let mut host = new_host();
    let completion = recover_member_with_held_pane_lock(
        &mut case.fixture.conn,
        &case.settings,
        matching.member_id,
        &matching,
        &case.placement,
        None,
        host.deadline,
        &held,
        &mut host,
    )
    .unwrap();
    assert_eq!(counts(&completion.report), (0, 0, 0));
    assert_eq!(completion.run, matching);
    assert_eq!(rows(&case.fixture.conn), before);
}

#[test]
fn runner_recovery_borrowed_expectations_reject_stable_run_and_concrete_placement_mismatches() {
    for field in [
        "run",
        "member",
        "fleet",
        "director",
        "backend",
        "nonce",
        "epoch",
        "owner_pid",
        "owner_start",
        "directory",
        "lock",
        "runtime_path",
        "runner",
        "child",
        "database",
        "pane",
        "session",
        "window",
        "coding_agent",
    ] {
        let mut case = Case::new();
        let mut expected = case.fixture.run.clone();
        let mut placement = case.placement.clone();
        match field {
            "run" => expected.run_id += 1,
            "member" => expected.member_id += 1,
            "fleet" => expected.fleet_id += 1,
            "director" => expected.director_member_id += 1,
            "backend" => expected.backend = "claude".into(),
            "nonce" => expected.launch_nonce = "b".repeat(64),
            "epoch" => expected.owner_epoch += 1,
            "owner_pid" => expected.owner_pid += 1,
            "owner_start" => expected.owner_start_identity.push_str("-foreign"),
            "directory" => expected.runtime_dir_identity_json = Some(json!({"device":0,"inode":0})),
            "lock" => expected.run_lock_identity_json = Some(json!({"device":0,"inode":0})),
            "runtime_path" => expected.runtime_path.push_str("-foreign"),
            "runner" => expected.runner_start_identity = Some("foreign-generation".into()),
            "child" => {
                expected.child_pid = Some(1);
                expected.child_start_identity = Some("foreign-generation".into());
            }
            "database" => expected.database_identity_json["instance_id"] = json!("0".repeat(32)),
            "pane" => placement.mux_pane_id = Some("%foreign".into()),
            "session" => placement.mux_session.push_str("-foreign"),
            "window" => placement.mux_window_id.push_str("-foreign"),
            "coding_agent" => placement.coding_agent = "claude".into(),
            _ => unreachable!(),
        }
        let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
            .unwrap()
            .unwrap();
        case.release();
        let before = rows(&case.fixture.conn);
        let mut host = new_host();
        assert!(
            recover_member_with_held_pane_lock(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                &expected,
                &placement,
                None,
                host.deadline,
                &held,
                &mut host
            )
            .is_err(),
            "{field}"
        );
        assert_eq!(rows(&case.fixture.conn), before, "{field}");
    }
}

#[test]
fn runner_recovery_borrowed_fixed_snapshot_accepts_changed_mutable_health_and_current_counters() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    let expected = case.fixture.run.clone();
    let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
        .unwrap()
        .unwrap();
    storage::reserve_event_sequences(&mut case.fixture.conn, &expected.owner()).unwrap();
    case.fixture.conn.execute("UPDATE member_runs SET last_health_at=?1,reachability='suspect',observation_gap_count=observation_gap_count+1 WHERE run_id=?2",rusqlite::params![NOW,expected.run_id]).unwrap();
    let ceiling = storage::get_run(&case.fixture.conn, expected.run_id)
        .unwrap()
        .event_seq_reserved_through;
    case.release();
    let mut host = new_host();
    let completion = recover_member_with_held_pane_lock(
        &mut case.fixture.conn,
        &case.settings,
        expected.member_id,
        &expected,
        &case.placement,
        None,
        host.deadline,
        &held,
        &mut host,
    )
    .unwrap();
    assert_eq!(counts(&completion.report), (1, 3, 0));
    assert!(
        events(&case)[1..]
            .iter()
            .all(|row| row.event.event_seq > ceiling)
    );
}

#[test]
fn runner_recovery_after_snapshot_precedes_real_pane_wait_and_revalidates_stable_owner() {
    use std::sync::mpsc;
    let mut case = Case::new();
    let pane = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
        .unwrap()
        .unwrap();
    let original_epoch = case.fixture.run.owner_epoch;
    let run_id = case.fixture.run.run_id;
    let path = case.fixture.conn.path().unwrap().to_owned();
    let (send, receive) = mpsc::sync_channel(1);
    let mut send = Some(send);
    let mut host = new_host();
    host.action = Some(Box::new(move |boundary, run, _| {
        if boundary == "after_snapshot"
            && let Some(send) = send.take()
        {
            assert_eq!(run.owner_epoch, original_epoch);
            send.send(()).unwrap();
        }
        if boundary == "after_takeover" {
            assert!(run.owner_epoch >= original_epoch + 2);
        }
        Ok(())
    }));
    std::thread::scope(|scope| {
        let mutation = scope.spawn(move || {
            receive.recv_timeout(Duration::from_secs(1)).unwrap();
            let connection = Connection::open(path).unwrap();
            connection
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [run_id],
                )
                .unwrap();
            drop(pane);
        });
        let result = case.recover(&mut host);
        mutation.join().unwrap();
        assert!(host.calls.iter().any(|name| name == "after_snapshot"));
        match result {
            Ok(report) => assert_eq!(report.status, RunStatus::Lost),
            Err(_) => assert!(events(&case).is_empty()),
        }
    });
}
