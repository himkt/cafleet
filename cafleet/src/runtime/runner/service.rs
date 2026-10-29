use super::codec::AdmittedControlFrame;
pub(super) use super::control::request;
use super::input::{InertInputHooks, InputCoordinator, InputServices, RequestTokenSource};
use super::input_clock::{RealInputDeadlineClock, receive_input_expiration};
use super::input_protocol::{ServiceControlRequest, decode_service_request, encode_input_reply};
use super::transport::TransportProgress;
use super::writer::EventWriter;
use super::*;

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
        crate::runtime::system::random_bytes(bytes).map_err(io_error)
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
    pub(crate) catalog: crate::coding_agent::ProfileCatalog,
    pub(crate) clock: Arc<dyn ObservationClock>,
    pub(crate) writer_hooks: Arc<dyn WriterHooks>,
    pub(crate) capture: Option<NativeCapturePort>,
    pub(crate) observation_hooks: Option<Arc<dyn ObservationHooks>>,
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
        let history:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM member_interactions WHERE run_id=?1) OR EXISTS(SELECT 1 FROM member_runtime_events WHERE run_id=?1)",[run.run_id],|row|row.get(0))?;
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
                                            let changed=conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3,last_health_at=?3 WHERE run_id=?4 AND owner_epoch=?5 AND status='prepared'",params![child.pid,child.start_identity,crate::time::now_stamp(),run.run_id,owner.owner_epoch])?;
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
                        conn.execute("UPDATE member_runs SET status='degraded',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='running' AND child_pid IS NOT NULL",params![crate::time::now_stamp(),run.run_id,owner.owner_epoch])?;
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
        conn.execute("UPDATE member_runs SET cleanup_state=?1,cleanup_diagnostics_json=?2,last_health_at=?3,status=CASE WHEN child_pid IS NULL THEN 'failed' ELSE status END,ended_at=CASE WHEN child_pid IS NULL THEN ?3 ELSE ended_at END WHERE run_id=?4 AND owner_epoch=?5",params![outcome.cleanup_state.as_str(),serde_json::to_string(&outcome.diagnostics).map_err(io_error)?,crate::time::now_stamp(),run.run_id,owner.owner_epoch])?;
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
pub(crate) mod test_support {
    use std::fs::File;
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{Command, ExitStatus, Stdio};
    use std::result::Result;
    use std::time::{Duration, Instant};

