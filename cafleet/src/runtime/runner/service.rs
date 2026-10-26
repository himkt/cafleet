pub(super) use super::control::request;
use super::writer::EventWriter;
use super::*;
use crate::broker::members::db_err;
use crate::broker::{
    self,
    runtime::{self as storage, *},
};
use crate::config::Settings;
use filedescriptor::FileDescriptor;
use rusqlite::params;
use serde_json::json;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) fn terminal_size(terminal: &impl std::os::fd::AsFd) -> Result<TerminalSize> {
    let size = rustix::termios::tcgetwinsize(terminal).map_err(io_error)?;
    let size = TerminalSize {
        rows: size.ws_row,
        cols: size.ws_col,
    };
    size.validate()?;
    Ok(size)
}
struct NonblockingFile {
    file: FileDescriptor,
    flags: nix::fcntl::OFlag,
    restored: bool,
}
impl NonblockingFile {
    fn new(source: &impl AsRawFd) -> Result<Self> {
        let mut file = FileDescriptor::dup(source).map_err(io_error)?;
        let flags = nix::fcntl::OFlag::from_bits_retain(
            nix::fcntl::fcntl(&file, nix::fcntl::FcntlArg::F_GETFL).map_err(io_error)?,
        );
        file.set_non_blocking(true).map_err(io_error)?;
        Ok(Self {
            file,
            flags,
            restored: false,
        })
    }
    fn restore(&mut self) -> Result<()> {
        if !self.restored {
            nix::fcntl::fcntl(&self.file, nix::fcntl::FcntlArg::F_SETFL(self.flags))
                .map_err(io_error)?;
            self.restored = true;
        }
        Ok(())
    }
}
impl Drop for NonblockingFile {
    fn drop(&mut self) {
        if let Err(cause) = self.restore() {
            eprintln!("{cause}");
        }
    }
}
impl ByteInput for NonblockingFile {
    fn try_read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Display for NonblockingFile {
    fn try_write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.file.write(bytes)
    }
    fn try_flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
struct OrderedObserver;
impl ScreenObserver for OrderedObserver {
    fn observe(&mut self, _: &ScreenSnapshot) -> Result<()> {
        Ok(())
    }
}
struct SignalGuards {
    ids: Vec<signal_hook::SigId>,
}
impl SignalGuards {
    fn new(terminate: Arc<AtomicBool>, resize: Arc<AtomicBool>) -> Result<Self> {
        let mut guard = Self { ids: Vec::new() };
        for signal in [nix::libc::SIGTERM, nix::libc::SIGINT, nix::libc::SIGHUP] {
            guard
                .ids
                .push(signal_hook::flag::register(signal, terminate.clone()).map_err(io_error)?);
        }
        guard
            .ids
            .push(signal_hook::flag::register(nix::libc::SIGWINCH, resize).map_err(io_error)?);
        Ok(guard)
    }
}
impl Drop for SignalGuards {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}
struct Client {
    id: u64,
    stream: UnixStream,
    decoder: ControlDecoder,
    output: VecDeque<u8>,
    closed: bool,
}
impl Client {
    fn poll(&mut self) -> Result<Vec<Value>> {
        let mut requests = Vec::new();
        let mut bytes = [0; 4096];
        match self.stream.read(&mut bytes) {
            Ok(0) => {
                self.decoder.finish()?;
                self.closed = true;
            }
            Ok(count) => requests = self.decoder.feed(&bytes[..count])?,
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(cause) => return Err(io_error(cause)),
        }
        ensure(requests.len() <= 16, "too many control requests per tick")?;
        self.flush()?;
        Ok(requests)
    }
    fn reply(&mut self, result: Result<Value>) -> Result<()> {
        let value = match result {
            Ok(value) => json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"result":value}),
            Err(cause) => {
                json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"error":cause.message()})
            }
        };
        let bytes = encode_control_frame(&value)?;
        ensure(
            self.output.len() + bytes.len() <= MAX_CONTROL_FRAME + 4,
            "control reply buffer is full",
        )?;
        self.output.extend(bytes);
        self.flush()
    }
    fn flush(&mut self) -> Result<()> {
        if !self.output.is_empty() {
            match self.stream.write(self.output.make_contiguous()) {
                Ok(0) => self.closed = true,
                Ok(count) => {
                    self.output.drain(..count);
                }
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(cause) => return Err(io_error(cause)),
            }
        }
        Ok(())
    }
}
fn lifecycle_event(
    run: &RunRecord,
    sequence: i64,
    kind: RuntimeEventKind,
    cause: Value,
    screen_seq: u64,
) -> Result<ObservedTransition> {
    let evidence=storage::evidence::EvidencePolicy::metadata().apply(&json!({"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"rule_id":"runner-lifecycle","screen_seq":screen_seq,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false}))?;
    Ok(ObservedTransition {
        run_id: run.run_id,
        event_seq: sequence,
        episode_seq: None,
        kind,
        classification: None,
        observed_at: crate::time::format_utc(crate::time::now_utc()),
        evidence_json: evidence,
        close_reason: None,
        cause_json: Some(cause),
    })
}
fn exit_cause(exit: ChildExit) -> Value {
    match exit {
        ChildExit::Code(code) => json!({"kind":"native_exit","exit_code":code,"exit_signal":null}),
        ChildExit::Signal(signal) => {
            json!({"kind":"native_exit","exit_code":null,"exit_signal":signal})
        }
    }
}
struct LiveShutdown<'a> {
    pty: &'a mut PtySession,
    transport: &'a mut Transport,
    input: &'a mut NonblockingFile,
    output: &'a mut NonblockingFile,
    raw: &'a mut RawTerminalGuard,
    writer: &'a mut EventWriter,
    clock: ShutdownClock,
    run: &'a RunRecord,
    sequence: &'a mut i64,
    exit_sent: &'a mut bool,
}
impl LiveShutdown<'_> {
    fn send_exit(&mut self) -> Result<()> {
        if !*self.exit_sent
            && let Some(exit) = self.pty.try_wait()?
        {
            let event = lifecycle_event(
                self.run,
                *self.sequence,
                RuntimeEventKind::RunnerExited,
                exit_cause(exit),
                self.transport.snapshot().screen_seq,
            )?;
            self.writer.submit(event)?;
            *self.sequence += 1;
            *self.exit_sent = true;
        }
        Ok(())
    }
}
impl ShutdownHost for LiveShutdown<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn gate_programmatic_input(&mut self) {
        self.writer.set_deadline(self.clock.storage_deadline());
    }
    fn term_verified_descendants(&mut self) -> Result<()> {
        self.pty.signal_verified(nix::sys::signal::Signal::SIGTERM)
    }
    fn drain_and_reap_until(&mut self, deadline: Instant) -> Result<Option<ChildExit>> {
        let mut observer = OrderedObserver;
        while Instant::now() < deadline {
            let progress = self
                .transport
                .tick(self.input, self.pty, self.output, &mut observer)?;
            self.send_exit()?;
            if !self.pty.survivors()? && progress.child_eof {
                break;
            }
            std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        self.pty.try_wait()
    }
    fn kill_verified_survivors(&mut self) -> Result<()> {
        self.pty.signal_verified(nix::sys::signal::Signal::SIGKILL)
    }
    fn close_pty(&mut self) -> Result<()> {
        self.pty.close()?;
        while Instant::now() < self.clock.storage_deadline() {
            if (self.pty.try_wait()?.is_some() || !self.pty.has_child()) && !self.pty.survivors()? {
                break;
            }
            std::thread::sleep(
                Duration::from_millis(2).min(
                    self.clock
                        .storage_deadline()
                        .saturating_duration_since(Instant::now()),
                ),
            );
        }
        self.send_exit()?;
        ensure(
            self.transport.pending_input_bytes() == 0,
            "human input tail remains undelivered after native termination",
        )?;
        ensure(
            !self.pty.survivors()?,
            "owned descendants remain unconfirmed",
        )
    }
    fn flush_display_until(&mut self, deadline: Instant) -> Result<u64> {
        while self.transport.pending_display_bytes() > 0 && Instant::now() < deadline {
            self.transport.flush_display(self.output)?;
            std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        Ok(self.transport.discard_display_tail() as u64)
    }
    fn final_native_exit(&mut self) -> Result<Option<ChildExit>> {
        self.pty.try_wait()
    }
    fn native_was_spawned(&self) -> bool {
        self.pty.has_child()
    }
    fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64)> {
        self.send_exit()?;
        if self.clock.remaining_storage_at(Instant::now()).is_zero() {
            return Err(crate::error::CafleetError::DeadlineExceeded);
        }
        Ok(self.writer.finish(deadline))
    }
    fn restore_terminal(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        if let Err(cause) = self.output.restore() {
            errors.push(cause.message().into_owned());
        }
        if let Err(cause) = self.input.restore() {
            errors.push(cause.message().into_owned());
        }
        if let Err(cause) = self.raw.restore() {
            errors.push(cause.message().into_owned());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(error(errors.join("; ")))
        }
    }
}

pub(crate) fn run(settings: &Settings, descriptor_path: &Path) -> Result<i32> {
    let mut conn = crate::db::connect(&settings.database_url)?;
    crate::diagnosis::classify_selected_schema(
        &conn,
        crate::db::head_version(),
        &settings.database_url,
    )
    .require_current()?;
    let descriptor = load_descriptor(&conn, descriptor_path)?;
    let original = storage::get_run(&conn, descriptor.run_id)?;
    let creator = ProcessIdentity {
        pid: original.owner_pid,
        start_identity: original.owner_start_identity.clone(),
    };
    ensure(
        creator.matches()?,
        "launch creator process identity changed",
    )?;
    let lock = Arc::new(
        RunLock::acquire(&conn, &original.owner())?
            .ok_or_else(|| error("run ownership lock is held"))?,
    );
    let process = ProcessIdentity::current()?;
    let mut handshake_deadline = Instant::now() + Duration::from_secs(10);
    let owner = lock.transfer_to_runner_with_deadline(
        &mut conn,
        &original.owner(),
        process.clone(),
        handshake_deadline,
    )?;
    let mut run = storage::get_run(&conn, owner.run_id)?;
    let artifacts = RunArtifacts::from_run(&conn, &run)?;
    let terminal = FileDescriptor::dup(&std::io::stdin())
        .map_err(io_error)?
        .as_file()
        .map_err(io_error)?;
    let initial_size = terminal_size(&terminal)?;
    let mut pty = PtySession::prepare(initial_size)?;
    let mut raw = RawTerminalGuard::enter(&terminal)?;
    let mut input = NonblockingFile::new(&std::io::stdin())?;
    let mut output = NonblockingFile::new(&std::io::stdout())?;
    let mut transport = Transport::new(
        initial_size,
        TransportLimits {
            input_bytes: 65536,
            display_bytes: 262144,
            bytes_per_tick: 16384,
        },
    )?;
    let listener = UnixListener::bind(&artifacts.paths.control_socket).map_err(|cause| {
        error(format!(
            "cannot bind private runtime control socket: {cause}"
        ))
    })?;
    std::fs::set_permissions(
        &artifacts.paths.control_socket,
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(io_error)?;
    listener.set_nonblocking(true).map_err(io_error)?;
    let range = with_database_deadline(&mut conn, handshake_deadline, |conn| {
        storage::reserve_event_sequences(conn, &owner)
    })?;
    let mut sequence = *range.start();
    let mut writer = EventWriter::start(
        settings.database_url.clone(),
        run.clone(),
        lock.clone(),
        artifacts.paths.outbox.clone(),
    )?;
    let terminate = Arc::new(AtomicBool::new(false));
    let resize = Arc::new(AtomicBool::new(false));
    let _signals = SignalGuards::new(terminate.clone(), resize.clone())?;
    let mut clients = Vec::<Client>::new();
    let mut next_client = 1u64;
    let mut creator_connection = None;
    let mut started = false;
    let mut exit_sent = false;
    let mut shutdown_client = None;
    let mut failure = None;
    let mut native_exit = None;
    let loop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<ShutdownClock> {
            loop {
                if terminate.load(Ordering::Acquire) {
                    return Ok(ShutdownClock::new(Instant::now()));
                }
                if !started {
                    check_deadline(Instant::now(), handshake_deadline)?;
                    ensure(
                        creator.matches()?,
                        "creator disconnected before native start",
                    )?;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true).map_err(io_error)?;
                        if clients.len() < 32 {
                            clients.push(Client {
                                id: next_client,
                                stream,
                                decoder: ControlDecoder::new(),
                                output: VecDeque::new(),
                                closed: false,
                            });
                            next_client = next_client
                                .checked_add(1)
                                .ok_or_else(|| error("control connection sequence exhausted"))?;
                        }
                    }
                    Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                    Err(cause) => return Err(io_error(cause)),
                }
                for client in &mut clients {
                    let requests = match client.poll() {
                        Ok(requests) => requests,
                        Err(_) => {
                            client.closed = true;
                            Vec::new()
                        }
                    };
                    if client.closed && creator_connection == Some(client.id) && !started {
                        return Err(error("creator control disconnected before start"));
                    }
                    for frame in requests {
                        let mut accepted_startup = false;
                        let result = (|| -> Result<Value> {
                            let kind = frame
                                .get("kind")
                                .and_then(Value::as_str)
                                .ok_or_else(|| error("control kind required"))?;
                            let identity: ControlIdentity = serde_json::from_value(
                                frame
                                    .get("identity")
                                    .cloned()
                                    .ok_or_else(|| error("control identity required"))?,
                            )
                            .map_err(|_| error("malformed control identity"))?;
                            if kind == "prepare" {
                                ensure(
                                    !started && creator_connection.is_none(),
                                    "preparation is single-use",
                                )?;
                                identity.validate(&original)?;
                                let remaining = frame
                                    .get("remaining_ms")
                                    .and_then(Value::as_u64)
                                    .ok_or_else(|| {
                                    error("remaining handshake budget required")
                                })?;
                                ensure(
                                    remaining > 0 && remaining <= 10000,
                                    "invalid handshake budget",
                                )?;
                                handshake_deadline = handshake_deadline
                                    .min(Instant::now() + Duration::from_millis(remaining));
                                creator_connection = Some(client.id);
                                accepted_startup = true;
                                artifacts.remove_descriptor()?;
                                return serde_json::to_value(Prepared {
                                    identity: ControlIdentity::from_run(&run),
                                    runner: process.clone(),
                                })
                                .map_err(io_error);
                            }
                            identity.validate(&run)?;
                            match kind {
                                "start" => {
                                    ensure(
                                        !started && creator_connection == Some(client.id),
                                        "start requires prepared creator connection",
                                    )?;
                                    let pane =
                                        frame.get("pane_id").and_then(Value::as_str).ok_or_else(
                                            || error("start requires concrete pane placement"),
                                        )?;
                                    accepted_startup = true;
                                    with_database_deadline(
                                        &mut conn,
                                        handshake_deadline,
                                        |conn| {
                                            lock.validate(conn, &owner)?;
                                            let current = storage::get_run(conn, run.run_id)?;
                                            ensure(
                                                current.status == RunStatus::Prepared,
                                                "start requires published prepared state",
                                            )?;
                                            let member = broker::get_member(
                                                conn,
                                                run.member_id,
                                                run.fleet_id,
                                            )?
                                            .ok_or_else(|| {
                                                error("prepared member became inactive")
                                            })?;
                                            ensure(
                                                member
                                                    .placement
                                                    .and_then(|placement| placement.mux_pane_id)
                                                    .as_deref()
                                                    == Some(pane),
                                                "start placement changed",
                                            )
                                        },
                                    )?;
                                    let child = pty.spawn(&descriptor.spawn)?;
                                    with_database_deadline(
                                        &mut conn,
                                        handshake_deadline,
                                        |conn| {
                                            lock.validate(conn, &owner)?;
                                            let changed=conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3,last_health_at=?3 WHERE run_id=?4 AND owner_epoch=?5 AND status='prepared'",params![child.pid,child.start_identity,crate::time::format_utc(crate::time::now_utc()),run.run_id,owner.owner_epoch]).map_err(db_err)?;
                                            ensure(
                                                changed == 1,
                                                "native spawn storage lost ownership",
                                            )
                                        },
                                    )?;
                                    run = storage::get_run(&conn, run.run_id)?;
                                    started = true;
                                    if run.coverage == Coverage::Unknown {
                                        writer.submit(lifecycle_event(
                                            &run,
                                            sequence,
                                            RuntimeEventKind::CoverageUnknown,
                                            json!({"kind":"coverage_unknown"}),
                                            transport.snapshot().screen_seq,
                                        )?)?;
                                        sequence += 1;
                                    }
                                    serde_json::to_value(Started {
                                        identity: ControlIdentity::from_run(&run),
                                        runner: process.clone(),
                                        child,
                                    })
                                    .map_err(io_error)
                                }
                                "health" => {
                                    with_database_deadline(
                                        &mut conn,
                                        Instant::now() + Duration::from_millis(10),
                                        |conn| lock.validate(conn, &owner),
                                    )?;
                                    serde_json::to_value(HealthSnapshot {
                                        identity: ControlIdentity::from_run(&run),
                                        runner: process.clone(),
                                        child: run
                                            .child_pid
                                            .zip(run.child_start_identity.clone())
                                            .map(|(pid, start_identity)| ProcessIdentity {
                                                pid,
                                                start_identity,
                                            }),
                                        output_seq: transport.snapshot().screen_seq,
                                    })
                                    .map_err(io_error)
                                }
                                "shutdown" => {
                                    shutdown_client = Some(client.id);
                                    Ok(Value::Null)
                                }
                                _ => Err(error("unknown runtime control operation")),
                            }
                        })();
                        if shutdown_client == Some(client.id) {
                            return Ok(ShutdownClock::new(Instant::now()));
                        }
                        let startup_failure = if accepted_startup {
                            result
                                .as_ref()
                                .err()
                                .map(|cause| cause.message().into_owned())
                        } else {
                            None
                        };
                        if client.reply(result).is_err() {
                            client.closed = true;
                        }
                        if let Some(cause) = startup_failure {
                            return Err(error(format!(
                                "native startup failed; owned compensation required: {cause}"
                            )));
                        }
                        if client.closed {
                            if creator_connection == Some(client.id) && !started {
                                return Err(error("creator control disconnected before start"));
                            }
                            break;
                        }
                    }
                }
                clients.retain(|client| !client.closed);
                if started {
                    let selected = run.database_identity_json["canonical_path"]
                        .as_str()
                        .expect("validated database path");
                    let meta = std::fs::metadata(selected).map_err(io_error)?;
                    ensure(
                        Some(meta.dev()) == run.database_identity_json["device"].as_u64()
                            && Some(meta.ino()) == run.database_identity_json["inode"].as_u64(),
                        "selected database was replaced; input fenced",
                    )?;
                    if resize.swap(false, Ordering::AcqRel) {
                        transport.resize(&mut pty, terminal_size(&terminal)?)?;
                    }
                    let progress =
                        transport.tick(&mut input, &mut pty, &mut output, &mut OrderedObserver)?;
                    if progress.outer_eof {
                        return Ok(ShutdownClock::new(Instant::now()));
                    }
                    if let Some(exit) = pty.try_wait()? {
                        native_exit = Some(exit);
                        writer.submit(lifecycle_event(
                            &run,
                            sequence,
                            RuntimeEventKind::RunnerExited,
                            exit_cause(exit),
                            transport.snapshot().screen_seq,
                        )?)?;
                        sequence += 1;
                        exit_sent = true;
                        return Ok(ShutdownClock::new(Instant::now()));
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        },
    ));
    let (loop_result, unwind) = match loop_result {
        Ok(result) => (result, None),
        Err(payload) => (Err(error("runner loop unwound")), Some(payload)),
    };
    let clock = match loop_result {
        Ok(clock) => clock,
        Err(cause) => {
            failure = Some(cause.message().into_owned());
            ShutdownClock::new(Instant::now())
        }
    };
    let clock = pty.begin_shutdown(clock);
    let mut live = LiveShutdown {
        pty: &mut pty,
        transport: &mut transport,
        input: &mut input,
        output: &mut output,
        raw: &mut raw,
        writer: &mut writer,
        clock,
        run: &run,
        sequence: &mut sequence,
        exit_sent: &mut exit_sent,
    };
    let mut outcome = shutdown(&mut live, clock, native_exit);
    if let Some(cause) = failure {
        outcome.diagnostics.push(cause);
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    if let Some(cause) = writer.diagnostic() {
        outcome.diagnostics.push(cause);
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    let persisted = with_database_deadline(&mut conn, clock.storage_deadline(), |conn| {
        lock.validate(conn, &owner)?;
        conn.execute("UPDATE member_runs SET cleanup_state=?1,cleanup_diagnostics_json=?2,last_health_at=?3,status=CASE WHEN child_pid IS NULL THEN 'failed' ELSE status END,ended_at=CASE WHEN child_pid IS NULL THEN ?3 ELSE ended_at END WHERE run_id=?4 AND owner_epoch=?5",params![outcome.cleanup_state.as_str(),serde_json::to_string(&outcome.diagnostics).map_err(io_error)?,crate::time::format_utc(crate::time::now_utc()),run.run_id,owner.owner_epoch]).map_err(db_err)?;
        Ok(())
    });
    if let Err(cause) = persisted {
        outcome
            .diagnostics
            .push(format!("cleanup evidence pending: {cause}"));
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    if let Some(id) = shutdown_client
        && let Some(client) = clients.iter_mut().find(|client| client.id == id)
    {
        if client
            .reply(serde_json::to_value(&outcome).map_err(io_error))
            .is_err()
        {
            client.closed = true;
        }
        while !client.output.is_empty() && Instant::now() < clock.storage_deadline() {
            if client.closed || client.flush().is_err() {
                client.closed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    for diagnostic in &outcome.diagnostics {
        eprintln!("member runtime: {diagnostic}");
    }
    if let Some(payload) = unwind {
        std::panic::resume_unwind(payload);
    }
    if !started {
        return Ok(1);
    }
    runner_exit_status(
        outcome.native_exit,
        outcome.cleanup_state == CleanupState::Complete,
    )
}
