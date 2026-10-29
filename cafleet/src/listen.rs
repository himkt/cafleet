use std::collections::HashMap;
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use rusqlite::Connection;
use serde_json::Value;

use crate::broker::messaging::{InboxClosure, InboxSnapshot, listen_snapshot};
use crate::broker::records::MessageRecord;
use crate::error::CafleetError;
use crate::input::Deadline;
use crate::presentation;
use crate::receive::{Clock, SystemClock};

const WORK_BUDGET: Duration = Duration::from_secs(5);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) trait ListenIo {
    fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError>;
}

pub(crate) trait ListenOutput {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError>;
}

struct BrokerListener<'a> {
    conn: &'a mut Connection,
    member_id: i64,
    fleet_id: Option<i64>,
}

impl ListenIo for BrokerListener<'_> {
    fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError> {
        let snapshot = listen_snapshot(self.conn, self.member_id, self.fleet_id, deadline)?;
        let (InboxSnapshot::Open { fleet_id, .. } | InboxSnapshot::Closed { fleet_id, .. }) =
            &snapshot;
        self.fleet_id = Some(*fleet_id);
        Ok(snapshot)
    }
}

#[derive(Default)]
struct EmittedMessages {
    pending: HashMap<i64, bool>,
}

impl EmittedMessages {
    fn reconcile(&mut self, messages: &[MessageRecord]) {
        for is_pending in self.pending.values_mut() {
            *is_pending = false;
        }
        for message in messages {
            if let Some(is_pending) = self.pending.get_mut(&message.message_id) {
                *is_pending = true;
            }
        }
        self.pending.retain(|_, is_pending| *is_pending);
    }

    fn contains(&self, message_id: i64) -> bool {
        self.pending.contains_key(&message_id)
    }

    fn flushed(&mut self, message_id: i64) {
        self.pending.insert(message_id, true);
    }
}

fn work_deadline(now: Instant, expiry: Option<Instant>) -> Deadline {
    let budget = now + WORK_BUDGET;
    Deadline::until(expiry.map_or(budget, |expiry| expiry.min(budget)))
}

fn expired(now: Instant, expiry: Option<Instant>) -> bool {
    expiry.is_some_and(|expiry| now >= expiry)
}

fn close(
    clock: &impl Clock,
    output: &mut impl ListenOutput,
    member_id: i64,
    reason: &str,
) -> Result<(), CafleetError> {
    output.emit(
        &presentation::listen_closed(member_id, reason),
        work_deadline(clock.now(), None),
    )
}

pub(crate) fn listen(
    clock: &impl Clock,
    io: &mut impl ListenIo,
    output: &mut impl ListenOutput,
    member_id: i64,
    timeout: Option<Duration>,
) -> Result<(), CafleetError> {
    assert!(timeout.is_none_or(|timeout| !timeout.is_zero()));
    let expiry = timeout.map(|timeout| clock.now() + timeout);
    let mut snapshot = io.snapshot(work_deadline(clock.now(), expiry))?;
    let fleet_id = match &snapshot {
        InboxSnapshot::Open { fleet_id, .. } => *fleet_id,
        InboxSnapshot::Closed { .. } => {
            return Err(CafleetError::App(
                "listener subject closed before startup validation".into(),
            ));
        }
    };
    let ready_deadline = work_deadline(clock.now(), expiry);
    ready_deadline.remaining_at(clock.now())?;
    output.emit(
        &presentation::listen_ready(member_id, fleet_id),
        ready_deadline,
    )?;
    let mut emitted = EmittedMessages::default();
    loop {
        let mut new_messages = 0;
        match snapshot {
            InboxSnapshot::Closed {
                fleet_id: current,
                reason,
            } => {
                assert_eq!(current, fleet_id, "the listener retains its pinned fleet");
                return close(
                    clock,
                    output,
                    member_id,
                    match reason {
                        InboxClosure::MemberInactive => "member_inactive",
                        InboxClosure::FleetDeleted => "fleet_deleted",
                    },
                );
            }
            InboxSnapshot::Open {
                fleet_id: current,
                messages,
            } => {
                assert_eq!(current, fleet_id, "the listener retains its pinned fleet");
                emitted.reconcile(&messages);
                for message in messages {
                    if expired(clock.now(), expiry) {
                        return close(clock, output, member_id, "timeout");
                    }
                    if emitted.contains(message.message_id) {
                        continue;
                    }
                    let message_id = message.message_id;
                    let output_deadline = work_deadline(clock.now(), expiry);
                    let record = presentation::listen_message(&message);
                    drop(message);
                    output.emit(&record, output_deadline)?;
                    emitted.flushed(message_id);
                    new_messages += 1;
                }
            }
        }
        let now = clock.now();
        if expired(now, expiry) {
            return close(clock, output, member_id, "timeout");
        }
        if new_messages == 0 {
            clock.sleep(expiry.map_or(IDLE_INTERVAL, |expiry| {
                IDLE_INTERVAL.min(expiry.duration_since(now))
            }));
        }
        if expired(clock.now(), expiry) {
            return close(clock, output, member_id, "timeout");
        }
        snapshot = io.snapshot(work_deadline(clock.now(), expiry))?;
    }
}

