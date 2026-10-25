use std::cell::{Cell, RefCell};
use std::io::{self, Write};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::broker::messaging::{
    InboxClosure, InboxSnapshot, listen_snapshot, listen_snapshot_with_clock,
};
use crate::broker::records::{MessageRecord, MessageStatus};
use crate::broker::{self, test_support as common};
use crate::error::CafleetError;
use crate::input::Deadline;
use crate::listen::{ListenIo, ListenOutput, listen, write_json_line};
use crate::receive::Clock;

struct TestClock {
    origin: Instant,
    elapsed: Cell<Duration>,
    sleeps: RefCell<Vec<Duration>>,
}

impl TestClock {
    fn new() -> Self {
        Self {
            origin: Instant::now() + Duration::from_secs(10_000),
            elapsed: Cell::new(Duration::ZERO),
            sleeps: RefCell::new(Vec::new()),
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        self.origin + self.elapsed.get()
    }

    fn sleep(&self, duration: Duration) {
        assert!(!duration.is_zero(), "idle checks advance the clock");
        self.sleeps.borrow_mut().push(duration);
        self.elapsed.set(self.elapsed.get() + duration);
    }
}

struct ScriptedInbox<'a, F> {
    clock: &'a TestClock,
    snapshots: Vec<(Duration, Duration)>,
    script: F,
}

impl<F> ListenIo for ScriptedInbox<'_, F>
where
    F: FnMut(Duration, usize) -> Result<InboxSnapshot, CafleetError>,
{
    fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError> {
        let now = self.clock.elapsed.get();
        self.snapshots
            .push((now, deadline.remaining_at(self.clock.now())?));
        assert!(
            self.snapshots.len() < 30,
            "pending emitted IDs must allow idle sleeps"
        );
        (self.script)(now, self.snapshots.len())
    }
}

struct RecordingOutput<'a> {
    clock: &'a TestClock,
    records: Vec<Value>,
    budgets: Vec<Duration>,
    fail_at: Option<usize>,
}

impl<'a> RecordingOutput<'a> {
    fn new(clock: &'a TestClock) -> Self {
        Self {
            clock,
            records: Vec::new(),
            budgets: Vec::new(),
            fail_at: None,
        }
    }
}

impl ListenOutput for RecordingOutput<'_> {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError> {
        self.budgets.push(deadline.remaining_at(self.clock.now())?);
        if self.fail_at == Some(self.records.len()) {
            return Err(CafleetError::App("injected output failure".into()));
        }
        self.records.push(record.clone());
        Ok(())
    }
}

fn message(id: i64) -> MessageRecord {
    MessageRecord {
        message_id: id,
        from_member_id: 2,
        to_member_id: 1,
        created_at: "2026-10-25T01:00:00.000000+00:00".into(),
        status: MessageStatus::InputRequired,
        status_timestamp: "2026-10-25T01:00:00.000000+00:00".into(),
        text: format!("original body {id}"),
    }
}

fn open(messages: Vec<MessageRecord>) -> InboxSnapshot {
    InboxSnapshot::Open {
        fleet_id: 7,
        messages,
    }
}

fn event_ids(output: &RecordingOutput<'_>) -> Vec<i64> {
    output
        .records
        .iter()
        .filter(|record| record["event"] == "message")
        .map(|record| record["message"]["message_id"].as_i64().unwrap())
        .collect()
}

#[test]
fn an_unacked_a_allows_one_second_idle_sleeps_before_b_arrives_without_repeating_a() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |at, _| {
            let messages = if at < Duration::from_secs(2) {
                vec![message(10)]
            } else {
                vec![message(10), message(11)]
            };
            Ok(open(messages))
        },
    };
    let mut output = RecordingOutput::new(&clock);
    listen(
        &clock,
        &mut io,
        &mut output,
        1,
        Some(Duration::from_secs(3)),
    )
    .unwrap();
    assert_eq!(event_ids(&output), [10, 11]);
    assert_eq!(*clock.sleeps.borrow(), vec![Duration::from_secs(1); 3]);
    assert_eq!(
        io.snapshots
            .iter()
            .map(|(at, _)| at.as_secs())
            .collect::<Vec<_>>(),
        [0, 0, 1, 2, 2]
    );
    assert_eq!(
        output.records[0],
        json!({"event":"ready", "member_id":1, "fleet_id":7})
    );
    assert_eq!(
        output.records.last().unwrap(),
        &json!({"event":"closed", "member_id":1, "reason":"timeout"})
    );
}

