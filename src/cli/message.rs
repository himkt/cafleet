use crate::broker;
use crate::broker::MessageRecord;
use crate::broker::messaging::InboxClosure;
use crate::broker::messaging::InboxSnapshot;
use crate::broker::messaging::listen_snapshot;
use crate::cli::emit;
use crate::cli::resolve_body;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::output::format_message;
use crate::output::format_message_sent;
use crate::output::format_messages;
use crate::runtime::ProcessTable;
use crate::runtime::SystemProcessTable;
use crate::time::Clock;
use crate::time::Deadline;
use crate::time::SystemClock;
use clap::Args;
use clap::Subcommand;
use nix::errno::Errno;
use nix::fcntl::FcntlArg;
use nix::fcntl::OFlag;
use nix::fcntl::fcntl;
use nix::poll::PollFd;
use nix::poll::PollFlags;
use nix::poll::PollTimeout;
use nix::poll::poll;
use rusqlite::Connection;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::io;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::fd::BorrowedFd;
use std::time::Duration;
use std::time::Instant;

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct BodyArgs {
    /// Inline message body. Exactly one of TEXT / --file.
    #[arg(value_name = "TEXT")]
    text: Option<String>,
    /// UTF-8 file carrying the body (`-` = stdin).
    #[arg(long, value_name = "PATH")]
    file: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum MessageCommand {
    /// Send a unicast message.
    Send {
        /// Sender's member ID.
        #[arg(long = "from-member-id")]
        from_member_id: i64,
        /// Recipient member ID.
        #[arg(long = "to-member-id")]
        to_member_id: i64,
        #[command(flatten)]
        body: BodyArgs,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Fetch and consume incoming messages.
    Poll {
        /// Recipient whose inbox is fetched.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Notify pending message IDs as JSONL without consuming them.
    Listen {
        /// Active recipient whose inbox is streamed.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Optional monotonic lease in seconds.
        #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: Option<u64>,
        /// Forward each record into this Codex thread through `codex queue`.
        #[arg(long = "codex-thread", value_name = "THREAD")]
        codex_thread: Option<String>,
    },
    /// Show one message.
    Show {
        /// Message to fetch.
        #[arg(value_name = "MESSAGE_ID")]
        message_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: MessageCommand,
) -> Result<(), CafleetError> {
    match command {
        MessageCommand::Send {
            from_member_id,
            to_member_id,
            body,
            json,
        } => {
            let text = resolve_body(body.text.as_deref(), body.file.as_deref())?;
            let deadline = crate::time::Deadline::new();
            let message = crate::db::with_database_budget(conn, deadline, |conn| {
                broker::send_message(conn, from_member_id, to_member_id, &text)
            })?;
            emit(
                json,
                &crate::output::json::message_envelope(&message),
                || format_message_sent(&message),
            )
        }
        MessageCommand::Poll { member_id, json } => {
            let messages = broker::poll_messages(conn, member_id)?;
            let payload = serde_json::Value::Array(
                messages.iter().map(crate::output::json::message).collect(),
            );
            emit(json, &payload, || format_messages(&messages))
        }
        MessageCommand::Listen {
            member_id,
            timeout,
            codex_thread,
        } => {
            let timeout = timeout.map(std::time::Duration::from_secs);
            match codex_thread {
                Some(thread) => run_forwarder(conn, settings, member_id, timeout, &thread),
                None => run_listener(conn, settings, member_id, timeout),
            }
        }
        MessageCommand::Show { message_id, json } => {
            let message = broker::get_message(conn, message_id)?;
            emit(
                json,
                &crate::output::json::message_envelope(&message),
                || format_message(&message),
            )
        }
    }
}

const WORK_BUDGET: Duration = Duration::from_secs(5);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) trait ListenIo {
    fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError>;
    /// Whether the session that owns this listener has ended; a listener without an owning
    /// session never ends this way.
    fn session_exited(&mut self) -> Result<bool, CafleetError> {
        Ok(false)
    }
}

pub(crate) enum Delivery {
    Delivered,
    /// The receiver declined the record for now; the listener retries from a fresh snapshot.
    Refused,
}

pub(crate) trait ListenOutput {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError>;
    /// Whether the owning session ended while a record was being delivered.
    fn session_exited(&self) -> bool {
        false
    }
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

const HOLD_CEILING: Duration = Duration::from_secs(300);

struct Held {
    ids: BTreeSet<i64>,
    at: Instant,
}

/// The IDs of the last delivered notice that are not yet polled; while any remain, later
/// arrivals wait for a consuming poll or the hold ceiling. Every notice names the full pending
/// set and a poll consumes that whole set, so any poll releases the hold.
struct NoticeHold(Option<Held>);

impl NoticeHold {
    fn reconcile(&mut self, pending: &[MessageRecord]) {
        if let Some(held) = &mut self.0 {
            let live: HashSet<i64> = pending.iter().map(|m| m.message_id).collect();
            held.ids.retain(|id| live.contains(id));
            if held.ids.is_empty() {
                self.0 = None;
            }
        }
    }

    fn notice(&self, pending: &[MessageRecord], now: Instant) -> Option<Vec<i64>> {
        if pending.is_empty() {
            return None;
        }
        let ids = || pending.iter().map(|m| m.message_id).collect();
        match &self.0 {
            None => Some(ids()),
            Some(held) => {
                let fresh = pending.iter().any(|m| !held.ids.contains(&m.message_id));
                (fresh && now.duration_since(held.at) >= HOLD_CEILING).then(ids)
            }
        }
    }

    fn commit(&mut self, ids: Vec<i64>, now: Instant) {
        assert!(!ids.is_empty(), "a notice names at least one message");
        self.0 = Some(Held {
            ids: ids.into_iter().collect(),
            at: now,
        });
    }
}

fn assert_delivered(delivery: Delivery) {
    assert!(
        matches!(delivery, Delivery::Delivered),
        "only a pending notice may be refused"
    );
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
    assert_delivered(output.emit(
        &crate::output::json::listen_closed(member_id, reason),
        work_deadline(clock.now(), None),
    )?);
    Ok(())
}

pub(crate) fn listen(
    clock: &impl Clock,
    io: &mut impl ListenIo,
    output: &mut impl ListenOutput,
    member_id: i64,
    timeout: Option<Duration>,
    next: &str,
) -> Result<(), CafleetError> {
    assert!(timeout.is_none_or(|timeout| !timeout.is_zero()));
    let expiry = timeout.map(|timeout| clock.now() + timeout);
    let mut snapshot = io.snapshot(work_deadline(clock.now(), expiry))?;
    let InboxSnapshot::Open { fleet_id, .. } = &snapshot else {
        unreachable!("an unpinned listener snapshot is open or fails")
    };
    let fleet_id = *fleet_id;
    let ready_deadline = work_deadline(clock.now(), expiry);
    ready_deadline.remaining_at(clock.now())?;
    assert_delivered(output.emit(
        &crate::output::json::listen_ready(member_id, fleet_id),
        ready_deadline,
    )?);
    let mut hold = NoticeHold(None);
    loop {
        let mut delivered = false;
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
                hold.reconcile(&messages);
                if let Some(ids) = hold.notice(&messages, clock.now()) {
                    drop(messages);
                    if expired(clock.now(), expiry) {
                        return close(clock, output, member_id, "timeout");
                    }
                    let record = crate::output::json::listen_pending(next);
                    let delivery = output.emit(&record, work_deadline(clock.now(), expiry))?;
                    if output.session_exited() {
                        return close(clock, output, member_id, "session_exited");
                    }
                    if let Delivery::Delivered = delivery {
                        hold.commit(ids, clock.now());
                        delivered = true;
                    }
                }
            }
        }
        let now = clock.now();
        if expired(now, expiry) {
            return close(clock, output, member_id, "timeout");
        }
        if !delivered {
            clock.sleep(expiry.map_or(IDLE_INTERVAL, |expiry| {
                IDLE_INTERVAL.min(expiry.duration_since(now))
            }));
        }
        if expired(clock.now(), expiry) {
            return close(clock, output, member_id, "timeout");
        }
        if io.session_exited()? {
            return close(clock, output, member_id, "session_exited");
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
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError> {
        write_json_line(
            &mut DescriptorWriter {
                fd: self.0,
                deadline,
            },
            record,
            deadline,
        )?;
        Ok(Delivery::Delivered)
    }
}

/// The command a `pending` record names: the listened subject's consuming poll, carrying the
/// database URL exactly when the listener was started with `--database-url`.
pub(crate) fn poll_command(member_id: i64, database_url: Option<&str>) -> String {
    let command = format!("cafleet message poll {member_id} --json");
    match database_url {
        Some(url) => format!("{command} --database-url {}", shell_word(url)),
        None => command,
    }
}

fn shell_word(value: &str) -> Cow<'_, str> {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./:~@%+=,-".contains(c);
    if value.chars().all(safe) {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("'{}'", value.replace('\'', r"'\''")))
    }
}

fn listener_next(settings: &Settings, member_id: i64) -> String {
    poll_command(
        member_id,
        settings
            .database_url_explicit
            .then_some(settings.database_url.as_str()),
    )
}

pub(crate) fn run_listener(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    timeout: Option<Duration>,
) -> Result<(), CafleetError> {
    run_on_descriptor(
        conn,
        member_id,
        timeout,
        &listener_next(settings, member_id),
        io::stdout().as_fd(),
    )
}

fn run_on_descriptor(
    conn: &mut Connection,
    member_id: i64,
    timeout: Option<Duration>,
    next: &str,
    descriptor: BorrowedFd<'_>,
) -> Result<(), CafleetError> {
    with_nonblocking(descriptor, || {
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
            next,
        )
    })
}

/// Runs `body` with `descriptor` switched to nonblocking writes, restoring its flags afterwards.
fn with_nonblocking(
    descriptor: BorrowedFd<'_>,
    body: impl FnOnce() -> Result<(), CafleetError>,
) -> Result<(), CafleetError> {
    let original = fcntl(descriptor, FcntlArg::F_GETFL).map_err(output_error)?;
    let flags = OFlag::from_bits_retain(original);
    let result = fcntl(descriptor, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
        .map_err(output_error)
        .and_then(|_| body());
    let cleanup = fcntl(descriptor, FcntlArg::F_SETFL(flags)).map_err(output_error);
    let cleanup = cleanup.map(|_| ()).map_err(|cleanup| {
        if result.is_err() {
            CafleetError::App(format!("restoring stdout flags failed: {cleanup}"))
        } else {
            cleanup
        }
    });
    CafleetError::merge_cleanup(result, cleanup)
}

const CODEX_QUEUE_BUDGET: Duration = Duration::from_secs(10);
const LISTEN_RECORD_PREFIX: &str = "[cafleet listen record] ";

#[cfg(not(test))]
fn codex_queue_budget() -> Duration {
    CODEX_QUEUE_BUDGET
}

/// Notifies `member_id`'s pending messages into a Codex thread: every notice and closing record
/// goes through `codex queue`, and stdout keeps only the `ready` and final `closed` records.
fn run_forwarder(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    timeout: Option<Duration>,
    thread: &str,
) -> Result<(), CafleetError> {
    use crate::coding_agent::SpawnProbe;
    let codex = crate::runtime::system::SystemProbe
        .resolve_binary("codex")
        .ok_or_else(|| CafleetError::App("binary codex not found on PATH".into()))?;
    let session = pin_codex_session()?.ok_or_else(|| {
        CafleetError::App("--codex-thread requires a codex ancestor process".into())
    })?;
    let next = listener_next(settings, member_id);
    let descriptor = io::stdout();
    let mut output = CodexForwarding {
        stdout: DescriptorOutput(descriptor.as_fd()),
        queue: CodexQueue {
            codex,
            thread,
            session: &session,
        },
        session_exited: false,
    };
    with_nonblocking(descriptor.as_fd(), || {
        listen(
            &SystemClock,
            &mut SessionBoundListener {
                broker: BrokerListener {
                    conn,
                    member_id,
                    fleet_id: None,
                },
                session: &session,
            },
            &mut output,
            member_id,
            timeout,
            &next,
        )
    })
}

struct SessionBoundListener<'a> {
    broker: BrokerListener<'a>,
    session: &'a crate::runtime::ProcessIdentity,
}

impl ListenIo for SessionBoundListener<'_> {
    fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError> {
        self.broker.snapshot(deadline)
    }

    fn session_exited(&mut self) -> Result<bool, CafleetError> {
        pinned_session_exited(self.session)
    }
}