    use nix::sys::signal::{Signal, kill};
    use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
    use nix::unistd::Pid;
    use serde_json::{Value, json};

    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::digest::sha256_hex;
    use crate::runtime::runner::test_support::*;
    use crate::runtime::runner::*;
    use crate::runtime::system::test_support::TestChild;
    pub(crate) const DESCRIPTOR_ENV: &str = "CAFLEET_REVIEW_RUNNER_DESCRIPTOR";
    pub(crate) const SERVICES_ENV: &str = "CAFLEET_REVIEW_TEST_SERVICES";
    pub(crate) fn wire_frame(value: &Value) -> Vec<u8> {
        let body = serde_json::to_vec(value).unwrap();
        assert!(body.len() <= 65536);
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    pub(crate) struct Wire(pub(crate) UnixStream);

    impl Wire {
        pub(crate) fn connect(path: &Path) -> std::io::Result<Self> {
            let stream = UnixStream::connect(path)?;
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            stream.set_write_timeout(Some(Duration::from_secs(1)))?;
            Ok(Self(stream))
        }

        pub(crate) fn exchange(&mut self, value: &Value) -> std::io::Result<Value> {
            self.0.write_all(&wire_frame(value))?;
            let mut header = [0; 4];
            self.0.read_exact(&mut header)?;
            let length = usize::try_from(u32::from_be_bytes(header)).unwrap();
            assert!(
                length > 0 && length <= 65536,
                "invalid reply length {length}"
            );
            let mut bytes = vec![0; length];
            self.0.read_exact(&mut bytes)?;
            let reply: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(reply["protocol_version"], 1);
            assert!(
                reply.get("result").is_some() ^ reply.get("error").is_some(),
                "{reply}"
            );
            Ok(reply)
        }
    }

    pub(crate) fn assert_rejected(reply: &std::io::Result<Value>) {
        match reply {
            Ok(value) => assert!(value.get("error").is_some(), "{value}"),
            Err(cause) => assert!(
                matches!(
                    cause.kind(),
                    ErrorKind::UnexpectedEof
                        | ErrorKind::ConnectionReset
                        | ErrorKind::BrokenPipe
                        | ErrorKind::NotConnected
                ),
                "request rejection requires a reply or closed connection: {cause}"
            ),
        }
    }

    pub(crate) struct ServiceFixture {
        pub(crate) fixture: Fixture,
        pub(crate) _artifacts: RunArtifacts,
        pub(crate) child: TestChild,
        pub(crate) process: Option<ProcessIdentity>,
        pub(crate) master: File,
        pub(crate) _slave: File,
        pub(crate) creator: Option<Wire>,
        pub(crate) prepared: Option<ControlIdentity>,
        pub(crate) output: Vec<u8>,
        pub(crate) stopped: bool,
        pub(crate) cleanup_deadline: Option<Instant>,
        pub(crate) startup_deadline: Instant,
    }

    impl ServiceFixture {
        pub(crate) fn assert_selected_config(&self, path: &Path) {
            let selected: Value = serde_json::from_slice(
                &std::fs::read(self.fixture._dir.path().join("config-selected.json"))
                    .expect("explicit configured service selection record"),
            )
            .unwrap();
            assert_eq!(selected["configuration"], json!(path));
            assert_eq!(
                selected["sha256"],
                sha256_hex(&std::fs::read(path).unwrap())
            );
        }
        pub(crate) fn connection_diagnostic(&mut self, stage: &str) -> String {
            let status = self.child.try_wait();
            let socket = std::fs::symlink_metadata(&self._artifacts.paths.control_socket);
            let stderr = self.child.stderr.as_mut().unwrap();
            let flags = nix::fcntl::OFlag::from_bits_retain(
                nix::fcntl::fcntl(&*stderr, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
            );
            nix::fcntl::fcntl(
                &*stderr,
                nix::fcntl::FcntlArg::F_SETFL(flags | nix::fcntl::OFlag::O_NONBLOCK),
            )
            .unwrap();
            let mut bytes = Vec::new();
            let read = stderr.take(64 * 1024).read_to_end(&mut bytes);
            let panics = input_service::panic_records(self.fixture._dir.path());
            format!(
                "stage={stage}; runner_pid={}; prepared={}; exit={status:?}; socket={socket:?}; stderr_read={read:?}; stderr={:?}; panic_records={panics:?}",
                self.child.id(),
                self.prepared.is_some(),
                String::from_utf8_lossy(&bytes),
            )
        }

        pub(crate) fn connect(&mut self, stage: &str) -> Wire {
            match Wire::connect(&self._artifacts.paths.control_socket) {
                Ok(wire) => wire,
                Err(cause) => panic!(
                    "owned runner connection failed: {cause}; {}; backtrace={}",
                    self.connection_diagnostic(stage),
                    std::backtrace::Backtrace::force_capture(),
                ),
            }
        }

        pub(crate) fn new(missing_executable: bool) -> Self {
            Self::configured(missing_executable, |_, _| {})
        }

        pub(crate) fn configured(
            missing_executable: bool,
            configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
        ) -> Self {
            Self::launch(missing_executable, configure, false, None)
        }

        pub(crate) fn configured_until(
            missing_executable: bool,
            until: Instant,
            configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
        ) -> Self {
            Self::launch(missing_executable, configure, false, Some(until))
        }

        pub(crate) fn startup_refusal(
            configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
        ) -> Self {
            Self::launch(false, configure, true, None)
        }

        pub(crate) fn launch(
            missing_executable: bool,
            configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
            refusal: bool,
            caller_deadline: Option<Instant>,
        ) -> Self {
            let startup_deadline = Instant::now() + Duration::from_secs(10);
            let until =
                caller_deadline.map_or(startup_deadline, |caller| caller.min(startup_deadline));
            assert!(Instant::now() < until, "original fixture startup deadline");
            let mut fixture = Fixture::new();
            fixture
                .conn
                .execute(
                    "UPDATE member_placements SET mux_pane_id=NULL WHERE member_id=?1",
                    [fixture.run.member_id],
                )
                .unwrap();
            let artifacts = fixture.artifacts();
            let mut spec = fixture.spec();
            spec.argv = if missing_executable {
                vec![
                    fixture
                        ._dir
                        .path()
                        .join("missing-native-executable")
                        .to_str()
                        .unwrap()
                        .into(),
                ]
            } else {
                vec!["/bin/sh".into(), "-c".into(), "printf 'review-native-ready\\n'; while IFS= read -r line; do case \"$line\" in finish) exit 0 ;; *) printf 'review-answer=%s\\n' \"$line\" ;; esac; done".into()]
            };
            configure(&mut fixture, &mut spec);
            artifacts
                .write_descriptor(&LaunchDescriptor::new(&fixture.run, spec).unwrap())
                .unwrap();
            let outer = nix::pty::openpty(
                Some(&nix::pty::Winsize {
                    ws_row: 24,
                    ws_col: 80,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                }),
                None,
            )
            .unwrap();
            let master = File::from(outer.master);
            let slave = File::from(outer.slave);
            for file in [&master, &slave] {
                nix::fcntl::fcntl(
                    file,
                    nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
                )
                .unwrap();
            }
            let flags = nix::fcntl::OFlag::from_bits_retain(
                nix::fcntl::fcntl(&master, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
            );
            nix::fcntl::fcntl(
                &master,
                nix::fcntl::FcntlArg::F_SETFL(flags | nix::fcntl::OFlag::O_NONBLOCK),
            )
            .unwrap();
            let capture_config = fixture._dir.path().join("capture-config.json");
            let capture = capture_config.exists();
            let collector_entry = fixture._dir.path().join("collector-review-entry");
            let capture_review = if collector_entry.exists() {
                assert!(
                    capture,
                    "collector review requires its private capture configuration"
                );
                Some(
                    match std::fs::read_to_string(collector_entry).unwrap().as_str() {
                        "resize" => runner_collector_resize_child_path(),
                        "export" => runner_collector_export_child_path(),
                        other => panic!("unknown owned collector entry: {other}"),
                    },
                )
            } else {
                None
            };
            let services_config = fixture._dir.path().join("observation-services.json");
            let services_selected = services_config.exists() && !capture;
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.env_remove(SERVICES_ENV);
            if services_selected {
                command.env(SERVICES_ENV, &services_config);
            }
            command.args([
                "--exact",
                if let Some(entry) = capture_review {
                    entry
                } else if capture {
                    native_capture_service_child_path()
                } else {
                    super::tests::control_service::runner_review_service_child_path()
                },
                "--nocapture",
                "--test-threads=1",
            ]);
            if capture {
                command
                    .arg("--ignored")
                    .env("CAFLEET_NATIVE_CAPTURE_CONFIG", &capture_config);
            }
            assert!(Instant::now() < until, "original fixture startup deadline");
            let child = TestChild::new(
                command
                    .env(DESCRIPTOR_ENV, &artifacts.paths.descriptor)
                    .env(
                        "CAFLEET_DATABASE_URL",
                        format!("sqlite:///{}", fixture.conn.path().unwrap()),
                    )
                    .stdin(Stdio::from(slave.try_clone().unwrap()))
                    .stdout(Stdio::from(slave.try_clone().unwrap()))
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
            let mut owned = Self {
                fixture,
                _artifacts: artifacts,
                child,
                process: None,
                master,
                _slave: slave,
                creator: None,
                prepared: None,
                output: Vec::new(),
                stopped: false,
                cleanup_deadline: caller_deadline,
                startup_deadline: until,
            };
            if refusal {
                assert!(!owned.wait().success(), "nonfresh startup is refused");
                if services_selected {
                    owned.assert_selected_config(&services_config);
                }
                return owned;
            }
            owned.process = Some(
                ProcessIdentity::read(i64::from(owned.child.id()))
                    .unwrap()
                    .unwrap(),
            );
            loop {
                if let Some(status) = owned.child.try_wait().unwrap() {
                    panic!(
                        "owned runner stopped before initial connection: {status}; {}",
                        owned.connection_diagnostic("startup"),
                    );
                }
                assert!(
                    Instant::now() < until,
                    "owned runner did not accept its initial connection: {}",
                    owned.connection_diagnostic("startup timeout"),
                );
                match Wire::connect(&owned._artifacts.paths.control_socket) {
                    Ok(wire) => {
                        owned.creator = Some(wire);
                        break;
                    }
                    Err(cause)
                        if matches!(
                            cause.kind(),
                            ErrorKind::NotFound | ErrorKind::ConnectionRefused
                        ) => {}
                    Err(cause) => panic!(
                        "owned runner initial connection failed: {cause}; {}",
                        owned.connection_diagnostic("startup"),
                    ),
                }
                std::thread::sleep(
                    Duration::from_millis(5).min(until.saturating_duration_since(Instant::now())),
                );
            }
            if services_selected {
                owned.assert_selected_config(&services_config);
            }
            owned
        }

        pub(crate) fn request(&self, kind: &str, identity: &ControlIdentity) -> Value {
            json!({"protocol_version":1,"kind":kind,"identity":identity})
        }

        pub(crate) fn prepare(&mut self) {
            let mut creator = self
                .creator
                .take()
                .expect("connected initial creator stream");
            let mut request =
                self.request("prepare", &ControlIdentity::from_run(&self.fixture.run));
            request["remaining_ms"] = json!(9000);
            let reply = creator
                .exchange(&request)
                .expect("versioned length-prefixed prepare reply");
            assert!(reply.get("error").is_none(), "{reply}");
            let prepared: Prepared = serde_json::from_value(reply["result"].clone()).unwrap();
            assert_eq!(prepared.identity.owner_epoch, 2);
            assert_eq!(
                storage::get_run(&self.fixture.conn, self.fixture.run.run_id)
                    .unwrap()
                    .child_pid,
                None
            );
            self.prepared = Some(prepared.identity);
            self.creator = Some(creator);
        }

        pub(crate) fn publish(&mut self) {
            self.fixture
                .conn
                .execute(
                    "UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1",
                    [self.fixture.run.member_id],
                )
                .unwrap();
            self.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                    [self.fixture.run.run_id],
                )
                .unwrap();
        }

        pub(crate) fn start(&mut self) -> std::io::Result<Value> {
            let mut request = self.request("start", self.prepared.as_ref().unwrap());
            request["pane_id"] = json!("%9");
            self.creator.as_mut().unwrap().exchange(&request)
        }

        pub(crate) fn start_running(&mut self) -> Started {
            self.prepare();
            self.publish();
            let reply = self.start().unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            let started: Started = serde_json::from_value(reply["result"].clone()).unwrap();
            self.read_until("review-native-ready");
            started
        }

        pub(crate) fn health(&mut self) -> HealthSnapshot {
            let mut client = self.connect("health");
            let reply = client
                .exchange(&self.request("health", self.prepared.as_ref().unwrap()))
                .unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            serde_json::from_value(reply["result"].clone()).unwrap()
        }

        pub(crate) fn read_until(&mut self, needle: &str) {
            let until = if needle == "review-native-ready" {
                self.startup_deadline
            } else {
                Instant::now() + Duration::from_secs(2)
            };
            loop {
                let mut bytes = [0; 4096];
                match self.master.read(&mut bytes) {
                    Ok(count) => self.output.extend_from_slice(&bytes[..count]),
                    Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                    Err(cause) => panic!("owned outer PTY read: {cause}"),
                }
                if String::from_utf8_lossy(&self.output).contains(needle) {
                    return;
                }
                assert!(
                    Instant::now() < until,
                    "missing native output {needle}: {}; outer_bytes={:?}",
                    self.connection_diagnostic("native output timeout"),
                    self.output,
                );
                assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "runner stopped during manual I/O: {}",
                    self.connection_diagnostic("native output runner exit"),
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        pub(crate) fn assert_manual_io(&mut self) {
            self.master.write_all(b"literal '$x' `bytes`\r").unwrap();
            self.read_until("review-answer=literal '$x' `bytes`");
        }

        pub(crate) fn wait(&mut self) -> ExitStatus {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return status;
                }
                assert!(
                    Instant::now() < until,
                    "owned runner exceeded bounded cleanup"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        pub(crate) fn finish(&mut self) {
            self.master.write_all(b"finish\r").unwrap();
            assert_eq!(self.wait().code(), Some(0));
            let run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
            assert_eq!(run.status, RunStatus::Exited);
            assert_eq!(run.exit_code, Some(0));
            assert_eq!(
                run.cleanup_state,
                CleanupState::Complete,
                "{:?}",
                run.cleanup_diagnostics_json
            );
        }

        pub(crate) fn stop_for_disconnect(&mut self) {
            assert!(self.process.as_ref().unwrap().matches().unwrap());
            let pid = Pid::from_raw(i32::try_from(self.child.id()).unwrap());
            kill(pid, Signal::SIGSTOP).unwrap();
            self.stopped = true;
            let until = Instant::now() + Duration::from_secs(1);
            loop {
                match waitpid(pid, Some(WaitPidFlag::WUNTRACED | WaitPidFlag::WNOHANG)).unwrap() {
                    WaitStatus::Stopped(observed, Signal::SIGSTOP) => {
                        assert_eq!(observed, pid);
                        return;
                    }
                    WaitStatus::StillAlive => {}
                    other => panic!("unexpected owned runner state: {other:?}"),
                }
                assert!(Instant::now() < until, "owned runner did not stop");
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        pub(crate) fn resume(&mut self) {
            assert!(self.process.as_ref().unwrap().matches().unwrap());
            kill(
                Pid::from_raw(i32::try_from(self.child.id()).unwrap()),
                Signal::SIGCONT,
            )
            .unwrap();
            self.stopped = false;
        }

        pub(crate) fn cleanup_recorded_native(&self) {
            let run = match storage::get_run(&self.fixture.conn, self.fixture.run.run_id) {
                Ok(run) => run,
                Err(cause) => {
                    eprintln!("owned fixture cleanup cannot inspect its run: {cause}");
                    return;
                }
            };
            let Some((pid, start_identity)) = run.child_pid.zip(run.child_start_identity) else {
                return;
            };
            let owned = ProcessIdentity {
                pid,
                start_identity,
            };
            match owned.matches() {
                Ok(true) => {
                    let raw = match i32::try_from(pid) {
                        Ok(raw) => raw,
                        Err(cause) => {
                            eprintln!("owned native fixture PID conversion failed: {cause}");
                            return;
                        }
                    };
                    if let Err(cause) = kill(Pid::from_raw(raw), Signal::SIGKILL) {
                        eprintln!("owned native fixture termination failed: {cause}");
                    }
                }
                Ok(false) => {}
                Err(cause) => eprintln!("owned native fixture identity is unconfirmed: {cause}"),
            }
        }
    }

    impl Drop for ServiceFixture {
        fn drop(&mut self) {
            self.creator.take();
            let raw_pid = match i32::try_from(self.child.id()) {
                Ok(pid) => pid,
                Err(cause) => {
                    eprintln!("owned runner PID conversion failed: {cause}");
                    self.cleanup_recorded_native();
                    return;
                }
            };
            let pid = Pid::from_raw(raw_pid);
            if self.stopped {
                if let Err(cause) = kill(pid, Signal::SIGCONT) {
                    eprintln!("owned runner resume failed during cleanup: {cause}");
                }
                self.stopped = false;
            }
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.cleanup_recorded_native();
                    return;
                }
                Ok(None) => {}
                Err(cause) => {
                    eprintln!("owned runner collection failed: {cause}");
                    self.cleanup_recorded_native();
                    return;
                }
            }
            if let Err(cause) = kill(pid, Signal::SIGTERM) {
                eprintln!("owned runner termination failed: {cause}");
            }
            let until = self
                .cleanup_deadline
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(5));
            let graceful = until - Duration::from_secs(1);
            while Instant::now() < graceful {
                match self.child.try_wait() {
                    Ok(Some(_)) => {
                        self.cleanup_recorded_native();
                        return;
                    }
                    Ok(None) => {}
                    Err(cause) => {
                        eprintln!("owned runner collection failed: {cause}");
                        self.cleanup_recorded_native();
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            self.cleanup_recorded_native();
            self.child.terminate_until(until);
        }
    }

    pub(crate) mod input_service {
        use super::observation_services::private_write;
        use super::*;
        use crate::error::CafleetError;
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        pub(crate) fn native(path: &Path) {
            install_panic_record(path.parent().unwrap(), "native");
            native::run(path);
        }
        pub(crate) const NATIVE_ENV: &str = "CAFLEET_REVIEW_SERVICE_INPUT_NATIVE";
        pub(crate) const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
        pub(crate) const PANIC_RECORD_LIMIT: usize = 16 * 1024;
        pub(crate) const PANIC_DIRECTORY: &str = "service-input-panics";

        pub(crate) fn install_panic_record(directory: &Path, role: &'static str) {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            assert!(matches!(role, "native" | "runner"));
            let directory = directory.join(PANIC_DIRECTORY);
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700,
            );
            let pid = std::process::id();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.join(format!("service-input-panic-{role}-{pid}.txt")))
                .unwrap();
            let record = std::sync::Mutex::new(Some(file));
            let prior = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                match record.lock() {
                    Ok(mut slot) => {
                        if let Some(mut file) = slot.take() {
                            let message = format!(
                                "role={role}; pid={pid}; location={:?}; panic={info}\n",
                                info.location(),
                            );
                            let bytes =
                                &message.as_bytes()[..message.len().min(PANIC_RECORD_LIMIT)];
                            if let Err(cause) = file.write_all(bytes).and_then(|()| file.flush()) {
                                eprintln!("owned {role} panic record failed: {cause}");
                            }
                        }
                    }
                    Err(cause) => eprintln!("owned {role} panic record lock failed: {cause}"),
                }
                prior(info);
            }));
        }

        pub(crate) fn panic_records(directory: &Path) -> std::io::Result<Vec<(String, String)>> {
            use std::os::unix::fs::PermissionsExt;

            let mut records = Vec::new();
            for entry in std::fs::read_dir(directory.join(PANIC_DIRECTORY))? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("service-input-panic-") {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(entry.path())?;
                if records.len() >= 2
                    || !metadata.is_file()
                    || metadata.permissions().mode() & 0o777 != 0o600
                {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        "owned panic records require two private regular files at most",
                    ));
                }
                let mut bytes = Vec::new();
                File::open(entry.path())?
                    .take(u64::try_from(PANIC_RECORD_LIMIT + 1).unwrap())
                    .read_to_end(&mut bytes)?;
                if bytes.len() > PANIC_RECORD_LIMIT {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidData,
                        "owned panic record exceeds its original limit",
                    ));
                }
                records.push((name, String::from_utf8_lossy(&bytes).into_owned()));
            }
            records.sort();
            Ok(records)
        }