#[test]
fn idle_checks_use_the_remaining_monotonic_lease_without_heartbeats() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, _| Ok(open(Vec::new())),
    };
    let mut output = RecordingOutput::new(&clock);
    listen(
        &clock,
        &mut io,
        &mut output,
        1,
        Some(Duration::from_millis(2500)),
    )
    .unwrap();
    assert_eq!(clock.elapsed.get(), Duration::from_millis(2500));
    assert_eq!(
        *clock.sleeps.borrow(),
        [
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(500)
        ]
    );
    assert_eq!(
        io.snapshots
            .iter()
            .map(|(_, budget)| budget.as_millis())
            .collect::<Vec<_>>(),
        [2500, 1500, 500]
    );
    assert_eq!(output.records.len(), 2);
    assert_eq!(
        output.budgets,
        [Duration::from_millis(2500), Duration::from_secs(5)]
    );
}

#[test]
fn activity_keeps_the_original_expiry_and_every_work_budget_is_at_most_five_seconds() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |at: Duration, _: usize| {
            Ok(open(vec![message(i64::try_from(at.as_secs()).unwrap())]))
        },
    };
    let mut output = RecordingOutput::new(&clock);
    listen(
        &clock,
        &mut io,
        &mut output,
        1,
        Some(Duration::from_secs(8)),
    )
    .unwrap();
    assert_eq!(clock.elapsed.get(), Duration::from_secs(8));
    assert_eq!(event_ids(&output), (0..8).collect::<Vec<_>>());
    assert!(
        io.snapshots
            .iter()
            .all(|(_, budget)| *budget <= Duration::from_secs(5))
    );
    assert!(
        output
            .budgets
            .iter()
            .all(|budget| *budget <= Duration::from_secs(5))
    );
}

#[test]
fn an_unbounded_lease_stops_on_observed_closure_and_maps_its_reason() {
    for (closure, reason) in [
        (InboxClosure::MemberInactive, "member_inactive"),
        (InboxClosure::FleetDeleted, "fleet_deleted"),
    ] {
        let clock = TestClock::new();
        let mut io = ScriptedInbox {
            clock: &clock,
            snapshots: Vec::new(),
            script: |at, _| {
                if at < Duration::from_secs(2) {
                    Ok(open(Vec::new()))
                } else {
                    Ok(InboxSnapshot::Closed {
                        fleet_id: 7,
                        reason: closure,
                    })
                }
            },
        };
        let mut output = RecordingOutput::new(&clock);
        listen(&clock, &mut io, &mut output, 1, None).unwrap();
        assert_eq!(
            output.records,
            [
                json!({"event":"ready", "member_id":1, "fleet_id":7}),
                json!({"event":"closed", "member_id":1, "reason":reason})
            ]
        );
        assert!(
            io.snapshots
                .iter()
                .all(|(_, budget)| *budget == Duration::from_secs(5))
        );
    }
}

#[test]
fn startup_validation_errors_precede_ready_and_midstream_errors_preserve_complete_records() {
    for fail_at in [1, 2] {
        let clock = TestClock::new();
        let mut io = ScriptedInbox {
            clock: &clock,
            snapshots: Vec::new(),
            script: |_, check| {
                if check == fail_at {
                    Err(CafleetError::App("snapshot failed".into()))
                } else {
                    Ok(open(vec![message(10)]))
                }
            },
        };
        let mut output = RecordingOutput::new(&clock);
        let error = listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(3)),
        )
        .unwrap_err();
        assert!(error.message().contains("snapshot failed"));
        assert_eq!(output.records.len(), if fail_at == 1 { 0 } else { 2 });
        assert!(
            output
                .records
                .iter()
                .all(|record| record["event"] != "closed")
        );
    }
}