fn pinned_session_exited(session: &crate::runtime::ProcessIdentity) -> Result<bool, CafleetError> {
    let pid = nix::unistd::Pid::from_raw(
        i32::try_from(session.pid).expect("a pinned pid fits the platform pid"),
    );
    if nix::sys::signal::kill(pid, None) == Err(Errno::ESRCH) {
        return Ok(true);
    }
    Ok(!session.matches()?)
}

/// Pins the nearest ancestor whose argv[0] basename is exactly `codex`, skipping shells,
/// sandbox wrappers and the `codex-linux-sandbox` helper, which shares the Codex executable.
fn pin_codex_session() -> Result<Option<crate::runtime::ProcessIdentity>, CafleetError> {
    let table = SystemProcessTable;
    let mut pid = i64::from(nix::unistd::getppid().as_raw());
    while pid > 1 {
        let Some(entry) = table.entry(pid)? else {
            return Ok(None);
        };
        let argv0 = entry.argv.first().map(String::as_str);
        if argv0.and_then(|argv0| argv0.rsplit('/').next()) == Some("codex") {
            return crate::runtime::ProcessIdentity::read(pid);
        }
        pid = entry.parent;
    }
    Ok(None)
}

struct CodexForwarding<'a> {
    stdout: DescriptorOutput<'a>,
    queue: CodexQueue<'a>,
    session_exited: bool,
}

