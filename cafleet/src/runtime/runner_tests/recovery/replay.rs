use super::*;

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
    assert!(rows.iter().all(
        |row| row.event.notification_state == NotificationState::Inbox
            && row.event.message_id.is_some()
    ));
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
fn runner_recovery_validates_all_complete_frames_before_any_replay_and_preserves_corruption() {
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
fn runner_recovery_incomplete_final_tail_replays_complete_originals_and_retains_a_diagnostic() {
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
fn runner_recovery_replayed_native_exit_preserves_exact_code_or_signal_without_fabricated_loss() {
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
fn runner_recovery_sequence_allocation_uses_current_reserved_ceiling_and_complete_spool_maximum() {
    let mut case = Case::new();
    case.seed(&[opening(&case.fixture.run, 1, 1)]);
    storage::reserve_event_sequences(&mut case.fixture.conn, &case.fixture.run.owner()).unwrap();
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
fn runner_recovery_expiry_after_a_commit_retains_counts_and_next_pass_counts_only_new_work() {
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
fn runner_recovery_interruption_boundaries_preserve_origin_and_idempotent_event_associations() {
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
fn runner_recovery_interruption_before_compaction_retains_originals_and_committed_delivery() {
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
            || result.is_ok_and(|report| matches!(report.outcome, RecoveryOutcome::Recovering))
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
                usize::try_from(u32::from_be_bytes(remaining[..4].try_into().unwrap())).unwrap();
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
