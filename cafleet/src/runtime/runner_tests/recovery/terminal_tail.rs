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
            crate::broker::get_message(&case.fixture.conn, event.message_id.unwrap()).unwrap();
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
fn runner_recovery_terminal_complete_corruption_before_tail_preserves_every_byte_and_commit() {
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