impl ListenOutput for CodexForwarding<'_> {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError> {
        match record["event"].as_str() {
            Some("ready") => self.stdout.emit(record, deadline),
            Some("pending") => match self.queue.forward(record)? {
                Forwarding::Accepted => Ok(Delivery::Delivered),
                Forwarding::SessionExited => {
                    self.session_exited = true;
                    Ok(Delivery::Refused)
                }
            },
            Some("closed") => {
                let session_ended = record["reason"] != "session_exited"
                    && self.queue.forward(record)? == Forwarding::SessionExited;
                let substitute;
                let record = if session_ended {
                    let member_id = record["member_id"]
                        .as_i64()
                        .expect("a closed record names its member");
                    substitute = crate::output::json::listen_closed(member_id, "session_exited");
                    &substitute
                } else {
                    record
                };
                // Forwarding may outlast the closing budget, so the write takes a fresh one. The
                // final record may target the exited session's PTY; the exit status reflects
                // reception, never this write.
                self.stdout
                    .emit(record, work_deadline(Instant::now(), None))
                    .ok();
                Ok(Delivery::Delivered)
            }
            other => unreachable!("the listener emits no {other:?} record"),
        }
    }

    fn session_exited(&self) -> bool {
        self.session_exited
    }
}

#[derive(PartialEq)]
enum Forwarding {
    Accepted,
    /// The pinned session ended before the thread accepted the record, so it has no reader.
    SessionExited,
}