        pub(crate) struct Clock;
        impl InputDeadlineClock for Clock {
            fn sample(&mut self) -> Result<InputClockSample, CafleetError> {
                let before = Instant::now();
                let value = rustix::time::clock_gettime_dynamic(
                    rustix::time::DynamicClockId::Known(rustix::time::ClockId::Monotonic),
                )
                .map_err(io_error)?;
                let after = Instant::now();
                let seconds = u64::try_from(value.tv_sec).unwrap();
                let nanos = u64::try_from(value.tv_nsec).unwrap();
                assert!(nanos < 1_000_000_000);
                Ok(InputClockSample {
                    before,
                    monotonic_ns: seconds
                        .checked_mul(1_000_000_000)
                        .unwrap()
                        .checked_add(nanos)
                        .unwrap(),
                    after,
                })
            }
        }
        pub(crate) fn expiration(until: Instant) -> u64 {
            InputExpiration::capture(until, &mut Clock)
                .unwrap()
                .monotonic_ns()
        }
        pub(crate) fn pause(until: Instant) {
            assert!(Instant::now() < until, "original owned service deadline");
            std::thread::sleep(
                Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
            );
        }

        pub(crate) fn configure(fixture: &mut Fixture, spec: &mut SpawnSpec, until: Instant) {
            use std::os::unix::fs::DirBuilderExt;

            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(fixture._dir.path().join(PANIC_DIRECTORY))
                .unwrap();
            let mut catalog =
                crate::coding_agent::profiles::catalog::test_support::synthetic_definitions();
            let mut executable = catalog["fixtures"][0].clone();
            executable["id"] = json!("synthetic-service-submission");
            catalog["fixtures"].as_array_mut().unwrap().push(executable);
            let rule = &mut catalog["profiles"][0]["rules"][0];
            rule["id"] = json!("synthetic-service-composer");
            rule["observation"] = json!("composer_empty");
            rule["region"]["start_pattern"] = json!("^Synthetic state$");
            rule["region"]["end_pattern"] = json!("^State end$");
            rule["matchers"] = json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
            rule["extract"] = json!({"operation":null,"choices":[],"native_request_id":null});
            rule["verified_noninteraction"] = json!(true);
            catalog["profiles"][0]["capabilities"] = json!({"detection":null,"annotations":null,"answers":[],"composer":{
                "rule_ids":["synthetic-service-composer"],"fixture_ids":[catalog["fixtures"][0]["id"]],
                "submission":{
                    "prompt":{"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]},
                    "exec":{"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]}
                }
            }});
            fixture.conn.execute("UPDATE member_runs SET native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',profile_match='exact',coverage='unknown' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let config = fixture._dir.path().join("service-input-native.json");
            private_write(
                &config,
                &serde_json::to_vec(&json!({
                    "database_url":format!("sqlite:///{}",fixture.conn.path().unwrap()),
                    "run_id":fixture.run.run_id,"expires_at_mono_ns":expiration(until)
                }))
                .unwrap(),
            );
            spec.argv = vec![
        std::env::current_exe().unwrap().to_str().unwrap().into(),
        "--exact".into(),
        crate::runtime::runner::service::tests::control_service::runner_review_service_child_path()
            .into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ];
            spec.environment
                .push((NATIVE_ENV.into(), config.to_str().unwrap().into()));
            private_write(
                &fixture._dir.path().join("observation-services.json"),
                &serde_json::to_vec(&json!({"catalog":catalog})).unwrap(),
            );
        }

        pub(crate) struct Peer {
            pub(crate) stream: UnixStream,
            pub(crate) decoder: ControlDecoder,
        }
        impl Peer {
            pub(crate) fn connect(fixture: &ServiceFixture) -> Self {
                let stream = UnixStream::connect(&fixture._artifacts.paths.control_socket).unwrap();
                stream.set_nonblocking(true).unwrap();
                Self {
                    stream,
                    decoder: ControlDecoder::new(),
                }
            }
            pub(crate) fn send(&mut self, bytes: &[u8], until: Instant) {
                let mut offset = 0;
                while offset < bytes.len() {
                    assert!(Instant::now() < until);
                    match self.stream.write(&bytes[offset..]) {
                        Ok(0) => panic!("owned control writer disconnected"),
                        Ok(count) => offset += count,
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                ErrorKind::WouldBlock | ErrorKind::Interrupted
                            ) =>
                        {
                            pause(until)
                        }
                        Err(cause) => panic!("owned control write: {cause}"),
                    }
                }
            }
            pub(crate) fn reply(&mut self, until: Instant) -> Value {
                loop {
                    assert!(Instant::now() < until, "original reply observation bound");
                    let mut bytes = [0; 4096];
                    match self.stream.read(&mut bytes) {
                        Ok(0) => {
                            self.decoder.finish().unwrap();
                            panic!("owned control disconnected before a complete reply");
                        }
                        Ok(count) => {
                            let mut frames = self.decoder.feed(&bytes[..count]).unwrap();
                            if !frames.is_empty() {
                                assert_eq!(frames.len(), 1);
                                self.decoder.finish().unwrap();
                                return frames.remove(0);
                            }
                        }
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                ErrorKind::WouldBlock | ErrorKind::Interrupted
                            ) =>
                        {
                            pause(until)
                        }
                        Err(cause) => panic!("owned control read: {cause}"),
                    }
                }
            }
        }
        pub(crate) fn request(
            fixture: &ServiceFixture,
            key: &str,
            literal: &str,
            kind: InputKind,
            message: Option<i64>,
            until: Instant,
        ) -> Value {
            json!({"protocol_version":1,"kind":if kind==InputKind::Exec {"submit_exec"} else {"submit_prompt"},
        "identity":fixture.prepared.as_ref().unwrap(),"expires_at_mono_ns":expiration(until),
        "operation_key":key,"input_kind":kind,"message_id":message,"literal":literal})
        }
        pub(crate) fn submission(reply: Value) -> SubmissionResult {
            assert!(
                reply.get("error").is_none(),
                "actual service submission: {reply}"
            );
            assert_eq!(reply["result"]["kind"], "submission", "{reply}");
            let result: SubmissionResult =
                serde_json::from_value(reply["result"]["submission"].clone()).unwrap();
            assert_eq!(result.status, SubmissionStatus::Accepted);
            assert_eq!(result.stage, SubmissionStage::Submitted);
            result
        }
        pub(crate) fn exchange(fixture: &ServiceFixture, value: &Value, until: Instant) -> Value {
            let mut peer = Peer::connect(fixture);
            peer.send(&encode_control_frame(value).unwrap(), until);
            peer.reply(until)
        }
        pub(crate) fn manual(fixture: &mut ServiceFixture, line: &[u8], until: Instant) {
            let mut offset = 0;
            while offset < line.len() {
                assert!(Instant::now() < until);
                match fixture.master.write(&line[offset..]) {
                    Ok(0) => panic!("owned outer PTY closed"),
                    Ok(count) => offset += count,
                    Err(cause)
                        if matches!(
                            cause.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
                    {
                        pause(until)
                    }
                    Err(cause) => panic!("owned manual write: {cause}"),
                }
            }
        }
        pub(crate) fn input_service_records(fixture: &ServiceFixture) -> Vec<Value> {
            let bytes = std::fs::read(
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("service-input-lines.jsonl"),
            )
            .unwrap();
            assert!(bytes.len() <= 65536);
            bytes
                .split_inclusive(|byte| *byte == b'\n')
                .filter(|line| line.ends_with(b"\n"))
                .map(|line| serde_json::from_slice(line).unwrap())
                .collect()
        }
        pub(crate) fn failure_diagnostic(fixture: &mut ServiceFixture, stage: &str) -> String {
            let flags = nix::fcntl::OFlag::from_bits_retain(
                nix::fcntl::fcntl(&fixture.master, nix::fcntl::FcntlArg::F_GETFL).unwrap(),
            );
            assert!(flags.contains(nix::fcntl::OFlag::O_NONBLOCK));
            let mut bytes = Vec::new();
            let outer_read = (&mut fixture.master)
                .take(64 * 1024)
                .read_to_end(&mut bytes);
            let prior =
                String::from_utf8_lossy(&fixture.output[..fixture.output.len().min(64 * 1024)])
                    .into_owned();
            let connection = fixture.connection_diagnostic(stage);
            format!(
                "{connection}; outer_read={outer_read:?}; outer_bytes={:?}; previous_outer_bytes={prior:?}",
                String::from_utf8_lossy(&bytes),
            )
        }
        pub(crate) fn wait_input_service_record(
            fixture: &mut ServiceFixture,
            line: &str,
            until: Instant,
        ) {
            while !input_service_records(fixture)
                .iter()
                .any(|record| record["line"] == line)
            {
                assert!(
                    fixture.child.try_wait().unwrap().is_none(),
                    "runner remains live while awaiting its native journal: {}",
                    failure_diagnostic(fixture, "native journal wait"),
                );
                pause(until);
            }
        }
        pub(crate) fn input_service_composer(
            fixture: &mut ServiceFixture,
            until: Instant,
        ) -> HealthSnapshot {
            loop {
                assert!(Instant::now() < until);
                let health = fixture.health();
                if health.observation.as_ref().is_some_and(|observation| {
                    observation.eligibility == InputEligibility::ComposerEmpty
                        && !observation.degraded
                }) {
                    return health;
                }
                pause(until);
            }
        }
        pub(crate) fn input_service_barrier(fixture: &mut ServiceFixture, until: Instant) {
            let before = fixture.health().output_seq;
            manual(fixture, b"barrier\r", until);
            wait_input_service_record(fixture, "barrier", until);
            while input_service_composer(fixture, until).output_seq <= before {
                pause(until);
            }
        }
        pub(crate) fn with_input_service(body: impl FnOnce(&mut ServiceFixture, Instant)) {
            let until = Instant::now() + Duration::from_secs(10);
            let mut fixture = ServiceFixture::configured_until(false, until, |fixture, spec| {
                configure(fixture, spec, until)
            });
            fixture.cleanup_deadline = Some(until);
            let primary = catch_unwind(AssertUnwindSafe(|| {
                fixture.start_running();
                input_service_composer(&mut fixture, until);
                body(&mut fixture, until);
            }));
            let cleanup = catch_unwind(AssertUnwindSafe(|| {
                if fixture.stopped {
                    fixture.resume();
                }
                manual(&mut fixture, b"finish\r", until);
                let status = fixture.child.poll_until(until).unwrap();
                if status.is_none() {
                    fixture.child.terminate_until(until);
                }
                assert_eq!(
                    status
                        .expect("owned service reaped under original deadline")
                        .code(),
                    Some(0),
                    "owned service cleanup retains its original exit expectation: {}",
                    failure_diagnostic(&mut fixture, "service cleanup"),
                );
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert_eq!(run.status, RunStatus::Exited);
                assert_eq!(
                    run.cleanup_state,
                    CleanupState::Complete,
                    "{:?}",
                    run.cleanup_diagnostics_json
                );
            }));
            if let Err(primary) = primary {
                if let Err(cleanup) = &cleanup {
                    eprintln!("service input cleanup also failed: {cleanup:?}");
                }
                resume_unwind(primary);
            }
            if let Err(cleanup) = cleanup {
                resume_unwind(cleanup);
            }
        }

        pub(crate) mod native {
            use super::*;
            use nix::fcntl::{FcntlArg, OFlag, fcntl};
            use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};
            use std::fs::OpenOptions;
            use std::os::unix::fs::OpenOptionsExt;

            pub(crate) fn receipt_inputs(
                conn: &rusqlite::Connection,
                run_id: i64,
                expires: u64,
            ) -> rusqlite::Result<Vec<Value>> {
                loop {
                    assert!(
                        Clock.sample().unwrap().monotonic_ns < expires,
                        "original owned native lifetime"
                    );
                    let result = (|| {
                        let mut statement = conn.prepare("SELECT input_id,state FROM member_input_operations WHERE run_id=?1 ORDER BY input_id")?;
                        let rows = statement.query_map([run_id], |row| {
                Ok(json!({"input_id":row.get::<_,i64>(0)?,"state":row.get::<_,String>(1)?}))
            })?;
                        rows.collect::<rusqlite::Result<Vec<_>>>()
                    })();
                    match result {
                        Err(cause)
                            if matches!(
                                cause.sqlite_error_code(),
                                Some(
                                    rusqlite::ErrorCode::DatabaseBusy
                                        | rusqlite::ErrorCode::DatabaseLocked
                                )
                            ) =>
                        {
                            let now = Clock.sample().unwrap().monotonic_ns;
                            assert!(now < expires, "original owned native lifetime");
                            std::thread::sleep(
                                Duration::from_millis(2).min(Duration::from_nanos(expires - now)),
                            );
                        }
                        result => return result,
                    }
                }
            }

            pub(crate) fn run(path: &Path) {
                let config: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                let directory = path.parent().unwrap();
                let expires = config["expires_at_mono_ns"].as_u64().unwrap();
                let run_id = config["run_id"].as_i64().unwrap();
                let conn = crate::db::connect(config["database_url"].as_str().unwrap()).unwrap();
                conn.busy_timeout(Duration::ZERO).unwrap();
                let stdin = std::io::stdin();
                let mut terminal = tcgetattr(&stdin).unwrap();
                terminal.local_flags.remove(LocalFlags::ECHO);
                tcsetattr(&stdin, SetArg::TCSANOW, &terminal).unwrap();
                let flags = OFlag::from_bits_retain(fcntl(&stdin, FcntlArg::F_GETFL).unwrap());
                fcntl(&stdin, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
                let mut journal = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(directory.join("service-input-lines.jsonl"))
                    .unwrap();
                private_write(
                    &directory.join("service-input-native-identity.json"),
                    &serde_json::to_vec(&ProcessIdentity::current().unwrap()).unwrap(),
                );
                let mut output = std::io::stdout().lock();
                output.write_all(b"review-native-ready\r\n").unwrap();
                output.write_all(SCREEN).unwrap();
                output.flush().unwrap();
                let mut input = stdin.lock();
                let mut pending = Vec::new();
                loop {
                    assert!(
                        Clock.sample().unwrap().monotonic_ns < expires,
                        "original owned native lifetime"
                    );
                    let mut bytes = [0; 4096];
                    match input.read(&mut bytes) {
                        Ok(0) => return,
                        Ok(count) => pending.extend_from_slice(&bytes[..count]),
                        Err(cause)
                            if matches!(
                                cause.kind(),
                                ErrorKind::WouldBlock | ErrorKind::Interrupted
                            ) => {}
                        Err(cause) => panic!("owned synthetic native input: {cause}"),
                    }
                    assert!(pending.len() <= 65536);
                    while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                        let mut line = pending.drain(..=end).collect::<Vec<_>>();
                        assert_eq!(line.pop(), Some(b'\n'));
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        let line = String::from_utf8(line).unwrap();
                        if line == "finish" {
                            return;
                        }
                        let inputs = receipt_inputs(&conn, run_id, expires).unwrap();
                        journal
                            .write_all(
                                &serde_json::to_vec(&json!({"line":line,"inputs":inputs})).unwrap(),
                            )
                            .unwrap();
                        journal.write_all(b"\n").unwrap();
                        journal.sync_all().unwrap();
                        output
                            .write_all(format!("review-answer={line}\r\n").as_bytes())
                            .unwrap();
                        output.write_all(SCREEN).unwrap();
                        output.flush().unwrap();
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    pub(crate) mod observation_services {
        use super::*;
        use crate::coding_agent::ProfileCatalog;
        use crate::error::CafleetError;
        use crate::runtime::runner::observation::test_support::synthetic;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::sync::Arc;

        pub(crate) struct Clock;
        impl ObservationClock for Clock {
            fn sample(&self) -> ObservationMoment {
                ObservationMoment {
                    monotonic: Instant::now(),
                    observed_at: crate::time::now_stamp(),
                }
            }
            fn kind(&self) -> ObservationClockKind {
                ObservationClockKind::Real
            }
        }
        pub(crate) struct Hooks {
            pub(crate) directory: std::path::PathBuf,
        }
        impl WriterHooks for Hooks {
            fn before(
                &self,
                boundary: WriterBoundary,
                deadline: Instant,
            ) -> Result<(), CafleetError> {
                if matches!(boundary, WriterBoundary::Sync)
                    && self.directory.join("fail-sync").exists()
                {
                    return Err(error("owned synthetic sync failure"));
                }
                if matches!(boundary, WriterBoundary::Persist) {
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(self.directory.join("persist-entered"))
                    {
                        Ok(_) => {}
                        Err(cause) if cause.kind() == ErrorKind::AlreadyExists => {}
                        Err(cause) => return Err(io_error(cause)),
                    }
                    while self.directory.join("block-persist").exists() {
                        if Instant::now() >= deadline {
                            return Err(CafleetError::DeadlineExceeded);
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
                Ok(())
            }
        }
        pub(crate) fn run_configured(
            settings: &Settings,
            descriptor: &Path,
            config: &Path,
        ) -> Result<i32, CafleetError> {
            let value: Value = serde_json::from_slice(&std::fs::read(config).map_err(io_error)?)
                .map_err(io_error)?;
            let catalog = ProfileCatalog::from_definitions(&value["catalog"])?;
            private_write(&config.parent().unwrap().join("config-selected.json"),&serde_json::to_vec(&json!({"configuration":config,"sha256":sha256_hex(&std::fs::read(config).map_err(io_error)?)})).map_err(io_error)?);
            let services = RunnerTestServices {
                catalog,
                clock: Arc::new(Clock),
                writer_hooks: Arc::new(Hooks {
                    directory: config.parent().unwrap().to_path_buf(),
                }),
                capture: None,
                observation_hooks: super::review_failures::hooks(&value, config.parent().unwrap())?,
            };
            run_with_test_services(settings, descriptor, services)
        }
        pub(crate) fn private_write(path: &Path, bytes: &[u8]) {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }
        pub(crate) fn wait_until(mut ready: impl FnMut() -> bool) {
            let until = Instant::now() + Duration::from_secs(3);
            while !ready() {
                assert!(Instant::now() < until, "bounded actual service condition");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        pub(crate) fn count(fixture: &ServiceFixture, kind: &str) -> i64 {
            fixture
                .fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind=?2",
                    rusqlite::params![fixture.fixture.run.run_id, kind],
                    |row| row.get(0),
                )
                .unwrap()
        }
        pub(crate) fn health(fixture: &mut ServiceFixture) -> Value {
            let mut client = fixture.connect("observation status");
            let reply = client
                .exchange(&fixture.request("health", fixture.prepared.as_ref().unwrap()))
                .unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            reply["result"].clone()
        }

        pub(crate) fn capture(fixture: &mut Fixture, spec: &mut SpawnSpec) {
            let arm_path = fixture._dir.path().join("capture-arm.json");
            let arm_parent = arm_path.parent().expect("owned capture arm parent");
            std::fs::set_permissions(arm_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(
                std::fs::symlink_metadata(arm_parent)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            synthetic(fixture, spec);
            let bundled = crate::coding_agent::ProfileCatalog::bundled()
                .unwrap()
                .capabilities(
                    crate::coding_agent::CodingAgent::Codex,
                    Some("synthetic-0.0.1"),
                );
            fixture
        .conn
        .execute(
            "UPDATE member_runs SET profile_version=?1,profile_match=?2,coverage=?3 WHERE run_id=?4",
            rusqlite::params![
                bundled.profile_version,
                bundled.profile_match.map(|value| value.as_str()),
                bundled.detection.as_str(),
                fixture.run.run_id
            ],
        )
        .unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let evidence = fixture._dir.path().join("synthetic-source.json");
            private_write(
                &evidence,
                b"{\"kind\":\"synthetic\",\"version\":\"synthetic-0.0.1\"}",
            );
            let hash = |path: &Path| sha256_hex(&std::fs::read(path).unwrap());
            let current = std::env::current_exe().unwrap();
            let executable = Path::new(&spec.argv[0]);
            let producer = json!({"schema_version":1,"kind":"synthetic","source_manifest_ref":evidence,"source_manifest_sha256":hash(&evidence),"libtest_executable_path":current,"libtest_executable_sha256":hash(&current),"parser_version":"0.18.1","collector_version":"1","catalog_origin":"bundled_unknown","native_executable":{"path":executable,"sha256":hash(executable),"version":"synthetic-0.0.1","version_evidence_ref":evidence},"launch_evidence_ref":evidence,"environment_evidence_ref":evidence});
            let producer_path = fixture._dir.path().join("capture-producer.json");
            private_write(&producer_path, &serde_json::to_vec(&producer).unwrap());
            let policy = fixture._dir.path().join("capture-policy.json");
            private_write(&policy, b"{\"schema_version\":1,\"masks\":[]}");
            let output = fixture._dir.path().join("capture-output");
            std::fs::create_dir(&output).unwrap();
            std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
            let paths =
                RuntimePaths::for_run(&fixture.run.database_identity_json, fixture.run.run_id)
                    .unwrap();
            let config = NativeCaptureConfig {
                schema_version: 1,
                database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
                descriptor_path: paths.descriptor,
                expected_run_id: fixture.run.run_id,
                expected_launch_nonce: fixture.run.launch_nonce.clone(),
                arm_path,
                output_directory: output,
                producer_manifest_path: producer_path,
                capture_policy_path: policy,
            };
            private_write(
                &fixture._dir.path().join("capture-config.json"),
                &serde_json::to_vec(&config).unwrap(),
            );
        }
    }

    pub(crate) mod review_failures {
        use super::observation_services::{count, health, private_write, wait_until};
        use super::*;
        use crate::error::CafleetError;
        use crate::runtime::runner::observation::test_support::synthetic;
        use std::sync::{Arc, OnceLock};

        pub(crate) const CANARY: &str = "private-observation-fault-canary-194";

        pub(crate) struct Hooks {
            pub(crate) site: ObservationSite,
            pub(crate) operation: ObservationOperation,
            pub(crate) directory: std::path::PathBuf,
            pub(crate) buffered_tail: Option<BufferedTail>,
        }

        pub(crate) struct BufferedTail {
            pub(crate) database_url: String,
            pub(crate) run_id: i64,
            pub(crate) deadline: OnceLock<Instant>,
        }

        impl BufferedTail {
            pub(crate) fn synchronize(&self, directory: &Path) -> Result<(), CafleetError> {
                let deadline = *self
                    .deadline
                    .get()
                    .expect("original service child deadline precedes the actual Parser operation");
                check_deadline(Instant::now(), deadline)?;
                ensure(
                    directory.join("buffered-tail-written").exists(),
                    "successful real native write precedes parse-after-exit Parser operation",
                )?;
                private_write(
                    &directory.join("buffered-tail-parser-entered"),
                    b"actual nonempty PTY read under original child deadline",
                );
                let mut conn = crate::db::connect(&self.database_url)?;
                let run = with_database_deadline(&mut conn, deadline, |conn| {
                    storage::get_run(conn, self.run_id)
                })?;
                let child = ProcessIdentity {
                    pid: run
                        .child_pid
                        .ok_or_else(|| error("original native child PID required"))?,
                    start_identity: run
                        .child_start_identity
                        .ok_or_else(|| error("original native child generation required"))?,
                };
                let current = ProcessIdentity::read(child.pid)?;
                ensure(
                    current.is_none(),
                    "PtySession reaps the actual native child before selected Parser entry",
                )?;
                private_write(
                    &directory.join("buffered-tail-native-terminated"),
                    &serde_json::to_vec(&child).map_err(io_error)?,
                );
                private_write(
                    &directory.join("buffered-tail-parser-native.json"),
                    &serde_json::to_vec(&json!({"pinned":child,"current":current}))
                        .map_err(io_error)?,
                );
                loop {
                    let count = with_database_deadline(&mut conn, deadline, |conn| {
                        conn.query_row("SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='runner_exited'", [self.run_id], |row| row.get::<_, i64>(0)).map_err(io_error)
                    })?;
                    if count == 1 {
                        break;
                    }
                    ensure(count == 0, "one original terminal event required")?;
                    check_deadline(Instant::now(), deadline)?;
                    std::thread::sleep(
                        Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                private_write(
            &directory.join("buffered-tail-post-fence-parser"),
            b"actual nonempty Parser operation after durable runner_exited under original deadline",
        );
                Ok(())
            }
        }
        impl ObservationHooks for Hooks {
            fn order_shutdown_parsing_after_native_exit(&self, deadline: Instant) -> bool {
                let Some(tail) = &self.buffered_tail else {
                    return false;
                };
                check_deadline(Instant::now(), deadline)
                    .expect("live original service child deadline");
                tail.deadline
                    .set(deadline)
                    .expect("one original shutdown child deadline");
                private_write(&self.directory.join("buffered-tail-order-selected.json"), &serde_json::to_vec(&json!({"run_id":tail.run_id,"remaining_ns":deadline.saturating_duration_since(Instant::now()).as_nanos()})).unwrap());
                true
            }
            fn before(
                &self,
                site: ObservationSite,
                operation: ObservationOperation,
            ) -> Result<(), CafleetError> {
                if site != self.site || operation != self.operation {
                    return Ok(());
                }
                let arm = self.directory.join("observation-fault-armed");
                if !arm.try_exists().map_err(io_error)? {
                    return Ok(());
                }
                if let Some(tail) = &self.buffered_tail
                    && let Err(cause) = tail.synchronize(&self.directory)
                {
                    private_write(
                        &self.directory.join("buffered-tail-gate-error"),
                        cause.message().as_bytes(),
                    );
                    panic!("owned buffered-tail prerequisite failed: {cause}");
                }
                std::fs::remove_file(arm).map_err(io_error)?;
                private_write(
                    &self.directory.join("observation-fault-fired"),
                    format!("{site:?}/{operation:?}").as_bytes(),
                );
                Err(error(CANARY))
            }
        }
        pub(crate) fn hooks(
            value: &Value,
            directory: &Path,
        ) -> Result<Option<Arc<dyn ObservationHooks>>, CafleetError> {
            let Some(selected) = value.get("observation_fault") else {
                return Ok(None);
            };
            let site = match selected["site"].as_str() {
                Some("child_output") => ObservationSite::ChildOutput,
                Some("quiet_timer") => ObservationSite::QuietTimer,
                Some("resize") => ObservationSite::Resize,
                Some("shutdown_drain") => ObservationSite::ShutdownDrain,
                _ => return Err(error("owned observation fault site required")),
            };
            let operation = match selected["operation"].as_str() {
                Some("parser") => ObservationOperation::Parser,
                Some("detector") => ObservationOperation::Detector,
                _ => return Err(error("owned observation fault operation required")),
            };
            Ok(Some(Arc::new(Hooks {
                site,
                operation,
                directory: directory.to_path_buf(),
                buffered_tail: value.get("buffered_tail").map(|tail| BufferedTail {
                    database_url: tail["database_url"]
                        .as_str()
                        .expect("owned buffered-tail database")
                        .into(),
                    run_id: tail["run_id"].as_i64().expect("owned buffered-tail run"),
                    deadline: OnceLock::new(),
                }),
            })))
        }
        pub(crate) fn configure(
            fixture: &mut Fixture,
            spec: &mut SpawnSpec,
            site: &str,
            operation: &str,
        ) {
            synthetic(fixture, spec);
            let path = fixture._dir.path().join("observation-services.json");
            let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            config["observation_fault"] = json!({"site":site,"operation":operation});
            std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        }
        pub(crate) fn arm(fixture: &ServiceFixture) {
            private_write(
                &fixture.fixture._dir.path().join("observation-fault-armed"),
                b"owned one-shot fault",
            );
        }
        pub(crate) fn assert_fired(fixture: &ServiceFixture, expected: &str) {
            let path = fixture.fixture._dir.path().join("observation-fault-fired");
            wait_until(|| path.exists());
            assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
            assert!(
                !fixture
                    .fixture
                    ._dir
                    .path()
                    .join("observation-fault-armed")
                    .exists()
            );
        }
        pub(crate) fn assert_private_cause_stays_local(fixture: &ServiceFixture) {
            let mut statement = fixture.fixture.conn.prepare(
        "SELECT payload_json FROM member_runtime_events UNION ALL SELECT evidence_json FROM member_interactions UNION ALL SELECT text FROM messages UNION ALL SELECT cleanup_diagnostics_json FROM member_runs WHERE cleanup_diagnostics_json IS NOT NULL",
    ).unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap();
            for row in rows {
                assert!(!row.unwrap().contains(CANARY));
            }
        }
        pub(crate) fn assert_durable_gap(fixture: &ServiceFixture) {
            wait_until(|| count(fixture, "runner_degraded") >= 2);
            let payloads: Vec<String> = fixture.fixture.conn.prepare(
        "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='runner_degraded' ORDER BY event_seq",
    ).unwrap().query_map([fixture.fixture.run.run_id], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            let causes: Vec<Value> = payloads
                .iter()
                .map(|text| serde_json::from_str::<Value>(text).unwrap()["cause"].clone())
                .collect();
            assert!(causes.iter().any(|cause| cause["kind"] == "observer_error"
                && cause["gap_count"].as_u64().unwrap() > 0));
            assert!(
                causes
                    .iter()
                    .any(|cause| cause["kind"] == "storage_recovered")
            );
        }
        pub(crate) fn assert_live_after_fault(
            fixture: &mut ServiceFixture,
            expected_diagnostic: &str,
        ) {
            assert!(fixture.child.try_wait().unwrap().is_none());
            let status = health(fixture);
            assert_eq!(status["observation"]["eligibility"]["state"], "hold");
            assert!(status["observation"]["gap_count"].as_u64().unwrap() > 0);
            if let Some(diagnostic) = status["observation"]["diagnostic"].as_str() {
                assert_eq!(diagnostic, expected_diagnostic);
            }
            fixture.assert_manual_io();
            assert_durable_gap(fixture);
        }
        pub(crate) fn install_trap(spec: &SpawnSpec, command: &str) {
            let script = Path::new(&spec.argv[0]);
            let body = std::fs::read_to_string(script).unwrap();
            let (prefix, suffix) = body
                .split_once("while IFS=")
                .expect("owned synthetic input loop");
            std::fs::write(
                script,
                format!("{prefix}trap '{command}' TERM\nwhile IFS={suffix}"),
            )
            .unwrap();
        }
        pub(crate) fn shutdown_reply(fixture: &mut ServiceFixture) -> ShutdownOutcome {
            let mut wire = fixture.connect("observation fault shutdown");
            let reply = wire
                .exchange(&fixture.request("shutdown", fixture.prepared.as_ref().unwrap()))
                .unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            serde_json::from_value(reply["result"].clone()).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::Shutdown;
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::path::Path;
    use std::time::{Duration, Instant};

    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use serde_json::{Value, json};

    use super::test_support::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::runtime::runner::*;

    pub(super) mod control_service {
        use super::*;
        pub(crate) fn runner_review_service_child_path() -> &'static str {
            test_path!(runner_review_service_child)
        }

        #[test]
        fn runner_review_service_child() {
            if let Some(config) =
                std::env::var_os(super::super::test_support::input_service::NATIVE_ENV)
            {
                super::super::test_support::input_service::native(Path::new(&config));
                std::process::exit(0);
            }
            let Some(descriptor) = std::env::var_os(DESCRIPTOR_ENV) else {
                return;
            };
            if let Some(configured) = std::env::var_os(SERVICES_ENV) {
                let directory = Path::new(&configured).parent().unwrap();
                if directory.join("service-input-native.json").exists() {
                    super::super::test_support::input_service::install_panic_record(
                        directory, "runner",
                    );
                }
            }
            let database_url = std::env::var("CAFLEET_DATABASE_URL").unwrap();
            let settings = Settings::for_database(
                Path::new(crate::db::database_path(&database_url).unwrap()),
                Some("tmux"),
            );
            let outcome = match std::env::var_os(SERVICES_ENV) {
                Some(configured) => {
                    let configured = Path::new(&configured);
                    assert!(
                        configured.is_absolute(),
                        "explicit service configuration is absolute"
                    );
                    observation_services::run_configured(
                        &settings,
                        Path::new(&descriptor),
                        configured,
                    )
                }
                None => run(&settings, Path::new(&descriptor)),
            };
            match outcome {
                Ok(status) => std::process::exit(status),
                Err(cause) => {
                    eprintln!("owned review runner failed: {cause}");
                    std::process::exit(1);
                }
            }
        }
        #[test]
        fn runner_review_control_stale_start_preserves_running_child_health_and_manual_io() {
            for stale in ["nonce", "epoch"] {
                let mut fixture = ServiceFixture::new(false);
                let started = fixture.start_running();
                let mut identity = fixture.prepared.as_ref().unwrap().clone();
                if stale == "nonce" {
                    identity.launch_nonce = "b".repeat(64);
                } else {
                    identity.owner_epoch += 1;
                }
                let mut request = fixture.request("start", &identity);
                request["pane_id"] = json!("%9");
                let mut client = fixture.connect("stale start");
                assert_rejected(&client.exchange(&request));
                assert_eq!(fixture.health().child, Some(started.child));
                fixture.assert_manual_io();
                fixture.finish();
            }
        }

        #[test]
        fn runner_review_control_invalid_protocol_versions_leave_the_running_child_available() {
            let mut fixture = ServiceFixture::new(false);
            let started = fixture.start_running();
            for version in [None, Some(json!(2)), Some(json!("1")), Some(json!(1.0))] {
                let mut request = fixture.request("health", fixture.prepared.as_ref().unwrap());
                if let Some(version) = version {
                    request["protocol_version"] = version;
                } else {
                    request.as_object_mut().unwrap().remove("protocol_version");
                }
                let mut client = fixture.connect("invalid protocol");
                assert_rejected(&client.exchange(&request));
                assert_eq!(fixture.health().child, Some(started.child.clone()));
            }
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn runner_review_control_noncreator_start_is_isolated_before_and_after_spawn() {
            let mut fixture = ServiceFixture::new(false);
            fixture.prepare();
            fixture.publish();
            let mut request = fixture.request("start", fixture.prepared.as_ref().unwrap());
            request["pane_id"] = json!("%9");
            let mut noncreator = fixture.connect("noncreator before start");
            assert_rejected(&noncreator.exchange(&request));
            assert_eq!(fixture.health().child, None);
            let reply = fixture.start().unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            let started: Started = serde_json::from_value(reply["result"].clone()).unwrap();
            fixture.read_until("review-native-ready");
            let mut second = fixture.connect("noncreator after start");
            assert_rejected(&second.exchange(&request));
            assert_eq!(fixture.health().child, Some(started.child));
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn runner_review_control_repeated_creator_start_preserves_child_health_and_manual_io() {
            let mut fixture = ServiceFixture::new(false);
            let started = fixture.start_running();
            assert_rejected(&fixture.start());
            assert_eq!(fixture.health().child, Some(started.child));
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn runner_review_control_disconnected_reply_preserves_health_and_manual_io() {
            let mut fixture = ServiceFixture::new(false);
            let started = fixture.start_running();
            fixture.stop_for_disconnect();
            let mut disconnected = fixture.connect("disconnected health peer");
            disconnected
                .0
                .write_all(&wire_frame(
                    &fixture.request("health", fixture.prepared.as_ref().unwrap()),
                ))
                .unwrap();
            disconnected.0.shutdown(Shutdown::Both).unwrap();
            drop(disconnected);
            fixture.resume();
            assert_eq!(fixture.health().child, Some(started.child));
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn runner_review_control_creator_disconnect_before_start_exits_without_spawning() {
            let mut fixture = ServiceFixture::new(false);
            fixture.prepare();
            drop(fixture.creator.take());
            assert_eq!(fixture.wait().code(), Some(1));
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert_eq!(run.child_pid, None);
            assert_eq!(run.status, RunStatus::Failed);
            assert!(run.ended_at.is_some());
            assert!(
                RunLock::acquire_released(&fixture.fixture.conn, &run.owner())
                    .unwrap()
                    .is_some()
            );
        }

        #[test]
        fn runner_review_control_accepted_start_spawn_and_storage_failures_compensate() {
            for storage_failure in [false, true] {
                let mut fixture = ServiceFixture::new(!storage_failure);
                fixture.prepare();
                fixture.publish();
                if storage_failure {
                    fixture.fixture.conn.execute_batch("CREATE TRIGGER reject_review_running BEFORE UPDATE OF status ON member_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'fixture running persistence blocked'); END").unwrap();
                }
                let reply = fixture.start();
                assert_rejected(&reply);
                assert_eq!(fixture.wait().code(), Some(1));
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert_eq!(run.status, RunStatus::Failed);
                assert!(run.ended_at.is_some());
                let diagnostic = run.cleanup_diagnostics_json.as_ref().unwrap().to_string();
                let reported = format!("{reply:?} {diagnostic}");
                if storage_failure {
                    assert!(
                        reported.contains("fixture running persistence blocked"),
                        "{reported}"
                    );
                }
                assert!(!fixture._artifacts.paths.descriptor.exists());
                assert!(
                    RunLock::acquire_released(&fixture.fixture.conn, &run.owner())
                        .unwrap()
                        .is_some()
                );
            }
        }
    }

    mod input_service {
        use super::super::test_support::input_service::*;
        use super::*;
        #[test]
        fn step5_service_input_socket_submission_persists_typed_outcome_and_matching_key_never_replays()
         {
            with_input_service(|fixture, until| {
                let key = "prompt:service:single";
                let literal = "socket prompt λ";
                let value = request(
                    fixture,
                    key,
                    literal,
                    InputKind::Prompt,
                    None,
                    until.min(Instant::now() + Duration::from_secs(3)),
                );
                let result = submission(exchange(fixture, &value, until));
                let id = result.input_id.expect("actual durable service input ID");
                let row = storage::get_input(&fixture.fixture.conn, id).unwrap();
                assert_eq!(row.state, InputState::Accepted);
                assert_eq!(row.operation_key, key);
                wait_input_service_record(fixture, literal, until);
                let replay = submission(exchange(fixture, &value, until));
                assert_eq!(replay.input_id, Some(id));
                input_service_barrier(fixture, until);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .filter(|record| record["line"] == literal)
                        .count(),
                    1
                );
                assert_eq!(storage::get_input(&fixture.fixture.conn, id).unwrap(), row);
                let received = input_service_records(fixture)
                    .into_iter()
                    .find(|record| record["line"] == literal)
                    .unwrap();
                assert!(
                    received["inputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|input| input["input_id"] == id)
                );
            });
        }

        #[test]
        fn step5_service_input_queued_expiration_keeps_manual_progress_and_uses_fresh_output_for_exec()
         {
            with_input_service(|fixture, until| {
                let before = input_service_composer(fixture, until).output_seq;
                let expires = Instant::now() + Duration::from_millis(800);
                let value = request(
                    fixture,
                    "exec:service:expired",
                    "expired command",
                    InputKind::Exec,
                    None,
                    expires,
                );
                let frame = encode_control_frame(&value).unwrap();
                let mut queued = Peer::connect(fixture);
                queued.send(&frame[..4], until);
                manual(fixture, b"manual progress\r", until);
                wait_input_service_record(fixture, "manual progress", expires.min(until));
                let observed = loop {
                    let sample = input_service_composer(fixture, expires.min(until));
                    if sample.output_seq > before {
                        break sample;
                    }
                    pause(expires.min(until));
                };
                while Instant::now() <= expires {
                    pause(until);
                }
                queued.send(&frame[4..], until);
                let rejected = queued.reply(until);
                assert!(
                    rejected["error"]
                        .as_str()
                        .is_some_and(|message| message.contains("deadline")),
                    "{rejected}"
                );
                let repeated = exchange(fixture, &value, until);
                assert!(
                    repeated["error"]
                        .as_str()
                        .is_some_and(|message| message.contains("deadline")),
                    "{repeated}"
                );
                assert!(
                    storage::find_input(
                        &fixture.fixture.conn,
                        fixture.fixture.run.run_id,
                        "exec:service:expired"
                    )
                    .unwrap()
                    .is_none()
                );
                let fresh = request(
                    fixture,
                    "exec:service:fresh",
                    "queued command λ",
                    InputKind::Exec,
                    None,
                    until.min(Instant::now() + Duration::from_secs(3)),
                );
                let reply = submission(exchange(fixture, &fresh, until));
                assert!(reply.output_seq.unwrap() >= observed.output_seq);
                wait_input_service_record(fixture, "! queued command λ", until);
                input_service_barrier(fixture, until);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .map(|record| record["line"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    ["manual progress", "! queued command λ", "barrier"]
                );
            });
        }

        #[test]
        fn step5_service_input_disconnected_preview_consumer_retains_outcome_without_automatic_retyping()
         {
            with_input_service(|fixture, until| {
                let message = crate::broker::send_message(
                    &mut fixture.fixture.conn,
                    fixture.fixture.run.director_member_id,
                    fixture.fixture.run.member_id,
                    "disconnected preview λ",
                )
                .unwrap();
                let key = format!(
                    "preview:{}:{}",
                    fixture.fixture.run.run_id, message.message_id
                );
                let accepted_until = until.min(Instant::now() + Duration::from_secs(3));
                let value = request(
                    fixture,
                    &key,
                    &message.text,
                    InputKind::Preview,
                    Some(message.message_id),
                    accepted_until,
                );
                let mut abandoned = Peer::connect(fixture);
                abandoned.send(&encode_control_frame(&value).unwrap(), until);
                abandoned.stream.shutdown(Shutdown::Read).unwrap();
                let row = loop {
                    let current = storage::find_input(
                        &fixture.fixture.conn,
                        fixture.fixture.run.run_id,
                        &key,
                    )
                    .unwrap();
                    if let Some(row) = current.as_ref()
                        && row.state == InputState::Accepted
                    {
                        break row.clone();
                    }
                    assert!(
                        Instant::now() < accepted_until,
                        "disconnected consumer's durable accepted outcome is absent: row={current:?}; native_records={:?}",
                        input_service_records(fixture),
                    );
                    assert!(fixture.child.try_wait().unwrap().is_none());
                    pause(accepted_until);
                };
                drop(abandoned);
                wait_input_service_record(fixture, &message.text, until);
                let retained = submission(exchange(fixture, &value, until));
                assert_eq!(retained.input_id, Some(row.input_id));
                input_service_barrier(fixture, until);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .filter(|record| record["line"] == message.text)
                        .count(),
                    1
                );
                assert_eq!(
                    storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
                    row
                );
                assert_eq!(row.message_id, Some(message.message_id));
                let notified: Option<String> = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT notified_at FROM messages WHERE message_id=?1",
                        [message.message_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert!(notified.is_none());
            });
        }

        #[test]
        fn step5_service_input_expired_unreserved_attempt_releases_pending_work_for_a_fresh_key() {
            const EXPIRATION_TOLERANCE: Duration = Duration::from_millis(100);

            with_input_service(|fixture, until| {
                input_service_composer(fixture, until);
                let journal: String = fixture
                    .fixture
                    .conn
                    .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(journal, "wal");
                let blocker = crate::db::connect(&format!(
                    "sqlite:///{}",
                    fixture.fixture.conn.path().unwrap()
                ))
                .unwrap();
                blocker.busy_timeout(Duration::ZERO).unwrap();
                blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
                assert!(!blocker.is_autocommit());
                let expired_key = "prompt:service:reservation-expired";
                let expires = Instant::now() + Duration::from_millis(400);
                assert!(expires < until);
                let expired = request(
                    fixture,
                    expired_key,
                    "expired reservation must stay unwritten",
                    InputKind::Prompt,
                    None,
                    expires,
                );
                let started = Instant::now();
                let failure = exchange(fixture, &expired, until);
                let returned = Instant::now();
                let elapsed = returned.duration_since(started);
                assert!(!blocker.is_autocommit());
                assert!(
                    returned >= expires - EXPIRATION_TOLERANCE
                        && returned <= expires + EXPIRATION_TOLERANCE,
                    "original expiration bounds the SQL wait with conservative conversion and scheduling tolerance: elapsed={elapsed:?}; returned={returned:?}; expires={expires:?}"
                );
                assert!(
                    failure["error"]
                        .as_str()
                        .is_some_and(|message| message.contains("deadline")
                            || message.contains("database is locked")),
                    "{failure}"
                );
                assert!(
                    storage::find_input(
                        &fixture.fixture.conn,
                        fixture.fixture.run.run_id,
                        expired_key
                    )
                    .unwrap()
                    .is_none()
                );
                assert!(input_service_records(fixture).is_empty());
                blocker.execute_batch("ROLLBACK").unwrap();
                assert!(blocker.is_autocommit());
                input_service_composer(fixture, until);
                let fresh_key = "prompt:service:after-expired-reservation";
                let literal = "fresh after expired reservation";
                let fresh = request(
                    fixture,
                    fresh_key,
                    literal,
                    InputKind::Prompt,
                    None,
                    until.min(Instant::now() + Duration::from_secs(3)),
                );
                let reply = exchange(fixture, &fresh, until);
                eprintln!(
                    "reservation expiry under actual {journal} writer lock: elapsed={elapsed:?}; expired_reply={failure}; fresh_reply={reply}; native_records={:?}",
                    input_service_records(fixture)
                );
                let accepted = submission(reply);
                let row =
                    storage::get_input(&fixture.fixture.conn, accepted.input_id.unwrap()).unwrap();
                assert_eq!(row.operation_key, fresh_key);
                assert_eq!(row.state, InputState::Accepted);
                wait_input_service_record(fixture, literal, until);
                input_service_barrier(fixture, until);
                assert_eq!(
                    input_service_records(fixture)
                        .iter()
                        .map(|record| record["line"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    [literal, "barrier"]
                );
                assert!(
                    storage::find_input(
                        &fixture.fixture.conn,
                        fixture.fixture.run.run_id,
                        expired_key
                    )
                    .unwrap()
                    .is_none()
                );
            });
        }

        #[test]
        fn step5_service_input_post_write_failure_preserves_known_identity_and_original_history() {
            with_input_service(|fixture, until| {
                let key = "prompt:service:known-input-error";
                let literal = "SERVICE_R2_PRIVATE_LITERAL_λ";
                fixture.fixture.conn.execute_batch(&format!(
            "CREATE TRIGGER owned_input_accept_failure BEFORE UPDATE OF state ON member_input_operations
             WHEN OLD.operation_key='{key}' AND OLD.state='in_progress' AND NEW.state='accepted'
             BEGIN SELECT RAISE(FAIL,'owned accepted update failure'); END"
        )).unwrap();
                let value = request(
                    fixture,
                    key,
                    literal,
                    InputKind::Prompt,
                    None,
                    until.min(Instant::now() + Duration::from_secs(3)),
                );
                let attempt = catch_unwind(AssertUnwindSafe(|| exchange(fixture, &value, until)));
                let removed = fixture
                    .fixture
                    .conn
                    .execute_batch("DROP TRIGGER owned_input_accept_failure");
                let reply = match attempt {
                    Ok(reply) => {
                        removed.unwrap();
                        reply
                    }
                    Err(primary) => {
                        if let Err(cleanup) = removed {
                            eprintln!("owned SQL trigger removal also failed: {cleanup}");
                        }
                        resume_unwind(primary);
                    }
                };
                let cause = reply["error"]
                    .as_str()
                    .expect("actual Accepted-persistence failure envelope");
                assert!(cause.contains("owned accepted update failure"), "{reply}");
                assert!(
                    !cause.contains(literal),
                    "safe identities replace literal payloads in errors"
                );
                let original =
                    storage::find_input(&fixture.fixture.conn, fixture.fixture.run.run_id, key)
                        .unwrap()
                        .expect("independent durable reservation after the actual write");
                assert_eq!(original.run_id, fixture.fixture.run.run_id);
                assert_eq!(original.operation_key, key);
                assert_eq!(original.state, InputState::InProgress);
                wait_input_service_record(fixture, literal, until);
                let history = exchange(fixture, &value, until);
                assert!(history.get("error").is_none(), "{history}");
                assert_eq!(history["result"]["kind"], "submission");
                let retained: SubmissionResult =
                    serde_json::from_value(history["result"]["submission"].clone()).unwrap();
                assert_eq!(retained.input_id, Some(original.input_id));
                assert_eq!(retained.status, SubmissionStatus::Uncertain);
                assert_eq!(retained.stage, SubmissionStage::Submitted);
                input_service_barrier(fixture, until);
                let count = input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == literal)
                    .count();
                assert_eq!(count, 1);
                assert_eq!(
                    storage::get_input(&fixture.fixture.conn, original.input_id).unwrap(),
                    original
                );
                eprintln!(
                    "actual post-write persistence error: cause={cause:?}; input_id={}; run_id={}; key={:?}; state={:?}; child_line_count={count}",
                    original.input_id, original.run_id, original.operation_key, original.state
                );
                assert!(
                    cause.contains(&format!("input_id={}", original.input_id))
                        && cause.contains(&format!("run_id={}", original.run_id))
                        && cause.contains(&format!(
                            "operation_key={}",
                            serde_json::to_string(&original.operation_key).unwrap()
                        )),
                    "actual bookkeeping failure retains the independently observed safe input identities: {cause}"
                );
            });
        }
    }

    mod observation_contract {
        use super::super::test_support::observation_services::{
            count, health, private_write, wait_until,
        };
        use super::*;
        use crate::runtime::runner::observation::test_support::synthetic;
        #[test]
        fn runner_batch2_service_sqlite_and_writer_failures_keep_health_manual_display_and_resize_live()
         {
            for boundary in ["fail-sync", "block-persist"] {
                let mut fixture = ServiceFixture::configured(false, synthetic);
                fixture.start_running();
                let hook = fixture.fixture._dir.path().join(boundary);
                private_write(&hook, b"owned test gate");
                fixture.master.write_all(b"form-pressure\r").unwrap();
                fixture.read_until("Operation: write form-pressure.txt");
                wait_until(|| {
                    health(&mut fixture)["observation"]["degraded"] == true
                        || fixture.fixture._dir.path().join("persist-entered").exists()
                });
                fixture
                    .fixture
                    .conn
                    .execute_batch("BEGIN IMMEDIATE")
                    .unwrap();
                let begin = Instant::now();
                let status = health(&mut fixture);
                assert!(begin.elapsed() < Duration::from_millis(500));
                assert_eq!(status["observation"]["eligibility"]["state"], "hold");
                assert!(status["observation"]["pending_count"].as_u64().unwrap() <= 130);
                fixture.assert_manual_io();
                let sequence = health(&mut fixture)["output_seq"].as_u64().unwrap();
                rustix::termios::tcsetwinsize(
                    &fixture.master,
                    rustix::termios::Winsize {
                        ws_row: 25,
                        ws_col: 90,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    },
                )
                .unwrap();
                assert!(fixture.process.as_ref().unwrap().matches().unwrap());
                kill(
                    Pid::from_raw(i32::try_from(fixture.child.id()).unwrap()),
                    Signal::SIGWINCH,
                )
                .unwrap();
                wait_until(|| health(&mut fixture)["output_seq"].as_u64().unwrap() > sequence);
                fixture.fixture.conn.execute_batch("ROLLBACK").unwrap();
                std::fs::remove_file(hook).unwrap();
                wait_until(|| count(&fixture, "interaction_opened") == 1);
                fixture.finish();
            }
        }
        #[test]
        fn runner_batch2_service_existing_reservation_is_refused_before_native_spawn_or_new_grant()
        {
            let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
                synthetic(fixture, spec);
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET event_seq_reserved_through=256 WHERE run_id=?1",
                        [fixture.run.run_id],
                    )
                    .unwrap();
                fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            });
            let after =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert!(after.child_pid.is_none());
            assert_eq!(after.event_seq_reserved_through, 256);
            assert_eq!(
                fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                        [after.run_id],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            assert_eq!(count(&fixture, "interaction_opened"), 0);
            assert!(!fixture.fixture._dir.path().join("native-started").exists());
        }

        #[test]
        fn runner_batch2_service_each_durable_history_table_is_checked_before_initial_reservation()
        {
            for interaction in [false, true] {
                let mut seeded = None;
                let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
                    synthetic(fixture, spec);
                    let run = &fixture.run;
                    let evidence=storage::evidence::EvidencePolicy::metadata().apply(&json!({"backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,"rule_id":"runner-lifecycle","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
                    if interaction {
                        seeded = Some(evidence.to_string());
                        fixture.conn.execute("INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json) VALUES(?1,1,'permission_request',?2,'2026-10-26T17:00:00Z',?3)",rusqlite::params![run.run_id,evidence["evidence_digest"].as_str().unwrap(),evidence.to_string()]).unwrap();
                    } else {
                        let payload = json!({"schema_version":1,"event_id":1,"event_seq":1,"run_id":run.run_id,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,"kind":"coverage_unknown","interaction_id":null,"classification":null,"observed_at":"2026-10-26T17:00:00Z","source":"runner_lifecycle","native_request_id":null,"decision_outcome":"unknown","evidence":evidence,"cause":{"kind":"coverage_unknown"}});
                        seeded = Some(payload.to_string());
                        fixture.conn.execute("INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,committed_at,payload_json,notification_state,archive_reason) VALUES(?1,1,'coverage_unknown','2026-10-26T17:00:00Z','2026-10-26T17:00:00Z',?2,'archived','member_inactive')",rusqlite::params![run.run_id,payload.to_string()]).unwrap();
                    }
                });
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert!(run.child_pid.is_none());
                assert_eq!(run.event_seq_reserved_through, 0);
                assert!(!fixture.fixture._dir.path().join("native-started").exists());
                assert_eq!(count(&fixture, "coverage_unknown"), i64::from(!interaction));
                let interactions: i64 = fixture
                    .fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM member_interactions WHERE run_id=?1",
                        [run.run_id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(interactions, i64::from(interaction));
                let query = if interaction {
                    "SELECT evidence_json FROM member_interactions WHERE run_id=?1"
                } else {
                    "SELECT payload_json FROM member_runtime_events WHERE run_id=?1"
                };
                let retained: String = fixture
                    .fixture
                    .conn
                    .query_row(query, [run.run_id], |row| row.get(0))
                    .unwrap();
                assert_eq!(Some(retained), seeded);
            }
        }

        #[test]
        fn runner_batch2_service_started_health_reports_observation_without_composer_authority() {
            let mut fixture = ServiceFixture::new(false);
            fixture.start_running();
            let mut client = fixture.connect("observation health");
            let reply = client
                .exchange(&fixture.request("health", fixture.prepared.as_ref().unwrap()))
                .unwrap();
            fixture.assert_manual_io();
            fixture.finish();
            assert!(reply.get("error").is_none(), "{reply}");
            let status = &reply["result"]["observation"];
            assert!(
                status.is_object(),
                "started health requires bounded observation status: {reply}"
            );
            assert_eq!(status["eligibility"]["state"], "hold");
            assert!(status["pending_count"].as_u64().unwrap() <= 130);
            assert!(status.get("operation").is_none());
            assert!(status.get("semantic_fingerprint").is_none());
        }
    }

    mod review_failures {
        use super::super::test_support::observation_services::count;
        use super::super::test_support::review_failures::*;
        use super::*;
        use crate::runtime::runner::observation::test_support::synthetic;
        #[test]
        fn runner_failure_with_a_live_native_child_persists_degraded_before_owned_cleanup_confirms_exit()
         {
            let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                synthetic(fixture, spec);
                install_trap(spec, "");
            });
            let started = fixture.start_running();
            assert!(started.child.matches().unwrap());
            assert!(fixture.process.as_ref().unwrap().matches().unwrap());
            let before =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert_eq!(before.status, RunStatus::Running);
            rustix::termios::tcsetwinsize(
                &fixture.master,
                rustix::termios::Winsize {
                    ws_row: 0,
                    ws_col: 0,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                },
            )
            .unwrap();
            kill(
                Pid::from_raw(i32::try_from(fixture.child.id()).unwrap()),
                Signal::SIGWINCH,
            )
            .unwrap();
            let until = Instant::now() + Duration::from_secs(5);
            let during = loop {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                if run.status != RunStatus::Running {
                    break run;
                }
                assert!(
                    Instant::now() < until,
                    "runner failure left the run running"
                );
                std::thread::sleep(Duration::from_millis(2));
            };
            assert_eq!(during.status, RunStatus::Degraded);
            assert_eq!(during.child_pid, Some(started.child.pid));
            assert!(
                started.child.matches().unwrap(),
                "the TERM-immune native child is still live while the run is degraded"
            );
            let exit = fixture.wait();
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            let diagnostics = run
                .cleanup_diagnostics_json
                .as_ref()
                .and_then(Value::as_array)
                .expect("runner failure records cleanup diagnostics");
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .as_str()
                    .is_some_and(|text| text.contains("terminal dimensions must be positive"))),
                "{diagnostics:?}"
            );
            assert_eq!(run.status, RunStatus::Exited, "{diagnostics:?}");
            assert_eq!(run.exit_signal, Some(9), "{diagnostics:?}");
            assert_eq!(run.child_pid, Some(started.child.pid));
            assert_eq!(
                run.child_start_identity.as_deref(),
                Some(started.child.start_identity.as_str())
            );
            assert_eq!(run.owner_epoch, before.owner_epoch);
            assert_eq!(run.cleanup_state, CleanupState::Incomplete);
            assert_eq!(exit.code(), Some(137));
            assert!(!started.child.matches().unwrap());
        }
        #[test]
        fn runner_review_r3_shutdown_detector_failure_records_gap_before_truthful_native_exit() {
            let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                configure(fixture, spec, "shutdown_drain", "detector");
                install_trap(
                    spec,
                    "printf \"\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write shutdown-failure.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \"; sleep 0.2; exit 0",
                );
            });
            fixture.start_running();
            arm(&fixture);
            let begin = Instant::now();
            let outcome = shutdown_reply(&mut fixture);
            assert!(begin.elapsed() < Duration::from_secs(3));
            assert_fired(&fixture, "ShutdownDrain/Detector");
            assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
            assert_eq!(
                outcome.cleanup_state,
                CleanupState::Complete,
                "{:?}",
                outcome.diagnostics
            );
            assert_eq!(outcome.pending_event_frames, 0);
            assert_eq!(outcome.unsaved_observation_gap, 0);
            assert_durable_gap(&fixture);
            assert_eq!(fixture.wait().code(), Some(0));
            assert_eq!(count(&fixture, "runner_exited"), 1);
            assert_private_cause_stays_local(&fixture);
        }

        #[test]
        fn runner_review_r3_shutdown_parser_loss_after_terminal_fence_remains_unsaved_without_live_append()
         {
            let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
                configure(fixture, spec, "shutdown_drain", "parser");
                let config_path = fixture._dir.path().join("observation-services.json");
                let mut config: Value =
                    serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
                config["buffered_tail"] = json!({"database_url":format!("sqlite:///{}",fixture.conn.path().unwrap()),"run_id":fixture.run.run_id});
                std::fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
                spec.environment.push((
                    "CAFLEET_REVIEW_BUFFERED_TAIL_WRITTEN".into(),
                    fixture
                        ._dir
                        .path()
                        .join("buffered-tail-written")
                        .to_str()
                        .unwrap()
                        .into(),
                ));
                install_trap(
                    spec,
                    "umask 077; if printf \"review-after-terminal-tail\\n\"; then /usr/bin/touch \"$CAFLEET_REVIEW_BUFFERED_TAIL_WRITTEN\" || exit 9; else exit 8; fi; exit 0",
                );
            });
            fixture.start_running();
            arm(&fixture);
            let mut wire = fixture.connect("post-terminal observation loss");
            wire.0
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = fixture.request("shutdown", fixture.prepared.as_ref().unwrap());
            let begin = Instant::now();
            let reply = wire.exchange(&request).unwrap_or_else(|cause| {
                panic!(
                    "bounded shutdown reply failed: {cause}; {}; buffered markers={:?}",
                    fixture.connection_diagnostic("buffered tail shutdown"),
                    std::fs::read_dir(fixture.fixture._dir.path())
                        .unwrap()
                        .map(|entry| entry.unwrap().file_name())
                        .collect::<Vec<_>>()
                )
            });
            assert!(begin.elapsed() < Duration::from_secs(5));
            assert!(reply.get("error").is_none(), "{reply}");
            let outcome: ShutdownOutcome = serde_json::from_value(reply["result"].clone()).unwrap();
            let written = fixture
                .fixture
                ._dir
                .path()
                .join("buffered-tail-written")
                .exists();
            let fired = fixture
                .fixture
                ._dir
                .path()
                .join("observation-fault-fired")
                .exists();
            let stages: Vec<_> = [
                "buffered-tail-order-selected.json",
                "buffered-tail-native-terminated",
                "buffered-tail-parser-entered",
                "buffered-tail-post-fence-parser",
            ]
            .into_iter()
            .map(|file| (file, fixture.fixture._dir.path().join(file).exists()))
            .collect();
            let gate_error = std::fs::read_to_string(
                fixture.fixture._dir.path().join("buffered-tail-gate-error"),
            );
            let native_at_parser = std::fs::read_to_string(
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("buffered-tail-parser-native.json"),
            );
            let final_kinds: Vec<String> = fixture
                .fixture
                .conn
                .prepare(
                    "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
                )
                .unwrap()
                .query_map([fixture.fixture.run.run_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let final_run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert!(
                written && fired,
                "post-terminal tail: written={written}, fired={fired}, stages={stages:?}, gate_error={gate_error:?}, native_at_parser={native_at_parser:?}, final_kinds={final_kinds:?}, final_run_status={:?}, outcome={outcome:?}; {}",
                final_run.status,
                fixture.connection_diagnostic("post-terminal tail write and parser hook")
            );
            assert_fired(&fixture, "ShutdownDrain/Parser");
            let selected: Value = serde_json::from_slice(
                &std::fs::read(
                    fixture
                        .fixture
                        ._dir
                        .path()
                        .join("buffered-tail-order-selected.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(selected["run_id"], fixture.fixture.run.run_id);
            let remaining_ns = selected["remaining_ns"].as_u64().unwrap();
            assert!(
                remaining_ns > 0 && remaining_ns <= 2_000_000_000,
                "actual original child budget: {selected}"
            );
            assert!(
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("buffered-tail-native-terminated")
                    .exists()
            );
            assert!(
                fixture
                    .fixture
                    ._dir
                    .path()
                    .join("buffered-tail-post-fence-parser")
                    .exists()
            );
            assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
            assert_eq!(outcome.pending_event_frames, 0);
            assert!(outcome.unsaved_observation_gap > 0);
            assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
            let _exit = fixture.wait();
            let kinds: Vec<String> = fixture
                .fixture
                .conn
                .prepare(
                    "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
                )
                .unwrap()
                .query_map([fixture.fixture.run.run_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(kinds, ["runner_exited"]);
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert_eq!(run.status, RunStatus::Exited);
            assert_eq!(run.exit_code, Some(0));
            assert_eq!(run.cleanup_state, CleanupState::Incomplete);
            assert_private_cause_stays_local(&fixture);
        }
    }
    mod pacing_tests {
        use super::super::*;
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
    mod input_admission_tests {
        use super::super::*;
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
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
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
        fn collect(
            client: &mut Client,
            count: usize,
            deadline: Instant,
        ) -> Vec<AdmittedControlFrame> {
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
                let converted = receive_input_expiration(
                    11_000_000_000,
                    actual.admitted_at(),
                    None,
                    &mut clock,
                )
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
}
