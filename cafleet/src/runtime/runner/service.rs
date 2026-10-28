use super::codec::AdmittedControlFrame;
pub(super) use super::control::request;
use super::input::{InertInputHooks, InputCoordinator, InputServices, RequestTokenSource};
use super::input_clock::{RealInputDeadlineClock, receive_input_expiration};
use super::input_protocol::{ServiceControlRequest, decode_service_request, encode_input_reply};
use super::transport::TransportProgress;
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
struct SystemRequestTokens;
impl RequestTokenSource for SystemRequestTokens {
    fn fill(&mut self, bytes: &mut [u8; 32]) -> Result<()> {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(bytes))
            .map_err(io_error)
    }
}

struct PendingInputClient {
    id: u64,
    deadline: Instant,
}

impl Client {
    fn poll(&mut self) -> Result<Vec<AdmittedControlFrame>> {
        let mut requests = Vec::new();
        let mut bytes = [0; 4096];
        match self.stream.read(&mut bytes) {
            Ok(0) => {
                self.decoder.finish()?;
                self.closed = true;
            }
            Ok(count) => {
                let observed_at = Instant::now();
                requests = self.decoder.feed_admitted(&bytes[..count], observed_at)?;
            }
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
        self.queue_reply(bytes)
    }
    fn reply_input(
        &mut self,
        reply: std::result::Result<&InputControlReply, &CafleetError>,
    ) -> Result<()> {
        self.queue_reply(encode_input_reply(reply)?)
    }
    fn queue_reply(&mut self, bytes: Vec<u8>) -> Result<()> {
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
struct LiveShutdown<'a> {
    pty: &'a mut PtySession,
    transport: &'a mut Transport,
    input: &'a mut NonblockingFile,
    output: &'a mut NonblockingFile,
    raw: &'a mut RawTerminalGuard,
    writer: &'a mut EventWriter,
    clock: ShutdownClock,
    controller: &'a mut ObservationController,
    observation_clock: &'a dyn ObservationClock,
    coordinator: Option<&'a mut InputCoordinator>,
}
fn record_observation_failure(
    controller: &mut ObservationController,
    progress: &TransportProgress,
    clock: &dyn ObservationClock,
) -> Result<()> {
    ensure(
        progress.observation_error.is_some() == progress.observation_failure.is_some(),
        "transport observation failure requires typed provenance",
    )?;
    if let Some(failure) = progress.observation_failure {
        controller.observer_failed(failure, clock)?;
    }
    Ok(())
}
impl LiveShutdown<'_> {
    #[cfg(test)]
    fn await_native_exit_before_parsing(&mut self, deadline: Instant) -> Result<()> {
        if !self
            .transport
            .order_shutdown_parsing_after_native_exit(deadline)
        {
            return Ok(());
        }
        while Instant::now() < deadline {
            self.transport.prefetch_child_bytes(self.pty)?;
            if self.pty.try_wait()?.is_some() {
                self.send_exit()?;
                if !self.controller.has_pending() {
                    return Ok(());
                }
            }
            std::thread::sleep(
                Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        Err(CafleetError::DeadlineExceeded)
    }
    fn send_exit(&mut self) -> Result<()> {
        if let Some(exit) = self.pty.try_wait()? {
            self.controller.child_exited(
                exit,
                &self.transport.snapshot(),
                self.writer,
                self.observation_clock,
            )?;
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
        #[cfg(test)]
        self.controller
            .begin_capture_shutdown(self.clock.storage_deadline());
    }
    fn term_verified_descendants(&mut self) -> Result<()> {
        self.pty.signal_verified(nix::sys::signal::Signal::SIGTERM)
    }
    fn drain_and_reap_until(&mut self, deadline: Instant) -> Result<Option<ChildExit>> {
        #[cfg(test)]
        self.transport
            .set_observation_phase(ObservationPhase::ShutdownDrain);
        #[cfg(test)]
        self.await_native_exit_before_parsing(deadline)?;
        while Instant::now() < deadline {
            let mut tick = |transport: &mut Transport| {
                transport.tick(
                    self.input,
                    self.pty,
                    self.output,
                    &mut self.controller.observer_for(
                        self.writer,
                        self.observation_clock,
                        ObservationPhase::ShutdownDrain,
                    ),
                )
            };
            let progress = match self.coordinator.as_mut() {
                Some(coordinator) => coordinator
                    .progress_with_manual(self.transport, tick)
                    .map_err(CafleetError::from)?,
                None => tick(self.transport)?,
            };
            record_observation_failure(self.controller, &progress, self.observation_clock)?;
            self.controller.poll_for(
                &self.transport.snapshot(),
                self.writer,
                self.observation_clock,
                ObservationPhase::ShutdownDrain,
            )?;
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
        while self.controller.has_pending() && Instant::now() < deadline {
            self.controller.poll_for(
                &self.transport.snapshot(),
                self.writer,
                self.observation_clock,
                ObservationPhase::ShutdownDrain,
            )?;
            std::thread::sleep(Duration::from_millis(2));
        }
        let (pending, gap) = self.writer.finish(deadline);
        Ok((
            pending
                .checked_add(self.controller.pending_transition_count())
                .ok_or_else(|| error("pending event count overflow"))?,
            gap.checked_add(self.controller.unsaved_gap_count())
                .ok_or_else(|| error("unsaved observation count overflow"))?,
        ))
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

struct RunnerDependencies {
    catalog: crate::coding_agent::ProfileCatalog,
    clock: Arc<dyn ObservationClock>,
    writer_hooks: Option<Arc<dyn WriterHooks>>,
    #[cfg(test)]
    capture: Option<NativeCapturePort>,
    #[cfg(test)]
    observation_hooks: Option<Arc<dyn ObservationHooks>>,
}
#[cfg(test)]
pub(crate) struct RunnerTestServices {
    pub catalog: crate::coding_agent::ProfileCatalog,
    pub clock: Arc<dyn ObservationClock>,
    pub writer_hooks: Arc<dyn WriterHooks>,
    pub capture: Option<NativeCapturePort>,
    pub observation_hooks: Option<Arc<dyn ObservationHooks>>,
}
#[cfg(test)]
pub(crate) fn run_with_test_services(
    settings: &Settings,
    descriptor: &Path,
    services: RunnerTestServices,
) -> Result<i32> {
    run_service(
        settings,
        descriptor,
        RunnerDependencies {
            catalog: services.catalog,
            clock: services.clock,
            writer_hooks: Some(services.writer_hooks),
            capture: services.capture,
            observation_hooks: services.observation_hooks,
        },
    )
}
pub(crate) fn run(settings: &Settings, descriptor_path: &Path) -> Result<i32> {
    run_service(
        settings,
        descriptor_path,
        RunnerDependencies {
            catalog: crate::coding_agent::ProfileCatalog::bundled()?,
            clock: Arc::new(RealObservationClock),
            writer_hooks: None,
            #[cfg(test)]
            capture: None,
            #[cfg(test)]
            observation_hooks: None,
        },
    )
}
fn run_service(
    settings: &Settings,
    descriptor_path: &Path,
    mut services: RunnerDependencies,
) -> Result<i32> {
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
    with_database_deadline(&mut conn, handshake_deadline, |conn| {
        lock.validate(conn, &owner)?;
        ensure(
            run.status == RunStatus::Starting
                && run.child_pid.is_none()
                && run.child_start_identity.is_none()
                && run.started_at.is_none()
                && run.ended_at.is_none()
                && run.recovery_origin_status.is_none()
                && run.event_seq_reserved_through == 0,
            "live startup requires fresh run",
        )?;
        let history:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM member_interactions WHERE run_id=?1) OR EXISTS(SELECT 1 FROM member_runtime_events WHERE run_id=?1)",[run.run_id],|row|row.get(0)).map_err(db_err)?;
        ensure(!history, "live startup refuses durable history")
    })?;
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
    run = storage::get_run(&conn, owner.run_id)?;
    let evidence_policy = match descriptor.spawn.evidence_policy["mode"].as_str() {
        Some("metadata") => storage::evidence::EvidencePolicy::metadata(),
        Some("redacted") => {
            let ids = services.catalog.rule_ids();
            let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
            storage::evidence::EvidencePolicy::redacted(
                &descriptor.spawn.evidence_policy["config"],
                &refs,
            )?
        }
        _ => return Err(error("invalid launch evidence policy")),
    };
    let mut controller =
        ObservationController::new(&run, &services.catalog, evidence_policy, range)?;
    #[cfg(test)]
    if let Some(hooks) = services.observation_hooks.take() {
        controller.set_observation_hooks(hooks.clone())?;
        transport.set_observation_hooks(hooks)?;
    }
    let mut writer = if let Some(hooks) = services.writer_hooks.take() {
        EventWriter::start_with_hooks(
            settings.database_url.clone(),
            run.clone(),
            lock.clone(),
            artifacts.paths.outbox.clone(),
            hooks,
        )?
    } else {
        EventWriter::start(
            settings.database_url.clone(),
            run.clone(),
            lock.clone(),
            artifacts.paths.outbox.clone(),
        )?
    };
    let terminate = Arc::new(AtomicBool::new(false));
    let resize = Arc::new(AtomicBool::new(false));
    let _signals = SignalGuards::new(terminate.clone(), resize.clone())?;
    let mut clients = Vec::<Client>::new();
    let mut next_client = 1u64;
    let mut creator_connection = None;
    let mut started = false;
    let mut coordinator = None::<InputCoordinator>;
    let mut pending_input_client = None::<PendingInputClient>;
    let mut shutdown_client = None;
    let mut failure = None;
    let mut native_exit = None;
    let loop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<ShutdownClock> {
            loop {
                let mut tick_progress = TransportProgress::default();
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
                        let frame = match decode_service_request(frame.body()) {
                            Ok(ServiceControlRequest::Input(request))
                                if matches!(
                                    request.operation,
                                    InputControlOperation::PermissionInspect(_)
                                ) =>
                            {
                                let interaction_id = match &request.operation {
                                    InputControlOperation::PermissionInspect(input) => {
                                        input.interaction_id
                                    }
                                    _ => unreachable!("matched inspection operation"),
                                };
                                let result = (|| -> Result<InputControlReply> {
                                    let deadline = receive_input_expiration(
                                        request.expires_at_mono_ns,
                                        frame.admitted_at(),
                                        None,
                                        &mut RealInputDeadlineClock,
                                    )?;
                                    request.identity.validate(&run)?;
                                    let coordinator = coordinator
                                        .as_mut()
                                        .ok_or_else(|| error("native input has not started"))?;
                                    coordinator
                                        .progress_with_manual(&mut transport, |transport| {
                                            transport.quiesce_for_input(
                                                &mut input,
                                                &mut pty,
                                                &mut output,
                                                &mut controller
                                                    .observer(&writer, services.clock.as_ref()),
                                                deadline,
                                            )
                                        })
                                        .map_err(CafleetError::from)?;
                                    coordinator
                                        .inspect_received(
                                            &mut conn,
                                            &run,
                                            *request,
                                            (&controller, &mut transport),
                                            deadline,
                                        )
                                        .map_err(CafleetError::from)
                                })()
                                .map_err(|source| {
                                    CafleetError::from(InputFailure {
                                        source,
                                        run_id: Some(run.run_id),
                                        interaction_id: Some(interaction_id),
                                        input_id: None,
                                        operation_key: None,
                                    })
                                });
                                let replied = match &result {
                                    Ok(reply) => client.reply_input(Ok(reply)),
                                    Err(cause) => client.reply_input(Err(cause)),
                                };
                                if replied.is_err() {
                                    client.closed = true;
                                    break;
                                }
                                continue;
                            }
                            Ok(ServiceControlRequest::Input(request)) => {
                                let response_context = match &request.operation {
                                    InputControlOperation::PermissionRespond(input) => {
                                        Some((input.interaction_id, input.operation_key.clone()))
                                    }
                                    _ => None,
                                };
                                let result = (|| -> Result<Option<InputControlReply>> {
                                    let deadline = receive_input_expiration(
                                        request.expires_at_mono_ns,
                                        frame.admitted_at(),
                                        None,
                                        &mut RealInputDeadlineClock,
                                    )?;
                                    let coordinator = coordinator
                                        .as_mut()
                                        .ok_or_else(|| error("native input has not started"))?;
                                    if response_context.is_some() {
                                        if let Some(reply) = coordinator
                                            .retained_permission_response(
                                                &mut conn, &run, &request, deadline,
                                            )
                                            .map_err(CafleetError::from)?
                                        {
                                            return Ok(Some(reply));
                                        }
                                        let admission = coordinator
                                            .progress_with_manual(&mut transport, |transport| {
                                                transport.quiesce_for_input(
                                                    &mut input,
                                                    &mut pty,
                                                    &mut output,
                                                    &mut controller
                                                        .observer(&writer, services.clock.as_ref()),
                                                    deadline,
                                                )
                                            })
                                            .map_err(CafleetError::from)?;
                                        if matches!(admission, InputDrain::Pending) {
                                            return Err(error(format!(
                                                "permission request is blocked: {}",
                                                transport
                                                    .drain_blocker()
                                                    .expect("a Pending drain names its reason")
                                            )));
                                        }
                                    }
                                    let reply = coordinator
                                        .handle_received(
                                            &mut conn,
                                            &run,
                                            *request,
                                            &controller,
                                            deadline,
                                        )
                                        .map_err(CafleetError::from)?;
                                    if reply.is_none() {
                                        ensure(
                                            pending_input_client.is_none(),
                                            "accepted pending input already has a client",
                                        )?;
                                        pending_input_client = Some(PendingInputClient {
                                            id: client.id,
                                            deadline,
                                        });
                                    }
                                    Ok(reply)
                                })()
                                .map_err(|source| match response_context {
                                    Some((interaction_id, operation_key)) => {
                                        CafleetError::from(InputFailure {
                                            source,
                                            run_id: Some(run.run_id),
                                            interaction_id: Some(interaction_id),
                                            input_id: None,
                                            operation_key: Some(operation_key),
                                        })
                                    }
                                    None => source,
                                });
                                let replied = match &result {
                                    Ok(Some(reply)) => client.reply_input(Ok(reply)),
                                    Err(cause) => client.reply_input(Err(cause)),
                                    Ok(None) => Ok(()),
                                };
                                if replied.is_err() {
                                    client.closed = true;
                                    break;
                                }
                                continue;
                            }
                            Ok(ServiceControlRequest::Legacy(frame)) => frame,
                            Err(cause) => {
                                if client.reply(Err(cause)).is_err() {
                                    client.closed = true;
                                    break;
                                }
                                continue;
                            }
                        };
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
                                    #[cfg(test)]
                                    if let Some(port) = &mut services.capture {
                                        let (prepared, placement) = with_database_deadline(
                                            &mut conn,
                                            handshake_deadline,
                                            |conn| {
                                                let prepared = storage::get_run(conn, run.run_id)?;
                                                let placement = broker::get_member(
                                                    conn,
                                                    run.member_id,
                                                    run.fleet_id,
                                                )?
                                                .and_then(|member| member.placement)
                                                .ok_or_else(|| {
                                                    error("capture prepared placement missing")
                                                })?;
                                                Ok((prepared, placement))
                                            },
                                        )?;
                                        port.bind_service_prepared(
                                            &prepared,
                                            &placement,
                                            &descriptor.spawn,
                                            services.clock.as_ref(),
                                        )?;
                                    }
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
                                    #[cfg(test)]
                                    if let Some(mut port) = services.capture.take() {
                                        let placement =
                                            broker::get_member(&conn, run.member_id, run.fleet_id)?
                                                .and_then(|member| member.placement)
                                                .ok_or_else(|| {
                                                    error("capture running placement missing")
                                                })?;
                                        port.finalize_source(&run, &placement, &child)?;
                                        controller.attach_capture(port)?;
                                    }
                                    controller.started(
                                        &run,
                                        &transport.snapshot(),
                                        &writer,
                                        services.clock.as_ref(),
                                    )?;
                                    coordinator = Some(InputCoordinator::new(
                                        &mut conn,
                                        &run,
                                        lock.clone(),
                                        InputServices {
                                            catalog: services.catalog.clone(),
                                            token_source: Box::new(SystemRequestTokens),
                                            hooks: Arc::new(InertInputHooks),
                                        },
                                        handshake_deadline,
                                    )?);
                                    serde_json::to_value(Started {
                                        identity: ControlIdentity::from_run(&run),
                                        runner: process.clone(),
                                        child,
                                    })
                                    .map_err(io_error)
                                }
                                "health" => serde_json::to_value(HealthSnapshot {
                                    identity: ControlIdentity::from_run(&run),
                                    runner: process.clone(),
                                    child: run.child_pid.zip(run.child_start_identity.clone()).map(
                                        |(pid, start_identity)| ProcessIdentity {
                                            pid,
                                            start_identity,
                                        },
                                    ),
                                    output_seq: transport.snapshot().screen_seq,
                                    observation: if started {
                                        Some(controller.health(&writer))
                                    } else {
                                        None
                                    },
                                })
                                .map_err(io_error),
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
                        controller
                            .observer_for(
                                &writer,
                                services.clock.as_ref(),
                                ObservationPhase::Resize,
                            )
                            .observe(&transport.snapshot())?;
                    }
                    let coordinator = coordinator
                        .as_mut()
                        .ok_or_else(|| error("running service requires input coordinator"))?;
                    let progress = coordinator
                        .progress_with_manual(&mut transport, |transport| {
                            transport.tick(
                                &mut input,
                                &mut pty,
                                &mut output,
                                &mut controller.observer(&writer, services.clock.as_ref()),
                            )
                        })
                        .map_err(CafleetError::from)?;
                    record_observation_failure(
                        &mut controller,
                        &progress,
                        services.clock.as_ref(),
                    )?;
                    controller.poll(&transport.snapshot(), &writer, services.clock.as_ref())?;
                    if progress.outer_eof {
                        return Ok(ShutdownClock::new(Instant::now()));
                    }
                    tick_progress = progress;
                    if let Some(exit) = pty.try_wait()? {
                        native_exit = Some(exit);
                        controller.child_exited(
                            exit,
                            &transport.snapshot(),
                            &writer,
                            services.clock.as_ref(),
                        )?;
                        return Ok(ShutdownClock::new(Instant::now()));
                    }
                    if let Some(pending) = &pending_input_client {
                        let deadline = pending.deadline;
                        let drained =
                            coordinator.progress_with_manual(&mut transport, |transport| {
                                transport.quiesce_for_input(
                                    &mut input,
                                    &mut pty,
                                    &mut output,
                                    &mut controller.observer(&writer, services.clock.as_ref()),
                                    deadline,
                                )
                            });
                        let result = match drained {
                            Err(failure) => Err(CafleetError::from(failure)),
                            Ok(InputDrain::Pending) => Ok(None),
                            Ok(InputDrain::Quiescent { .. }) => {
                                if terminate.load(Ordering::Acquire) {
                                    return Ok(ShutdownClock::new(Instant::now()));
                                }
                                if let Some(exit) = pty.try_wait()? {
                                    native_exit = Some(exit);
                                    controller.child_exited(
                                        exit,
                                        &transport.snapshot(),
                                        &writer,
                                        services.clock.as_ref(),
                                    )?;
                                    return Ok(ShutdownClock::new(Instant::now()));
                                }
                                if resize.load(Ordering::Acquire) {
                                    Ok(None)
                                } else {
                                    coordinator
                                        .advance(
                                            &mut conn,
                                            &run,
                                            &mut transport,
                                            &mut pty,
                                            &controller,
                                            deadline,
                                        )
                                        .map_err(CafleetError::from)
                                }
                            }
                        };
                        if !matches!(result, Ok(None)) {
                            coordinator.finish_pending_attempt();
                            let pending = pending_input_client
                                .take()
                                .expect("completed input retains its original client");
                            if let Some(client) = clients
                                .iter_mut()
                                .find(|client| client.id == pending.id && !client.closed)
                            {
                                let replied = match &result {
                                    Ok(Some(reply)) => client.reply_input(Ok(reply)),
                                    Err(cause) => client.reply_input(Err(cause)),
                                    Ok(None) => unreachable!("completed input has a reply"),
                                };
                                if replied.is_err() {
                                    client.closed = true;
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(tick_sleep(
                    &tick_progress,
                    controller.status().next_deadline,
                    Instant::now(),
                ));
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
            let clock = ShutdownClock::new(Instant::now());
            let mut message = cause.message().into_owned();
            if started {
                let degraded = with_database_deadline(
                    &mut conn,
                    clock.storage_deadline(),
                    |conn| {
                        lock.validate(conn, &owner)?;
                        conn.execute("UPDATE member_runs SET status='degraded',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='running' AND child_pid IS NOT NULL",params![crate::time::format_utc(crate::time::now_utc()),run.run_id,owner.owner_epoch]).map_err(db_err)?;
                        Ok(())
                    },
                );
                if let Err(cause) = degraded {
                    message.push_str(&format!("; degraded status: {}", cause.message()));
                }
            }
            failure = Some(message);
            clock
        }
    };
    if let Some(coordinator) = coordinator.as_mut() {
        coordinator.finish_pending_attempt();
        if let Err(cause) = coordinator.fence(clock.storage_deadline()) {
            let message = cause.message();
            if let Some(prior) = &mut failure {
                prior.push_str(&format!("; input fence: {message}"));
            } else {
                failure = Some(message.into_owned());
            }
        }
    }
    let clock = pty.begin_shutdown(clock);
    let mut live = LiveShutdown {
        pty: &mut pty,
        transport: &mut transport,
        input: &mut input,
        output: &mut output,
        raw: &mut raw,
        writer: &mut writer,
        clock,
        controller: &mut controller,
        observation_clock: services.clock.as_ref(),
        coordinator: coordinator.as_mut(),
    };
    let mut outcome = shutdown(&mut live, clock, native_exit);
    if let Some(cause) = failure {
        outcome.diagnostics.push(cause);
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    if let Some(cause) = controller.pending_error() {
        outcome.diagnostics.push(cause.message().into_owned());
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

const IDLE_TICK_SLEEP: Duration = Duration::from_millis(2);

fn tick_sleep(
    progress: &TransportProgress,
    next_deadline: Option<Instant>,
    now: Instant,
) -> Duration {
    if progress.human_bytes > 0 || progress.forwarded_bytes > 0 {
        return Duration::ZERO;
    }
    next_deadline.map_or(IDLE_TICK_SLEEP, |at| {
        IDLE_TICK_SLEEP.min(at.saturating_duration_since(now))
    })
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    use crate::runtime::runner::transport::TransportProgress;

    fn progress(human_bytes: usize, forwarded_bytes: usize) -> TransportProgress {
        TransportProgress {
            human_bytes,
            forwarded_bytes,
            ..TransportProgress::default()
        }
    }

    #[test]
    fn pacing_starts_the_next_tick_at_once_while_outer_input_moves() {
        let now = Instant::now();
        for (case, human_bytes, forwarded_bytes) in [
            ("outer bytes read", 4096, 0),
            ("positive master write", 0, 2696),
            ("both", 1, 1),
        ] {
            for next_deadline in [None, Some(now + Duration::from_secs(1))] {
                assert_eq!(
                    tick_sleep(&progress(human_bytes, forwarded_bytes), next_deadline, now),
                    Duration::ZERO,
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn pacing_keeps_the_idle_sleep_without_outer_input() {
        let now = Instant::now();
        let mut child_only = progress(0, 0);
        child_only.child_bytes = 512;
        child_only.display_bytes = 512;
        for idle in [progress(0, 0), child_only] {
            assert_eq!(tick_sleep(&idle, None, now), Duration::from_millis(2));
            assert_eq!(
                tick_sleep(&idle, Some(now + Duration::from_secs(1)), now),
                Duration::from_millis(2)
            );
            assert_eq!(
                tick_sleep(&idle, Some(now + Duration::from_micros(500)), now),
                Duration::from_micros(500),
                "an earlier observation deadline still shortens the idle sleep"
            );
            assert_eq!(
                tick_sleep(&idle, Some(now - Duration::from_millis(1)), now),
                Duration::ZERO
            );
        }
    }

    #[test]
    fn pacing_keeps_the_idle_sleep_when_a_blocked_master_accepted_no_queued_input() {
        let now = Instant::now();
        let blocked = progress(0, 0);
        assert_eq!(tick_sleep(&blocked, None, now), Duration::from_millis(2));
        assert_eq!(
            tick_sleep(&blocked, Some(now + Duration::from_micros(500)), now),
            Duration::from_micros(500)
        );
    }
}

#[cfg(test)]
mod input_admission_tests {
    use super::*;
    use crate::runtime::runner::codec::AdmittedControlFrame;
    use crate::runtime::runner::input_clock::receive_input_expiration;
    use crate::runtime::runner::{InputClockSample, InputDeadlineClock};
    use std::io::Write;
    use std::net::Shutdown;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }
    fn pair() -> (Client, UnixStream) {
        let (stream, peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        (
            Client {
                id: 1,
                stream,
                decoder: ControlDecoder::new(),
                output: VecDeque::new(),
                closed: false,
            },
            peer,
        )
    }
    fn send(peer: &mut UnixStream, mut bytes: &[u8], deadline: Instant) {
        while !bytes.is_empty() {
            assert!(Instant::now() < deadline, "bounded peer send");
            match peer.write(bytes) {
                Ok(0) => panic!("peer send progresses"),
                Ok(count) => bytes = &bytes[count..],
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    std::thread::yield_now()
                }
                Err(error) => panic!("peer send: {error}"),
            }
        }
    }
    fn after(previous: Instant, deadline: Instant) {
        while Instant::now() <= previous {
            assert!(
                Instant::now() < deadline,
                "fixture clock advances within its bound"
            );
            std::thread::yield_now();
        }
    }
    fn collect(client: &mut Client, count: usize, deadline: Instant) -> Vec<AdmittedControlFrame> {
        let mut frames = Vec::new();
        while frames.len() < count {
            assert!(Instant::now() < deadline, "bounded actual client polling");
            frames.extend(client.poll().unwrap());
            std::thread::yield_now();
        }
        assert_eq!(frames.len(), count);
        frames
    }
    fn first(
        client: &mut Client,
        peer: &mut UnixStream,
        byte: u8,
        deadline: Instant,
    ) -> (Instant, Instant) {
        client.decoder.finish().unwrap();
        send(peer, &[byte], deadline);
        let before = Instant::now();
        assert!(client.poll().unwrap().is_empty());
        let after = Instant::now();
        assert!(
            client.decoder.finish().is_err(),
            "this actual poll consumed the first header byte"
        );
        (before, after)
    }

    #[test]
    fn step5_wire_a1_actual_poll_proves_first_byte_before_later_completion() {
        let (mut client, mut peer) = pair();
        let deadline = Instant::now() + Duration::from_secs(1);
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        let bytes = frame(body);
        let (before, first_after) = first(&mut client, &mut peer, bytes[0], deadline);
        after(first_after, deadline);
        send(&mut peer, &bytes[1..], deadline);
        let frames = collect(&mut client, 1, deadline);
        assert_eq!(frames[0].body(), body);
        assert!(before <= frames[0].admitted_at() && frames[0].admitted_at() <= first_after);
        client.decoder.finish().unwrap();
    }
    #[test]
    fn step5_wire_a2_empty_actual_poll_does_not_supply_admission_time() {
        let (mut client, mut peer) = pair();
        let deadline = Instant::now() + Duration::from_secs(1);
        client.decoder.finish().unwrap();
        let empty_at = Instant::now();
        assert!(client.poll().unwrap().is_empty());
        client.decoder.finish().unwrap();
        after(empty_at, deadline);
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        let bytes = frame(body);
        let (before, end) = first(&mut client, &mut peer, bytes[0], deadline);
        after(end, deadline);
        send(&mut peer, &bytes[1..], deadline);
        let frames = collect(&mut client, 1, deadline);
        assert!(frames[0].admitted_at() > empty_at);
        assert!(before <= frames[0].admitted_at() && frames[0].admitted_at() <= end);
        assert_eq!(frames[0].body(), body);
        client.decoder.finish().unwrap();
    }
    struct Clock {
        sample: Option<std::result::Result<InputClockSample, CafleetError>>,
        calls: usize,
    }
    impl Clock {
        fn at(instant: Instant) -> Self {
            Self {
                sample: Some(Ok(InputClockSample {
                    before: instant,
                    monotonic_ns: 1_000_000_000,
                    after: instant,
                })),
                calls: 0,
            }
        }
    }
    impl InputDeadlineClock for Clock {
        fn sample(&mut self) -> Result<InputClockSample> {
            self.calls += 1;
            self.sample.take().expect("one original paired sample")
        }
    }
    #[test]
    fn step5_wire_a3_actual_frames_keep_first_time_through_coalescence_queue_and_conversion() {
        let (mut client, mut peer) = pair();
        let deadline = Instant::now() + Duration::from_secs(1);
        let first_body = br#"{"protocol_version":1,"kind":"health"}"#;
        let second_body = br#"{"protocol_version":1,"kind":"shutdown"}"#;
        let bytes = frame(first_body);
        let (before, first_after) = first(&mut client, &mut peer, bytes[0], deadline);
        after(first_after, deadline);
        let later_before = Instant::now();
        let mut rest = bytes[1..].to_vec();
        rest.extend(frame(second_body));
        send(&mut peer, &rest, deadline);
        let frames = collect(&mut client, 2, deadline);
        let later_after = Instant::now();
        assert_eq!(frames[0].body(), first_body);
        assert_eq!(frames[1].body(), second_body);
        let times = [frames[0].admitted_at(), frames[1].admitted_at()];
        assert!(before <= times[0] && times[0] <= first_after);
        assert!(later_before <= times[1] && times[1] <= later_after);
        let mut queue: VecDeque<_> = frames.into();
        after(later_after, deadline);
        for expected in times {
            let actual = queue.pop_front().unwrap();
            assert_eq!(actual.admitted_at(), expected);
            let now = Instant::now();
            let mut clock = Clock::at(now);
            let converted =
                receive_input_expiration(11_000_000_000, actual.admitted_at(), None, &mut clock)
                    .unwrap();
            assert_eq!(converted, actual.admitted_at() + Duration::from_secs(5));
            assert_eq!(clock.calls, 1);
        }
        assert!(queue.is_empty());
        client.decoder.finish().unwrap();
    }
    #[test]
    fn step5_wire_a4_actual_client_rejects_partial_eof_and_oversized_header() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let (mut good, mut peer) = pair();
        let body = br#"{"protocol_version":1,"kind":"health"}"#;
        send(&mut peer, &frame(body), deadline);
        assert_eq!(collect(&mut good, 1, deadline)[0].body(), body);
        good.decoder.finish().unwrap();
        let (mut partial, mut peer) = pair();
        first(&mut partial, &mut peer, 0, deadline);
        peer.shutdown(Shutdown::Write).unwrap();
        assert!(partial.poll().is_err());
        for size in [65537u32, u32::MAX] {
            let (mut client, mut peer) = pair();
            send(&mut peer, &size.to_be_bytes(), deadline);
            assert!(client.poll().is_err());
        }
    }
}
