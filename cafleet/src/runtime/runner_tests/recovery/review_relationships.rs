use super::*;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug)]
enum Relationship {
    MemberFleet,
    DirectorFleet,
    FleetDirector,
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    OrdinaryTakeover,
    ExceptionalTakeover,
    Finalization,
}
impl Stage {
    fn boundary(self) -> &'static str {
        match self {
            Self::OrdinaryTakeover | Self::ExceptionalTakeover => "before_takeover",
            Self::Finalization => "before_finish",
        }
    }
}

fn established_terminal() -> Case {
    let mut case = Case::new();
    let transition = exited(&case.fixture.run, 1, Some(7), None);
    storage::persist_transition(
        &mut case.fixture.conn,
        &case.fixture.run.owner(),
        &transition,
        PersistenceMode::Live,
    )
    .unwrap();
    assert_eq!(
        case.fixture
            .conn
            .execute(
                "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
                [case.fixture.run.run_id],
            )
            .unwrap(),
        1
    );
    case.refresh();
    assert_eq!(case.fixture.run.status, RunStatus::Exited);
    assert_eq!(case.fixture.run.exit_code, Some(7));
    assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
    case
}

fn prepare(stage: Stage) -> Case {
    match stage {
        Stage::OrdinaryTakeover => established_terminal(),
        Stage::ExceptionalTakeover => {
            let mut case = established_terminal();
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            case
        }
        Stage::Finalization => {
            let case = Case::new();
            case.seed(&[exited(&case.fixture.run, 1, Some(7), None)]);
            assert!(events(&case).is_empty());
            case
        }
    }
}

fn invoke(case: &mut Case, host: &mut Host) -> TestResult<RecoveryReport> {
    case.release();
    let deadline = host.deadline;
    with_database_deadline(&mut case.fixture.conn, deadline, |conn| {
        recover_member_with_host(
            conn,
            &case.settings,
            case.fixture.run.member_id,
            None,
            deadline,
            host,
        )
    })
}

fn target(case: &mut Case, relationship: Relationship) -> i64 {
    match relationship {
        Relationship::MemberFleet | Relationship::DirectorFleet => {
            common::create_fleet(&mut case.fixture.conn, "other-relationship-fleet").0
        }
        Relationship::FleetDirector => {
            crate::broker::register_member(
                &mut case.fixture.conn,
                case.fixture.run.fleet_id,
                "alternate-director",
                "owned historical relationship fixture",
                None,
            )
            .unwrap()
            .member_id
        }
    }
}

fn change(conn: &Connection, run: &RunRecord, relationship: Relationship, target: i64) {
    let changed = match relationship {
        Relationship::MemberFleet => conn.execute(
            "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
            rusqlite::params![target, run.member_id],
        ),
        Relationship::DirectorFleet => conn.execute(
            "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
            rusqlite::params![target, run.director_member_id],
        ),
        Relationship::FleetDirector => conn.execute(
            "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
            rusqlite::params![target, run.fleet_id],
        ),
    }
    .unwrap();
    assert_eq!(changed, 1);
    let selected: i64 = match relationship {
        Relationship::MemberFleet | Relationship::DirectorFleet => conn.query_row(
            "SELECT fleet_id FROM members WHERE member_id=?1",
            [if matches!(relationship, Relationship::MemberFleet) {
                run.member_id
            } else {
                run.director_member_id
            }],
            |row| row.get(0),
        ),
        Relationship::FleetDirector => conn.query_row(
            "SELECT director_member_id FROM fleets WHERE fleet_id=?1",
            [run.fleet_id],
            |row| row.get(0),
        ),
    }
    .unwrap();
    assert_eq!(selected, target);
}