#[test]
fn a_startup_snapshot_that_outlives_the_lease_fails_before_emitting_ready() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, check| {
            assert_eq!(check, 1);
            clock.elapsed.set(Duration::from_secs(2));
            Ok(open(Vec::new()))
        },
    };
    let mut output = RecordingOutput::new(&clock);
    assert!(matches!(
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(1))
        ),
        Err(CafleetError::DeadlineExceeded)
    ));
    assert!(output.records.is_empty());
    assert_eq!(io.snapshots.len(), 1);
}

#[test]
fn a_snapshot_budget_failure_terminates_the_stream_instead_of_becoming_a_timeout() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, check| {
            if check == 1 {
                Ok(open(Vec::new()))
            } else {
                Err(CafleetError::DeadlineExceeded)
            }
        },
    };
    let mut output = RecordingOutput::new(&clock);
    assert!(matches!(
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(2))
        ),
        Err(CafleetError::DeadlineExceeded)
    ));
    assert_eq!(output.records.len(), 1);
}

#[test]
fn output_failure_stops_before_another_snapshot_and_retains_only_flushed_records() {
    for fail_at in [0, 1, 2] {
        let clock = TestClock::new();
        let mut io = ScriptedInbox {
            clock: &clock,
            snapshots: Vec::new(),
            script: |_, _| Ok(open(vec![message(10), message(11)])),
        };
        let mut output = RecordingOutput::new(&clock);
        output.fail_at = Some(fail_at);
        let error = listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(2)),
        )
        .unwrap_err();
        assert!(error.message().contains("injected output failure"));
        assert_eq!(output.records.len(), fail_at);
        assert_eq!(io.snapshots.len(), 1);
    }
}

#[test]
fn an_ack_output_race_preserves_the_snapshot_record_and_a_restart_replays_only_pending_ids() {
    let clock = TestClock::new();
    let acknowledged = Cell::new(false);
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, _| {
            Ok(open(if acknowledged.get() {
                vec![message(11)]
            } else {
                vec![message(10), message(11)]
            }))
        },
    };
    struct AckDuringReady<'a> {
        output: RecordingOutput<'a>,
        acknowledged: &'a Cell<bool>,
    }
    impl ListenOutput for AckDuringReady<'_> {
        fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError> {
            self.output.emit(record, deadline)?;
            if record["event"] == "ready" {
                self.acknowledged.set(true);
            }
            if record["event"] == "message" {
                assert!(self.acknowledged.get());
            }
            Ok(())
        }
    }
    let mut output = AckDuringReady {
        output: RecordingOutput::new(&clock),
        acknowledged: &acknowledged,
    };
    listen(
        &clock,
        &mut io,
        &mut output,
        1,
        Some(Duration::from_secs(1)),
    )
    .unwrap();
    assert_eq!(event_ids(&output.output), [10, 11]);
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, _| Ok(open(vec![message(11)])),
    };
    let mut restarted = RecordingOutput::new(&clock);
    listen(
        &clock,
        &mut io,
        &mut restarted,
        1,
        Some(Duration::from_secs(1)),
    )
    .unwrap();
    assert_eq!(event_ids(&restarted), [11]);
}

#[test]
fn a_failed_final_closed_record_is_an_output_error() {
    let clock = TestClock::new();
    let mut io = ScriptedInbox {
        clock: &clock,
        snapshots: Vec::new(),
        script: |_, _| Ok(open(Vec::new())),
    };
    let mut output = RecordingOutput::new(&clock);
    output.fail_at = Some(1);
    let error = listen(
        &clock,
        &mut io,
        &mut output,
        1,
        Some(Duration::from_secs(1)),
    )
    .unwrap_err();
    assert!(error.message().contains("injected output failure"));
    assert_eq!(output.records.len(), 1);
    assert_eq!(output.budgets.last(), Some(&Duration::from_secs(5)));
}

struct FaultWriter {
    bytes: Vec<u8>,
    chunk: usize,
    stop_at: Option<usize>,
    zero: bool,
    fail_flush: bool,
    flushes: usize,
}