enum QueueFailure {
    Exit(String),
    Deadline,
}

struct CodexQueue<'a> {
    codex: std::path::PathBuf,
    thread: &'a str,
    session: &'a crate::runtime::ProcessIdentity,
}

impl CodexQueue<'_> {
    /// Queues `record` once; a failed attempt ends forwarding.
    fn forward(&self, record: &Value) -> Result<Forwarding, CafleetError> {
        match self.queue(&format!("{LISTEN_RECORD_PREFIX}{record}")) {
            Ok(()) => Ok(Forwarding::Accepted),
            Err(_) if pinned_session_exited(self.session)? => Ok(Forwarding::SessionExited),
            Err(QueueFailure::Deadline) => Err(CafleetError::App(self.deadline_diagnostic())),
            Err(QueueFailure::Exit(cause)) => Err(CafleetError::App(format!(
                "failed to forward record to codex thread {}: {cause}",
                self.thread
            ))),
        }
    }

    fn deadline_diagnostic(&self) -> String {
        format!(
            "codex queue for thread {} did not finish within {} s",
            self.thread,
            codex_queue_budget().as_secs()
        )
    }

    fn queue(&self, text: &str) -> Result<(), QueueFailure> {
        use std::io::Read;
        let mut child = std::process::Command::new(&self.codex)
            .args(["queue", "--thread", self.thread, "--message", text])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| QueueFailure::Exit(format!("could not start codex: {error}")))?;
        let mut stderr = child.stderr.take().expect("piped codex stderr");
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            sender
                .send(stderr.read_to_string(&mut text).map(|_| text))
                .ok();
        });
        let deadline = Instant::now() + codex_queue_budget();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    child.kill().ok();
                    child.wait().ok();
                    return Err(QueueFailure::Deadline);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    return Err(QueueFailure::Exit(format!(
                        "could not wait for codex: {error}"
                    )));
                }
            }
        };
        if status.success() {
            return Ok(());
        }
        let stderr = receiver
            .recv_timeout(Duration::from_secs(1))
            .ok()
            .and_then(Result::ok)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        Err(QueueFailure::Exit(
            stderr.unwrap_or_else(|| format!("codex exited with {status}")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::message::*;

    mod runner_pull {
        use crate::runtime::service::test_support::input_service::*;

        #[test]
        fn message_send_stores_the_message_and_writes_no_byte_to_the_recipients_child() {
            with_input_service(|fixture, _| {
                let run = fixture.fixture.run.clone();
                let url = format!("sqlite:///{}", fixture.fixture.conn.path().unwrap());
                let (from, to) = (
                    run.director_member_id.to_string(),
                    run.member_id.to_string(),
                );
                crate::cli::run_args(
                    [
                        "cafleet",
                        "--database-url",
                        &url,
                        "message",
                        "send",
                        "--from-member-id",
                        &from,
                        "--to-member-id",
                        &to,
                        "stored for the runner",
                    ]
                    .map(std::ffi::OsString::from),
                )
                .unwrap();
                assert!(input_service_records(fixture).is_empty());
                let (status, notified_at): (String, Option<String>) = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT status_state,notified_at FROM messages WHERE to_member_id=?1 AND text='stored for the runner'",
                        [run.member_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(status, "input_required");
                assert_eq!(notified_at, None);
            });
        }
    }

    #[test]
    fn message_subcommands_outside_the_registry_are_unrecognized() {
        for args in [
            ["message", "ack", "44"],
            ["message", "wait", "17"],
            ["message", "listen", "1"],
        ] {
            let error = crate::cli::test_support::usage_error(&args);
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::InvalidSubcommand,
                "{args:?}: {error}"
            );
            assert!(error.to_string().contains(args[1]), "{args:?}: {error}");
        }
    }

    mod next_command {
        use crate::cli::message::tests::*;

        #[test]
        fn the_next_poll_command_appends_the_database_url_quoted_only_when_needed() {
            let every_safe_class = "sqlite:///Home/u_1/~x/a-b.c@d%e+f=g,h.db";
            for (name, member, url, expected) in [
                (
                    "omitted database URL",
                    7,
                    None,
                    "cafleet message poll 7 --json".to_owned(),
                ),
                (
                    "only safe characters",
                    7,
                    Some("sqlite:///tmp/fleet.db"),
                    "cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"
                        .to_owned(),
                ),
                (
                    "every safe character class",
                    42,
                    Some(every_safe_class),
                    format!("cafleet message poll 42 --json --database-url {every_safe_class}"),
                ),
                (
                    "a space",
                    7,
                    Some("sqlite:///tmp/my fleet.db"),
                    "cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"
                        .to_owned(),
                ),
                (
                    "a single quote closed, escaped and reopened",
                    7,
                    Some("sqlite:///tmp/it's.db"),
                    r"cafleet message poll 7 --json --database-url 'sqlite:///tmp/it'\''s.db'"
                        .to_owned(),
                ),
            ] {
                assert_eq!(poll_command(member, url), expected, "{name}");
            }
        }

        #[test]
        fn every_character_outside_the_safe_set_quotes_the_whole_url() {
            for unsafe_char in ['$', ';', '*', '"', '\\', '&', '|', '`', '#', '!', '\t', 'é'] {
                let url = format!("sqlite:///tmp/a{unsafe_char}b.db");
                assert_eq!(
                    poll_command(3, Some(&url)),
                    format!("cafleet message poll 3 --json --database-url '{url}'"),
                    "{unsafe_char:?}"
                );
            }
        }
    }
}
