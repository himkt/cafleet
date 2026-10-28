use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};

type CompletedSnapshot = Arc<Mutex<Option<Vec<TableSnapshot>>>>;

fn complete(
    conn: &Connection,
    run_id: i64,
    status: RunStatus,
    foreign_owner: bool,
) -> Vec<TableSnapshot> {
    assert_eq!(conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='complete',exit_code=?2,exit_signal=NULL,ended_at=?3,cleanup_diagnostics_json=?4,owner_epoch=owner_epoch+?5 WHERE run_id=?6",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, json!([{"kind":"same_owner_completion","status":status.as_str()}]).to_string(), i64::from(foreign_owner), run_id],
    ).unwrap(), 1);
    rows(conn)
}

fn prepare(exceptional: bool) -> (Case, Option<OwnerTerminationAttestation>) {
    let mut case = Case::new();
    let claim = exceptional.then(|| OwnerTerminationAttestation {
        expected_run_id: case.fixture.run.run_id,
        expected_owner_epoch: case.fixture.run.owner_epoch,
        evidence_ref: "audit:completion-race".into(),
    });
    case.release();
    if exceptional {
        std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    }
    (case, claim)
}

fn invoke(
    case: &mut Case,
    host: &mut Host,
    claim: Option<&OwnerTerminationAttestation>,
) -> TestResult<RecoveryReport> {
    recover_member_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.member_id,
        claim,
        host.deadline,
        host,
    )
}

fn assert_completed(
    case: &Case,
    host: &Host,
    report: &RecoveryReport,
    status: RunStatus,
    expected: &CompletedSnapshot,
) {
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(report.status, status);
    assert_eq!(counts(report), (0, 0, 0));
    assert_eq!(
        &rows(&case.fixture.conn),
        expected.lock().unwrap().as_ref().unwrap(),
        "same-owner terminal completion remains immutable"
    );
    assert!(!host.calls.iter().any(|name| matches!(
        name.as_str(),
        "after_takeover" | "before_replay" | "cleanup"
    )));
    let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
    assert_eq!(run.owner(), case.fixture.run.owner());
    assert_eq!(
        run.observation_gap_count,
        case.fixture.run.observation_gap_count
    );
    assert_eq!(
        run.exit_code,
        if status == RunStatus::Exited {
            Some(7)
        } else {
            None
        }
    );
    assert_eq!(
        &report.cleanup_diagnostics,
        run.cleanup_diagnostics_json
            .as_ref()
            .unwrap()
            .as_array()
            .unwrap()
    );
    assert!(events(case).is_empty());
}

fn immediate_completion(exceptional: bool, foreign_owner: bool) {
    for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
        let (mut case, claim) = prepare(exceptional);
        let expected: CompletedSnapshot = Arc::new(Mutex::new(None));
        let published = expected.clone();
        let path = case.fixture.conn.path().unwrap().to_owned();
        let mut host = new_host();
        if exceptional {
            host.process = Evidence::Unknown;
        }
        host.action = Some(Box::new(move |boundary, run, _| {
            if boundary == "before_takeover" {
                let conn = Connection::open(&path).unwrap();
                assert_eq!(
                    storage::get_run(&conn, run.run_id).unwrap().owner(),
                    run.owner()
                );
                *published.lock().unwrap() =
                    Some(complete(&conn, run.run_id, status, foreign_owner));
            }
            Ok(())
        }));
        let result = invoke(&mut case, &mut host, claim.as_ref());
        if foreign_owner {
            assert!(
                result.is_err(),
                "a terminal shortcut still validates the fixed owner and attestation"
            );
            assert_eq!(
                &rows(&case.fixture.conn),
                expected.lock().unwrap().as_ref().unwrap()
            );
        } else {
            assert_completed(&case, &host, &result.unwrap(), status, &expected);
        }
    }
}

#[test]
fn runner_recovery_review_r1_same_owner_completion_before_ordinary_takeover_has_zero_effects() {
    immediate_completion(false, false);
}

#[test]
fn runner_recovery_review_r1_same_owner_completion_before_exceptional_takeover_has_zero_effects() {
    immediate_completion(true, false);
}

#[test]
fn runner_recovery_review_r1_completed_foreign_owner_still_rejects_fixed_snapshot_and_attestation()
{
    immediate_completion(false, true);
    immediate_completion(true, true);
}

fn completion_behind_write_lock(exceptional: bool) {
    for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
        let (mut case, claim) = prepare(exceptional);
        let expected: CompletedSnapshot = Arc::new(Mutex::new(None));
        let published = expected.clone();
        let path = case.fixture.conn.path().unwrap().to_owned();
        let run_id = case.fixture.run.run_id;
        let (start, started) = mpsc::sync_channel(1);
        let (held, is_held) = mpsc::sync_channel(1);
        let (begin, beginning) = mpsc::sync_channel(1);
        let armed = Arc::new(AtomicBool::new(false));
        let observed_begin = Arc::new(AtomicBool::new(false));
        let enabled = armed.clone();
        let observed = observed_begin.clone();
        let mut begin = Some(begin);
        case.fixture
            .conn
            .authorizer(Some(move |context: AuthContext<'_>| {
                if enabled.load(Ordering::Acquire)
                    && matches!(
                        context.action,
                        AuthAction::Transaction {
                            operation: TransactionOperation::Begin
                        }
                    )
                    && let Some(begin) = begin.take()
                {
                    observed.store(true, Ordering::Release);
                    begin.send(()).unwrap();
                }
                Authorization::Allow
            }))
            .unwrap();
        let mut host = new_host();
        if exceptional {
            host.process = Evidence::Unknown;
        }
        let until = host.deadline;
        let mut start = Some(start);
        host.action = Some(Box::new(move |boundary, _, deadline| {
            if boundary == "before_takeover"
                && let Some(start) = start.take()
            {
                start.send(()).unwrap();
                is_held
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                armed.store(true, Ordering::Release);
            }
            Ok(())
        }));
        let result = std::thread::scope(|scope| {
            let writer = scope.spawn(move || {
                started
                    .recv_timeout(until.saturating_duration_since(Instant::now()))
                    .unwrap();
                let conn = Connection::open(path).unwrap();
                conn.execute_batch("BEGIN IMMEDIATE").unwrap();
                *published.lock().unwrap() = Some(complete(&conn, run_id, status, false));
                held.send(()).unwrap();
                beginning
                    .recv_timeout(until.saturating_duration_since(Instant::now()))
                    .unwrap();
                conn.execute_batch("COMMIT").unwrap();
            });
            let result = invoke(&mut case, &mut host, claim.as_ref());
            writer.join().unwrap();
            result
        });
        case.fixture
            .conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        assert!(
            observed_begin.load(Ordering::Acquire),
            "the caller attempted its real immediate transaction while completion held the write lock"
        );
        assert_completed(&case, &host, &result.unwrap(), status, &expected);
    }
}

#[test]
fn runner_recovery_review_r1_ordinary_completion_is_rechecked_after_actual_write_lock_wait() {
    completion_behind_write_lock(false);
}

#[test]
fn runner_recovery_review_r1_exceptional_completion_is_rechecked_after_actual_write_lock_wait() {
    completion_behind_write_lock(true);
}