impl Write for FaultWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.zero {
            return Ok(0);
        }
        if self.stop_at == Some(self.bytes.len()) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected broken pipe",
            ));
        }
        let count = buffer.len().min(self.chunk).min(
            self.stop_at
                .map_or(usize::MAX, |stop| stop - self.bytes.len()),
        );
        self.bytes.extend_from_slice(&buffer[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}

fn writer() -> FaultWriter {
    FaultWriter {
        bytes: Vec::new(),
        chunk: 3,
        stop_at: None,
        zero: false,
        fail_flush: false,
        flushes: 0,
    }
}

#[test]
fn a_record_continues_partial_writes_from_the_byte_offset_and_flushes_the_lf() {
    let mut output = writer();
    let record = json!({"event":"message", "message":{"text":"界\n\t🙂"}});
    write_json_line(&mut output, &record, Deadline::new()).unwrap();
    let mut expected = serde_json::to_vec(&record).unwrap();
    expected.push(b'\n');
    assert_eq!(output.bytes, expected);
    assert_eq!(output.flushes, 1);
}

#[test]
fn write_zero_broken_pipe_and_flush_failure_are_visible_record_failures() {
    for failure in ["zero", "broken", "flush"] {
        let mut output = writer();
        match failure {
            "zero" => output.zero = true,
            "broken" => output.stop_at = Some(5),
            "flush" => output.fail_flush = true,
            _ => unreachable!(),
        }
        let error =
            write_json_line(&mut output, &json!({"event":"ready"}), Deadline::new()).unwrap_err();
        assert!(!error.message().is_empty());
        if failure == "broken" {
            assert_eq!(output.bytes.len(), 5);
            assert_eq!(output.flushes, 0);
        }
        if failure == "flush" {
            assert!(error.message().contains("injected flush failure"));
        }
    }
}

#[test]
fn an_expired_record_budget_fails_before_writing_any_bytes() {
    let mut output = writer();
    let deadline = Deadline::until(Instant::now() - Duration::from_millis(1));
    assert!(matches!(
        write_json_line(&mut output, &json!({"event":"ready"}), deadline),
        Err(CafleetError::DeadlineExceeded)
    ));
    assert!(output.bytes.is_empty());
    assert_eq!(output.flushes, 0);
}

fn broker_fixture() -> (tempfile::TempDir, Connection, i64, i64, i64) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = common::migrated_conn(&dir);
    let (fleet, director) = common::create_fleet(&mut conn, "listen-snapshot");
    let worker = common::register(&mut conn, fleet, "worker", Some("%2"));
    (dir, conn, fleet, director, worker)
}

fn snapshot_messages(snapshot: InboxSnapshot, expected_fleet: i64) -> Vec<MessageRecord> {
    match snapshot {
        InboxSnapshot::Open { fleet_id, messages } => {
            assert_eq!(fleet_id, expected_fleet);
            messages
        }
        InboxSnapshot::Closed { .. } => panic!("the live inbox remains open"),
    }
}

fn assert_handler_cleared(conn: &Connection) {
    let result: i64 = conn.query_row(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT sum(x) FROM n", [], |row| row.get(0),
    ).expect("a later SQL query runs after the listener-owned deadline expires");
    assert_eq!(result, 50_005_000);
    assert!(
        conn.is_autocommit(),
        "the snapshot releases its transaction"
    );
}

#[test]
fn broker_snapshots_are_read_only_oldest_first_and_ignore_preview_metadata() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    let later = broker::send_message(&mut conn, worker, &director.to_string(), "later").unwrap();
    let earlier = broker::send_message(&mut conn, director, &director.to_string(), "self").unwrap();
    let tied = broker::send_message(&mut conn, worker, &director.to_string(), "claimed").unwrap();
    conn.execute("UPDATE messages SET created_at='1900-01-01T00:00:00+00:00', preview_started_at=status_timestamp WHERE message_id IN (?1, ?2)", [earlier.message_id, tied.message_id]).unwrap();
    let changes = conn.total_changes();
    let messages = snapshot_messages(
        listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
        fleet,
    );
    assert_eq!(
        messages.iter().map(|m| m.message_id).collect::<Vec<_>>(),
        [earlier.message_id, tied.message_id, later.message_id]
    );
    assert_eq!(conn.total_changes(), changes);
    assert_handler_cleared(&conn);
}

