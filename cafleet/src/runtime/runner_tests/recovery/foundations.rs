use super::*;
use crate::input::{PaneLock, pane_lock_for};

#[test]
fn runner_recovery_clock_clips_each_original_boundary_and_never_renews_time() {
    let start = Instant::now();
    for offset in [0, 1, 2, 3, 4, 9] {
        let until = start + Duration::from_secs(offset);
        let clock = ShutdownClock::new(start).clipped(until);
        assert_eq!(
            clock.child_deadline(),
            (start + Duration::from_secs(2)).min(until)
        );
        assert_eq!(
            clock.display_deadline(),
            (start + Duration::from_secs(3)).min(until)
        );
        assert_eq!(
            clock.storage_deadline(),
            (start + Duration::from_secs(4)).min(until)
        );
        assert_eq!(clock.remaining_storage_at(until), Duration::ZERO);
    }
    let past = start.checked_sub(Duration::from_secs(1)).unwrap();
    let clock = ShutdownClock::new(start).clipped(past);
    assert_eq!(clock.child_deadline(), past);
    assert_eq!(clock.display_deadline(), past);
    assert_eq!(clock.storage_deadline(), past);
    assert_eq!(clock.remaining_storage_at(start), Duration::ZERO);
}

#[test]
fn runner_recovery_clock_repeated_clipping_only_tightens_the_same_original_schedule() {
    let start = Instant::now();
    let first = start + Duration::from_millis(2500);
    let second = start + Duration::from_millis(1500);
    let clock = ShutdownClock::new(start)
        .clipped(first)
        .clipped(start + Duration::from_secs(20));
    assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
    assert_eq!(clock.display_deadline(), first);
    assert_eq!(clock.storage_deadline(), first);
    let clock = clock.clipped(second).clipped(first);
    assert_eq!(clock.child_deadline(), second);
    assert_eq!(clock.display_deadline(), second);
    assert_eq!(clock.storage_deadline(), second);
}

