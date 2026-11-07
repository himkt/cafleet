use crate::broker;
use crate::broker::MessageRecord;
use crate::broker::messaging::InboxClosure;
use crate::broker::messaging::InboxSnapshot;
use crate::broker::messaging::listen_snapshot;
use crate::broker::runtime::DeliveryProjection;
use crate::broker::runtime::DeliveryStatus;
use crate::cli::emit;
use crate::cli::resolve_body;
use crate::config::Settings;
use crate::delivery::AttemptEnd;
use crate::delivery::AttemptOutcome;
use crate::delivery::BrokerReceiver;
use crate::error::CafleetError;
use crate::output::format_message;
use crate::output::format_message_acknowledged;
use crate::output::format_message_sent;
use crate::output::format_message_wait;
use crate::output::format_messages;
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
    /// Fetch un-acked incoming messages.
    Poll {
        /// Recipient whose inbox is fetched.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Receive incoming messages and retry pending ordinary-member previews.
    Wait {
        /// Recipient whose inbox is received.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Maximum seconds to wait.
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=1800))]
        timeout: u64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Stream full pending inbox records as JSONL without consuming them.
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
    /// Acknowledge a received message.
    Ack {
        /// Message to acknowledge.
        #[arg(value_name = "MESSAGE_ID")]
        message_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
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
            let text = resolve_body(body.text.as_deref(), body.file.as_deref(), "--file")?;
            let deadline = crate::time::Deadline::new();
            let message = crate::db::with_database_budget(conn, deadline, |conn| {
                broker::send_message(conn, from_member_id, to_member_id, &text)
            })?;
            let message_id = message.message_id;
            let outcome = if from_member_id == to_member_id {
                None
            } else {
                match crate::delivery::deliver_pending(
                    conn,
                    settings,
                    to_member_id,
                    None,
                    deadline,
                    &std::collections::BTreeSet::new(),
                ) {
                    Ok(outcome) => {
                        if let AttemptEnd::Failed(error) = &outcome.end {
                            eprintln!("Message {message_id} was persisted. {}", error.error);
                        }
                        Some(outcome)
                    }
                    Err(error) => {
                        eprintln!("Message {message_id} was persisted. {}", error.message());
                        None
                    }
                }
            };
            let notification_sent = outcome
                .as_ref()
                .is_some_and(|outcome| outcome.sent.contains(&message_id));
            let (input_id, delivery) = send_delivery(conn, &message, outcome.as_ref())?;
            emit(
                json,
                &crate::output::json::send_outcome(
                    &message,
                    notification_sent,
                    input_id,
                    &delivery,
                ),
                || format_message_sent(&message, input_id, &delivery),
            )
        }
        MessageCommand::Poll { member_id, json } => {
            let messages = broker::poll_messages(conn, member_id)?;
            let payload = serde_json::Value::Array(
                messages.iter().map(crate::output::json::message).collect(),
            );
            emit(json, &payload, || format_messages(&messages))
        }
        MessageCommand::Wait {
            member_id,
            timeout,
            json,
        } => {
            let mut receiver = BrokerReceiver::new(conn, settings, member_id);
            let messages = &crate::delivery::wait(
                &SystemClock,
                &mut receiver,
                std::time::Duration::from_secs(timeout),
            )?;
            let payload = serde_json::json!({
                "member_id":member_id,
                "outcome":if messages.is_empty() { "timeout" } else { "messages" },
                "messages":messages.iter().map(crate::output::json::message).collect::<Vec<_>>(),
            });
            emit(json, &payload, || format_message_wait(member_id, messages))
        }
        MessageCommand::Listen {
            member_id,
            timeout,
            codex_thread,
        } => {
            let timeout = timeout.map(std::time::Duration::from_secs);
            match codex_thread {
                Some(thread) => run_forwarder(conn, member_id, timeout, &thread),
                None => run_listener(conn, member_id, timeout),
            }
        }
        MessageCommand::Ack { message_id, json } => {
            let message = broker::ack_message(conn, message_id)?;
            emit(
                json,
                &crate::output::json::message_envelope(&message),
                || format_message_acknowledged(&message),
            )
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

fn send_delivery(
    conn: &Connection,
    message: &crate::broker::MessageRecord,
    outcome: Option<&AttemptOutcome>,
) -> Result<(Option<i64>, DeliveryProjection), CafleetError> {
    let projection = |status, method, reason: Option<&str>| DeliveryProjection {
        status,
        method,
        reason: reason.map(str::to_owned),
    };
    if message.from_member_id == message.to_member_id {
        return Ok((
            None,
            projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
        ));
    }
    let recipient = match broker::active_member_fleet(conn, message.to_member_id)? {
        Some(fleet) => broker::get_member(conn, message.to_member_id, fleet)?,
        None => None,
    };
    let Some(recipient) = recipient else {
        return Ok((
            None,
            projection(DeliveryStatus::Held, None, Some("notification_held")),
        ));
    };
    if recipient.kind == crate::broker::MemberKind::Director {
        return Ok((
            None,
            projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
        ));
    }
    if let Some(runtime) = &recipient.runtime {
        let key = format!("preview:{}:{}", runtime.run.run_id, message.message_id);
        if let Some(input) = crate::broker::runtime::find_input(conn, runtime.run.run_id, &key)?
            && let Some(delivery) = input.delivery()
        {
            return Ok((Some(input.input_id), delivery));
        }
    }
    let reason = match outcome {
        Some(AttemptOutcome {
            held_reason: Some(reason),
            ..
        }) => reason.as_str(),
        Some(AttemptOutcome {
            end: AttemptEnd::Failed(_),
            ..
        })
        | None => "notification_failed",
        Some(_) => "notification_held",
    };
    Ok((None, projection(DeliveryStatus::Held, None, Some(reason))))
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

pub(crate) trait ListenOutput {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError>;
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

#[derive(Default)]
struct EmittedMessages(HashSet<i64>);

impl EmittedMessages {
    fn reconcile(&mut self, messages: &[MessageRecord]) {
        let live: HashSet<i64> = messages.iter().map(|m| m.message_id).collect();
        self.0.retain(|id| live.contains(id));
    }

    fn contains(&self, message_id: i64) -> bool {
        self.0.contains(&message_id)
    }

    fn flushed(&mut self, message_id: i64) {
        self.0.insert(message_id);
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
        &crate::output::json::listen_closed(member_id, reason),
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
        &crate::output::json::listen_ready(member_id, fleet_id),
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
                    let record = crate::output::json::listen_message(&message);
                    drop(message);
                    output.emit(&record, output_deadline)?;
                    if output.session_exited() {
                        return close(clock, output, member_id, "session_exited");
                    }
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

pub(crate) fn run_listener(
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

const FORWARDED_TEXT_LIMIT: usize = 65_536;
const CODEX_QUEUE_BUDGET: Duration = Duration::from_secs(10);
const LISTEN_RECORD_PREFIX: &str = "[cafleet listen record] ";
const FULL_QUEUE_REFUSAL: &str = "queue cannot contain more than";
const FULL_QUEUE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const FULL_QUEUE_LIMIT: Duration = Duration::from_secs(600);

#[cfg(not(test))]
fn full_queue_limit() -> Duration {
    FULL_QUEUE_LIMIT
}

#[cfg(not(test))]
fn codex_queue_budget() -> Duration {
    CODEX_QUEUE_BUDGET
}

#[cfg(not(test))]
fn full_queue_retry_interval() -> Duration {
    FULL_QUEUE_RETRY_INTERVAL
}

/// A test-built listener takes a shortened limit, in whole seconds, from its environment.
#[cfg(test)]
fn full_queue_limit() -> Duration {
    std::env::var("CAFLEET_TEST_CODEX_FULL_QUEUE_LIMIT_SECS").map_or(FULL_QUEUE_LIMIT, |secs| {
        Duration::from_secs(secs.parse().expect("the test limit is whole seconds"))
    })
}

/// A test-built listener takes a shortened queue budget, in milliseconds, from its environment.
#[cfg(test)]
fn codex_queue_budget() -> Duration {
    millis_override("CAFLEET_TEST_CODEX_QUEUE_BUDGET_MS", CODEX_QUEUE_BUDGET)
}

/// A test-built listener takes a shortened retry interval, in milliseconds, from its environment.
#[cfg(test)]
fn full_queue_retry_interval() -> Duration {
    millis_override(
        "CAFLEET_TEST_CODEX_FULL_QUEUE_RETRY_MS",
        FULL_QUEUE_RETRY_INTERVAL,
    )
}

#[cfg(test)]
fn millis_override(name: &str, production: Duration) -> Duration {
    std::env::var(name).map_or(production, |millis| {
        Duration::from_millis(millis.parse().expect("the override is whole milliseconds"))
    })
}

/// Streams `member_id`'s inbox into a Codex thread: every record goes through `codex queue`, and
/// stdout keeps only the `ready` and final `closed` records.
fn run_forwarder(
    conn: &mut Connection,
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
    let descriptor = io::stdout();
    let mut output = CodexForwarding {
        stdout: DescriptorOutput(descriptor.as_fd()),
        queue: CodexQueue {
            codex,
            thread,
            session: &session,
        },
        ready: false,
        session_exited: false,
    };
    let result = with_nonblocking(descriptor.as_fd(), || {
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
        )
    });
    match result {
        Err(error) if output.ready => Err(output.queue.closed_notice(error)),
        result => result,
    }
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
    let mut pid = i64::from(nix::unistd::getppid().as_raw());
    while pid > 1 {
        let Some((parent, argv0)) = process_parent_and_argv0(pid)? else {
            return Ok(None);
        };
        if argv0.rsplit('/').next() == Some("codex") {
            return crate::runtime::ProcessIdentity::read(pid);
        }
        pid = parent;
    }
    Ok(None)
}

#[cfg(target_os = "linux")]
fn process_parent_and_argv0(pid: i64) -> Result<Option<(i64, String)>, CafleetError> {
    let read = |file: &str| match std::fs::read(format!("/proc/{pid}/{file}")) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CafleetError::App(format!(
            "cannot read /proc/{pid}/{file}: {error}"
        ))),
    };
    let (Some(stat), Some(cmdline)) = (read("stat")?, read("cmdline")?) else {
        return Ok(None);
    };
    let stat = String::from_utf8_lossy(&stat);
    let parent = stat
        .rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(1))
        .and_then(|field| field.parse().ok())
        .ok_or_else(|| CafleetError::App(format!("malformed /proc/{pid}/stat")))?;
    let argv0 = cmdline
        .split(|byte| *byte == 0)
        .next()
        .expect("split yields a first segment");
    Ok(Some((parent, String::from_utf8_lossy(argv0).into_owned())))
}

#[cfg(not(target_os = "linux"))]
fn process_parent_and_argv0(pid: i64) -> Result<Option<(i64, String)>, CafleetError> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-o", "args=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| CafleetError::App(format!("cannot run ps: {error}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(line) = text.lines().find(|line| !line.trim().is_empty()) else {
        return Ok(None);
    };
    let mut fields = line.split_whitespace();
    let parent = fields
        .next()
        .and_then(|field| field.parse().ok())
        .ok_or_else(|| CafleetError::App(format!("ps printed no parent for {pid}: {line}")))?;
    let argv0 = fields
        .next()
        .ok_or_else(|| CafleetError::App(format!("ps printed no command for {pid}: {line}")))?;
    Ok(Some((parent, argv0.to_owned())))
}

struct CodexForwarding<'a> {
    stdout: DescriptorOutput<'a>,
    queue: CodexQueue<'a>,
    ready: bool,
    session_exited: bool,
}

impl ListenOutput for CodexForwarding<'_> {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<(), CafleetError> {
        match record["event"].as_str() {
            Some("ready") => {
                self.stdout.emit(record, deadline)?;
                self.ready = true;
                Ok(())
            }
            Some("message") => {
                self.session_exited =
                    self.queue.forward_record(record)? == Forwarding::SessionExited;
                Ok(())
            }
            Some("closed") => {
                if record["reason"] != "session_exited"
                    && self.queue.forward_record(record)? == Forwarding::SessionExited
                {
                    let member_id = record["member_id"]
                        .as_i64()
                        .expect("a closed record names its member");
                    let record = crate::output::json::listen_closed(member_id, "session_exited");
                    self.stdout.emit(&record, deadline).ok();
                    return Ok(());
                }
                // The final record may target the exited session's PTY; the exit status
                // reflects reception, never this write.
                self.stdout.emit(record, deadline).ok();
                Ok(())
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
    FullQueue(String),
    Deadline,
}

struct CodexQueue<'a> {
    codex: std::path::PathBuf,
    thread: &'a str,
    session: &'a crate::runtime::ProcessIdentity,
}

impl CodexQueue<'_> {
    /// Queues `record`, retrying it while the thread's queue is full so later records keep
    /// their order behind it.
    fn forward_record(&self, record: &Value) -> Result<Forwarding, CafleetError> {
        let text = Self::forwarded_text(record);
        let mut full_since = None;
        loop {
            match self.queue(&text) {
                Ok(()) => return Ok(Forwarding::Accepted),
                Err(QueueFailure::FullQueue(_)) => {
                    let limit = full_queue_limit();
                    if full_since.get_or_insert_with(Instant::now).elapsed() >= limit {
                        return Err(CafleetError::App(format!(
                            "codex thread {} queue stayed full for {} s",
                            self.thread,
                            limit.as_secs()
                        )));
                    }
                    std::thread::sleep(full_queue_retry_interval());
                    if pinned_session_exited(self.session)? {
                        return Ok(Forwarding::SessionExited);
                    }
                }
                Err(QueueFailure::Deadline | QueueFailure::Exit(_))
                    if pinned_session_exited(self.session)? =>
                {
                    return Ok(Forwarding::SessionExited);
                }
                Err(QueueFailure::Deadline) => {
                    return Err(CafleetError::App(self.deadline_diagnostic()));
                }
                Err(QueueFailure::Exit(cause)) => {
                    return Err(CafleetError::App(format!(
                        "failed to forward record to codex thread {}: {cause}",
                        self.thread
                    )));
                }
            }
        }
    }

    fn forwarded_text(record: &Value) -> String {
        let mut text = format!("{LISTEN_RECORD_PREFIX}{record}");
        if text.len() > FORWARDED_TEXT_LIMIT {
            let message = &record["message"];
            text = format!(
                "{LISTEN_RECORD_PREFIX}{} — read it with: cafleet message show {} --json",
                serde_json::json!({
                    "event": "message_ref",
                    "message_id": message["message_id"],
                    "from_member_id": message["from_member_id"],
                    "bytes": text.len(),
                }),
                message["message_id"],
            );
        }
        text
    }

    /// Makes one bounded attempt to tell the session that reception ended, keeping `error`.
    fn closed_notice(&self, error: CafleetError) -> CafleetError {
        let notice = format!("[cafleet listen closed] exit 1: {}", error.message());
        match self.queue(&notice) {
            Ok(()) => error,
            Err(failure) => {
                let cause = match failure {
                    QueueFailure::Exit(cause) | QueueFailure::FullQueue(cause) => cause,
                    QueueFailure::Deadline => self.deadline_diagnostic(),
                };
                CafleetError::App(format!(
                    "{}\nfailed to forward the closed notice to codex thread {}: {cause}",
                    error.message(),
                    self.thread
                ))
            }
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
        Err(match stderr {
            Some(text) if text.lines().any(|line| line.contains(FULL_QUEUE_REFUSAL)) => {
                QueueFailure::FullQueue(text)
            }
            stderr => {
                QueueFailure::Exit(stderr.unwrap_or_else(|| format!("codex exited with {status}")))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::runtime::DeliveryMethod;
    use crate::cli::message::*;
    use crate::db::test_support::migrated_conn;

    fn projection(
        status: DeliveryStatus,
        method: Option<DeliveryMethod>,
        reason: Option<&str>,
    ) -> DeliveryProjection {
        DeliveryProjection {
            status,
            method,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn send_delivery_reports_self_sends_the_root_director_and_run_less_recipients() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "branches");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        for (from, to, expected) in [
            (
                director,
                director,
                projection(DeliveryStatus::NotApplicable, None, Some("self_send")),
            ),
            (
                worker,
                director,
                projection(DeliveryStatus::NotApplicable, None, Some("director_stdout")),
            ),
        ] {
            let message = broker::send_message(&mut conn, from, to, "branch").unwrap();
            assert_eq!(
                send_delivery(&conn, &message, None).unwrap(),
                (None, expected),
                "{from} -> {to}"
            );
        }
        let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()));
        let (output, pane_calls) = crate::cli::test_support::isolated_cli(
            &settings,
            &[
                "message",
                "send",
                "--from-member-id",
                &director.to_string(),
                "--to-member-id",
                &worker.to_string(),
                "committed run-less assignment",
                "--json",
            ],
        );
        assert_eq!(output.status.code(), Some(0));
        let stdout = String::from_utf8(output.stdout).unwrap();
        let payload: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("successful send JSON"),
        )
        .unwrap();
        let message_id = payload["message"]["message_id"].as_i64().unwrap();
        let message = broker::get_message(&conn, message_id).unwrap();
        assert_eq!(message.from_member_id, director);
        assert_eq!(message.to_member_id, worker);
        assert_eq!(message.text, "committed run-less assignment");
        assert_eq!(payload["message"], crate::output::json::message(&message));
        assert_eq!(
            payload["delivery"],
            serde_json::json!({
                "status":"held", "method":null, "reason":"notification_failed",
            })
        );
        assert_eq!(payload["notification_sent"], false);
        assert_eq!(payload["input_id"], serde_json::Value::Null);
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "Message {message_id} was persisted. message {message_id} was not notified to member {worker}: member {worker} has no recorded run; the next send to member {worker} or the next message wait retries it.\n"
            ),
        );
        assert!(pane_calls.is_empty(), "{pane_calls}");
        assert_eq!(broker::poll_messages(&conn, worker).unwrap(), [message]);
        assert_eq!(
            conn.query_row(
                "SELECT notified_at FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn message_wait_repeats_missing_run_causes_for_each_pending_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "run-less wait");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let messages = ["first", "second"]
            .map(|body| broker::send_message(&mut conn, director, worker, body).unwrap());
        let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()));
        let expected = messages.iter().map(|message| format!(
            "message {} was not notified to member {worker}: member {worker} has no recorded run; the next send to member {worker} or the next message wait retries it.\n",
            message.message_id,
        )).collect::<String>();
        let (output, pane_calls) = crate::cli::test_support::isolated_cli(
            &settings,
            &[
                "message",
                "wait",
                &director.to_string(),
                "--timeout",
                "2",
                "--json",
            ],
        );
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
        assert!(pane_calls.is_empty(), "{pane_calls}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let payload: serde_json::Value = serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("wait JSON"),
        )
        .unwrap();
        assert_eq!(payload["member_id"], director);
        assert_eq!(payload["outcome"], "timeout");
        assert_eq!(payload["messages"], serde_json::json!([]));
        assert_eq!(
            broker::poll_messages(&conn, worker).unwrap(),
            messages.iter().rev().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE to_member_id=?1 AND notified_at IS NOT NULL",
                [worker],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn send_delivery_reports_a_wrapped_self_send_and_the_queued_preview_receipt() {
        let mut fixture = crate::runtime::test_support::Fixture::new();
        let run = fixture.run.clone();
        let message = broker::send_message(
            &mut fixture.conn,
            run.director_member_id,
            run.member_id,
            "queued",
        )
        .unwrap();
        fixture
            .conn
            .execute(
                "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,message_id,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'preview',?3,'ready',?4,?5,?5,?5,'interaction_open')",
                rusqlite::params![
                    run.run_id,
                    format!("preview:{}:{}", run.run_id, message.message_id),
                    "d".repeat(64),
                    message.message_id,
                    "2026-10-30T10:00:00Z",
                ],
            )
            .unwrap();
        let receipt = fixture.conn.last_insert_rowid();
        assert_eq!(
            send_delivery(&fixture.conn, &message, None).unwrap(),
            (
                Some(receipt),
                projection(DeliveryStatus::Queued, None, Some("interaction_open"))
            )
        );
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let (fleet, director) = create_fleet_fixture(&mut conn, "deregistered");
        let worker = register(&mut conn, fleet, "worker", Some("%2"));
        let committed = broker::send_message(&mut conn, director, worker, "committed").unwrap();
        assert!(
            crate::broker::members::test_support::deregister_member(&mut conn, worker).unwrap()
        );
        assert_eq!(
            send_delivery(&conn, &committed, None)
                .expect("a committed send reports its record after the recipient is gone"),
            (
                None,
                projection(DeliveryStatus::Held, None, Some("notification_held"))
            )
        );
    }

    use crate::cli::message::run_on_descriptor;
    use nix::fcntl::FcntlArg;
    use nix::fcntl::OFlag;
    use nix::fcntl::fcntl;
    use std::cell::Cell;
    use std::io;
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::os::fd::OwnedFd;
    use std::time::Duration;
    use std::time::Instant;

    use serde_json::Value;
    use serde_json::json;

    use crate::broker;
    use crate::broker::MessageRecord;
    use crate::broker::messaging::InboxClosure;
    use crate::broker::messaging::InboxSnapshot;
    use crate::broker::messaging::test_support::broker_fixture;
    use crate::cli::message::ListenIo;
    use crate::cli::message::ListenOutput;
    use crate::cli::message::listen;
    use crate::cli::message::write_json_line;
    use crate::error::CafleetError;
    use crate::time::Clock;
    use crate::time::Deadline;
    use crate::time::test_support::TestClock;

    fn test_clock() -> TestClock {
        TestClock::starting_at(Instant::now() + Duration::from_secs(10_000))
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
        let clock = test_clock();
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
        let clock = test_clock();
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
        let clock = test_clock();
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
            let clock = test_clock();
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
            let clock = test_clock();
            let mut io = ScriptedInbox {
                clock: &clock,
                snapshots: Vec::new(),
                script: |_, check| match (check == fail_at, fail_at) {
                    (true, 1) => Err(CafleetError::App("snapshot failed".into())),
                    (true, _) => Err(CafleetError::DeadlineExceeded),
                    (false, _) => Ok(open(vec![message(10)])),
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
            if fail_at == 1 {
                assert!(error.message().contains("snapshot failed"));
            } else {
                assert!(
                    matches!(error, CafleetError::DeadlineExceeded),
                    "a midstream snapshot budget failure ends the stream instead of a timeout"
                );
            }
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
        let clock = test_clock();
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
    fn output_failure_stops_before_another_snapshot_and_retains_only_flushed_records() {
        for fail_at in [0, 1] {
            let clock = test_clock();
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
    fn an_ack_output_race_preserves_the_snapshot_record() {
        let clock = test_clock();
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
    }

    #[test]
    fn a_failed_final_closed_record_is_an_output_error() {
        let clock = test_clock();
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
        let (_dir, mut conn, _, director, worker) = broker_fixture();
        let pending =
            broker::send_message(&mut conn, worker, director, &"雪".repeat(1024 * 1024)).unwrap();
        for (broken, nonblocking) in [(false, true), (true, false)] {
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
            assert_eq!(
                broker::poll_messages(&conn, director).unwrap(),
                std::slice::from_ref(&pending)
            );
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
        let (read_end, write_end) = std::io::pipe().unwrap();
        let (read_end, write_end) = (OwnedFd::from(read_end), OwnedFd::from(write_end));
        let initial = fcntl(&write_end, FcntlArg::F_GETFL).unwrap();
        let mut flags = OFlag::from_bits_retain(initial) | OFlag::O_APPEND;
        flags.set(OFlag::O_NONBLOCK, nonblocking);
        fcntl(&write_end, FcntlArg::F_SETFL(flags)).unwrap();
        // Darwin adds a kernel-managed flag on first write; establish it before measuring restoration.
        assert_eq!(nix::unistd::write(&write_end, b"x").unwrap(), 1);
        let mut byte = [0];
        assert_eq!(nix::unistd::read(&read_end, &mut byte).unwrap(), 1);
        assert_eq!(byte, *b"x");
        let retained = write_end.try_clone().unwrap();
        let original = fcntl(&retained, FcntlArg::F_GETFL).unwrap();
        let flags = OFlag::from_bits_retain(original);
        assert!(flags.contains(OFlag::O_APPEND));
        assert_eq!(flags.contains(OFlag::O_NONBLOCK), nonblocking);
        (read_end, write_end, retained, original)
    }

    mod codex_forwarding {
        use crate::cli::message::tests::*;
        use crate::runtime::system::test_support::TestChild;
        use std::os::unix::process::CommandExt;
        use std::path::PathBuf;
        use std::process::Command;
        use std::process::Stdio;

        const SESSION_CONFIG: &str = "CAFLEET_TEST_CODEX_SESSION";
        const THREAD: &str = "019a5c3e-7f21-7c11-9d3b-2b6f0c8e4a10";
        const RECORD_PREFIX: &str = "[cafleet listen record] ";
        const RECORD_LIMIT: usize = 65_536;
        /// Whole seconds that replace the 600 s full-queue limit in a test-built listener.
        const FULL_QUEUE_LIMIT_SECS: &str = "CAFLEET_TEST_CODEX_FULL_QUEUE_LIMIT_SECS";
        const QUEUE_BUDGET_MS: &str = "CAFLEET_TEST_CODEX_QUEUE_BUDGET_MS";
        const RETRY_MS: &str = "CAFLEET_TEST_CODEX_FULL_QUEUE_RETRY_MS";
        const QUEUE_BUDGET: Duration = Duration::from_millis(1000);
        const RETRY: Duration = Duration::from_millis(100);
        const FULL_QUEUE_REFUSAL: &str = "printf '%s\\n' '⚠ install out of sync' 'Error: queue cannot contain more than 100 submissions (code -32600)' >&2\nexit 1";

        /// Appends the stub's argv to the log: fields end with US, invocations with RS.
        const LOG_ARGV: &str = r#"{ for arg in "$@"; do printf '%s\037' "$arg"; done; printf '\036'; } >>"$CAFLEET_TEST_CODEX_LOG""#;

        /// Runs the listener as a child of this shell and records its pid and exit status.
        const LISTENER_WRAPPER: &str = r#""$@" >"$CAFLEET_TEST_LISTENER_STDOUT" 2>"$CAFLEET_TEST_LISTENER_STDERR" </dev/null &
listener=$!
printf '%s' "$listener" >"$CAFLEET_TEST_LISTENER_PID"
wait "$listener"
printf '%s' "$?" >"$CAFLEET_TEST_LISTENER_STATUS""#;

        fn harness_args(test: &'static str) -> [&'static str; 6] {
            [
                "--exact",
                test,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
                "--quiet",
            ]
        }

        fn listener_command(wrapper: &str) -> Command {
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", wrapper, "sh"])
                .arg(std::env::current_exe().unwrap())
                .args(harness_args(crate::cli::tests::isolated_cli_child_path()));
            command
        }

        /// Plays the Codex session process: its argv[0] is `codex`, and with `sandbox`
        /// it reaches the listener through a `codex-linux-sandbox` intermediate.
        #[test]
        #[ignore = "owned codex session subprocess entry"]
        fn codex_session_child() {
            let config: Value = serde_json::from_str(
                &std::env::var(SESSION_CONFIG).expect("explicit codex session configuration"),
            )
            .unwrap();
            let mut command = if config["role"] == "session" && config["sandbox"] == true {
                let mut sandbox = Command::new(std::env::current_exe().unwrap());
                sandbox
                    .arg0("codex-linux-sandbox")
                    .args(harness_args(test_path!(codex_session_child)))
                    .env(SESSION_CONFIG, json!({"role":"sandbox"}).to_string());
                sandbox
            } else {
                listener_command(LISTENER_WRAPPER)
            };
            let status = command.stdin(Stdio::null()).status().unwrap();
            std::process::exit(status.code().unwrap_or(1));
        }

        enum Launch {
            Session,
            SandboxedSession,
            /// The listener's parent shell is orphaned, so no ancestor is a Codex session.
            Orphan,
        }

        struct Forwarder {
            dir: tempfile::TempDir,
            top: TestChild,
        }

        impl Forwarder {
            fn start(
                database_url: &str,
                args: &[&str],
                codex: Option<&str>,
                launch: Launch,
                env: &[(&str, &str)],
            ) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let bin = dir.path().join("bin");
                std::fs::create_dir(&bin).unwrap();
                let path = match codex {
                    Some(body) => {
                        crate::coding_agent::test_support::executable_stub(&bin, "codex", body);
                        format!("{}:/usr/bin:/bin", bin.display())
                    }
                    None => bin.display().to_string(),
                };
                let mut command = match launch {
                    Launch::Session | Launch::SandboxedSession => {
                        let mut session = Command::new(std::env::current_exe().unwrap());
                        session
                            .arg0("codex")
                            .args(harness_args(test_path!(codex_session_child)))
                            .env(
                                SESSION_CONFIG,
                                json!({
                                    "role": "session",
                                    "sandbox": matches!(launch, Launch::SandboxedSession),
                                })
                                .to_string(),
                            );
                        session
                    }
                    Launch::Orphan => listener_command(&format!("( {LISTENER_WRAPPER} ) &")),
                };
                let file = |name: &str| dir.path().join(name);
                command
                    .env("PATH", path)
                    .env("CAFLEET_TEST_CODEX_LOG", file("codex.log"))
                    .env("CAFLEET_TEST_LISTENER_STDOUT", file("stdout"))
                    .env("CAFLEET_TEST_LISTENER_STDERR", file("stderr"))
                    .env("CAFLEET_TEST_LISTENER_PID", file("pid"))
                    .env("CAFLEET_TEST_LISTENER_STATUS", file("status"))
                    .env(
                        "CAFLEET_TEST_ISOLATED_CLI",
                        json!({"database_url": database_url, "args": args}).to_string(),
                    )
                    .env("RUST_BACKTRACE", "0")
                    .env(QUEUE_BUDGET_MS, QUEUE_BUDGET.as_millis().to_string())
                    .env(RETRY_MS, RETRY.as_millis().to_string())
                    .envs(env.iter().copied())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let top = TestChild::new(command.spawn().unwrap());
                Self { dir, top }
            }

            fn file(&self, name: &str) -> PathBuf {
                self.dir.path().join(name)
            }

            fn read(&self, name: &str) -> String {
                std::fs::read_to_string(self.file(name)).unwrap_or_default()
            }

            /// Complete JSON lines only; the test harness adds its own non-JSON lines.
            fn stdout_records(&self) -> Vec<Value> {
                self.read("stdout")
                    .split_inclusive('\n')
                    .filter(|line| line.starts_with('{') && line.ends_with('\n'))
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            }

            fn stderr(&self) -> String {
                self.read("stderr")
            }

            fn status(&self) -> Option<i32> {
                self.read("status").parse().ok()
            }

            fn forwarded(&self) -> Vec<Vec<String>> {
                self.read("codex.log")
                    .split('\u{1e}')
                    .filter(|invocation| !invocation.is_empty())
                    .map(|invocation| {
                        invocation
                            .strip_suffix('\u{1f}')
                            .unwrap_or_else(|| panic!("complete invocation: {invocation:?}"))
                            .split('\u{1f}')
                            .map(str::to_owned)
                            .collect()
                    })
                    .collect()
            }

            /// Gaps between consecutive `codex` invocations, as stamped by [`full_queue_codex`].
            fn invocation_gaps(&self) -> Vec<Duration> {
                let stamps = self
                    .read("codex.log.times")
                    .lines()
                    .map(|line| line.parse::<u64>().unwrap())
                    .collect::<Vec<_>>();
                stamps
                    .windows(2)
                    .map(|pair| Duration::from_nanos(pair[1] - pair[0]))
                    .collect()
            }

            fn wait_for(&self, what: &str, within: Duration, mut done: impl FnMut(&Self) -> bool) {
                let deadline = Instant::now() + within;
                while !done(self) {
                    assert!(
                        Instant::now() < deadline,
                        "{what} within {within:?}: stdout {:?} stderr {:?} forwarded {:?}",
                        self.read("stdout"),
                        self.stderr(),
                        self.forwarded()
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }

            fn wait_ready(&self) {
                self.wait_for("ready", Duration::from_secs(10), |forwarder| {
                    forwarder
                        .stdout_records()
                        .first()
                        .is_some_and(|record| record["event"] == "ready")
                });
            }

            fn wait_forwarded(&self, count: usize) {
                self.wait_for(
                    &format!("{count} forwarded records"),
                    Duration::from_secs(10),
                    |forwarder| forwarder.forwarded().len() >= count,
                );
            }

            fn wait_exit(&self, within: Duration) -> i32 {
                self.wait_for("listener exit", within, |forwarder| {
                    forwarder.status().is_some()
                });
                self.status().unwrap()
            }

            fn end_session(&mut self) {
                self.top.kill().unwrap();
                self.top.wait().unwrap();
            }
        }

        impl Drop for Forwarder {
            fn drop(&mut self) {
                if self.status().is_none()
                    && let Ok(pid) = self.read("pid").parse::<i32>()
                {
                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    )
                    .ok();
                }
            }
        }

        struct Broker {
            _dir: tempfile::TempDir,
            conn: Connection,
            url: String,
            fleet: i64,
            director: i64,
            worker: i64,
        }

        fn broker() -> Broker {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "codex forwarding");
            let worker = register(&mut conn, fleet, "worker", Some("%2"));
            let url =
                Settings::for_database(std::path::Path::new(conn.path().unwrap())).database_url;
            Broker {
                _dir: dir,
                conn,
                url,
                fleet,
                director,
                worker,
            }
        }

        impl Broker {
            fn send(&mut self, from: i64, to: i64, body: &str) -> i64 {
                broker::send_message(&mut self.conn, from, to, body)
                    .unwrap()
                    .message_id
            }

            fn record_text(&self, message_id: i64) -> String {
                let message = broker::get_message(&self.conn, message_id).unwrap();
                format!(
                    "{RECORD_PREFIX}{}",
                    crate::output::json::listen_message(&message)
                )
            }
        }

        fn listen_args(member_id: i64, timeout: Option<u64>) -> Vec<String> {
            let mut args = vec![
                "message".to_owned(),
                "listen".to_owned(),
                member_id.to_string(),
            ];
            if let Some(timeout) = timeout {
                args.extend(["--timeout".to_owned(), timeout.to_string()]);
            }
            args.extend(["--codex-thread".to_owned(), THREAD.to_owned()]);
            args
        }

        fn start(
            broker: &Broker,
            args: &[String],
            codex: Option<&str>,
            launch: Launch,
        ) -> Forwarder {
            start_with_env(broker, args, codex, launch, &[])
        }

        fn start_with_env(
            broker: &Broker,
            args: &[String],
            codex: Option<&str>,
            launch: Launch,
            env: &[(&str, &str)],
        ) -> Forwarder {
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            Forwarder::start(&broker.url, &args, codex, launch, env)
        }

        fn queued(text: &str) -> Vec<String> {
            ["queue", "--thread", THREAD, "--message", text]
                .map(str::to_owned)
                .to_vec()
        }

        fn closed_text(member_id: i64, reason: &str) -> String {
            format!(
                "{RECORD_PREFIX}{}",
                crate::output::json::listen_closed(member_id, reason)
            )
        }

        fn accepting_codex() -> String {
            format!("{LOG_ARGV}\nexit 0")
        }

        /// Accepts only the closed notice and answers every record with `record_reply`.
        fn codex_refusing_records(record_reply: &str) -> String {
            format!(
                "{LOG_ARGV}\ncase \"$*\" in\n  *'[cafleet listen closed]'*) exit 0 ;;\nesac\n{record_reply}"
            )
        }

        /// Accepts only the closed notice and holds every record until the test creates
        /// [`RELEASE`], then answers it with `record_reply`.
        fn codex_failing_released_records(record_reply: &str) -> String {
            codex_refusing_records(&format!(
                "while [ ! -e \"$CAFLEET_TEST_CODEX_LOG.release\" ]; do sleep 0.05; done\n{record_reply}"
            ))
        }

        const RELEASE: &str = "codex.log.release";

        fn assert_closed_with_session_exited(forwarder: &Forwarder, broker: &Broker, member: i64) {
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(member, broker.fleet),
                    crate::output::json::listen_closed(member, "session_exited"),
                ]
            );
            assert!(
                !forwarder.stderr().contains("failed to forward"),
                "{}",
                forwarder.stderr()
            );
        }

        /// Stamps every invocation, answers the first `refusals` (every one when `None`) with
        /// Codex's full-queue refusal, and runs `then` for the rest.
        fn full_queue_codex(refusals: Option<u32>, then: &str) -> String {
            let refuse = match refusals {
                Some(refusals) => {
                    format!("if [ \"$count\" -le {refusals} ]; then\n{FULL_QUEUE_REFUSAL}\nfi")
                }
                None => FULL_QUEUE_REFUSAL.to_owned(),
            };
            format!(
                "{LOG_ARGV}\ndate +%s%N >>\"$CAFLEET_TEST_CODEX_LOG.times\"\ncount=$(( $(cat \"$CAFLEET_TEST_CODEX_LOG.count\" 2>/dev/null || echo 0) + 1 ))\nprintf '%s' \"$count\" >\"$CAFLEET_TEST_CODEX_LOG.count\"\n{refuse}\n{then}"
            )
        }

        fn assert_stdout_has_no_forwarded_records(forwarder: &Forwarder, broker: &Broker) {
            let records = forwarder.stdout_records();
            assert_eq!(
                records.first(),
                Some(&crate::output::json::listen_ready(
                    broker.director,
                    broker.fleet
                )),
                "{records:?}"
            );
            assert!(
                records[1..]
                    .iter()
                    .all(|record| record["event"] == "closed"),
                "stdout carries no record between ready and closed: {records:?}"
            );
        }

        #[test]
        fn forwarding_queues_each_message_then_the_closing_record_and_stdout_keeps_only_ready_and_closed()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let backlog =
                ["first", "second", "third"].map(|body| broker.send(worker, director, body));
            let forwarder = start(
                &broker,
                &listen_args(director, Some(4)),
                Some(&accepting_codex()),
                Launch::Session,
            );
            forwarder.wait_forwarded(3);
            let later = broker.send(worker, director, "fourth");
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(15)),
                0,
                "{}",
                forwarder.stderr()
            );
            let mut expected = backlog
                .iter()
                .chain([&later])
                .map(|id| queued(&broker.record_text(*id)))
                .collect::<Vec<_>>();
            expected.push(queued(&closed_text(director, "timeout")));
            assert_eq!(forwarder.forwarded(), expected, "ready is never forwarded");
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(director, broker.fleet),
                    crate::output::json::listen_closed(director, "timeout"),
                ]
            );
            assert_eq!(
                broker::poll_messages(&broker.conn, director).unwrap().len(),
                4,
                "forwarding consumes nothing"
            );
        }

        #[test]
        fn forwarding_queues_a_member_inactive_closure_and_writes_it_last_on_stdout() {
            let mut broker = broker();
            let worker = broker.worker;
            let forwarder = start(
                &broker,
                &listen_args(worker, None),
                Some(&accepting_codex()),
                Launch::Session,
            );
            forwarder.wait_ready();
            assert!(
                crate::broker::members::test_support::deregister_member(&mut broker.conn, worker)
                    .unwrap()
            );
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(10)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert_eq!(
                forwarder.forwarded(),
                [queued(&closed_text(worker, "member_inactive"))]
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(worker, broker.fleet),
                    crate::output::json::listen_closed(worker, "member_inactive"),
                ]
            );
        }

        #[test]
        fn a_record_text_above_the_argument_limit_is_forwarded_as_a_message_reference() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let probe = broker.send(worker, director, &"x".repeat(1000));
            let overhead = broker.record_text(probe).len() - 1000;
            broker::ack_message(&mut broker.conn, probe).unwrap();
            let fits = broker.send(worker, director, &"x".repeat(RECORD_LIMIT - overhead));
            let exceeds = broker.send(worker, director, &"x".repeat(RECORD_LIMIT + 1 - overhead));
            assert_eq!(broker.record_text(fits).len(), RECORD_LIMIT);
            assert_eq!(broker.record_text(exceeds).len(), RECORD_LIMIT + 1);
            let forwarder = start(
                &broker,
                &listen_args(director, Some(3)),
                Some(&accepting_codex()),
                Launch::Session,
            );
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(15)),
                0,
                "{}",
                forwarder.stderr()
            );
            let forwarded = forwarder.forwarded();
            assert_eq!(forwarded.len(), 3, "{forwarded:?}");
            assert_eq!(forwarded[0], queued(&broker.record_text(fits)));
            assert_eq!(forwarded[2], queued(&closed_text(director, "timeout")));
            assert_eq!(forwarded[1][..4], queued("")[..4]);
            let reference = forwarded[1][4]
                .strip_prefix(RECORD_PREFIX)
                .and_then(|rest| {
                    rest.strip_suffix(&format!(
                        " — read it with: cafleet message show {exceeds} --json"
                    ))
                })
                .unwrap_or_else(|| panic!("{}", forwarded[1][4]));
            let reference: Value = serde_json::from_str(reference).unwrap();
            crate::broker::runtime::test_support::assert_key_order(
                &reference,
                &["event", "message_id", "from_member_id", "bytes"],
            );
            assert_eq!(reference["event"], "message_ref");
            assert_eq!(reference["message_id"], exceeds);
            assert_eq!(reference["from_member_id"], worker);
            assert_eq!(
                reference["bytes"].as_u64(),
                Some(broker.record_text(exceeds).len() as u64),
                "bytes is the length of the text the reference replaces"
            );
        }

        #[test]
        fn a_codex_queue_past_its_deadline_is_killed_and_the_closed_notice_names_the_deadline() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            broker.send(worker, director, "slow");
            let started = Instant::now();
            let forwarder = start(
                &broker,
                &listen_args(director, None),
                Some(&codex_refusing_records("exec sleep 30")),
                Launch::Session,
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(10)), 1);
            let elapsed = started.elapsed();
            assert!(
                elapsed >= QUEUE_BUDGET && elapsed < Duration::from_secs(10),
                "{elapsed:?}"
            );
            let diagnostic = format!(
                "codex queue for thread {THREAD} did not finish within {} s",
                QUEUE_BUDGET.as_secs()
            );
            assert!(
                forwarder.stderr().contains(&diagnostic),
                "{}",
                forwarder.stderr()
            );
            let forwarded = forwarder.forwarded();
            assert_eq!(forwarded.len(), 2, "{forwarded:?}");
            assert_eq!(
                forwarded[1],
                queued(&format!("[cafleet listen closed] exit 1: {diagnostic}"))
            );
            assert_stdout_has_no_forwarded_records(&forwarder, &broker);
        }

        #[test]
        fn a_failed_closed_notice_keeps_the_original_diagnostic_and_exit_one() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            broker.send(worker, director, "unforwardable");
            let forwarder = start(
                &broker,
                &listen_args(director, None),
                Some(&format!(
                    "{LOG_ARGV}\nprintf '%s\\n' 'codex unavailable' >&2\nexit 4"
                )),
                Launch::Session,
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(25)), 1);
            let stderr = forwarder.stderr();
            assert!(
                stderr.contains(&format!(
                    "failed to forward record to codex thread {THREAD}: "
                )),
                "{stderr}"
            );
            assert!(
                stderr.contains(&format!(
                    "failed to forward the closed notice to codex thread {THREAD}: "
                )),
                "{stderr}"
            );
            let forwarded = forwarder.forwarded();
            assert_eq!(
                forwarded.len(),
                2,
                "one record attempt and one notice: {forwarded:?}"
            );
            assert!(forwarded[1][4].starts_with("[cafleet listen closed] exit 1: "));
        }

        #[test]
        fn full_queue_refusals_retry_the_same_record_each_second_until_accepted_and_later_records_keep_their_order()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let records =
                ["first", "second", "third"].map(|body| broker.send(worker, director, body));
            let forwarder = start(
                &broker,
                &listen_args(director, Some(1)),
                Some(&full_queue_codex(Some(2), "exit 0")),
                Launch::Session,
            );
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(10)),
                0,
                "{}",
                forwarder.stderr()
            );
            let first = queued(&broker.record_text(records[0]));
            let mut expected = vec![first; 3];
            expected.extend(
                records[1..]
                    .iter()
                    .map(|id| queued(&broker.record_text(*id))),
            );
            expected.push(queued(&closed_text(director, "timeout")));
            assert_eq!(
                forwarder.forwarded(),
                expected,
                "two refusals, one acceptance, then the later records in order"
            );
            let gaps = forwarder.invocation_gaps();
            assert!(
                gaps[..2].iter().all(|gap| *gap >= RETRY),
                "each retry waits the retry interval: {gaps:?}"
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(director, broker.fleet),
                    crate::output::json::listen_closed(director, "timeout"),
                ]
            );
        }

        #[test]
        fn the_pinned_session_ending_during_a_full_queue_wait_closes_the_listener_with_session_exited()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let message = broker.send(worker, director, "waiting");
            let mut forwarder = start(
                &broker,
                &listen_args(director, None),
                Some(&full_queue_codex(None, "exit 0")),
                Launch::Session,
            );
            forwarder.wait_forwarded(2);
            forwarder.end_session();
            let ended = Instant::now();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert!(
                ended.elapsed() < Duration::from_secs(3),
                "{:?}",
                ended.elapsed()
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(director, broker.fleet),
                    crate::output::json::listen_closed(director, "session_exited"),
                ]
            );
            let record = queued(&broker.record_text(message));
            let forwarded = forwarder.forwarded();
            assert!(
                forwarded.iter().all(|invocation| *invocation == record),
                "only the waiting record is attempted, with no closed notice: {forwarded:?}"
            );
        }

        #[test]
        fn a_codex_queue_failing_after_the_pinned_session_ended_closes_the_listener_with_session_exited()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let message = broker.send(worker, director, "in flight");
            let mut forwarder = start(
                &broker,
                &listen_args(director, None),
                Some(&codex_failing_released_records(&format!(
                    "printf '%s\\n' 'thread {THREAD} not found' >&2\nexit 2"
                ))),
                Launch::Session,
            );
            forwarder.wait_forwarded(1);
            forwarder.end_session();
            std::fs::write(forwarder.file(RELEASE), "").unwrap();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert_closed_with_session_exited(&forwarder, &broker, director);
            assert_eq!(
                forwarder.forwarded(),
                [queued(&broker.record_text(message))],
                "no closed notice goes to the exited session"
            );
        }

        #[test]
        fn a_closing_record_failing_after_the_pinned_session_ended_closes_the_listener_with_session_exited()
         {
            let mut broker = broker();
            let worker = broker.worker;
            let mut forwarder = start(
                &broker,
                &listen_args(worker, None),
                Some(&codex_failing_released_records(&format!(
                    "printf '%s\\n' 'thread {THREAD} not found' >&2\nexit 2"
                ))),
                Launch::Session,
            );
            forwarder.wait_ready();
            assert!(
                crate::broker::members::test_support::deregister_member(&mut broker.conn, worker)
                    .unwrap()
            );
            forwarder.wait_forwarded(1);
            forwarder.end_session();
            std::fs::write(forwarder.file(RELEASE), "").unwrap();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert_closed_with_session_exited(&forwarder, &broker, worker);
            assert_eq!(
                forwarder.forwarded(),
                [queued(&closed_text(worker, "member_inactive"))],
                "no closed notice goes to the exited session"
            );
        }

        #[test]
        fn full_queue_refusals_past_the_limit_exit_one_after_a_single_closed_notice_attempt() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let message = broker.send(worker, director, "never accepted");
            let started = Instant::now();
            let forwarder = start_with_env(
                &broker,
                &listen_args(director, None),
                Some(&full_queue_codex(None, "exit 0")),
                Launch::Session,
                &[(FULL_QUEUE_LIMIT_SECS, "2")],
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(15)), 1);
            let elapsed = started.elapsed();
            assert!(elapsed >= Duration::from_secs(2), "{elapsed:?}");
            let diagnostic = format!("codex thread {THREAD} queue stayed full for 2 s");
            let stderr = forwarder.stderr();
            assert!(stderr.contains(&diagnostic), "{stderr}");
            assert!(
                stderr.contains(&format!(
                    "failed to forward the closed notice to codex thread {THREAD}: "
                )) && stderr.contains("queue cannot contain more than 100 submissions"),
                "the notice met the full queue too: {stderr}"
            );
            let forwarded = forwarder.forwarded();
            let (notice, attempts) = forwarded.split_last().unwrap();
            assert_eq!(
                notice,
                &queued(&format!("[cafleet listen closed] exit 1: {diagnostic}")),
                "the closed notice is attempted once: {forwarded:?}"
            );
            let record = queued(&broker.record_text(message));
            assert!(
                attempts.len() >= 2 && attempts.iter().all(|attempt| *attempt == record),
                "{forwarded:?}"
            );
            assert_stdout_has_no_forwarded_records(&forwarder, &broker);
        }

        #[test]
        fn a_non_full_queue_failure_after_a_refusal_fails_without_another_wait() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let message = broker.send(worker, director, "refused");
            let wait = Duration::from_millis(500);
            let forwarder = start_with_env(
                &broker,
                &listen_args(director, None),
                Some(&full_queue_codex(
                    Some(1),
                    &format!(
                        "case \"$*\" in\n  *'[cafleet listen closed]'*) exit 0 ;;\nesac\nprintf '%s\\n' 'thread {THREAD} not found' >&2\nexit 2"
                    ),
                )),
                Launch::Session,
                &[(RETRY_MS, &wait.as_millis().to_string())],
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(15)), 1);
            let stderr = forwarder.stderr();
            let diagnostic = stderr
                .lines()
                .find(|line| {
                    line.contains(&format!(
                        "failed to forward record to codex thread {THREAD}: "
                    ))
                })
                .unwrap_or_else(|| panic!("{stderr}"));
            assert!(
                diagnostic.contains(&format!("thread {THREAD} not found")),
                "{stderr}"
            );
            let record = queued(&broker.record_text(message));
            let forwarded = forwarder.forwarded();
            assert_eq!(forwarded.len(), 3, "{forwarded:?}");
            assert_eq!(forwarded[..2], [record.clone(), record]);
            assert!(
                forwarded[2][4].starts_with(&format!(
                    "[cafleet listen closed] exit 1: failed to forward record to codex thread {THREAD}: "
                )) && forwarded[2][4].contains(&format!("thread {THREAD} not found")),
                "{forwarded:?}"
            );
            let gaps = forwarder.invocation_gaps();
            assert!(
                gaps[0] >= wait && gaps[1] < wait,
                "only the full-queue refusal waits: {gaps:?}"
            );
            assert_stdout_has_no_forwarded_records(&forwarder, &broker);
        }

        #[test]
        fn a_missing_codex_binary_fails_before_ready() {
            let broker = broker();
            let forwarder = start(
                &broker,
                &listen_args(broker.director, None),
                None,
                Launch::Session,
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(10)), 1);
            assert!(
                forwarder
                    .stderr()
                    .contains("Error: binary codex not found on PATH"),
                "{}",
                forwarder.stderr()
            );
            assert!(forwarder.stdout_records().is_empty());
        }

        #[test]
        fn a_listener_without_a_codex_ancestor_fails_before_ready() {
            let broker = broker();
            let forwarder = start(
                &broker,
                &listen_args(broker.director, None),
                Some(&accepting_codex()),
                Launch::Orphan,
            );
            assert_eq!(forwarder.wait_exit(Duration::from_secs(10)), 1);
            assert!(
                forwarder
                    .stderr()
                    .contains("Error: --codex-thread requires a codex ancestor process"),
                "{}",
                forwarder.stderr()
            );
            assert!(forwarder.stdout_records().is_empty());
            assert!(forwarder.forwarded().is_empty());
        }

        #[test]
        fn the_session_pinned_past_a_sandbox_helper_ending_closes_the_listener_with_session_exited()
        {
            let broker = broker();
            let mut forwarder = start(
                &broker,
                &listen_args(broker.director, None),
                Some(&accepting_codex()),
                Launch::SandboxedSession,
            );
            forwarder.wait_ready();
            forwarder.end_session();
            let ended = Instant::now();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert!(
                ended.elapsed() < Duration::from_secs(3),
                "{:?}",
                ended.elapsed()
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(broker.director, broker.fleet),
                    crate::output::json::listen_closed(broker.director, "session_exited"),
                ]
            );
            assert!(
                forwarder.forwarded().is_empty(),
                "nothing is forwarded to an exited session"
            );
        }
    }
}