#[test]
fn broker_snapshots_exclude_acks_committed_before_the_snapshot() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    let acknowledged =
        broker::send_message(&mut conn, worker, &director.to_string(), "consumed").unwrap();
    broker::ack_message(&mut conn, acknowledged.message_id).unwrap();
    assert!(
        snapshot_messages(
            listen_snapshot(&mut conn, director, None, Deadline::new()).unwrap(),
            fleet
        )
        .is_empty()
    );
}

#[test]
fn broker_snapshots_validate_pinned_identity_and_distinguish_normal_closure() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    assert!(listen_snapshot(&mut conn, worker, Some(fleet + 1), Deadline::new()).is_err());
    conn.execute(
        "UPDATE members SET status='deregistered' WHERE member_id=?1",
        [worker],
    )
    .unwrap();
    assert!(listen_snapshot(&mut conn, worker, None, Deadline::new()).is_err());
    assert!(matches!(
        listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap(),
        InboxSnapshot::Closed {
            reason: InboxClosure::MemberInactive,
            ..
        }
    ));
    conn.execute(
        "UPDATE fleets SET deleted_at=created_at WHERE fleet_id=?1",
        [fleet],
    )
    .unwrap();
    assert!(matches!(
        listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap(),
        InboxSnapshot::Closed {
            reason: InboxClosure::FleetDeleted,
            ..
        }
    ));
    assert!(matches!(
        listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
        InboxSnapshot::Closed {
            reason: InboxClosure::FleetDeleted,
            ..
        }
    ));
}

#[test]
fn broker_snapshot_requires_a_concrete_root_placement_but_allows_no_subject_placement() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    conn.execute("DELETE FROM member_placements WHERE member_id=?1", [worker])
        .unwrap();
    listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).unwrap();
    conn.execute(
        "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
        [director],
    )
    .unwrap();
    assert!(listen_snapshot(&mut conn, worker, Some(fleet), Deadline::new()).is_err());
    assert_handler_cleared(&conn);
}

#[test]
fn a_long_running_select_without_lock_contention_is_interrupted_and_cleans_its_handler() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    broker::send_message(
        &mut conn,
        worker,
        &director.to_string(),
        "expensive selection",
    )
    .unwrap();
    conn.execute_batch(
        "ALTER TABLE messages RENAME TO stored_messages;
         CREATE VIEW messages AS
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000)
         SELECT m.message_id+n.x*1000000 AS message_id, m.from_member_id, m.to_member_id,
                printf('%09d', 1000000-n.x) AS created_at, m.status_state, m.status_timestamp, m.text,
                m.notified_at, m.preview_started_at
         FROM stored_messages m CROSS JOIN n;"
    ).unwrap();
    let started = Instant::now();
    let result = listen_snapshot(
        &mut conn,
        director,
        Some(fleet),
        Deadline::until(started + Duration::from_millis(25)),
    );
    assert!(
        matches!(result, Err(CafleetError::DeadlineExceeded)),
        "{result:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_handler_cleared(&conn);
}

#[test]
fn row_collection_discards_an_expired_partial_snapshot_using_clock_boundary_checks() {
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    for index in 0..100 {
        broker::send_message(
            &mut conn,
            worker,
            &director.to_string(),
            &format!("row {index}"),
        )
        .unwrap();
    }
    let origin = Instant::now() + Duration::from_secs(10_000);
    let reads = Cell::new(0_u64);
    let deadline = Deadline::until(origin + Duration::from_millis(40));
    let result = listen_snapshot_with_clock(&mut conn, director, Some(fleet), deadline, || {
        let read = reads.get();
        reads.set(read + 1);
        origin + Duration::from_millis(read)
    });
    assert!(
        matches!(result, Err(CafleetError::DeadlineExceeded)),
        "{result:?}"
    );
    assert!(reads.get() >= 40);
    assert_handler_cleared(&conn);
    assert_eq!(
        snapshot_messages(
            listen_snapshot(&mut conn, director, Some(fleet), Deadline::new()).unwrap(),
            fleet
        )
        .len(),
        100
    );
}

