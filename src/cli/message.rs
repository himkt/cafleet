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
                        for error in &outcome.notification_errors {
                            eprintln!("Message {message_id} was persisted. {}", error.error);
                        }
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
        if let Some(attempted) =
            outcome.and_then(|outcome| outcome.attempted_inputs.get(&message.message_id))
        {
            if let Some(input_id) = attempted {
                let input = crate::broker::runtime::get_input(conn, *input_id)?;
                if input.run_id != runtime.run.run_id
                    || input.kind != crate::broker::runtime::InputKind::Preview
                    || input.message_id != Some(message.message_id)
                {
                    return Err(CafleetError::App(
                        "invocation preview receipt association changed".into(),
                    ));
                }
                let delivery = input.delivery().ok_or_else(|| {
                    CafleetError::App("invocation preview has no delivery projection".into())
                })?;
                return Ok((Some(*input_id), delivery));
            }
        } else if let Some(input) = crate::broker::runtime::find_latest_preview_input(
            conn,
            runtime.run.run_id,
            message.message_id,
        )? && let Some(delivery) = input.delivery()
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
const FULL_QUEUE_REFUSAL: &str = "queue cannot contain more than";
const FULL_QUEUE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(not(test))]
fn codex_queue_budget() -> Duration {
    CODEX_QUEUE_BUDGET
}

#[cfg(not(test))]
fn full_queue_retry_interval() -> Duration {
    FULL_QUEUE_RETRY_INTERVAL
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
            &next,
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
    ready: bool,
    session_exited: bool,
}