fn output_error(error: impl std::fmt::Display) -> CafleetError {
    CafleetError::App(format!("listener output failed: {error}"))
}

pub(crate) fn write_json_line(
    output: &mut impl Write,
    record: &Value,
    deadline: Deadline,
) -> Result<(), CafleetError> {
    deadline.remaining()?;
    let mut bytes = serde_json::to_vec(record).map_err(output_error)?;
    bytes.push(b'\n');
    output.write_all(&bytes).map_err(output_error)?;
    deadline.remaining()?;
    output.flush().map_err(output_error)?;
    deadline.remaining()?;
    Ok(())
}

struct DescriptorWriter<'a> {
    fd: BorrowedFd<'a>,
    deadline: Deadline,
}

impl DescriptorWriter<'_> {
    fn wait_writable(&self) -> io::Result<()> {
        loop {
            let remaining = self
                .deadline
                .remaining()
                .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))?;
            let timeout = PollTimeout::try_from(remaining.min(Duration::from_millis(100)))
                .expect("a bounded output poll fits the platform timeout");
            let mut descriptors = [PollFd::new(self.fd, PollFlags::POLLOUT)];
            match poll(&mut descriptors, timeout) {
                Ok(0) | Err(Errno::EINTR) => continue,
                Ok(_) => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Write for DescriptorWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            self.deadline
                .remaining()
                .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))?;
            match nix::unistd::write(self.fd, bytes) {
                Ok(count) => return Ok(count),
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => self.wait_writable()?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct DescriptorOutput<'a>(BorrowedFd<'a>);

impl ListenOutput for DescriptorOutput<'_> {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError> {
        write_json_line(
            &mut DescriptorWriter {
                fd: self.0,
                deadline,
            },
            record,
            deadline,
        )
    }
}

pub(crate) fn run(
    conn: &mut Connection,
    member_id: i64,
    timeout: Option<Duration>,
) -> Result<(), CafleetError> {
    run_on_descriptor(conn, member_id, timeout, io::stdout().as_fd())
}