#[test]
fn success_and_query_failure_both_remove_the_owned_handler_and_restore_busy_timeout() {
    for corrupt in [false, true] {
        let (_dir, mut conn, fleet, director, _) = broker_fixture();
        conn.busy_timeout(Duration::from_millis(137)).unwrap();
        if corrupt {
            conn.execute_batch("DROP TABLE messages").unwrap();
        }
        let deadline = Deadline::until(Instant::now() + Duration::from_millis(50));
        let result = listen_snapshot(&mut conn, director, Some(fleet), deadline);
        assert_eq!(result.is_err(), corrupt);
        std::thread::sleep(Duration::from_millis(60));
        assert_handler_cleared(&conn);
        let timeout: u32 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, 137);
    }
}

#[test]
fn a_failed_snapshot_commit_preserves_rollback_failure_and_removes_the_progress_handler() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    use std::sync::{Arc, Mutex};

    let (_dir, mut conn, fleet, director, _) = broker_fixture();
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&attempts);
    conn.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            // rusqlite 0.39 maps SQLite's COMMIT authorizer operation to Unknown.
            operation: TransactionOperation::Unknown,
        } => {
            observed.lock().unwrap().push("commit");
            Authorization::Deny
        }
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => {
            observed.lock().unwrap().push("rollback");
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))
    .unwrap();
    let expiry = Instant::now() + Duration::from_millis(200);
    let result = listen_snapshot(&mut conn, director, Some(fleet), Deadline::until(expiry));
    let transaction_open = !conn.is_autocommit();
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    if transaction_open {
        conn.execute_batch("ROLLBACK").unwrap();
    }
    std::thread::sleep(
        expiry.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
    );
    assert_handler_cleared(&conn);

    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts.first(), Some(&"commit"));
    assert!(attempts[1..].contains(&"rollback"), "{attempts:?}");
    assert!(
        transaction_open,
        "the denied rollback leaves its transaction open for explicit cleanup"
    );
    let error = result.unwrap_err();
    let diagnostic = error.message();
    assert!(
        diagnostic.contains("rolling back listener snapshot failed"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.matches("not authorized").count() >= 2,
        "commit and rollback causes both remain visible: {diagnostic}"
    );
}

#[test]
fn an_unrelated_sqlite_interrupt_preserves_its_cause_before_the_deadline() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let (_dir, mut conn, fleet, director, worker) = broker_fixture();
    broker::send_message(
        &mut conn,
        worker,
        &director.to_string(),
        "externally interrupted selection",
    )
    .unwrap();
    conn.execute_batch(
        "ALTER TABLE messages RENAME TO stored_messages;
         CREATE VIEW messages AS
         WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000)
         SELECT m.message_id+n.x*1000000 AS message_id, m.from_member_id, m.to_member_id,
                printf('%09d', 1000000-n.x) AS created_at, m.status_state, m.status_timestamp, m.text,
                m.notified_at, m.preview_started_at
         FROM stored_messages m CROSS JOIN n;"
    ).unwrap();
    let interrupt = conn.get_interrupt_handle();
    let (prepared, preparation) = std::sync::mpsc::sync_channel(1);
    let interrupted = std::thread::spawn(move || {
        preparation.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        interrupt.interrupt();
    });
    conn.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(
            context.action,
            AuthAction::Read {
                table_name: "messages",
                ..
            }
        ) {
            match prepared.try_send(()) {
                Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => {}
                Err(error) => panic!("interruption observer disappeared: {error}"),
            }
        }
        Authorization::Allow
    }))
    .unwrap();
    let result = listen_snapshot(&mut conn, director, Some(fleet), Deadline::new());
    interrupted.join().unwrap();
    assert!(result.is_err());
    assert!(!matches!(result, Err(CafleetError::DeadlineExceeded)));
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_handler_cleared(&conn);
}