impl ListenOutput for CodexForwarding<'_> {
    fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError> {
        match record["event"].as_str() {
            Some("ready") => {
                let delivery = self.stdout.emit(record, deadline)?;
                self.ready = true;
                Ok(delivery)
            }
            Some("pending") => match self.queue.attempt(record)? {
                Attempt::Accepted => Ok(Delivery::Delivered),
                Attempt::FullQueue => Ok(Delivery::Refused),
                Attempt::SessionExited => {
                    self.session_exited = true;
                    Ok(Delivery::Refused)
                }
            },
            Some("closed") => {
                let session_ended = record["reason"] != "session_exited"
                    && self.queue.forward_closing(record)? == Forwarding::SessionExited;
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

enum Attempt {
    Accepted,
    FullQueue,
    /// The attempt failed after the pinned session ended, so the record has no reader.
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
    fn attempt(&self, record: &Value) -> Result<Attempt, CafleetError> {
        match self.queue(&format!("{LISTEN_RECORD_PREFIX}{record}")) {
            Ok(()) => Ok(Attempt::Accepted),
            Err(QueueFailure::FullQueue(_)) => Ok(Attempt::FullQueue),
            Err(QueueFailure::Deadline | QueueFailure::Exit(_))
                if pinned_session_exited(self.session)? =>
            {
                Ok(Attempt::SessionExited)
            }
            Err(QueueFailure::Deadline) => Err(CafleetError::App(self.deadline_diagnostic())),
            Err(QueueFailure::Exit(cause)) => Err(CafleetError::App(format!(
                "failed to forward record to codex thread {}: {cause}",
                self.thread
            ))),
        }
    }

    /// Queues a closing record, retrying it while the thread's queue is full for as long as
    /// the pinned session lives.
    fn forward_closing(&self, record: &Value) -> Result<Forwarding, CafleetError> {
        loop {
            match self.attempt(record)? {
                Attempt::Accepted => return Ok(Forwarding::Accepted),
                Attempt::SessionExited => return Ok(Forwarding::SessionExited),
                Attempt::FullQueue => {
                    std::thread::sleep(full_queue_retry_interval());
                    if pinned_session_exited(self.session)? {
                        return Ok(Forwarding::SessionExited);
                    }
                }
            }
        }
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

    mod composer_reliability {
        use crate::broker::runtime as storage;
        use crate::cli::message::tests::*;

        fn preview(
            fixture: &mut storage::test_support::Fixture,
            message: &MessageRecord,
        ) -> storage::InputOperation {
            let owner = fixture.run.owner();
            let request = crate::runtime::SubmitInput {
                operation_key: storage::preview_operation_key(owner.run_id, message.message_id, 1)
                    .unwrap(),
                input_kind: storage::InputKind::Preview,
                message_id: Some(message.message_id),
                literal: crate::delivery::canonical_preview_line(message),
            };
            storage::test_support::reserve_input(
                &mut fixture.conn,
                &owner,
                &storage::NewInput {
                    operation_key: request.operation_key.clone(),
                    kind: request.input_kind,
                    payload_digest: storage::input_payload_digest(owner.run_id, &request),
                    message_id: request.message_id,
                },
            )
            .unwrap()
        }

        fn message(fixture: &mut storage::test_support::Fixture, text: &str) -> MessageRecord {
            broker::send_message(&mut fixture.conn, fixture.director, fixture.member, text).unwrap()
        }

        #[test]
        fn send_projects_this_invocations_left_attempt_instead_of_its_ready_successor() {
            let mut fixture = storage::test_support::Fixture::new();
            let owner = fixture.activate();
            let message = message(&mut fixture, "private left preview");
            let input = preview(&mut fixture, &message);
            let (left, next) = storage::settle_left_and_queue_redelivery(
                &mut fixture.conn,
                &owner,
                input.input_id,
                storage::StopCause::SubmitUnconfirmed,
                storage::LeftStage::Submitted,
            )
            .unwrap();
            let mut outcome = AttemptOutcome::ended(AttemptEnd::NoPending);
            outcome
                .attempted_inputs
                .insert(message.message_id, Some(left.input_id));
            let (id, delivery) = send_delivery(&fixture.conn, &message, Some(&outcome)).unwrap();
            assert_eq!(id, Some(left.input_id));
            assert_eq!(
                delivery,
                projection(
                    DeliveryStatus::LeftInComposer,
                    Some(DeliveryMethod::Profile),
                    None
                )
            );
            let json = crate::output::json::send_outcome(&message, false, id, &delivery);
            assert_eq!(json["input_id"], left.input_id);
            assert_eq!(json["delivery"]["status"], "left_in_composer");
            assert_ne!(json["input_id"], next.input_id);
        }

        #[test]
        fn unattempted_messages_project_their_latest_ready_successor() {
            let mut fixture = storage::test_support::Fixture::new();
            let owner = fixture.activate();
            let message = message(&mut fixture, "unattempted latest preview");
            let input = preview(&mut fixture, &message);
            let (_, next) = storage::settle_left_and_queue_redelivery(
                &mut fixture.conn,
                &owner,
                input.input_id,
                storage::StopCause::ManualInput,
                storage::LeftStage::LiteralComplete,
            )
            .unwrap();
            let outcome = AttemptOutcome::ended(AttemptEnd::Held);
            assert_eq!(
                send_delivery(&fixture.conn, &message, Some(&outcome)).unwrap(),
                (
                    Some(next.input_id),
                    projection(DeliveryStatus::Queued, None, Some("redelivery")),
                )
            );
        }

        #[test]
        fn an_attempt_with_no_receipt_keeps_its_own_hold_instead_of_a_later_receipt() {
            let mut fixture = storage::test_support::Fixture::new();
            let owner = fixture.activate();
            let message = message(&mut fixture, "receipt created after a held invocation");
            let input = preview(&mut fixture, &message);
            let mut outcome = AttemptOutcome::ended(AttemptEnd::Held);
            outcome.held_reason = Some("observer_unavailable".into());
            outcome.attempted_inputs.insert(message.message_id, None);
            storage::requeue_zero_byte_immediate(
                &mut fixture.conn,
                &owner,
                input.input_id,
                "interaction_open",
            )
            .unwrap();
            assert_eq!(
                send_delivery(&fixture.conn, &message, Some(&outcome)).unwrap(),
                (
                    None,
                    projection(DeliveryStatus::Held, None, Some("observer_unavailable")),
                )
            );
        }

        #[test]
        fn recorded_input_ids_require_the_exact_preview_message_and_kind() {
            let mut fixture = storage::test_support::Fixture::new();
            let owner = fixture.activate();
            let requested = message(&mut fixture, "requested message");
            let other = message(&mut fixture, "other message");
            let other_preview = preview(&mut fixture, &other);
            let prompt = fixture.input(
                &owner,
                "prompt:wrong-kind",
                storage::InputKind::Prompt,
                None,
            );
            for input_id in [other_preview.input_id, prompt.input_id, i64::MAX] {
                let mut outcome = AttemptOutcome::ended(AttemptEnd::NoPending);
                outcome
                    .attempted_inputs
                    .insert(requested.message_id, Some(input_id));
                assert!(
                    send_delivery(&fixture.conn, &requested, Some(&outcome)).is_err(),
                    "recorded input {input_id} must bind this preview"
                );
            }
        }

        #[test]
        fn recorded_input_ids_require_the_current_recipient_run() {
            let mut fixture = storage::test_support::Fixture::new();
            fixture.activate();
            let requested = message(&mut fixture, "requested run");
            let previous = preview(&mut fixture, &requested);
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='exited',ended_at=?1 WHERE run_id=?2",
                    rusqlite::params![crate::time::now_stamp(), fixture.run.run_id],
                )
                .unwrap();
            let mut replacement = storage::test_support::Fixture::spec(
                &fixture.conn,
                fixture.fleet,
                fixture.director,
                fixture.member,
            );
            replacement.launch_nonce = "b".repeat(64);
            replacement.runtime_path.push_str("-replacement");
            fixture.run =
                storage::test_support::create_run(&mut fixture.conn, &replacement).unwrap();
            fixture.activate();
            assert_ne!(fixture.run.run_id, previous.run_id);
            assert_eq!(previous.kind, storage::InputKind::Preview);
            assert_eq!(previous.message_id, Some(requested.message_id));
            let mut outcome = AttemptOutcome::ended(AttemptEnd::NoPending);
            outcome
                .attempted_inputs
                .insert(requested.message_id, Some(previous.input_id));
            assert!(send_delivery(&fixture.conn, &requested, Some(&outcome)).is_err());
        }
    }

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
    fn send_delivery_reports_self_sends_and_the_root_director() {
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
                "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,message_id,preview_attempt,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'preview',?3,'ready',?4,1,?5,?5,?5,'interaction_open')",
                rusqlite::params![
                    run.run_id,
                    crate::broker::runtime::preview_operation_key(run.run_id, message.message_id, 1).unwrap(),
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
    use std::cell::RefCell;
    use std::io;
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::os::fd::OwnedFd;
    use std::rc::Rc;
    use std::time::Duration;
    use std::time::Instant;

    use serde_json::Value;
    use serde_json::json;

    use crate::broker;
    use crate::broker::MessageRecord;
    use crate::broker::messaging::InboxClosure;
    use crate::broker::messaging::InboxSnapshot;
    use crate::broker::messaging::test_support::broker_fixture;
    use crate::cli::message::Delivery;
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

    const NEXT: &str = "cafleet message poll 1 --json --database-url 'sqlite:///tmp/my fleet.db'";

    /// The ordered message IDs of the latest open snapshot, shared by the scripted inbox that
    /// writes them and the recording output that copies them beside each `pending` record.
    type SnapshotIds = Rc<RefCell<Vec<i64>>>;

    fn record_snapshot_ids(ids: &SnapshotIds, snapshot: &InboxSnapshot) {
        if let InboxSnapshot::Open { messages, .. } = snapshot {
            *ids.borrow_mut() = messages.iter().map(|m| m.message_id).collect();
        }
    }

    struct ScriptedInbox<'a, F> {
        clock: &'a TestClock,
        snapshots: Vec<(Duration, Duration)>,
        snapshot_ids: SnapshotIds,
        script: F,
    }

    impl<'a, F> ScriptedInbox<'a, F> {
        fn new(clock: &'a TestClock, output: &RecordingOutput<'_>, script: F) -> Self {
            Self {
                clock,
                snapshots: Vec::new(),
                snapshot_ids: output.snapshot_ids.clone(),
                script,
            }
        }
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
                self.snapshots.len() < 1_000,
                "a held notice must allow idle sleeps"
            );
            let snapshot = (self.script)(now, self.snapshots.len())?;
            record_snapshot_ids(&self.snapshot_ids, &snapshot);
            Ok(snapshot)
        }
    }

    /// Records every delivered record with the clock time it was delivered at, and beside each
    /// delivered or refused `pending` record the pending set of the snapshot that produced it.
    /// A `pending` attempt is refused when its index in `pending_refusals` is `true`; any other
    /// record is refused when its event is `refused_event`.
    struct RecordingOutput<'a> {
        clock: &'a TestClock,
        snapshot_ids: SnapshotIds,
        records: Vec<Value>,
        delivered_at: Vec<Duration>,
        notice_ids: Vec<Vec<i64>>,
        budgets: Vec<Duration>,
        fail_at: Option<usize>,
        pending_refusals: Vec<bool>,
        pending_attempts: usize,
        refused: Vec<(Duration, Vec<i64>)>,
        refused_event: Option<&'static str>,
    }

    impl<'a> RecordingOutput<'a> {
        fn new(clock: &'a TestClock) -> Self {
            Self {
                clock,
                snapshot_ids: SnapshotIds::default(),
                records: Vec::new(),
                delivered_at: Vec::new(),
                notice_ids: Vec::new(),
                budgets: Vec::new(),
                fail_at: None,
                pending_refusals: Vec::new(),
                pending_attempts: 0,
                refused: Vec::new(),
                refused_event: None,
            }
        }

        fn refusing_pending(clock: &'a TestClock, refusals: &[bool]) -> Self {
            Self {
                pending_refusals: refusals.to_vec(),
                ..Self::new(clock)
            }
        }
    }

    impl ListenOutput for RecordingOutput<'_> {
        fn emit(&mut self, record: &Value, deadline: Deadline) -> Result<Delivery, CafleetError> {
            self.budgets.push(deadline.remaining_at(self.clock.now())?);
            if self.fail_at == Some(self.records.len()) {
                return Err(CafleetError::App("injected output failure".into()));
            }
            let pending = record["event"] == "pending";
            if pending {
                assert_eq!(record, &json!({"event":"pending", "next":NEXT}));
            }
            let refuse = if pending {
                self.pending_attempts += 1;
                self.pending_refusals
                    .get(self.pending_attempts - 1)
                    .copied()
                    .unwrap_or(false)
            } else {
                self.refused_event == record["event"].as_str()
            };
            if refuse {
                if pending {
                    self.refused
                        .push((self.clock.elapsed.get(), self.snapshot_ids.borrow().clone()));
                }
                return Ok(Delivery::Refused);
            }
            if pending {
                self.notice_ids.push(self.snapshot_ids.borrow().clone());
            }
            self.records.push(record.clone());
            self.delivered_at.push(self.clock.elapsed.get());
            Ok(Delivery::Delivered)
        }
    }

    fn message(id: i64) -> MessageRecord {
        MessageRecord::fixture(id, 2, 1, &format!("original body {id}"))
    }

    fn messages(ids: &[i64]) -> Vec<MessageRecord> {
        ids.iter().copied().map(message).collect()
    }

    fn open(messages: Vec<MessageRecord>) -> InboxSnapshot {
        InboxSnapshot::Open {
            fleet_id: 7,
            messages,
        }
    }

    /// The pending set of every delivered `pending` notice, in delivery order.
    fn notices(output: &RecordingOutput<'_>) -> Vec<Vec<i64>> {
        output.notice_ids.clone()
    }

    /// The whole second at which each delivered `pending` notice was delivered.
    fn notice_times(output: &RecordingOutput<'_>) -> Vec<u64> {
        output
            .records
            .iter()
            .zip(&output.delivered_at)
            .filter(|(record, _)| record["event"] == "pending")
            .map(|(_, at)| at.as_secs())
            .collect()
    }

    /// The whole second and pending set of every refused `pending` attempt.
    fn refusals(output: &RecordingOutput<'_>) -> Vec<(u64, Vec<i64>)> {
        output
            .refused
            .iter()
            .map(|(at, ids)| (at.as_secs(), ids.clone()))
            .collect()
    }

    /// Runs one listener over `script`, which maps each snapshot's whole second to the
    /// pending IDs it returns, until the lease of `lease_secs` expires.
    fn listen_over<'a>(
        clock: &'a TestClock,
        output: &mut RecordingOutput<'a>,
        lease_secs: u64,
        script: impl Fn(u64) -> Vec<i64>,
    ) {
        let mut io = ScriptedInbox::new(clock, output, |at: Duration, _| {
            Ok(open(messages(&script(at.as_secs()))))
        });
        listen(
            clock,
            &mut io,
            output,
            1,
            Some(Duration::from_secs(lease_secs)),
            NEXT,
        )
        .unwrap();
        assert_eq!(
            output.records.last().unwrap(),
            &json!({"event":"closed", "member_id":1, "reason":"timeout"})
        );
    }

    #[test]
    fn message_ack_and_message_wait_are_unrecognized_subcommands() {
        for args in [["message", "ack", "44"], ["message", "wait", "17"]] {
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
        fn an_omitted_database_url_appends_nothing() {
            assert_eq!(poll_command(7, None), "cafleet message poll 7 --json");
        }

        #[test]
        fn a_database_url_of_only_safe_characters_is_appended_verbatim() {
            assert_eq!(
                poll_command(7, Some("sqlite:///tmp/fleet.db")),
                "cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"
            );
            let every_safe_class = "sqlite:///Home/u_1/~x/a-b.c@d%e+f=g,h.db";
            assert_eq!(
                poll_command(42, Some(every_safe_class)),
                format!("cafleet message poll 42 --json --database-url {every_safe_class}")
            );
        }

        #[test]
        fn a_database_url_with_a_space_is_single_quoted() {
            assert_eq!(
                poll_command(7, Some("sqlite:///tmp/my fleet.db")),
                "cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"
            );
        }

        #[test]
        fn a_single_quote_in_the_database_url_is_closed_escaped_and_reopened() {
            assert_eq!(
                poll_command(7, Some("sqlite:///tmp/it's.db")),
                r"cafleet message poll 7 --json --database-url 'sqlite:///tmp/it'\''s.db'"
            );
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

    #[test]
    fn a_held_notice_blocks_a_new_arrival_and_allows_one_second_idle_sleeps() {
        let clock = test_clock();
        let mut output = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &output, |at, _| {
            Ok(open(if at < Duration::from_secs(2) {
                messages(&[10])
            } else {
                messages(&[10, 11])
            }))
        });
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(3)),
            NEXT,
        )
        .unwrap();
        assert_eq!(
            output.records,
            [
                json!({"event":"ready", "member_id":1, "fleet_id":7}),
                json!({"event":"pending", "next":NEXT}),
                json!({"event":"closed", "member_id":1, "reason":"timeout"}),
            ]
        );
        assert_eq!(notices(&output), [vec![10]]);
        assert_eq!(*clock.sleeps.borrow(), vec![Duration::from_secs(1); 3]);
        assert_eq!(
            io.snapshots
                .iter()
                .map(|(at, _)| at.as_secs())
                .collect::<Vec<_>>(),
            [0, 0, 1, 2],
            "a delivered notice rechecks at once and every quiet snapshot sleeps"
        );
    }

    mod notice_hold {
        use crate::cli::message::tests::*;

        #[test]
        fn a_backlog_burst_coalesces_into_one_notice_in_snapshot_order() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 3, |_| vec![12, 10, 11]);
            assert_eq!(output.records[1], json!({"event":"pending", "next":NEXT}));
            assert_eq!(notices(&output), [vec![12, 10, 11]]);
        }

        #[test]
        fn a_burst_arriving_while_a_notice_is_held_produces_no_further_notice() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 5, |at| {
                if at < 2 {
                    vec![10]
                } else {
                    std::iter::once(10).chain(11..=160).collect()
                }
            });
            assert_eq!(notices(&output), [vec![10]]);
        }

        #[test]
        fn a_released_hold_with_nothing_pending_emits_no_notice_until_a_message_arrives() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 6, |at| match at {
                0..2 => vec![10],
                2..4 => Vec::new(),
                _ => vec![11],
            });
            assert_eq!(notices(&output), [vec![10], vec![11]]);
            assert_eq!(notice_times(&output), [0, 4]);
        }

        #[test]
        fn an_empty_inbox_emits_no_pending_record() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 3, |_| Vec::new());
            assert_eq!(
                output.records,
                [
                    json!({"event":"ready", "member_id":1, "fleet_id":7}),
                    json!({"event":"closed", "member_id":1, "reason":"timeout"}),
                ]
            );
        }

        #[test]
        fn the_hold_ceiling_re_notices_the_full_pending_set_at_300_s_when_fresh_ids_are_pending() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 400, |at| {
                if at < 10 { vec![10] } else { vec![10, 11] }
            });
            assert_eq!(notices(&output), [vec![10], vec![10, 11]]);
            assert_eq!(notice_times(&output), [0, 300]);
        }

        #[test]
        fn the_next_hold_ceiling_is_measured_from_the_re_notice() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            listen_over(&clock, &mut output, 700, |at| match at {
                0..10 => vec![10],
                10..350 => vec![10, 11],
                _ => vec![10, 11, 12],
            });
            assert_eq!(notices(&output), [vec![10], vec![10, 11], vec![10, 11, 12]]);
            assert_eq!(notice_times(&output), [0, 300, 600]);
        }

        #[test]
        fn a_restarted_listener_first_notices_every_pending_id_right_after_ready() {
            let first_clock = test_clock();
            let mut first = RecordingOutput::new(&first_clock);
            listen_over(&first_clock, &mut first, 2, |_| vec![10, 11]);
            assert_eq!(notices(&first), [vec![10, 11]]);

            let clock = test_clock();
            let mut restarted = RecordingOutput::new(&clock);
            listen_over(&clock, &mut restarted, 2, |_| vec![10, 11, 12]);
            assert_eq!(restarted.records[0]["event"], "ready");
            assert_eq!(
                restarted.records[1],
                json!({"event":"pending", "next":NEXT})
            );
            assert_eq!(notices(&restarted), [vec![10, 11, 12]]);
        }

        #[test]
        fn a_refused_re_notice_keeps_the_held_instant_so_the_retry_follows_one_idle_sleep() {
            let clock = test_clock();
            let mut output = RecordingOutput::refusing_pending(&clock, &[false, true]);
            listen_over(&clock, &mut output, 400, |at| {
                if at < 5 { vec![10] } else { vec![10, 11] }
            });
            assert_eq!(refusals(&output), [(300, vec![10, 11])]);
            assert_eq!(notices(&output), [vec![10], vec![10, 11]]);
            assert_eq!(notice_times(&output), [0, 301]);
        }

        #[test]
        fn a_released_hold_whose_pending_set_empties_during_refusals_emits_nothing() {
            let clock = test_clock();
            let mut output = RecordingOutput::refusing_pending(&clock, &[true; 100]);
            listen_over(&clock, &mut output, 5, |at| {
                if at < 2 { vec![10] } else { Vec::new() }
            });
            assert_eq!(refusals(&output), [(0, vec![10]), (1, vec![10])]);
            assert!(notices(&output).is_empty());
        }

        #[test]
        #[should_panic(expected = "only a pending notice may be refused")]
        fn a_refused_ready_record_panics() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            output.refused_event = Some("ready");
            listen_over(&clock, &mut output, 2, |_| Vec::new());
        }

        #[test]
        #[should_panic(expected = "only a pending notice may be refused")]
        fn a_refused_closed_record_panics() {
            let clock = test_clock();
            let mut output = RecordingOutput::new(&clock);
            output.refused_event = Some("closed");
            listen_over(&clock, &mut output, 2, |_| Vec::new());
        }

        mod consuming_polls {
            use crate::broker::messaging::listen_snapshot;
            use crate::cli::message::tests::*;

            enum Act {
                Send(usize),
                Poll,
            }

            /// A real broker inbox for the Director whose scripted sends and consuming polls
            /// run, in order, before the first snapshot taken at or after their second.
            struct ScriptedBroker<'a> {
                _dir: tempfile::TempDir,
                clock: &'a TestClock,
                conn: Connection,
                fleet: i64,
                director: i64,
                worker: i64,
                script: Vec<(u64, Act)>,
                applied: usize,
                snapshots: usize,
                snapshot_ids: SnapshotIds,
                sent: Vec<i64>,
                polls: Vec<Vec<i64>>,
            }

            impl ListenIo for ScriptedBroker<'_> {
                fn snapshot(&mut self, deadline: Deadline) -> Result<InboxSnapshot, CafleetError> {
                    self.snapshots += 1;
                    assert!(
                        self.snapshots < 2_000,
                        "a held notice must allow idle sleeps"
                    );
                    let now = self.clock.elapsed.get().as_secs();
                    while let Some((at, act)) = self.script.get(self.applied)
                        && *at <= now
                    {
                        match act {
                            Act::Send(count) => {
                                for _ in 0..*count {
                                    let id = broker::send_message(
                                        &mut self.conn,
                                        self.worker,
                                        self.director,
                                        "scripted",
                                    )
                                    .unwrap()
                                    .message_id;
                                    self.sent.push(id);
                                }
                            }
                            Act::Poll => {
                                let consumed = broker::poll_messages(&self.conn, self.director)
                                    .unwrap()
                                    .into_iter()
                                    .map(|message| message.message_id)
                                    .collect();
                                self.polls.push(consumed);
                            }
                        }
                        self.applied += 1;
                    }
                    let snapshot =
                        listen_snapshot(&mut self.conn, self.director, Some(self.fleet), deadline)?;
                    record_snapshot_ids(&self.snapshot_ids, &snapshot);
                    Ok(snapshot)
                }
            }

            fn listen_broker<'a>(
                clock: &'a TestClock,
                output: &mut RecordingOutput<'a>,
                lease_secs: u64,
                script: Vec<(u64, Act)>,
            ) -> ScriptedBroker<'a> {
                let (dir, conn, fleet, director, worker) = broker_fixture();
                let mut io = ScriptedBroker {
                    _dir: dir,
                    clock,
                    conn,
                    fleet,
                    director,
                    worker,
                    script,
                    applied: 0,
                    snapshots: 0,
                    snapshot_ids: output.snapshot_ids.clone(),
                    sent: Vec::new(),
                    polls: Vec::new(),
                };
                listen(
                    clock,
                    &mut io,
                    output,
                    director,
                    Some(Duration::from_secs(lease_secs)),
                    NEXT,
                )
                .unwrap();
                assert_eq!(
                    output.records.last().unwrap()["reason"],
                    "timeout",
                    "{:?}",
                    output.records
                );
                io
            }

            #[test]
            fn a_consuming_poll_releases_the_hold_and_the_next_notice_names_only_post_poll_ids() {
                let clock = test_clock();
                let mut output = RecordingOutput::new(&clock);
                let broker = listen_broker(
                    &clock,
                    &mut output,
                    6,
                    vec![
                        (0, Act::Send(1)),
                        (2, Act::Send(1)),
                        (3, Act::Poll),
                        (4, Act::Send(1)),
                    ],
                );
                let sent = &broker.sent;
                assert_eq!(broker.polls, [vec![sent[1], sent[0]]]);
                assert_eq!(notices(&output), [vec![sent[0]], vec![sent[2]]]);
                assert_eq!(notice_times(&output), [0, 4]);
                assert!(
                    notices(&output).iter().flatten().all(|id| *id != sent[1]),
                    "an ID polled while held is never notified"
                );
            }

            #[test]
            fn mid_turn_polls_never_re_list_an_id_and_bound_the_notice_count() {
                let clock = test_clock();
                let mut output = RecordingOutput::new(&clock);
                let mut script = Vec::new();
                for second in 0..120 {
                    script.push((second, Act::Send(1 + (second as usize % 3))));
                    if second % 7 == 3 || second % 11 == 5 {
                        script.push((second, Act::Poll));
                    }
                }
                let broker = listen_broker(&clock, &mut output, 120, script);
                let notified = notices(&output);
                let mut seen = std::collections::BTreeSet::new();
                for id in notified.iter().flatten() {
                    assert!(
                        seen.insert(*id),
                        "message {id} appears in two notices: {notified:?}"
                    );
                    assert!(broker.sent.contains(id));
                }
                let consuming_polls = broker.polls.iter().filter(|ids| !ids.is_empty()).count();
                assert!(consuming_polls >= 10);
                assert!(
                    notified.len() <= 1 + consuming_polls,
                    "{} notices for {consuming_polls} consuming polls",
                    notified.len()
                );
                assert!(notified.len() >= 2, "every release admits a later notice");
            }

            #[test]
            fn an_unpolled_hold_with_a_fresh_arrival_re_notices_once_at_the_ceiling_naming_held_and_fresh_ids()
             {
                let clock = test_clock();
                let mut output = RecordingOutput::new(&clock);
                let broker = listen_broker(
                    &clock,
                    &mut output,
                    700,
                    vec![(0, Act::Send(1)), (10, Act::Send(1))],
                );
                let sent = &broker.sent;
                assert_eq!(notices(&output), [vec![sent[0]], vec![sent[0], sent[1]]]);
                assert_eq!(
                    notice_times(&output),
                    [0, HOLD_CEILING.as_secs()],
                    "the re-notice fires exactly at the ceiling, never before it"
                );
            }

            #[test]
            fn an_unpolled_hold_without_a_fresh_arrival_never_re_notices() {
                let clock = test_clock();
                let mut output = RecordingOutput::new(&clock);
                let broker = listen_broker(&clock, &mut output, 700, vec![(0, Act::Send(2))]);
                assert_eq!(notices(&output), std::slice::from_ref(&broker.sent));
            }

            #[test]
            fn a_poll_before_the_ceiling_cancels_the_re_notice() {
                let clock = test_clock();
                let mut output = RecordingOutput::new(&clock);
                let broker = listen_broker(
                    &clock,
                    &mut output,
                    400,
                    vec![
                        (0, Act::Send(1)),
                        (10, Act::Send(1)),
                        (200, Act::Poll),
                        (350, Act::Send(1)),
                    ],
                );
                let sent = &broker.sent;
                assert_eq!(broker.polls, [vec![sent[1], sent[0]]]);
                assert_eq!(notices(&output), [vec![sent[0]], vec![sent[2]]]);
                assert_eq!(notice_times(&output), [0, 350]);
            }

            #[test]
            fn a_refused_notice_is_rebuilt_from_the_current_pending_set_and_only_delivery_commits_the_hold()
             {
                let clock = test_clock();
                let mut output = RecordingOutput::refusing_pending(&clock, &[true, true]);
                let broker = listen_broker(
                    &clock,
                    &mut output,
                    6,
                    vec![
                        (0, Act::Send(1)),
                        (1, Act::Send(1)),
                        (2, Act::Poll),
                        (2, Act::Send(2)),
                        (4, Act::Send(1)),
                    ],
                );
                let sent = &broker.sent;
                assert_eq!(
                    refusals(&output),
                    [(0, vec![sent[0]]), (1, vec![sent[0], sent[1]])],
                    "each retry follows a one-second idle sleep and a fresh snapshot"
                );
                assert_eq!(notices(&output), [vec![sent[2], sent[3]]]);
                assert_eq!(notice_times(&output), [2]);
            }

            #[test]
            fn a_refused_re_notice_is_abandoned_once_a_poll_consumes_its_ids() {
                let clock = test_clock();
                let mut output = RecordingOutput::refusing_pending(&clock, &[false, true, true]);
                let broker = listen_broker(
                    &clock,
                    &mut output,
                    400,
                    vec![(0, Act::Send(1)), (5, Act::Send(1)), (301, Act::Poll)],
                );
                let sent = &broker.sent;
                assert_eq!(refusals(&output), [(300, vec![sent[0], sent[1]])]);
                assert_eq!(notices(&output), [vec![sent[0]]]);
                assert_eq!(broker.polls, [vec![sent[1], sent[0]]]);
            }
        }

        #[test]
        #[should_panic]
        fn committing_an_empty_hold_panics() {
            let mut hold = NoticeHold(None);
            hold.commit(Vec::new(), Instant::now());
        }
    }

    #[test]
    fn idle_checks_use_the_remaining_monotonic_lease_without_heartbeats() {
        let clock = test_clock();
        let mut output = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &output, |_, _| Ok(open(Vec::new())));
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_millis(2500)),
            NEXT,
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
        let mut output = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &output, |at: Duration, _: usize| {
            Ok(open(vec![message(i64::try_from(at.as_secs()).unwrap())]))
        });
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(8)),
            NEXT,
        )
        .unwrap();
        assert_eq!(clock.elapsed.get(), Duration::from_secs(8));
        assert_eq!(
            notices(&output),
            (0..8).map(|id| vec![id]).collect::<Vec<_>>(),
            "each second a poll consumes the held ID, so the release notices the new arrival"
        );
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
            let mut output = RecordingOutput::new(&clock);
            let mut io = ScriptedInbox::new(&clock, &output, |at, _| {
                if at < Duration::from_secs(2) {
                    Ok(open(Vec::new()))
                } else {
                    Ok(InboxSnapshot::Closed {
                        fleet_id: 7,
                        reason: closure,
                    })
                }
            });
            listen(&clock, &mut io, &mut output, 1, None, NEXT).unwrap();
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
            let mut output = RecordingOutput::new(&clock);
            let mut io = ScriptedInbox::new(&clock, &output, |_, check| {
                match (check == fail_at, fail_at) {
                    (true, 1) => Err(CafleetError::App("snapshot failed".into())),
                    (true, _) => Err(CafleetError::DeadlineExceeded),
                    (false, _) => Ok(open(vec![message(10)])),
                }
            });
            let error = listen(
                &clock,
                &mut io,
                &mut output,
                1,
                Some(Duration::from_secs(3)),
                NEXT,
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
        let mut output = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &output, |_, check| {
            assert_eq!(check, 1);
            clock.elapsed.set(Duration::from_secs(2));
            Ok(open(Vec::new()))
        });
        assert!(matches!(
            listen(
                &clock,
                &mut io,
                &mut output,
                1,
                Some(Duration::from_secs(1)),
                NEXT
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
            let mut output = RecordingOutput::new(&clock);
            let mut io = ScriptedInbox::new(&clock, &output, |_, _| {
                Ok(open(vec![message(10), message(11)]))
            });
            output.fail_at = Some(fail_at);
            let error = listen(
                &clock,
                &mut io,
                &mut output,
                1,
                Some(Duration::from_secs(2)),
                NEXT,
            )
            .unwrap_err();
            assert!(error.message().contains("injected output failure"));
            assert_eq!(output.records.len(), fail_at);
            assert_eq!(io.snapshots.len(), 1);
        }
    }

    #[test]
    fn a_poll_output_race_preserves_the_snapshot_notice() {
        let clock = test_clock();
        let consumed = Cell::new(false);
        let recording = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &recording, |_, _| {
            Ok(open(if consumed.get() {
                Vec::new()
            } else {
                messages(&[10, 11])
            }))
        });
        struct PollDuringReady<'a> {
            output: RecordingOutput<'a>,
            consumed: &'a Cell<bool>,
        }
        impl ListenOutput for PollDuringReady<'_> {
            fn emit(
                &mut self,
                record: &Value,
                deadline: Deadline,
            ) -> Result<Delivery, CafleetError> {
                let delivery = self.output.emit(record, deadline)?;
                if record["event"] == "ready" {
                    self.consumed.set(true);
                }
                if record["event"] == "pending" {
                    assert!(self.consumed.get());
                }
                Ok(delivery)
            }
        }
        let mut output = PollDuringReady {
            output: recording,
            consumed: &consumed,
        };
        listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(1)),
            NEXT,
        )
        .unwrap();
        assert_eq!(
            notices(&output.output),
            [vec![10, 11]],
            "the notice names the snapshot that produced it"
        );
    }

    #[test]
    fn a_failed_final_closed_record_is_an_output_error() {
        let clock = test_clock();
        let mut output = RecordingOutput::new(&clock);
        let mut io = ScriptedInbox::new(&clock, &output, |_, _| Ok(open(Vec::new())));
        output.fail_at = Some(1);
        let error = listen(
            &clock,
            &mut io,
            &mut output,
            1,
            Some(Duration::from_secs(1)),
            NEXT,
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
        let record = json!({"event":"closed", "member_id":1, "reason":"界\n\t🙂"});
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
        let pending = broker::send_message(&mut conn, worker, director, "pending").unwrap();
        for (broken, nonblocking) in [(false, true), (true, false)] {
            let (read_end, write_end, retained, original) = primed_pipe(nonblocking);
            let mut read_end = Some(read_end);
            let filled = if broken {
                drop(read_end.take());
                0
            } else {
                fill_pipe(&write_end)
            };
            let started = Instant::now();
            let error = run_on_descriptor(
                &mut conn,
                director,
                Some(Duration::from_millis(100)),
                NEXT,
                write_end.as_fd(),
            )
            .unwrap_err();
            assert!(started.elapsed() < Duration::from_secs(2), "{error}");
            assert_eq!(fcntl(&retained, FcntlArg::F_GETFL).unwrap(), original);
            assert!(conn.is_autocommit());
            assert_eq!(
                broker::get_message(&conn, pending.message_id).unwrap(),
                pending,
                "the listener consumes nothing"
            );
            if let Some(read_end) = read_end {
                let flags = OFlag::from_bits_retain(fcntl(&read_end, FcntlArg::F_GETFL).unwrap());
                fcntl(&read_end, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
                let mut drained = Vec::new();
                let mut bytes = [0; 4096];
                loop {
                    match nix::unistd::read(&read_end, &mut bytes) {
                        Ok(0) | Err(nix::errno::Errno::EAGAIN) => break,
                        Ok(count) => drained.extend_from_slice(&bytes[..count]),
                        Err(error) => panic!("draining the primed pipe: {error}"),
                    }
                }
                assert_eq!(drained.len(), filled);
                assert!(
                    drained.iter().all(|byte| *byte == b'x'),
                    "the blocked listener wrote no partial record"
                );
            }
        }
    }

    /// Fills a nonblocking pipe until the kernel refuses another byte; returns the byte count.
    fn fill_pipe(write_end: &OwnedFd) -> usize {
        let mut filled = 0;
        for chunk in [4096, 1] {
            loop {
                match nix::unistd::write(write_end, &vec![b'x'; chunk]) {
                    Ok(count) => filled += count,
                    Err(nix::errno::Errno::EAGAIN) => break,
                    Err(error) => panic!("filling the pipe: {error}"),
                }
            }
        }
        filled
    }

    #[test]
    fn a_drained_native_pipe_emits_full_jsonl_and_restores_its_shared_flags() {
        use std::io::Read;
        let (_dir, mut conn, _, director, worker) = broker_fixture();
        let pending = broker::send_message(&mut conn, worker, director, "雪\n\t").unwrap();
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
            NEXT,
            write_end.as_fd(),
        )
        .unwrap();
        assert_eq!(fcntl(&retained, FcntlArg::F_GETFL).unwrap(), original);
        drop(write_end);
        drop(retained);
        let bytes = consumer.join().unwrap();
        let lines: Vec<&[u8]> = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        let records: Vec<Value> = lines
            .iter()
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["event"], "ready");
        assert_eq!(
            std::str::from_utf8(lines[1]).unwrap(),
            format!(r#"{{"event":"pending","next":"{NEXT}"}}"#),
            "the notice line is the compact record carrying the supplied next command"
        );
        assert_eq!(records[2]["reason"], "timeout");
        assert_eq!(
            broker::get_message(&conn, pending.message_id).unwrap(),
            pending,
            "the listener consumes nothing"
        );
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
        }

        /// The harness always starts the forwarder with an explicit `--database-url`, so its
        /// notice's `next` carries that URL as a shell word.
        fn pending_text(broker: &Broker, member_id: i64) -> String {
            format!(
                r#"{RECORD_PREFIX}{{"event":"pending","next":"cafleet message poll {member_id} --json --database-url {}"}}"#,
                shell_word(&broker.url)
            )
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
        fn forwarding_queues_one_notice_per_hold_then_the_closing_record_and_stdout_keeps_only_ready_and_closed()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let backlog =
                ["first", "second", "third"].map(|body| broker.send(worker, director, body));
            let forwarder = start(
                &broker,
                &listen_args(director, Some(5)),
                Some(&accepting_codex()),
                Launch::Session,
            );
            forwarder.wait_forwarded(1);
            let held = broker.send(worker, director, "fourth");
            let later = broker.send(worker, director, "fifth");
            std::thread::sleep(Duration::from_millis(1500));
            assert_eq!(
                forwarder.forwarded(),
                [queued(&pending_text(&broker, director))],
                "arrivals while the notice is held are not forwarded"
            );
            let consumed = broker::poll_messages(&broker.conn, director)
                .unwrap()
                .iter()
                .map(|message| message.message_id)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                consumed,
                backlog.into_iter().chain([held, later]).collect(),
                "one poll consumes the held and later arrivals together"
            );
            for body in ["sixth", "seventh"] {
                broker.send(worker, director, body);
            }
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(15)),
                0,
                "{}",
                forwarder.stderr()
            );
            let notice = queued(&pending_text(&broker, director));
            assert_eq!(
                forwarder.forwarded(),
                [
                    notice.clone(),
                    notice,
                    queued(&closed_text(director, "timeout")),
                ],
                "ready is never forwarded, and the poll releases the hold for one later notice"
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(director, broker.fleet),
                    crate::output::json::listen_closed(director, "timeout"),
                ]
            );
            assert_eq!(
                broker::poll_messages(&broker.conn, director).unwrap().len(),
                2,
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
        fn a_refused_notice_is_retried_each_second_from_the_current_pending_set_until_accepted() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            for body in ["first", "second"] {
                broker.send(worker, director, body);
            }
            let forwarder = start(
                &broker,
                &listen_args(director, Some(5)),
                Some(&full_queue_codex(Some(2), "exit 0")),
                Launch::Session,
            );
            forwarder.wait_forwarded(1);
            broker.send(worker, director, "third");
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(15)),
                0,
                "{}",
                forwarder.stderr()
            );
            let notice = queued(&pending_text(&broker, director));
            assert_eq!(
                forwarder.forwarded(),
                [
                    notice.clone(),
                    notice.clone(),
                    notice,
                    queued(&closed_text(director, "timeout")),
                ],
                "two refusals, one acceptance, then the closing record"
            );
            let gaps = forwarder.invocation_gaps();
            assert!(
                gaps[..2].iter().all(|gap| *gap >= Duration::from_secs(1)),
                "a refused notice waits the listener's one-second idle interval: {gaps:?}"
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(director, broker.fleet),
                    crate::output::json::listen_closed(director, "timeout"),
                ]
            );
        }

        /// Answers every `pending` notice with Codex's full-queue refusal and accepts the rest.
        fn codex_refusing_notices() -> String {
            format!(
                "{LOG_ARGV}\ncase \"$*\" in\n  *'\"event\":\"pending\"'*)\n{FULL_QUEUE_REFUSAL}\n;;\nesac\nexit 0"
            )
        }

        #[test]
        fn a_refused_notice_whose_ids_are_polled_during_refusals_is_abandoned() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            let message = broker.send(worker, director, "polled while refused");
            let forwarder = start(
                &broker,
                &listen_args(director, Some(5)),
                Some(&codex_refusing_notices()),
                Launch::Session,
            );
            forwarder.wait_forwarded(2);
            assert_eq!(
                broker::poll_messages(&broker.conn, director)
                    .unwrap()
                    .iter()
                    .map(|consumed| consumed.message_id)
                    .collect::<Vec<_>>(),
                [message]
            );
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(15)),
                0,
                "{}",
                forwarder.stderr()
            );
            let notice = queued(&pending_text(&broker, director));
            assert_eq!(
                forwarder.forwarded(),
                [
                    notice.clone(),
                    notice,
                    queued(&closed_text(director, "timeout")),
                ],
                "no notice is attempted once the pending set is empty"
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
            broker.send(worker, director, "waiting");
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
            let notice = queued(&pending_text(&broker, director));
            let forwarded = forwarder.forwarded();
            assert!(
                forwarded.iter().all(|invocation| *invocation == notice),
                "only the refused notice is attempted, with no closed notice: {forwarded:?}"
            );
        }

        #[test]
        fn a_codex_queue_failing_after_the_pinned_session_ended_closes_the_listener_with_session_exited()
         {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            broker.send(worker, director, "in flight");
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
                [queued(&pending_text(&broker, director))],
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

        /// Deregisters `worker` under a running forwarder and waits until its closing record has
        /// been refused for more than the five-second closing output budget.
        fn refuse_the_closing_record_past_five_seconds(forwarder: &Forwarder, broker: &mut Broker) {
            forwarder.wait_ready();
            assert!(
                crate::broker::members::test_support::deregister_member(
                    &mut broker.conn,
                    broker.worker
                )
                .unwrap()
            );
            let closed = Instant::now();
            forwarder.wait_for(
                "closing-record refusals past 6 s",
                Duration::from_secs(20),
                |forwarder| {
                    closed.elapsed() >= Duration::from_secs(6) && forwarder.forwarded().len() >= 2
                },
            );
            assert_eq!(
                forwarder.status(),
                None,
                "a refused closing record keeps the forwarder alive: {}",
                forwarder.stderr()
            );
        }

        fn assert_only_the_closing_record_was_retried(forwarder: &Forwarder, worker: i64) {
            let closing = queued(&closed_text(worker, "member_inactive"));
            let forwarded = forwarder.forwarded();
            assert!(
                forwarded.len() >= 10 && forwarded.iter().all(|attempt| *attempt == closing),
                "{forwarded:?}"
            );
            let gaps = forwarder.invocation_gaps();
            assert!(
                gaps.iter().all(|gap| *gap >= RETRY),
                "each closing retry waits the retry interval: {gaps:?}"
            );
        }

        #[test]
        fn a_refused_closing_record_retries_until_the_pinned_session_ends_and_closes_with_session_exited()
         {
            let mut broker = broker();
            let worker = broker.worker;
            let mut forwarder = start(
                &broker,
                &listen_args(worker, None),
                Some(&full_queue_codex(None, "exit 0")),
                Launch::Session,
            );
            refuse_the_closing_record_past_five_seconds(&forwarder, &mut broker);
            forwarder.end_session();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert_closed_with_session_exited(&forwarder, &broker, worker);
            assert_only_the_closing_record_was_retried(&forwarder, worker);
        }

        #[test]
        fn a_closing_record_refused_for_more_than_five_seconds_is_written_to_stdout_after_acceptance()
         {
            let mut broker = broker();
            let worker = broker.worker;
            let forwarder = start(
                &broker,
                &listen_args(worker, None),
                Some(&format!(
                    "{LOG_ARGV}\ndate +%s%N >>\"$CAFLEET_TEST_CODEX_LOG.times\"\nif [ ! -e \"$CAFLEET_TEST_CODEX_LOG.release\" ]; then\n{FULL_QUEUE_REFUSAL}\nfi\nexit 0"
                )),
                Launch::Session,
            );
            refuse_the_closing_record_past_five_seconds(&forwarder, &mut broker);
            std::fs::write(forwarder.file(RELEASE), "").unwrap();
            assert_eq!(
                forwarder.wait_exit(Duration::from_secs(5)),
                0,
                "{}",
                forwarder.stderr()
            );
            assert_eq!(
                forwarder.stdout_records(),
                [
                    crate::output::json::listen_ready(worker, broker.fleet),
                    crate::output::json::listen_closed(worker, "member_inactive"),
                ]
            );
            assert_only_the_closing_record_was_retried(&forwarder, worker);
        }

        #[test]
        fn a_non_full_queue_failure_after_a_refusal_fails_without_another_wait() {
            let mut broker = broker();
            let (director, worker) = (broker.director, broker.worker);
            broker.send(worker, director, "refused");
            let wait = Duration::from_secs(3);
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
            let notice = queued(&pending_text(&broker, director));
            let forwarded = forwarder.forwarded();
            assert_eq!(forwarded.len(), 3, "{forwarded:?}");
            assert_eq!(forwarded[..2], [notice.clone(), notice]);
            assert!(
                forwarded[2][4].starts_with(&format!(
                    "[cafleet listen closed] exit 1: failed to forward record to codex thread {THREAD}: "
                )) && forwarded[2][4].contains(&format!("thread {THREAD} not found")),
                "{forwarded:?}"
            );
            let gaps = forwarder.invocation_gaps();
            assert!(
                gaps[0] >= Duration::from_secs(1) && gaps[0] < wait,
                "a refused notice waits the listener's idle interval, not the closing retry interval: {gaps:?}"
            );
            assert!(
                gaps[1] < Duration::from_secs(1),
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