fn run_on_descriptor(
    conn: &mut Connection,
    member_id: i64,
    timeout: Option<Duration>,
    descriptor: BorrowedFd<'_>,
) -> Result<(), CafleetError> {
    let original = fcntl(descriptor, FcntlArg::F_GETFL).map_err(output_error)?;
    let flags = OFlag::from_bits_retain(original);
    let result = fcntl(descriptor, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
        .map_err(output_error)
        .and_then(|_| {
            listen(
                &SystemClock,
                &mut BrokerListener {
                    conn,
                    member_id,
                    fleet_id: None,
                },
                &mut DescriptorOutput(descriptor),
                member_id,
                timeout,
            )
        });
    let cleanup = fcntl(descriptor, FcntlArg::F_SETFL(flags)).map_err(output_error);
    match (result, cleanup) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.with_cleanup(format!("restoring stdout flags failed: {cleanup}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run_on_descriptor;
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::cell::{Cell, RefCell};
    use std::io::{self, Write};
    use std::os::fd::{AsFd, OwnedFd};
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    use crate::broker;
    use crate::broker::messaging::test_support::broker_fixture;
    use crate::broker::messaging::{InboxClosure, InboxSnapshot};
    use crate::broker::records::MessageRecord;
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
        MessageRecord::fixture(id, 2, 1, &format!("original body {id}"))
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
            let error = write_json_line(&mut output, &json!({"event":"ready"}), Deadline::new())
                .unwrap_err();
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

    #[test]
    fn real_pipe_backpressure_and_broken_consumers_restore_flags_and_preserve_pending_rows() {
        for (broken, nonblocking) in [(false, false), (false, true), (true, false), (true, true)] {
            let (_dir, mut conn, _, director, worker) = broker_fixture();
            let pending =
                broker::send_message(&mut conn, worker, director, &"雪".repeat(1024 * 1024))
                    .unwrap();
            let (read_end, write_end, retained, original) = primed_pipe(nonblocking);
            let mut read_end = Some(read_end);
            if broken {
                drop(read_end.take());
            }
            let started = Instant::now();
            let error = run_on_descriptor(
                &mut conn,
                director,
                Some(Duration::from_millis(100)),
                write_end.as_fd(),
            )
            .unwrap_err();
            assert!(started.elapsed() < Duration::from_secs(2), "{error}");
            assert_eq!(fcntl(&retained, FcntlArg::F_GETFL).unwrap(), original);
            assert!(conn.is_autocommit());
            assert_eq!(broker::poll_messages(&conn, director).unwrap(), [pending]);
            if let Some(read_end) = read_end {
                let flags = OFlag::from_bits_retain(fcntl(&read_end, FcntlArg::F_GETFL).unwrap());
                fcntl(&read_end, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
                let mut bytes = [0; 4096];
                let count = nix::unistd::read(&read_end, &mut bytes).unwrap();
                let ready = bytes[..count].split(|byte| *byte == b'\n').next().unwrap();
                let ready: Value = serde_json::from_slice(ready).unwrap();
                assert_eq!(ready["event"], "ready");
            }
        }
    }

    #[test]
    fn a_drained_native_pipe_emits_full_jsonl_and_restores_its_shared_flags() {
        use std::io::Read;
        let (_dir, mut conn, _, director, worker) = broker_fixture();
        let pending =
            broker::send_message(&mut conn, worker, director, &"雪\n\t".repeat(32 * 1024)).unwrap();
        let (read_end, write_end, retained, original) = primed_pipe(false);
        let consumer = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            std::fs::File::from(read_end)
                .read_to_end(&mut bytes)
                .unwrap();
            bytes
        });
        run_on_descriptor(
            &mut conn,
            director,
            Some(Duration::from_millis(500)),
            write_end.as_fd(),
        )
        .unwrap();
        assert_eq!(fcntl(&retained, FcntlArg::F_GETFL).unwrap(), original);
        drop(write_end);
        drop(retained);
        let bytes = consumer.join().unwrap();
        let records: Vec<Value> = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["event"], "ready");
        assert_eq!(records[1]["message"]["text"], pending.text);
        assert_eq!(records[2]["reason"], "timeout");
        assert_eq!(broker::poll_messages(&conn, director).unwrap(), [pending]);
        assert!(conn.is_autocommit());
    }

    fn primed_pipe(nonblocking: bool) -> (OwnedFd, OwnedFd, OwnedFd, i32) {
        let (read_end, write_end) = nix::unistd::pipe().unwrap();
        let initial = fcntl(&write_end, FcntlArg::F_GETFL).unwrap();
        let mut flags = OFlag::from_bits_retain(initial) | OFlag::O_APPEND;
        flags.set(OFlag::O_NONBLOCK, nonblocking);
        fcntl(&write_end, FcntlArg::F_SETFL(flags)).unwrap();
        // Darwin adds a kernel-managed flag on first write; establish it before measuring restoration.
        assert_eq!(nix::unistd::write(&write_end, b"x").unwrap(), 1);
        let mut byte = [0];
        assert_eq!(nix::unistd::read(&read_end, &mut byte).unwrap(), 1);
        assert_eq!(byte, *b"x");
        let retained = nix::unistd::dup(&write_end).unwrap();
        let original = fcntl(&retained, FcntlArg::F_GETFL).unwrap();
        let flags = OFlag::from_bits_retain(original);
        assert!(flags.contains(OFlag::O_APPEND));
        assert_eq!(flags.contains(OFlag::O_NONBLOCK), nonblocking);
        (read_end, write_end, retained, original)
    }
}