fn refusal(stage: Stage, relationship: Relationship) {
    let mut case = prepare(stage);
    let original = case.fixture.run.clone();
    let target = target(&mut case, relationship);
    let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
    let expected = Arc::new(Mutex::new(None));
    let published = expected.clone();
    let mut host = new_host();
    host.action = Some(Box::new(move |boundary, run, deadline| {
        if boundary != stage.boundary() {
            return Ok(());
        }
        assert!(Instant::now() < deadline);
        assert!(published.lock().unwrap().is_none());
        assert_eq!(run.status, RunStatus::Exited);
        assert_eq!(run.cleanup_state, CleanupState::Incomplete);
        assert_eq!(run.exit_code, Some(7));
        assert_eq!(
            run.owner_epoch,
            original.owner_epoch + i64::from(matches!(stage, Stage::Finalization))
        );
        let count: i64 = mutation.query_row(
            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='runner_exited'",
            [run.run_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            count, 1,
            "terminal truth exists before the injected relationship change"
        );
        change(&mutation, run, relationship, target);
        *published.lock().unwrap() = Some(rows(&mutation));
        Ok(())
    }));
    let result = invoke(&mut case, &mut host);
    let expected = expected.lock().unwrap();
    assert!(
        expected.is_some(),
        "the selected actual coordinator boundary executes"
    );
    if matches!(stage, Stage::Finalization) {
        for boundary in [
            "before_replay",
            "after_commit",
            "before_compact",
            "before_finish",
        ] {
            assert!(host.calls.iter().any(|call| call == boundary), "{boundary}");
        }
        assert!(case.spool_bytes().unwrap().is_empty());
    }
    let actual = rows(&case.fixture.conn);
    let changed = actual
        .iter()
        .zip(expected.as_ref().unwrap())
        .filter_map(|(actual, expected)| (actual != expected).then_some(actual.name.as_str()))
        .collect::<Vec<_>>();
    let current = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
    eprintln!(
        "relationship refusal: stage={stage:?}, relationship={relationship:?}, returned_error={}, changed={changed:?}, epoch={}, cleanup={:?}, gap={}",
        result.is_err(),
        current.owner_epoch,
        current.cleanup_state,
        current.observation_gap_count
    );
    assert!(
        actual.as_slice() == expected.as_ref().unwrap().as_slice(),
        "relationship refusal preserves the exact database after the injected change: {stage:?}/{relationship:?}"
    );
    assert!(
        result.is_err(),
        "a broker relationship mismatch rejects its actual mutation transaction"
    );
    if !matches!(stage, Stage::Finalization) {
        assert!(!host.calls.iter().any(|call| matches!(
            call.as_str(),
            "after_takeover" | "before_replay" | "cleanup"
        )));
    }
}

#[test]
fn runner_recovery_review_r5_ordinary_takeover_refuses_actual_member_fleet_change_atomically() {
    refusal(Stage::OrdinaryTakeover, Relationship::MemberFleet);
}
#[test]
fn runner_recovery_review_r5_ordinary_takeover_refuses_actual_director_fleet_change_atomically() {
    refusal(Stage::OrdinaryTakeover, Relationship::DirectorFleet);
}
#[test]
fn runner_recovery_review_r5_ordinary_takeover_refuses_actual_fleet_director_change_atomically() {
    refusal(Stage::OrdinaryTakeover, Relationship::FleetDirector);
}
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

#[derive(Clone, Copy, Debug)]
enum Lifecycle {
    MemberInactive,
    DirectorInactive,
    FleetDeleted,
}
impl Lifecycle {
    fn apply(self, conn: &Connection, run: &RunRecord) {
        let changed = match self {
            Self::MemberInactive | Self::DirectorInactive => conn.execute(
                "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                rusqlite::params![
                    NOW,
                    if matches!(self, Self::MemberInactive) {
                        run.member_id
                    } else {
                        run.director_member_id
                    }
                ],
            ),
            Self::FleetDeleted => conn.execute(
                "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                rusqlite::params![NOW, run.fleet_id],
            ),
        }
        .unwrap();
        assert_eq!(changed, 1);
    }
    fn archive_reason(self) -> ArchiveReason {
        match self {
            Self::MemberInactive => ArchiveReason::MemberInactive,
            Self::DirectorInactive => ArchiveReason::DirectorInactive,
            Self::FleetDeleted => ArchiveReason::FleetDeleted,
        }
    }
}

fn retained_history(conn: &Connection) -> Vec<TableSnapshot> {
    rows(conn)
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

fn historical_control(stage: Stage) {
    for lifecycle in [
        Lifecycle::MemberInactive,
        Lifecycle::DirectorInactive,
        Lifecycle::FleetDeleted,
    ] {
        let mut case = prepare(stage);
        let original = case.fixture.run.clone();
        let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
        let prior = Arc::new(Mutex::new(None));
        let published = prior.clone();
        let mut host = new_host();
        host.action = Some(Box::new(move |boundary, run, _| {
            if boundary == stage.boundary() {
                assert!(published.lock().unwrap().is_none());
                lifecycle.apply(&mutation, run);
                *published.lock().unwrap() = Some(retained_history(&mutation));
            }
            Ok(())
        }));
        let report = invoke(&mut case, &mut host).unwrap();
        assert!(prior.lock().unwrap().is_some());
        assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
        assert_eq!(report.status, RunStatus::Exited);
        assert_eq!(
            counts(&report),
            if matches!(stage, Stage::Finalization) {
                (1, 1, 0)
            } else {
                (0, 0, 0)
            }
        );
        let run = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
        assert_eq!(run.owner_epoch, original.owner_epoch + 1);
        assert_eq!(run.owner_kind, OwnerKind::Reconciler);
        assert_eq!(run.status, RunStatus::Exited);
        assert_eq!(run.exit_code, Some(7));
        assert_eq!(run.exit_signal, None);
        assert_eq!(run.cleanup_state, CleanupState::Complete);
        assert_eq!(
            run.event_seq_reserved_through,
            original.event_seq_reserved_through
        );
        assert_eq!(
            run.observation_gap_count,
            original.observation_gap_count + i64::from(matches!(stage, Stage::ExceptionalTakeover))
        );
        assert_eq!(
            &retained_history(&case.fixture.conn),
            prior.lock().unwrap().as_ref().unwrap(),
            "{stage:?}/{lifecycle:?}"
        );
        assert_eq!(events(&case).len(), 1);
        assert_eq!(
            events(&case)[0].event.notification_state,
            NotificationState::Inbox
        );
        if matches!(stage, Stage::ExceptionalTakeover) {
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|call| matches!(call.as_str(), "before_replay" | "cleanup"))
            );
        }
    }
}

#[test]
fn runner_recovery_review_r5_valid_historical_ordinary_takeover_keeps_truth_and_existing_inbox() {
    historical_control(Stage::OrdinaryTakeover);
}
#[test]
fn runner_recovery_review_r5_valid_historical_exceptional_zero_event_bookkeeping_keeps_history() {
    historical_control(Stage::ExceptionalTakeover);
}
#[test]
fn runner_recovery_review_r5_valid_historical_lifecycle_change_after_replay_allows_finalization() {
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