#[test]
fn runner_recovery_lock_transfer_commits_fresh_epochs_and_preserves_first_origin_status() {
    let mut fixture = Fixture::new();
    let _artifacts = fixture.artifacts();
    let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .unwrap();
    let original = fixture.run.clone();
    let process = ProcessIdentity::current().unwrap();
    for offset in 1..=2 {
        let owner = lock
            .transfer_to_reconciler_with_deadline(
                &mut fixture.conn,
                &fixture.run,
                process.clone(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        fixture.run = storage::get_run(&fixture.conn, original.run_id).unwrap();
        assert_eq!(owner, fixture.run.owner());
        assert_eq!(fixture.run.owner_epoch, original.owner_epoch + offset);
        assert_eq!(fixture.run.owner_kind, OwnerKind::Reconciler);
        assert_eq!(fixture.run.status, RunStatus::Recovering);
        assert_eq!(fixture.run.recovery_origin_status, Some(original.status));
        assert_eq!(fixture.run.owner_pid, process.pid);
        assert_eq!(fixture.run.owner_start_identity, process.start_identity);
        assert_eq!(
            fixture.run.runtime_dir_identity_json,
            original.runtime_dir_identity_json
        );
        assert_eq!(
            fixture.run.run_lock_identity_json,
            original.run_lock_identity_json
        );
        lock.validate(&fixture.conn, &owner).unwrap();
    }
    assert!(storage::reserve_event_sequences(&mut fixture.conn, &original.owner()).is_err());
}

#[test]
fn runner_recovery_lock_transfer_rejects_every_changed_stable_expected_identity() {
    for changed in [
        "epoch",
        "owner",
        "nonce",
        "backend",
        "director",
        "runtime_path",
        "directory",
        "lock",
        "runner",
        "child",
        "database",
    ] {
        let mut fixture = Fixture::new();
        let _artifacts = fixture.artifacts();
        let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .unwrap();
        let before = fixture.run.clone();
        let mut expected = before.clone();
        match changed {
            "epoch" => expected.owner_epoch += 1,
            "owner" => expected.owner_start_identity.push_str("-foreign"),
            "nonce" => expected.launch_nonce = "b".repeat(64),
            "backend" => expected.backend = "claude".into(),
            "director" => expected.director_member_id += 1,
            "runtime_path" => expected.runtime_path.push_str("-foreign"),
            "directory" => expected.runtime_dir_identity_json = Some(json!({"device":0,"inode":0})),
            "lock" => expected.run_lock_identity_json = Some(json!({"device":0,"inode":0})),
            "runner" => {
                expected.runner_pid = Some(1);
                expected.runner_start_identity = Some("foreign".into());
            }
            "child" => {
                expected.child_pid = Some(1);
                expected.child_start_identity = Some("foreign".into());
            }
            "database" => expected.database_identity_json["instance_id"] = json!("0".repeat(32)),
            _ => unreachable!(),
        }
        assert!(
            lock.transfer_to_reconciler_with_deadline(
                &mut fixture.conn,
                &expected,
                ProcessIdentity::current().unwrap(),
                Instant::now() + Duration::from_secs(1),
            )
            .is_err(),
            "{changed}"
        );
        assert_eq!(
            storage::get_run(&fixture.conn, before.run_id).unwrap(),
            before,
            "{changed}"
        );
        lock.validate(&fixture.conn, &before.owner()).unwrap();
    }
}

#[test]
fn runner_recovery_lock_transfer_ignores_mutable_health_diagnostics_and_sequence_snapshots() {
    let mut fixture = Fixture::new();
    let _artifacts = fixture.artifacts();
    let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .unwrap();
    let mut expected = fixture.run.clone();
    expected.last_health_at = Some("2000-01-01T00:00:00Z".into());
    expected.reachability = Reachability::Suspect;
    expected.cleanup_diagnostics_json = Some(json!(["old bounded diagnostic"]));
    expected.event_seq_reserved_through += 256;
    expected.observation_gap_count += 1;
    lock.transfer_to_reconciler_with_deadline(
        &mut fixture.conn,
        &expected,
        ProcessIdentity::current().unwrap(),
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    let current = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    assert_eq!(
        current.event_seq_reserved_through,
        fixture.run.event_seq_reserved_through
    );
    assert_eq!(
        current.observation_gap_count,
        fixture.run.observation_gap_count
    );
}

#[test]
fn runner_recovery_lock_transfer_commit_failure_returns_no_uncommitted_owner() {
    let mut fixture = Fixture::new();
    let _artifacts = fixture.artifacts();
    let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .unwrap();
    fixture.conn.commit_hook(Some(|| true));
    let result = lock.transfer_to_reconciler_with_deadline(
        &mut fixture.conn,
        &fixture.run,
        ProcessIdentity::current().unwrap(),
        Instant::now() + Duration::from_secs(1),
    );
    fixture.conn.commit_hook(None::<fn() -> bool>);
    assert!(result.is_err());
    assert_eq!(
        storage::get_run(&fixture.conn, fixture.run.run_id).unwrap(),
        fixture.run
    );
    lock.validate(&fixture.conn, &fixture.run.owner()).unwrap();
}

#[test]
fn runner_recovery_lock_transfer_contention_uses_the_original_absolute_deadline() {
    let mut fixture = Fixture::new();
    let _artifacts = fixture.artifacts();
    let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .unwrap();
    let writer = Connection::open(fixture.conn.path().unwrap()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let result = lock.transfer_to_reconciler_with_deadline(
        &mut fixture.conn,
        &fixture.run,
        ProcessIdentity::current().unwrap(),
        started + Duration::from_millis(80),
    );
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_millis(750));
    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        storage::get_run(&fixture.conn, fixture.run.run_id).unwrap(),
        fixture.run
    );
}

#[test]
fn runner_recovery_pane_guard_validates_the_actual_database_placement_and_named_inode() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let configuration = crate::config::Settings {
        database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
        max_text_len: 200,
        multiplexer: None,
    };
    let placement =
        crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
    let held = PaneLock::try_acquire(&configuration.database_url, &placement)
        .unwrap()
        .unwrap();
    held.validate_binding(&configuration.database_url, &placement)
        .unwrap();
    for field in ["backend", "session", "pane"] {
        let mut changed = placement.clone();
        match field {
            "backend" => changed.backend = "herdr".into(),
            "session" => changed.mux_session.push_str("-foreign"),
            "pane" => changed.mux_pane_id = Some("%foreign".into()),
            _ => unreachable!(),
        }
        assert!(
            held.validate_binding(&configuration.database_url, &changed)
                .is_err()
        );
    }
    let sibling_path = fixture._dir.path().join("sibling.db");
    let mut sibling = Connection::open(&sibling_path).unwrap();
    crate::db::migrate_to_head(&mut sibling).unwrap();
    let sibling_url = format!("sqlite:///{}", sibling_path.display());
    assert!(held.validate_binding(&sibling_url, &placement).is_err());
    let path = crate::input::lock_path(&configuration.database_url, &placement).unwrap();
    let original = path.with_extension("retained");
    std::fs::rename(&path, &original).unwrap();
    std::fs::write(&path, b"replacement lock identity").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        held.validate_binding(&configuration.database_url, &placement)
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement lock identity");
}

#[test]
fn runner_recovery_pane_guard_selection_reuses_a_deduplicated_physical_guard() {
    let fixture = Fixture::new();
    let database_url = format!("sqlite:///{}", fixture.conn.path().unwrap());
    let placement =
        crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
    let guards = vec![
        PaneLock::try_acquire(&database_url, &placement)
            .unwrap()
            .unwrap(),
    ];
    let selected = pane_lock_for(&guards, &database_url, &placement).unwrap();
    assert!(std::ptr::eq(selected, &guards[0]));
    let mut shared = placement.clone();
    shared.mux_window_id = "@shared".into();
    assert!(std::ptr::eq(
        pane_lock_for(&guards, &database_url, &shared).unwrap(),
        &guards[0]
    ));
    assert!(
        PaneLock::try_acquire(&database_url, &placement)
            .unwrap()
            .is_none()
    );
    assert!(pane_lock_for(&[], &database_url, &placement).is_err());
}

#[test]
fn runner_recovery_presentation_pins_ordered_json_outcomes_and_exact_cleanup_text() {
    for (outcome, wire, status) in [
        (RecoveryOutcome::Recovered, "recovered", RunStatus::Lost),
        (
            RecoveryOutcome::Recovering,
            "recovering",
            RunStatus::Recovering,
        ),
        (
            RecoveryOutcome::LiveOrUnreachable,
            "live_or_unreachable",
            RunStatus::Running,
        ),
        (
            RecoveryOutcome::OperatorReconciliationRequired,
            "operator_reconciliation_required",
            RunStatus::Starting,
        ),
    ] {
        let diagnostics = vec![
            json!("bounded diagnostic"),
            json!({"source":"operator_via_director","evidence_ref":"audit:termination"}),
        ];
        let report = RecoveryReport {
            member_id: 7,
            run_id: 11,
            outcome,
            replayed_count: 2,
            inbox_count: 4,
            archived_count: 0,
            status,
            cleanup_diagnostics: diagnostics.clone(),
        };
        let value = crate::presentation::runtime_recovery(&report);
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "member_id",
                "run_id",
                "outcome",
                "replayed_count",
                "inbox_count",
                "archived_count",
                "status",
                "cleanup_diagnostics"
            ]
        );
        assert_eq!(
            value,
            json!({"member_id":7,"run_id":11,"outcome":wire,"replayed_count":2,"inbox_count":4,"archived_count":0,"status":status.as_str(),"cleanup_diagnostics":diagnostics})
        );
        assert_eq!(
            crate::output::format_runtime_recovery(&report),
            format!(
                "Recovery: member=7 run=11 outcome={wire} status={} replayed=2 inbox=4 archived=0\nCleanup: bounded diagnostic\nCleanup: {}",
                status.as_str(),
                diagnostics[1]
            )
        );
    }
}
