use super::*;

fn ordinary_connection_work(conn: &rusqlite::Connection, timeout: i64) {
    assert_eq!(
        conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        timeout
    );
    assert_eq!(
        conn.query_row(
            "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        50005000
    );
}

#[test]
fn runner_review_r2_compaction_cancels_supplied_connection_sql_preserves_bytes_and_retries() {
    let mut fixture = WriterFixture::new();
    let original = transition(&fixture, 1);
    storage::persist_transition(
        &mut fixture.fixture.conn,
        &fixture.fixture.run.owner(),
        &original,
        PersistenceMode::Live,
    )
    .unwrap();
    let io = FaultIo::from_bytes(frame(&original));
    let mut outbox = open(&fixture, io.clone());
    let before = io.snapshot_bytes();
    fixture
        .fixture
        .conn
        .busy_timeout(Duration::from_millis(137))
        .unwrap();
    fixture.fixture.conn.execute_batch(
        "CREATE TEMP VIEW member_runtime_events AS SELECT original.* FROM main.member_runtime_events original WHERE (WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter)>0",
    ).unwrap();
    let interrupt = fixture.fixture.conn.get_interrupt_handle();
    let emergency = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let fired = emergency.clone();
    let (stop, receive) = std::sync::mpsc::channel();
    let begin = Instant::now();
    let until = begin + Duration::from_millis(60);
    let (result, elapsed) = std::thread::scope(|scope| {
        let watchdog = scope.spawn(move || {
            if matches!(
                receive.recv_timeout(Duration::from_millis(650)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                fired.store(true, Ordering::Release);
                interrupt.interrupt();
            }
        });
        let result = outbox.compact_committed(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            until,
        );
        let elapsed = begin.elapsed();
        let _ = stop.send(());
        watchdog.join().unwrap();
        (result, elapsed)
    });
    assert!(
        result.is_err(),
        "expensive supplied-connection SQL must be cancelled"
    );
    assert_eq!(io.snapshot_bytes(), before);
    fixture
        .fixture
        .conn
        .execute_batch("DROP VIEW temp.member_runtime_events")
        .unwrap();
    std::thread::sleep(until.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    ordinary_connection_work(&fixture.fixture.conn, 137);
    outbox
        .compact_committed(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            deadline(),
        )
        .unwrap();
    assert!(io.snapshot_bytes().is_empty());
    ordinary_connection_work(&fixture.fixture.conn, 137);
    assert!(
        elapsed < Duration::from_millis(400) && !emergency.load(Ordering::Acquire),
        "compaction exceeded its 60 ms SQL budget: {elapsed:?}; emergency interrupt fired={} ",
        emergency.load(Ordering::Acquire)
    );
}

struct PanicAtCompaction;
impl WriterHooks for PanicAtCompaction {
    fn before(&self, boundary: WriterBoundary, _: Instant) -> Result<()> {
        if matches!(boundary, WriterBoundary::Compact) {
            panic!("owned compaction unwind");
        }
        Ok(())
    }
}

#[test]
fn runner_review_r2_compaction_restores_supplied_connection_after_success_error_and_unwind() {
    for outcome in ["success", "error", "unwind"] {
        let mut fixture = WriterFixture::new();
        let original = transition(&fixture, 1);
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &original,
            PersistenceMode::Live,
        )
        .unwrap();
        let io = FaultIo::from_bytes(frame(&original));
        let mut outbox = open(&fixture, io.clone());
        let before = io.snapshot_bytes();
        fixture
            .fixture
            .conn
            .busy_timeout(Duration::from_millis(173))
            .unwrap();
        if outcome == "error" {
            fixture
                .fixture
                .conn
                .execute("UPDATE messages SET text='owned invalid association'", [])
                .unwrap();
        }
        if outcome == "unwind" {
            outbox.set_hooks(Arc::new(PanicAtCompaction));
        }
        let until = Instant::now() + Duration::from_millis(80);
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            outbox.compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                until,
            )
        }));
        match outcome {
            "success" => assert!(attempt.unwrap().is_ok()),
            "error" => assert!(attempt.unwrap().is_err()),
            "unwind" => {
                let payload = attempt.expect_err("compaction preserves the original unwind");
                assert_eq!(
                    payload.downcast_ref::<&str>(),
                    Some(&"owned compaction unwind")
                );
            }
            _ => unreachable!(),
        }
        if outcome == "success" {
            assert!(io.snapshot_bytes().is_empty());
        } else {
            assert_eq!(io.snapshot_bytes(), before);
        }
        std::thread::sleep(
            until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
        );
        ordinary_connection_work(&fixture.fixture.conn, 173);
    }
}
