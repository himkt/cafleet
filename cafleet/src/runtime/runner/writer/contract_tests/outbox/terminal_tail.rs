use super::*;

fn terminal_spool(committed: bool) -> (WriterFixture, FaultIo, Vec<u8>) {
    let mut fixture = WriterFixture::new();
    let mut bytes = Vec::new();
    if committed {
        let original = ObservedTransition {
            kind: RuntimeEventKind::RunnerExited,
            cause_json: Some(
                serde_json::json!({"kind":"native_exit","exit_code":7,"exit_signal":null}),
            ),
            ..transition(&fixture, 1)
        };
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &original,
            PersistenceMode::Live,
        )
        .unwrap();
        bytes.extend(frame(&original));
    } else {
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='exited',exit_code=7 WHERE run_id=?1",
                [fixture.fixture.run.run_id],
            )
            .unwrap();
    }
    fixture
        .fixture
        .conn
        .execute(
            "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
            [fixture.fixture.run.run_id],
        )
        .unwrap();
    fixture.fixture.run =
        storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    bytes.extend([0, 0, 0, 40, 1, 2, 3]);
    let io = FaultIo::from_bytes(bytes.clone());
    (fixture, io, bytes)
}

#[test]
fn runner_recovery_terminal_tail_compaction_truncates_and_syncs_empty_or_committed_originals() {
    for committed in [false, true] {
        let (mut fixture, io, _) = terminal_spool(committed);
        let mut outbox = open(&fixture, io.clone());
        let scan = outbox.scan(deadline()).unwrap();
        assert!(scan.incomplete_tail.is_some());
        assert_eq!(scan.frames.len(), usize::from(committed));
        let before = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        let syncs = io.0.lock().unwrap().syncs;
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert!(io.snapshot_bytes().is_empty());
        let state = io.0.lock().unwrap();
        assert_eq!(state.truncations.last(), Some(&0));
        assert!(
            state.syncs > syncs,
            "successful tail retirement includes fsync"
        );
        drop(state);
        assert_eq!(
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
            before
        );
    }
}

#[test]
fn runner_recovery_terminal_tail_truncate_failure_retains_bytes_and_retries_the_same_owner() {
    for committed in [false, true] {
        let (mut fixture, io, bytes) = terminal_spool(committed);
        let mut outbox = open(&fixture, io.clone());
        let before = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        io.0.lock().unwrap().fail_truncate = true;
        let error = outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .expect_err("incomplete tail truncate failure remains explicit");
        assert!(error.message().contains("fixture frame truncate failure"));
        assert_eq!(io.snapshot_bytes(), bytes);
        assert_eq!(
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
            before
        );
        let syncs = io.0.lock().unwrap().syncs;
        io.0.lock().unwrap().fail_truncate = false;
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert!(io.snapshot_bytes().is_empty());
        assert!(io.0.lock().unwrap().syncs > syncs);
    }
}

#[test]
fn runner_recovery_terminal_tail_sync_failure_requires_successful_sync_on_retry_even_after_truncation()
 {
    for committed in [false, true] {
        let (mut fixture, io, bytes) = terminal_spool(committed);
        let mut outbox = open(&fixture, io.clone());
        let before = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        io.0.lock().unwrap().fail_sync = true;
        let error = outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .expect_err("incomplete tail sync failure remains explicit");
        assert!(error.message().contains("fixture frame sync failure"));
        let retained = io.snapshot_bytes();
        assert!(
            retained == bytes || retained.is_empty() || retained == bytes[..bytes.len() - 7],
            "failed synchronization preserves a valid original prefix"
        );
        assert_eq!(
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
            before
        );
        let failed_syncs = io.0.lock().unwrap().syncs;
        assert!(failed_syncs > 0);
        io.0.lock().unwrap().fail_sync = false;
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert!(io.snapshot_bytes().is_empty());
        assert!(
            io.0.lock().unwrap().syncs > failed_syncs,
            "a retry completes fsync even when the failed attempt already truncated the file"
        );
    }
}
