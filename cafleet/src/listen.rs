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
    let mut offset = 0;
    while offset < bytes.len() {
        deadline.remaining()?;
        match output.write(&bytes[offset..]) {
            Ok(0) => return Err(output_error(io::ErrorKind::WriteZero)),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(deadline.remaining()?.min(Duration::from_millis(1)));
            }
            Err(error) => return Err(output_error(error)),
        }
    }
    loop {
        deadline.remaining()?;
        match output.flush() {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(deadline.remaining()?.min(Duration::from_millis(1)));
            }
            Err(error) => return Err(output_error(error)),
        }
    }
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
    let stdout = io::stdout();
    let original = fcntl(&stdout, FcntlArg::F_GETFL).map_err(output_error)?;
    let flags = OFlag::from_bits_retain(original);
    let result = fcntl(&stdout, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
        .map_err(output_error)
        .and_then(|_| {
            listen(
                &SystemClock,
                &mut BrokerListener {
                    conn,
                    member_id,
                    fleet_id: None,
                },
                &mut DescriptorOutput(stdout.as_fd()),
                member_id,
                timeout,
            )
        });
    let cleanup = fcntl(&stdout, FcntlArg::F_SETFL(flags)).map_err(output_error);
    match (result, cleanup) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.with_cleanup(format!("restoring stdout flags failed: {cleanup}")))
        }
    }
}
