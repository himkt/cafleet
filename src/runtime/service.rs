use crate::broker;
use crate::broker::runtime as storage;
use crate::broker::runtime::*;
use crate::config::Settings;
use crate::runtime::control::AdmittedControlFrame;
use crate::runtime::control::ServiceControlRequest;
use crate::runtime::control::decode_service_request;
use crate::runtime::control::encode_input_reply;
use crate::runtime::input::InputCoordinator;
use crate::runtime::input::InputServices;
use crate::runtime::input::RequestTokenSource;
use crate::runtime::input::receive_input_expiration;
use crate::runtime::transport::TransportProgress;
use crate::runtime::*;
use filedescriptor::FileDescriptor;
use rusqlite::Connection;
use rusqlite::params;
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::ops::ControlFlow;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

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

struct NativeIo<'a> {
    input: &'a mut NonblockingFile,
    pty: &'a mut PtySession,
    output: &'a mut NonblockingFile,
    transport: &'a mut Transport,
    controller: &'a mut ObservationController,
    writer: &'a EventWriter,
    clock: &'a dyn crate::time::Clock,
}
impl NativeIo<'_> {
    fn shutdown_deadlines(&self) -> ShutdownDeadlines {
        ShutdownDeadlines::starting_now(self.clock)
    }
    fn quiesce(
        &mut self,
        coordinator: &mut InputCoordinator,
        deadline: Instant,
    ) -> std::result::Result<InputDrain, InputFailure> {
        coordinator.progress_with_manual(self.transport, |transport| {
            transport.quiesce_for_input(
                self.input,
                self.pty,
                self.output,
                &mut self.controller.observer(self.writer, self.clock),
                deadline,
            )
        })
    }
    /// Records a native exit after `before_exit` has queued every deferred result, so the
    /// results precede `runner_exited`.
    fn observe_exit(
        &mut self,
        native_exit: &mut Option<ChildExit>,
        before_exit: impl FnOnce(&ObservationController) -> Result<()>,
    ) -> Result<Option<ShutdownDeadlines>> {
        let Some(exit) = self.pty.try_wait()? else {
            return Ok(None);
        };
        *native_exit = Some(exit);
        before_exit(self.controller)?;
        self.controller
            .child_exited(exit, &self.transport.snapshot(), self.writer, self.clock)?;
        Ok(Some(self.shutdown_deadlines()))
    }
}

fn input_deadline(
    request: &InputControlRequest,
    admitted_at: Instant,
    clock: &dyn crate::time::Clock,
) -> Result<Instant> {
    receive_input_expiration(request.expires_at_mono_ns, admitted_at, clock)
}
impl RunnerLoop<'_> {
    fn inspect(
        &mut self,
        request: InputControlRequest,
        admitted_at: Instant,
    ) -> Result<InputControlReply> {
        let InputControlOperation::PermissionInspect(input) = &request.operation else {
            unreachable!("inspection receives an inspection operation");
        };
        let input = input.clone();
        let context = InputContext {
            run_id: Some(self.run.run_id),
            interaction_id: Some(input.interaction_id),
            ..InputContext::default()
        };
        self.inspect_within_deadline(&request, input, admitted_at)
            .map_err(|source| CafleetError::from(context.failure(source)))
    }
    fn inspect_within_deadline(
        &mut self,
        request: &InputControlRequest,
        input: PermissionInspectInput,
        admitted_at: Instant,
    ) -> Result<InputControlReply> {
        let deadline = input_deadline(request, admitted_at, self.native.clock)?;
        request.identity.validate(&self.run)?;
        let coordinator = self
            .coordinator
            .as_mut()
            .ok_or_else(|| error("native input has not started"))?;
        self.native.quiesce(coordinator, deadline)?;
        coordinator
            .inspect_received(
                self.conn,
                input,
                (&*self.native.controller, &mut *self.native.transport),
                deadline,
            )
            .map_err(CafleetError::from)
    }
    fn submit(
        &mut self,
        request: InputControlRequest,
        admitted_at: Instant,
        client_id: u64,
    ) -> Result<Option<InputControlReply>> {
        let context = match &request.operation {
            InputControlOperation::PermissionRespond(input) => Some(InputContext {
                run_id: Some(self.run.run_id),
                interaction_id: Some(input.interaction_id),
                input_id: None,
                operation_key: None,
            }),
            _ => None,
        };
        self.submit_within_deadline(request, context.is_some(), admitted_at, client_id)
            .map_err(|source| match context {
                Some(context) => CafleetError::from(context.failure(source)),
                None => source,
            })
    }
    fn submit_within_deadline(
        &mut self,
        request: InputControlRequest,
        responds: bool,
        admitted_at: Instant,
        client_id: u64,
    ) -> Result<Option<InputControlReply>> {
        let deadline = input_deadline(&request, admitted_at, self.native.clock)?;
        let coordinator = self
            .coordinator
            .as_mut()
            .ok_or_else(|| error("native input has not started"))?;
        if responds {
            if let Some(reply) = coordinator.answered_episode_refusal(&request)? {
                return Ok(Some(reply));
            }
            let admission = self.native.quiesce(coordinator, deadline)?;
            if matches!(admission, InputDrain::Pending) {
                return Err(error(format!(
                    "permission request is blocked: {}",
                    self.native
                        .transport
                        .drain_blocker()
                        .expect("a Pending drain names its reason")
                )));
            }
        }
        let reply =
            coordinator.handle_received(self.conn, request, self.native.controller, deadline)?;
        if reply.is_none() {
            self.turns.admit_client(client_id, deadline)?;
        }
        Ok(reply)
    }
}

impl Client {
    fn poll(&mut self, clock: &dyn crate::time::Clock) -> Result<Vec<AdmittedControlFrame>> {
        let mut requests = Vec::new();
        let mut bytes = [0; 4096];
        match self.stream.read(&mut bytes) {
            Ok(0) => {
                self.decoder.finish()?;
                self.closed = true;
            }
            Ok(count) => {
                let observed_at = clock.now();
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
    /// Queues a lifecycle reply; `false`, with the client closed, when it cannot be queued.
    fn answer(&mut self, result: Result<Value>) -> bool {
        let queued = crate::runtime::control::encode_reply(result.as_ref())
            .and_then(|bytes| self.queue_reply(bytes));
        self.closed |= queued.is_err();
        queued.is_ok()
    }
    /// Queues an input reply; `false`, with the client closed, when it cannot be queued.
    fn answer_input(
        &mut self,
        reply: std::result::Result<&InputControlReply, &CafleetError>,
    ) -> bool {
        let queued = encode_input_reply(reply).and_then(|bytes| self.queue_reply(bytes));
        self.closed |= queued.is_err();
        queued.is_ok()
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
    deadlines: ShutdownDeadlines,
    controller: &'a mut ObservationController,
    clock: &'a dyn crate::time::Clock,
    coordinator: Option<&'a mut InputCoordinator>,
}
impl LiveShutdown<'_> {
    fn poll_shutdown(&mut self) -> Result<ObservationStatus> {
        let snapshot = self.transport.snapshot();
        #[cfg(test)]
        return self.controller.poll_for(
            &snapshot,
            self.writer,
            self.clock,
            ObservationPhase::ShutdownDrain,
        );
        #[cfg(not(test))]
        self.controller.poll(&snapshot, self.writer, self.clock)
    }
    #[cfg(test)]
    fn await_native_exit_before_parsing(&mut self, deadline: Instant) -> Result<()> {
        if !self
            .transport
            .order_shutdown_parsing_after_native_exit(deadline)
        {
            return Ok(());
        }
        while self.clock.now() < deadline {
            self.transport.prefetch_child_bytes(self.pty)?;
            if self.pty.try_wait()?.is_some() {
                self.send_exit()?;
                if !self.controller.has_pending() {
                    return Ok(());
                }
            }
            nap_until(self.clock, deadline);
        }
        Err(CafleetError::DeadlineExceeded)
    }

    fn send_exit(&mut self) -> Result<()> {
        if let Some(exit) = self.pty.try_wait()? {
            self.controller.child_exited(
                exit,
                &self.transport.snapshot(),
                self.writer,
                self.clock,
            )?;
        }
        Ok(())
    }
}
impl ShutdownHost for LiveShutdown<'_> {
    fn clock(&self) -> &dyn crate::time::Clock {
        self.clock
    }
    fn gate_programmatic_input(&mut self) {
        self.writer.set_deadline(self.deadlines.storage_deadline());
        #[cfg(test)]
        self.controller
            .begin_capture_shutdown(self.deadlines.storage_deadline());
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

        while self.clock.now() < deadline {
            let mut tick = |transport: &mut Transport| {
                #[cfg(test)]
                let mut observer = self.controller.observer_for(
                    self.writer,
                    self.clock,
                    ObservationPhase::ShutdownDrain,
                );
                #[cfg(not(test))]
                let mut observer = self.controller.observer(self.writer, self.clock);
                transport.tick(self.input, self.pty, self.output, &mut observer)
            };
            let progress = match self.coordinator.as_mut() {
                Some(coordinator) => coordinator.progress_with_manual(self.transport, tick)?,
                None => tick(self.transport)?,
            };
            self.poll_shutdown()?;
            self.send_exit()?;
            if !self.pty.survivors()? && progress.child_eof {
                break;
            }
            nap_until(self.clock, deadline);
        }
        self.pty.try_wait()
    }
    fn kill_verified_survivors(&mut self) -> Result<()> {
        self.pty.signal_verified(nix::sys::signal::Signal::SIGKILL)
    }
    fn close_pty(&mut self) -> Result<()> {
        self.pty.close()?;
        while self.clock.now() < self.deadlines.storage_deadline() {
            if (self.pty.try_wait()?.is_some() || !self.pty.has_child()) && !self.pty.survivors()? {
                break;
            }
            nap_until(self.clock, self.deadlines.storage_deadline());
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
        while self.transport.pending_display_bytes() > 0 && self.clock.now() < deadline {
            self.transport.flush_display(self.output)?;
            nap_until(self.clock, deadline);
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
        if self
            .deadlines
            .remaining_storage_at(self.clock.now())
            .is_zero()
        {
            return Err(crate::error::CafleetError::DeadlineExceeded);
        }
        while self.controller.has_pending() && self.clock.now() < deadline {
            self.poll_shutdown()?;
            nap_until(self.clock, deadline);
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
        note(&mut errors, self.output.restore());
        note(&mut errors, self.input.restore());
        note(&mut errors, self.raw.restore());
        if errors.is_empty() {
            Ok(())
        } else {
            Err(error(errors.join("; ")))
        }
    }
}

pub(crate) struct RunnerDependencies {
    #[cfg(test)]
    pub(crate) catalog: &'static crate::coding_agent::ProfileCatalog,
    pub(crate) clock: Arc<dyn crate::time::Clock>,
    pub(crate) diagnostics: crate::runtime::diagnostics::RunnerDiagnostics,
    #[cfg(test)]
    pub(crate) writer_hooks: Arc<dyn WriterHooks>,

    #[cfg(test)]
    pub(crate) capture: Option<crate::runtime::observation::native_capture::NativeCapturePort>,

    #[cfg(test)]
    pub(crate) observation_hooks: Option<Arc<dyn ObservationHooks>>,

    #[cfg(test)]
    pub(crate) preview_sweep: bool,
}
pub(crate) fn run(settings: &Settings, descriptor_path: &Path) -> Result<i32> {
    run_service(
        settings,
        descriptor_path,
        RunnerDependencies {
            #[cfg(test)]
            catalog: crate::coding_agent::ProfileCatalog::bundled(),
            clock: Arc::new(crate::time::SystemClock),
            diagnostics: crate::runtime::diagnostics::RunnerDiagnostics::default(),
            #[cfg(test)]
            writer_hooks: crate::runtime::writer::test_support::default_writer_hooks(),

            #[cfg(test)]
            capture: None,

            #[cfg(test)]
            observation_hooks: None,

            #[cfg(test)]
            preview_sweep: true,
        },
    )
}
pub(crate) fn run_service(
    settings: &Settings,
    descriptor_path: &Path,
    services: RunnerDependencies,
) -> Result<i32> {
    #[cfg(test)]
    let mut services = services;
    let clock = services.clock.clone();
    let shutdown_deadlines = || ShutdownDeadlines::starting_now(clock.as_ref());
    #[cfg(not(test))]
    let catalog = crate::coding_agent::ProfileCatalog::bundled();
    #[cfg(test)]
    let catalog = services.catalog;

    let mut conn = crate::db::connect(&settings.database_url)?;
    crate::db::classify_selected_schema(&conn, &settings.database_url).require_current()?;
    let descriptor = load_descriptor(&conn, descriptor_path)?;
    let original = storage::get_run(&conn, descriptor.identity.run_id)?;
    let creator = original.owner_process();
    ensure(
        creator.matches()?,
        "launch creator process identity changed",
    )?;
    let lock = Arc::new(
        RunLock::acquire(&conn, &original.owner())?
            .ok_or_else(|| error("run ownership lock is held"))?,
    );
    let process = ProcessIdentity::current()?;
    let handshake_deadline = clock.now() + Duration::from_secs(10);
    let owner = lock.transfer_to_runner_with_deadline(
        &mut conn,
        &original.owner(),
        process.clone(),
        handshake_deadline,
        clock.as_ref(),
    )?;
    let run = storage::get_run(&conn, owner.run_id)?;
    with_database_deadline(&mut conn, handshake_deadline, |conn| {
        lock.validate(conn, &owner)?;
        ensure(
            run.status == RunStatus::Starting
                && run.child_pid.is_none()
                && run.child_start_identity.is_none()
                && run.started_at.is_none()
                && run.ended_at.is_none()
                && run.recovery_origin_status.is_none(),
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
    let mut pty = PtySession::prepare(initial_size, clock.clone())?;
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
        clock.clone(),
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
    let mut controller = ObservationController::new(&run, catalog)?;
    controller.share_diagnostics(services.diagnostics.clone());
    #[cfg(test)]
    if let Some(hooks) = services.observation_hooks.take() {
        controller.set_observation_hooks(hooks.clone())?;
        transport.set_observation_hooks(hooks)?;
    }

    #[cfg(not(test))]
    let start_writer = EventWriter::start;
    #[cfg(test)]
    let start_writer = EventWriter::start_with_hooks;

    let mut writer = start_writer(
        settings.database_url.clone(),
        run.clone(),
        lock.clone(),
        clock.clone(),
        #[cfg(test)]
        services.writer_hooks.clone(),
        services.diagnostics.clone(),
    )?;
    let terminate = Arc::new(AtomicBool::new(false));
    let resize = Arc::new(AtomicBool::new(false));
    let _signals = SignalGuards::new(terminate.clone(), resize.clone())?;
    let mut failure = None;
    let mut runner = RunnerLoop {
        native: NativeIo {
            input: &mut input,
            pty: &mut pty,
            output: &mut output,
            transport: &mut transport,
            controller: &mut controller,
            writer: &writer,
            clock: clock.as_ref(),
        },
        conn: &mut conn,
        original: &original,
        run,
        lock: &lock,
        artifacts: &artifacts,
        descriptor: &descriptor,
        process: &process,
        creator: &creator,
        listener: &listener,
        terminal: &terminal,
        terminate: &terminate,
        resize: &resize,
        catalog,
        clock: clock.clone(),
        #[cfg(test)]
        capture: services.capture.take(),
        handshake_deadline,
        clients: Vec::new(),
        next_client: 1,
        creator_connection: None,
        started: false,
        coordinator: None,
        #[cfg(not(test))]
        turns: InputTurns::new(),
        #[cfg(test)]
        turns: if services.preview_sweep {
            InputTurns::new()
        } else {
            InputTurns::without_sweep()
        },
        shutdown_client: None,
        native_exit: None,
    };
    let loop_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner.serve()));
    let RunnerLoop {
        run,
        started,
        mut coordinator,
        mut clients,
        shutdown_client,
        native_exit,
        ..
    } = runner;
    let (loop_result, unwind) = match loop_result {
        Ok(result) => (result, None),
        Err(payload) => (Err(error("runner loop unwound")), Some(payload)),
    };
    let deadlines = match loop_result {
        Ok(deadlines) => deadlines,
        Err(cause) => {
            let deadlines = shutdown_deadlines();
            let mut message = cause.message().into_owned();
            if started {
                let degraded = with_database_deadline(
                    &mut conn,
                    deadlines.storage_deadline(),
                    |conn| {
                        lock.validate(conn, &owner)?;
                        conn.execute("UPDATE member_runs SET status='degraded',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='running' AND child_pid IS NOT NULL",params![clock.wall_stamp(),run.run_id,owner.owner_epoch])?;
                        Ok(())
                    },
                );
                if let Err(cause) = degraded {
                    message.push_str(&format!("; degraded status: {}", cause.message()));
                }
            }
            failure = Some(message);
            deadlines
        }
    };
    if let Some(coordinator) = coordinator.as_mut() {
        let fenced = coordinator.fence(deadlines.storage_deadline());
        coordinator.finish_pending_attempt();
        let unpublished = with_database_deadline(&mut conn, deadlines.storage_deadline(), |conn| {
            storage::unpublished_delivery_results(conn, run.run_id)
        });
        if let Ok(unpublished) = &unpublished {
            for &(input_id, stage, cause) in unpublished {
                controller.publish_delivery_result(storage::input_delivery_result_cause(
                    input_id, stage, cause,
                ));
            }
        }
        for (step, result) in [
            ("input fence", fenced.map(|_| ())),
            ("result publication", unpublished.map(|_| ())),
        ] {
            if let Err(cause) = result {
                let message = format!("{step}: {}", cause.message());
                if let Some(prior) = &mut failure {
                    prior.push_str(&format!("; {message}"));
                } else {
                    failure = Some(message);
                }
            }
        }
    }
    let deadlines = pty.begin_shutdown(deadlines);
    let mut live = LiveShutdown {
        pty: &mut pty,
        transport: &mut transport,
        input: &mut input,
        output: &mut output,
        raw: &mut raw,
        writer: &mut writer,
        deadlines,
        controller: &mut controller,
        clock: clock.as_ref(),
        coordinator: coordinator.as_mut(),
    };
    let mut outcome = shutdown(&mut live, deadlines, native_exit);
    if let Some(cause) = failure {
        outcome.fail(cause);
    }
    if let Some(cause) = controller.pending_error() {
        outcome.fail(cause.message());
    }
    if let Some(cause) = writer.diagnostic() {
        outcome.fail(cause);
    }
    let diagnostics_at = clock.wall_stamp();
    let diagnostics = shutdown_diagnostics(&services.diagnostics, &outcome, &diagnostics_at);
    let persisted = with_database_deadline(&mut conn, deadlines.storage_deadline(), |conn| {
        lock.validate(conn, &owner)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE member_runs SET cleanup_state=?1,cleanup_diagnostics_json=?2,last_health_at=?3,status=CASE WHEN child_pid IS NULL THEN 'failed' ELSE status END,ended_at=CASE WHEN child_pid IS NULL THEN ?3 ELSE ended_at END WHERE run_id=?4 AND owner_epoch=?5",params![outcome.cleanup_state.as_str(),serde_json::to_string(&outcome.diagnostics).map_err(io_error)?,clock.wall_stamp(),run.run_id,owner.owner_epoch])?;
        storage::write_runner_diagnostics(
            &tx,
            run.run_id,
            &diagnostics.0,
            diagnostics.1,
            &diagnostics_at,
        )?;
        tx.commit()?;
        Ok(())
    });
    if let Err(cause) = persisted {
        outcome.fail(format!("cleanup evidence pending: {cause}"));
    }
    if let Some(id) = shutdown_client
        && let Some(client) = clients.iter_mut().find(|client| client.id == id)
    {
        client.answer(serde_json::to_value(&outcome).map_err(io_error));
        while !client.output.is_empty() && clock.now() < deadlines.storage_deadline() {
            if client.closed || client.flush().is_err() {
                client.closed = true;
                break;
            }
            clock.sleep(Duration::from_millis(1));
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

fn started(coordinator: &mut Option<InputCoordinator>) -> &mut InputCoordinator {
    coordinator
        .as_mut()
        .expect("a started runner has its input coordinator")
}

/// The runner's control loop: accepts control clients, serves their requests and, once the
/// native child runs, ticks its transport and input.
struct RunnerLoop<'a> {
    native: NativeIo<'a>,
    conn: &'a mut Connection,
    original: &'a RunRecord,
    run: RunRecord,
    lock: &'a Arc<RunLock>,
    artifacts: &'a RunArtifacts,
    descriptor: &'a LaunchDescriptor,
    process: &'a ProcessIdentity,
    creator: &'a ProcessIdentity,
    listener: &'a UnixListener,
    terminal: &'a std::fs::File,
    terminate: &'a AtomicBool,
    resize: &'a AtomicBool,
    catalog: &'static crate::coding_agent::ProfileCatalog,
    clock: Arc<dyn crate::time::Clock>,
    #[cfg(test)]
    capture: Option<crate::runtime::observation::native_capture::NativeCapturePort>,
    handshake_deadline: Instant,
    clients: Vec<Client>,
    next_client: u64,
    creator_connection: Option<u64>,
    started: bool,
    coordinator: Option<InputCoordinator>,
    turns: InputTurns,
    shutdown_client: Option<u64>,
    native_exit: Option<ChildExit>,
}

impl RunnerLoop<'_> {
    fn shutdown_deadlines(&self) -> ShutdownDeadlines {
        self.native.shutdown_deadlines()
    }

    fn serve(&mut self) -> Result<ShutdownDeadlines> {
        loop {
            if self.terminate.load(Ordering::Acquire) {
                return Ok(self.shutdown_deadlines());
            }
            if !self.started {
                check_deadline(self.native.clock.now(), self.handshake_deadline)?;
                ensure(
                    self.creator.matches()?,
                    "creator disconnected before native start",
                )?;
            }
            self.accept()?;
            for index in 0..self.clients.len() {
                if let Some(deadlines) = self.serve_client(index)? {
                    return Ok(deadlines);
                }
            }
            self.clients.retain(|client| !client.closed);
            let progress = if self.started {
                match self.tick_running()? {
                    ControlFlow::Break(deadlines) => return Ok(deadlines),
                    ControlFlow::Continue(progress) => progress,
                }
            } else {
                TransportProgress::default()
            };
            self.native.clock.sleep(tick_sleep(
                &progress,
                self.native.controller.status().next_deadline,
                self.native.clock.now(),
            ));
        }
    }

    fn accept(&mut self) -> Result<()> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(cause) if cause.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(cause) => return Err(io_error(cause)),
        };
        stream.set_nonblocking(true).map_err(io_error)?;
        if self.clients.len() < 32 {
            self.clients.push(Client {
                id: self.next_client,
                stream,
                decoder: ControlDecoder::new(),
                output: VecDeque::new(),
                closed: false,
            });
            self.next_client = self
                .next_client
                .checked_add(1)
                .ok_or_else(|| error("control connection sequence exhausted"))?;
        }
        Ok(())
    }

    fn creator_lost(&self, client_id: u64) -> bool {
        self.creator_connection == Some(client_id) && !self.started
    }

    fn serve_client(&mut self, index: usize) -> Result<Option<ShutdownDeadlines>> {
        let client_id = self.clients[index].id;
        let requests = match self.clients[index].poll(self.native.clock) {
            Ok(requests) => requests,
            Err(_) => {
                self.clients[index].closed = true;
                Vec::new()
            }
        };
        if self.clients[index].closed && self.creator_lost(client_id) {
            return Err(error("creator control disconnected before start"));
        }
        for frame in requests {
            let lifecycle = match decode_service_request(frame.body()) {
                Ok(ServiceControlRequest::Input(request)) => {
                    if !self.serve_input(index, *request, frame.admitted_at()) {
                        break;
                    }
                    continue;
                }
                Ok(ServiceControlRequest::Lifecycle(lifecycle)) => lifecycle,
                Err(cause) => {
                    if !self.clients[index].answer(Err(cause)) {
                        break;
                    }
                    continue;
                }
            };
            let mut accepted_startup = false;
            let result = self.serve_lifecycle(client_id, &lifecycle, &mut accepted_startup);
            if self.shutdown_client == Some(client_id) {
                return Ok(Some(self.shutdown_deadlines()));
            }
            let startup_failure = result
                .as_ref()
                .err()
                .filter(|_| accepted_startup)
                .map(|cause| cause.message().into_owned());
            self.clients[index].answer(result);
            if let Some(cause) = startup_failure {
                return Err(error(format!(
                    "native startup failed; owned compensation required: {cause}"
                )));
            }
            if self.clients[index].closed {
                if self.creator_lost(client_id) {
                    return Err(error("creator control disconnected before start"));
                }
                break;
            }
        }
        Ok(None)
    }

    /// Serves one input request; `false` when its reply could not be queued.
    fn serve_input(
        &mut self,
        index: usize,
        request: InputControlRequest,
        admitted_at: Instant,
    ) -> bool {
        let client_id = self.clients[index].id;
        if matches!(
            request.operation,
            InputControlOperation::PermissionInspect(_)
        ) {
            let result = self.inspect(request, admitted_at);
            return self.clients[index].answer_input(result.as_ref());
        }
        let result = self.submit(request, admitted_at, client_id);
        match result.as_ref().map(Option::as_ref).transpose() {
            Some(reply) => self.clients[index].answer_input(reply),
            None => true,
        }
    }

    fn serve_lifecycle(
        &mut self,
        client_id: u64,
        request: &LifecycleRequest,
        accepted_startup: &mut bool,
    ) -> Result<Value> {
        match request {
            LifecycleRequest::Prepare {
                identity,
                remaining_ms,
                ..
            } => self.prepare(client_id, identity, *remaining_ms, accepted_startup),
            LifecycleRequest::Start {
                identity, pane_id, ..
            } => {
                identity.validate(&self.run)?;
                self.start(client_id, pane_id, accepted_startup)
            }
            LifecycleRequest::Health { identity, .. } => {
                identity.validate(&self.run)?;
                self.health()
            }
            LifecycleRequest::Shutdown { identity, .. } => {
                identity.validate(&self.run)?;
                self.shutdown_client = Some(client_id);
                Ok(Value::Null)
            }
        }
    }

    fn prepare(
        &mut self,
        client_id: u64,
        identity: &ControlIdentity,
        remaining: u64,
        accepted_startup: &mut bool,
    ) -> Result<Value> {
        ensure(
            !self.started && self.creator_connection.is_none(),
            "preparation is single-use",
        )?;
        identity.validate(self.original)?;
        ensure(
            remaining > 0 && remaining <= 10000,
            "invalid handshake budget",
        )?;
        self.handshake_deadline = self
            .handshake_deadline
            .min(self.native.clock.now() + Duration::from_millis(remaining));
        self.creator_connection = Some(client_id);
        *accepted_startup = true;
        self.artifacts.remove_descriptor()?;
        serde_json::to_value(Prepared {
            identity: ControlIdentity::from_run(&self.run),
            runner: self.process.clone(),
        })
        .map_err(io_error)
    }

    fn start(&mut self, client_id: u64, pane: &str, accepted_startup: &mut bool) -> Result<Value> {
        ensure(
            !self.started && self.creator_connection == Some(client_id),
            "start requires prepared creator connection",
        )?;
        *accepted_startup = true;
        with_database_deadline(self.conn, self.handshake_deadline, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            let current = storage::get_run(conn, self.run.run_id)?;
            ensure(
                current.status == RunStatus::Prepared,
                "start requires published prepared state",
            )?;
            let member = broker::get_member(conn, self.run.member_id, self.run.fleet_id)?
                .ok_or_else(|| error("prepared member became inactive"))?;
            ensure(
                member.placement.mux_pane_id.as_deref() == Some(pane),
                "start placement changed",
            )
        })?;
        #[cfg(test)]
        let capture_placement = match &mut self.capture {
            Some(port) => {
                let (prepared, placement) =
                    with_database_deadline(self.conn, self.handshake_deadline, |conn| {
                        let prepared = storage::get_run(conn, self.run.run_id)?;
                        let placement =
                            broker::get_member(conn, self.run.member_id, self.run.fleet_id)?
                                .map(|member| member.placement)
                                .ok_or_else(|| error("capture prepared placement missing"))?;
                        Ok((prepared, placement))
                    })?;
                port.bind_service_prepared(
                    &prepared,
                    &placement,
                    &self.descriptor.spawn,
                    self.native.clock,
                )?;
                Some(placement)
            }
            None => None,
        };
        let child = self.native.pty.spawn(&self.descriptor.spawn)?;
        with_database_deadline(self.conn, self.handshake_deadline, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            let changed=conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3,last_health_at=?3 WHERE run_id=?4 AND owner_epoch=?5 AND status='prepared'",params![child.pid,child.start_identity,self.native.clock.wall_stamp(),self.run.run_id,self.run.owner_epoch])?;
            ensure(changed == 1, "native spawn storage lost ownership")
        })?;
        self.run = storage::get_run(self.conn, self.run.run_id)?;
        self.started = true;
        #[cfg(test)]
        if let Some(mut port) = self.capture.take() {
            let placement = capture_placement.expect("a bound capture retains its placement");
            port.finalize_source(&self.run, &placement, &child)?;
            self.native.controller.attach_capture(port)?;
        }

        self.native.controller.started(
            &self.run,
            &self.native.transport.snapshot(),
            self.native.writer,
            self.native.clock,
        )?;
        let mut coordinator = InputCoordinator::new(
            self.conn,
            &self.run,
            self.lock.clone(),
            InputServices {
                catalog: self.catalog.clone(),
                token_source: Box::new(SystemRequestTokens),
                clock: self.clock.clone(),
                #[cfg(test)]
                stage_settle: crate::runtime::input::STAGE_SETTLE,
                #[cfg(test)]
                hooks: crate::runtime::input::test_support::default_input_hooks(),
            },
            self.handshake_deadline,
        )?;
        coordinator.share_diagnostics(self.native.writer.diagnostics().clone());
        self.coordinator = Some(coordinator);
        serde_json::to_value(Started {
            identity: ControlIdentity::from_run(&self.run),
            runner: self.process.clone(),
            child,
        })
        .map_err(io_error)
    }

    fn health(&mut self) -> Result<Value> {
        let child = self.run.child_process()?;
        let observation = self
            .started
            .then(|| self.native.controller.health(self.native.writer));
        serde_json::to_value(HealthSnapshot {
            identity: ControlIdentity::from_run(&self.run),
            runner: self.process.clone(),
            child,
            output_seq: self.native.transport.snapshot().screen_seq,
            observation,
        })
        .map_err(io_error)
    }

    fn tick_running(&mut self) -> Result<ControlFlow<ShutdownDeadlines, TransportProgress>> {
        let selected = self.run.database_identity_json["canonical_path"]
            .as_str()
            .expect("validated database path");
        let meta = std::fs::metadata(selected).map_err(io_error)?;
        ensure(
            Some(meta.dev()) == self.run.database_identity_json["device"].as_u64()
                && Some(meta.ino()) == self.run.database_identity_json["inode"].as_u64(),
            "selected database was replaced; input fenced",
        )?;
        let native = &mut self.native;
        if self.resize.swap(false, Ordering::AcqRel) {
            native
                .transport
                .resize(native.pty, terminal_size(self.terminal)?)?;
            #[cfg(test)]
            let mut observer = native.controller.observer_for(
                native.writer,
                native.clock,
                ObservationPhase::Resize,
            );
            #[cfg(not(test))]
            let mut observer = native.controller.observer(native.writer, native.clock);
            observer.observe(&native.transport.snapshot())?;
        }
        let coordinator = started(&mut self.coordinator);
        let progress = coordinator.progress_with_manual(native.transport, |transport| {
            transport.tick(
                native.input,
                native.pty,
                native.output,
                &mut native.controller.observer(native.writer, native.clock),
            )
        })?;
        native
            .controller
            .poll(&native.transport.snapshot(), native.writer, native.clock)?;
        if progress.outer_eof {
            return Ok(ControlFlow::Break(self.shutdown_deadlines()));
        }
        if let Some(deadlines) = self.observe_exit()? {
            return Ok(ControlFlow::Break(deadlines));
        }
        let mut io = LoopIo {
            native: &mut self.native,
            terminate: self.terminate,
            resize: self.resize,
            native_exit: &mut self.native_exit,
            run_id: self.run.run_id,
        };
        let coordinator = started(&mut self.coordinator);
        match self
            .turns
            .turn(coordinator, self.conn, &self.run, &mut io)?
        {
            InputTurnOutcome::Continue => {}
            InputTurnOutcome::Shutdown(deadlines) => return Ok(ControlFlow::Break(deadlines)),
            InputTurnOutcome::Reply { client, result } => {
                if let Some(client) = self
                    .clients
                    .iter_mut()
                    .find(|candidate| candidate.id == client && !candidate.closed)
                {
                    client.answer_input(result.as_ref());
                }
            }
        }
        Ok(ControlFlow::Continue(progress))
    }

    fn observe_exit(&mut self) -> Result<Option<ShutdownDeadlines>> {
        let coordinator = started(&mut self.coordinator);
        let (conn, run_id) = (&mut *self.conn, self.run.run_id);
        let clock = self.native.clock;
        self.native
            .observe_exit(&mut self.native_exit, |controller| {
                publish_stopped_results(coordinator, conn, run_id, controller, clock)
            })
    }
}

/// Every runner diagnostic still buffered once the event writer finished, followed by one
/// `shutdown` entry per outcome diagnostic, with the buffer's dropped count.
fn shutdown_diagnostics(
    diagnostics: &crate::runtime::diagnostics::RunnerDiagnostics,
    outcome: &ShutdownOutcome,
    at: &str,
) -> (Vec<RunnerDiagnosticEntry>, u64) {
    let (mut entries, dropped) = diagnostics.drain();
    entries.extend(
        outcome
            .diagnostics
            .iter()
            .map(|diagnostic| RunnerDiagnosticEntry {
                source: RunnerDiagnosticSource::Shutdown,
                message: diagnostic_text(diagnostic),
                first_at: at.to_owned(),
                last_at: at.to_owned(),
                count: 1,
            }),
    );
    (entries, dropped)
}

fn record_sweep_failure(coordinator: &InputCoordinator, cause: &CafleetError) {
    coordinator.record_diagnostic(
        RunnerDiagnosticSource::PreviewSweep,
        &format!("preview sweep: {}", cause.message()),
    );
}

/// The sweep's read snapshot: the run's member is an active ordinary member whose recorded run
/// is this run with a concrete placement, and its oldest unnotified pending message.
fn sweep_snapshot(conn: &mut Connection, run: &RunRecord) -> Result<Option<broker::MessageRecord>> {
    let tx = conn.transaction()?;
    let Some(fleet_id) = broker::active_member_fleet(&tx, run.member_id)? else {
        return Ok(None);
    };
    let current = broker::get_member(&tx, run.member_id, fleet_id)?.filter(|member| {
        member.kind == broker::MemberKind::Member
            && member.placement.mux_pane_id.is_some()
            && member
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.run.run_id == run.run_id)
    });
    let message = match current {
        Some(_) => broker::oldest_pending_preview(&tx, run.member_id)?,
        None => None,
    };
    tx.commit()?;
    Ok(message)
}

/// The in-flight immediate input attempt that later turns drive to its reply.
enum PendingAttempt {
    /// A control client's attempt, answered on that client.
    Client { id: u64, deadline: Instant },
    /// The runner's own sweep preview, which has no client to answer.
    Sweep { deadline: Instant },
}

/// What a turn's input work asks the runner loop to do next.
pub(crate) enum InputTurnOutcome {
    Continue,
    Shutdown(ShutdownDeadlines),
    /// A control client's completed immediate attempt and its reply.
    Reply {
        client: u64,
        result: Result<InputControlReply>,
    },
}

/// The runner I/O that input work drives between transport turns.
pub(crate) trait InputTurnIo {
    fn clock(&self) -> &dyn crate::time::Clock;
    fn controller(&self) -> &ObservationController;
    fn quiesce(
        &mut self,
        coordinator: &mut InputCoordinator,
        deadline: Instant,
    ) -> std::result::Result<InputDrain, InputFailure>;
    fn advance(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        deadline: Instant,
    ) -> Result<Option<InputControlReply>>;
    /// A termination request, which preempts any further write.
    fn terminate_requested(&self) -> bool;
    /// A pending resize, which defers a write to a later turn.
    fn resize_requested(&self) -> bool;
    /// The deadlines of the shutdown a termination request starts.
    fn shutdown_deadlines(&self) -> ShutdownDeadlines;
    /// Records an observed native exit and returns the shutdown it starts.
    fn observe_exit(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
    ) -> Result<Option<ShutdownDeadlines>>;
}

/// The runner's input work after each transport turn: queue housekeeping, the in-flight
/// immediate attempt, deferred retries and the preview sweep.
pub(crate) struct InputTurns {
    pending: Option<PendingAttempt>,
    preview_sweep: PreviewSweep,
    deferred_retry: DeferredRetry,
    queue_housekeeping_at: Option<Instant>,
}

impl InputTurns {
    pub(crate) fn new() -> Self {
        Self {
            pending: None,
            preview_sweep: PreviewSweep::default(),
            deferred_retry: DeferredRetry::default(),
            queue_housekeeping_at: None,
        }
    }

    /// Holds a client's accepted immediate attempt until a later turn completes it.
    pub(crate) fn admit_client(&mut self, id: u64, deadline: Instant) -> Result<()> {
        ensure(
            self.pending.is_none(),
            "an accepted immediate input requires no other attempt in flight",
        )?;
        self.pending = Some(PendingAttempt::Client { id, deadline });
        Ok(())
    }

    /// Runs one turn's input work in the runner loop's order.
    pub(crate) fn turn(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        run: &RunRecord,
        io: &mut impl InputTurnIo,
    ) -> Result<InputTurnOutcome> {
        self.housekeep_queue(coordinator, conn, io)?;
        if self.pending.is_some() {
            return self.complete_pending_input(coordinator, conn, io);
        }
        self.retry_deferred(coordinator, conn, io)?;
        self.sweep_previews(coordinator, conn, run, io)?;
        Ok(InputTurnOutcome::Continue)
    }

    /// Drives the in-flight immediate input to its reply: a client-owned attempt answers its
    /// client, and the runner's own sweep attempt settles without a reply.
    fn complete_pending_input(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        io: &mut impl InputTurnIo,
    ) -> Result<InputTurnOutcome> {
        let deadline = match self.pending.as_ref().expect("an attempt is in flight") {
            PendingAttempt::Client { deadline, .. } | PendingAttempt::Sweep { deadline } => {
                *deadline
            }
        };
        let result = match io.quiesce(coordinator, deadline) {
            Err(failure) => coordinator
                .pending_attempt_failed(conn, io.controller(), failure)
                .map(Some)
                .map_err(CafleetError::from),
            Ok(InputDrain::Pending) => Ok(None),
            Ok(InputDrain::Quiescent) => {
                if io.terminate_requested() {
                    return Ok(InputTurnOutcome::Shutdown(io.shutdown_deadlines()));
                }
                if let Some(deadlines) = io.observe_exit(coordinator, conn)? {
                    return Ok(InputTurnOutcome::Shutdown(deadlines));
                }
                if io.resize_requested() {
                    Ok(None)
                } else {
                    io.advance(coordinator, conn, deadline)
                }
            }
        };
        if matches!(result, Ok(None)) {
            return Ok(InputTurnOutcome::Continue);
        }
        coordinator.finish_pending_attempt();
        self.deferred_retry.next_at = None;
        match self.pending.take().expect("an attempt is in flight") {
            PendingAttempt::Sweep { .. } => {
                self.finish_sweep(
                    coordinator,
                    result.map(|reply| reply.expect("a completed sweep attempt has a reply")),
                    io.clock().now(),
                );
                Ok(InputTurnOutcome::Continue)
            }
            PendingAttempt::Client { id, .. } => Ok(InputTurnOutcome::Reply {
                client: id,
                result: result.map(|reply| reply.expect("completed input has a reply")),
            }),
        }
    }

    fn retry_deferred(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        io: &mut impl InputTurnIo,
    ) -> Result<()> {
        if !coordinator.has_deferred_work()
            || !self.deferred_retry.due(
                coordinator.deferred_attempt_writing(),
                io.controller().status(),
                io.clock().now(),
            )
        {
            return Ok(());
        }
        let deadline = io.clock().now() + Duration::from_secs(5);
        let drained = io.quiesce(coordinator, deadline);
        if let Ok(InputDrain::Quiescent) = drained
            && !io.terminate_requested()
            && !io.resize_requested()
            && let Err(failure) = io.advance(coordinator, conn, deadline)
        {
            if coordinator.deferred_failed() {
                return Err(failure);
            }
            coordinator.record_diagnostic(
                RunnerDiagnosticSource::DeferredInput,
                &format!("deferred input: {}", failure.message()),
            );
        }
        Ok(())
    }

    fn housekeep_queue(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        io: &impl InputTurnIo,
    ) -> Result<()> {
        let now = io.clock().now();
        if self.queue_housekeeping_at.is_some_and(|at| now < at) {
            return Ok(());
        }
        self.queue_housekeeping_at = Some(now + QUEUE_HOUSEKEEPING_INTERVAL);
        if let Err(failure) = coordinator.queue_housekeeping(conn, io.controller()) {
            if coordinator.deferred_failed() {
                return Err(failure);
            }
            coordinator.record_diagnostic(
                RunnerDiagnosticSource::QueueHousekeeping,
                &format!("queue housekeeping: {}", failure.message()),
            );
        }
        Ok(())
    }

    /// Delivers the member's oldest unnotified pending message, strictly oldest-first, when no
    /// ordinary attempt is active and the FIFO is empty.
    fn sweep_previews(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        run: &RunRecord,
        io: &impl InputTurnIo,
    ) -> Result<()> {
        let now = io.clock().now();
        self.preview_sweep
            .observe(io.controller().status().eligibility);
        if !coordinator.sweep_may_start() || !self.preview_sweep.due(now) {
            return Ok(());
        }
        let deadline = now + SWEEP_ATTEMPT_BUDGET;
        match with_database_deadline(conn, deadline, |conn| sweep_snapshot(conn, run)) {
            Ok(Some(message)) => {
                let reply = coordinator
                    .handle_preview(conn, &message, io.controller(), deadline)
                    .map_err(CafleetError::from);
                match reply {
                    Ok(None) => self.pending = Some(PendingAttempt::Sweep { deadline }),
                    Ok(Some(reply)) => {
                        self.finish_sweep(coordinator, Ok(reply), io.clock().now());
                    }
                    Err(cause) => self.finish_sweep(coordinator, Err(cause), io.clock().now()),
                }
            }
            Ok(None) => {}
            Err(cause) => {
                record_sweep_failure(coordinator, &cause);
                self.preview_sweep.back_off(io.clock().now());
            }
        }
        Ok(())
    }

    /// Feeds a completed sweep attempt's outcome to the run-wide backoff: a submitted preview
    /// resets it, and a transient obstacle arms it.
    fn finish_sweep(
        &mut self,
        coordinator: &InputCoordinator,
        reply: Result<InputControlReply>,
        now: Instant,
    ) {
        let submission = match reply {
            Ok(InputControlReply::Submission(submission)) => submission,
            Ok(_) => unreachable!("a preview submission replies with a submission"),
            Err(cause) => {
                record_sweep_failure(coordinator, &cause);
                self.preview_sweep.back_off(now);
                return;
            }
        };
        match submission.status {
            SubmissionStatus::Accepted => self.preview_sweep.reset(),
            SubmissionStatus::Held
                if matches!(
                    submission.reason.as_deref(),
                    Some(
                        "pane_input_busy"
                            | "storage_unavailable"
                            | "observer_unavailable"
                            | "delivery_budget_exhausted"
                    )
                ) =>
            {
                self.preview_sweep.back_off(now)
            }
            SubmissionStatus::Held
            | SubmissionStatus::Queued
            | SubmissionStatus::Rejected
            | SubmissionStatus::Uncertain => {}
        }
    }
}

#[cfg(test)]
impl InputTurns {
    /// Turns whose sweep never comes due, so a runner fixture's own preview delivery races no
    /// sweep.
    pub(crate) fn without_sweep() -> Self {
        Self {
            preview_sweep: PreviewSweep::disabled(),
            ..Self::new()
        }
    }

    /// Whether no immediate attempt is in flight.
    pub(crate) fn idle(&self) -> bool {
        self.pending.is_none()
    }

    /// The earliest instants at which a turn has timer work: the next sweep pass (after any
    /// backoff) and the next queue housekeeping.
    pub(crate) fn wakes(&self, now: Instant) -> [Option<Instant>; 2] {
        let sweep = self
            .preview_sweep
            .next_at
            .into_iter()
            .chain(self.preview_sweep.backoff_until)
            .max()
            .unwrap_or(now);
        [Some(sweep), self.queue_housekeeping_at]
    }
}

/// The live runner's I/O for its input turns.
struct LoopIo<'a, 'b> {
    native: &'a mut NativeIo<'b>,
    terminate: &'a AtomicBool,
    resize: &'a AtomicBool,
    native_exit: &'a mut Option<ChildExit>,
    run_id: i64,
}

impl InputTurnIo for LoopIo<'_, '_> {
    fn clock(&self) -> &dyn crate::time::Clock {
        self.native.clock
    }
    fn controller(&self) -> &ObservationController {
        self.native.controller
    }
    fn quiesce(
        &mut self,
        coordinator: &mut InputCoordinator,
        deadline: Instant,
    ) -> std::result::Result<InputDrain, InputFailure> {
        self.native.quiesce(coordinator, deadline)
    }
    fn advance(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
        deadline: Instant,
    ) -> Result<Option<InputControlReply>> {
        coordinator
            .advance(
                conn,
                self.native.transport,
                self.native.pty,
                self.native.controller,
                deadline,
            )
            .map_err(CafleetError::from)
    }
    fn terminate_requested(&self) -> bool {
        self.terminate.load(Ordering::Acquire)
    }
    fn resize_requested(&self) -> bool {
        self.resize.load(Ordering::Acquire)
    }
    fn shutdown_deadlines(&self) -> ShutdownDeadlines {
        self.native.shutdown_deadlines()
    }
    fn observe_exit(
        &mut self,
        coordinator: &mut InputCoordinator,
        conn: &mut Connection,
    ) -> Result<Option<ShutdownDeadlines>> {
        let (clock, run_id) = (self.native.clock, self.run_id);
        self.native.observe_exit(self.native_exit, |controller| {
            publish_stopped_results(coordinator, conn, run_id, controller, clock)
        })
    }
}

/// Fences ordinary input after the native exit and queues every durable deferred result that
/// has no event yet.
fn publish_stopped_results(
    coordinator: &mut InputCoordinator,
    conn: &mut Connection,
    run_id: i64,
    controller: &ObservationController,
    clock: &dyn crate::time::Clock,
) -> Result<()> {
    let deadline = clock.now() + Duration::from_secs(1);
    coordinator.fence(deadline)?;
    let unpublished = with_database_deadline(conn, deadline, |conn| {
        storage::unpublished_delivery_results(conn, run_id)
    })?;
    for (input_id, stage, cause) in unpublished {
        controller
            .publish_delivery_result(storage::input_delivery_result_cause(input_id, stage, cause));
    }
    Ok(())
}

const IDLE_TICK_SLEEP: Duration = Duration::from_millis(2);
const DEFERRED_RETRY_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const PREVIEW_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const QUEUE_HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(1);
const SWEEP_BACKOFF_FLOOR: Duration = Duration::from_secs(1);
const SWEEP_BACKOFF_CAP: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub(crate) struct SweepBackoff {
    delay: Duration,
}

impl SweepBackoff {
    pub(crate) fn delay(&self) -> Duration {
        self.delay
    }

    pub(crate) fn transient(&mut self) {
        self.delay = if self.delay.is_zero() {
            SWEEP_BACKOFF_FLOOR
        } else {
            (self.delay * 2).min(SWEEP_BACKOFF_CAP)
        };
    }

    pub(crate) fn reset(&mut self) {
        self.delay = Duration::ZERO;
    }
}

const SWEEP_ATTEMPT_BUDGET: Duration = Duration::from_secs(5);

/// Paces the pending-preview sweep: one pass per interval, none before the run-wide backoff
/// expires.
#[derive(Default)]
struct PreviewSweep {
    next_at: Option<Instant>,
    backoff: SweepBackoff,
    backoff_until: Option<Instant>,
    eligibility: Option<InputEligibility>,
    #[cfg(test)]
    disabled: bool,
}

impl PreviewSweep {
    fn due(&mut self, now: Instant) -> bool {
        #[cfg(test)]
        if self.disabled {
            return false;
        }
        let due = self.next_at.is_none_or(|next_at| now >= next_at)
            && self.backoff_until.is_none_or(|until| now >= until);
        if due {
            self.next_at = Some(now + PREVIEW_SWEEP_INTERVAL);
        }
        due
    }

    /// An interaction closing or a candidate clearing resets the backoff.
    fn observe(&mut self, eligibility: InputEligibility) {
        let released = matches!(
            self.eligibility,
            Some(InputEligibility::Hold(
                GateReason::Interaction | GateReason::Candidate
            ))
        ) && !matches!(
            eligibility,
            InputEligibility::Hold(GateReason::Interaction | GateReason::Candidate)
        );
        if released {
            self.reset();
        }
        self.eligibility = Some(eligibility);
    }

    fn back_off(&mut self, now: Instant) {
        self.backoff.transient();
        self.backoff_until = Some(now + self.backoff.delay());
    }

    fn reset(&mut self) {
        self.backoff.reset();
        self.backoff_until = None;
    }
}

#[cfg(test)]
impl PreviewSweep {
    /// A sweep that never comes due, so a runner fixture's own preview delivery races no sweep.
    fn disabled() -> Self {
        Self {
            disabled: true,
            ..Self::default()
        }
    }
}

/// Paces queued-input retries on the 100 ms timer, waking early when the observed gate changes,
/// such as an interaction closing or a candidate clearing, or when an active input completes.
#[derive(Default)]
struct DeferredRetry {
    next_at: Option<Instant>,
    gate: Option<(Visibility, InputEligibility)>,
}
impl DeferredRetry {
    fn due(&mut self, writing: bool, status: ObservationStatus, now: Instant) -> bool {
        let gate = (status.visibility, status.eligibility);
        let due =
            writing || self.gate != Some(gate) || self.next_at.is_none_or(|next_at| now >= next_at);
        if due {
            self.gate = Some(gate);
            self.next_at = Some(now + DEFERRED_RETRY_INTERVAL);
        }
        due
    }
}

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
    use std::io::ErrorKind;
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::Command;
    use std::process::ExitStatus;
    use std::process::Stdio;
    use std::result::Result;
    use std::time::Duration;
    use std::time::Instant;

    use nix::sys::signal::Signal;
    use nix::sys::signal::kill;
    use nix::sys::wait::WaitPidFlag;
    use nix::sys::wait::WaitStatus;
    use nix::sys::wait::waitpid;
    use nix::unistd::Pid;
    use serde_json::Value;
    use serde_json::json;

    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::config::Settings;
    use crate::digest::sha256_hex;
    use crate::runtime::system::test_support::TestChild;
    use crate::runtime::test_support::*;
    use crate::runtime::*;
    pub(crate) const DESCRIPTOR_ENV: &str = "CAFLEET_REVIEW_RUNNER_DESCRIPTOR";
    pub(crate) const SERVICES_ENV: &str = "CAFLEET_REVIEW_TEST_SERVICES";
    pub(crate) const DATABASE_ENV: &str = "CAFLEET_REVIEW_RUNNER_DATABASE_URL";
    pub(crate) fn wire_frame(value: &Value) -> Vec<u8> {
        let body = serde_json::to_vec(value).unwrap();
        assert!(body.len() <= 65536);
        crate::runtime::control::test_support::independent_frame(&body)
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
        /// A path inside the fixture's private directory.
        pub(crate) fn path(&self, name: impl AsRef<std::path::Path>) -> PathBuf {
            self.fixture._dir.path().join(name)
        }
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
            let panics = crate::runtime::service::test_support::input_service::panic_records(
                self.fixture._dir.path(),
            );
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
                    "UPDATE member_placements SET mux_pane_id=NULL, mux_window_id=NULL, mux_window_name=NULL WHERE member_id=?1",
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
                        "resize" => crate::runtime::observation::native_capture::runner_collector_resize_child_path(),
                        "export" => crate::runtime::observation::native_capture::runner_collector_export_child_path(),
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
                    crate::runtime::observation::native_capture::native_capture_service_child_path()
                } else {
                    crate::runtime::service::tests::control_service::service_child_path()
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
                        DATABASE_ENV,
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
                    "UPDATE member_placements SET mux_pane_id='%9', mux_window_id=COALESCE(mux_window_id, '@1'), mux_window_name=COALESCE(mux_window_name, 'main') WHERE member_id=?1",
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

        /// Resizes the outer terminal and signals the runner the way a terminal emulator does.
        pub(crate) fn resize(&self, rows: u16, cols: u16) {
            rustix::termios::tcsetwinsize(
                &self.master,
                rustix::termios::Winsize {
                    ws_row: rows,
                    ws_col: cols,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                },
            )
            .unwrap();
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(self.child.id()).unwrap()),
                nix::sys::signal::Signal::SIGWINCH,
            )
            .unwrap();
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
            self.child
                .poll_until(Instant::now() + Duration::from_secs(5))
                .unwrap()
                .expect("owned runner exceeded bounded cleanup")
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
            match self.child.poll_until(until - Duration::from_secs(1)) {
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
            self.cleanup_recorded_native();
            self.child.terminate_until(until);
        }
    }

    pub(crate) mod input_service {
        use crate::runtime::service::test_support::observation_services::private_write;
        use crate::runtime::service::test_support::*;
        use std::panic::AssertUnwindSafe;
        use std::panic::catch_unwind;
        use std::panic::resume_unwind;
        pub(crate) fn native(path: &Path) {
            install_panic_record(path.parent().unwrap(), "native");
            crate::runtime::service::test_support::input_service::native::run(path);
        }
        pub(crate) const NATIVE_ENV: &str = "CAFLEET_REVIEW_SERVICE_INPUT_NATIVE";
        pub(crate) const SCREEN: &[u8] = b"\x1b[2J\x1b[23;1HSynthetic state\r\nState end";
        /// A manual line that makes the native child show a detected permission form instead of
        /// journaling it; the next journaled line restores the composer screen.
        pub(crate) const FORM_LINE: &str = "show-permission-form";
        pub(crate) const FORM: &[u8] = b"\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: write held.txt\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ";
        /// A manual line that makes the native child show a composer holding a draft, which no
        /// composer rule matches; the next journaled line restores the composer screen.
        pub(crate) const DRAFT_LINE: &str = "show-composer-draft";
        pub(crate) const DRAFT: &[u8] =
            b"\x1b[2J\x1b[23;1HSynthetic draft: half-typed\r\nState end";
        /// Literal text that makes the native child show the detected permission form while the
        /// literal is still unsubmitted, once per native process, so the write stops mid-stage.
        pub(crate) const FORM_MID_WRITE: &str = "open-form-mid-write";
        pub(crate) const PANIC_RECORD_LIMIT: usize = 16 * 1024;
        pub(crate) const PANIC_DIRECTORY: &str = "service-input-panics";

        pub(crate) fn install_panic_record(directory: &Path, role: &'static str) {
            use std::os::unix::fs::OpenOptionsExt;
            use std::os::unix::fs::PermissionsExt;

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

        pub(crate) use crate::time::SystemClock as Clock;
        pub(crate) fn expiration(until: Instant) -> u64 {
            InputExpiration::capture(until, &Clock)
                .unwrap()
                .monotonic_ns()
        }
        pub(crate) fn pause(until: Instant) {
            crate::runtime::control::pause(until, &crate::time::SystemClock)
                .expect("original owned service deadline");
        }

        /// `detecting` adds the synthetic permission-form detector, so `FORM_LINE` opens a
        /// detected interaction and the run's coverage is tested.
        pub(crate) fn configure_with(
            fixture: &mut Fixture,
            spec: &mut SpawnSpec,
            until: Instant,
            detecting: bool,
        ) {
            use std::os::unix::fs::DirBuilderExt;

            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(fixture._dir.path().join(PANIC_DIRECTORY))
                .unwrap();
            let mut catalog = crate::coding_agent::catalog::test_support::synthetic_definitions();
            let permission_rule = catalog["codex"]["profile"]["rules"][0].clone();
            catalog["codex"]["screens"]["synthetic-service-submission"] =
                crate::coding_agent::catalog::test_support::fixture_at(
                    crate::coding_agent::catalog::test_support::VERSION,
                );
            let rule = &mut catalog["codex"]["profile"]["rules"][0];
            rule["id"] = json!("synthetic-service-composer");
            rule["observation"] = json!("composer_empty");
            rule["region"]["start_pattern"] = json!("^Synthetic state$");
            rule["region"]["end_pattern"] = json!("^State end$");
            rule["matchers"] = json!([{"id":"state","scope":"line","pattern":"^Synthetic state$"}]);
            rule["extract"] = json!({"operation":null,"choices":[]});
            rule["verified_noninteraction"] = json!(true);
            catalog["codex"]["profile"]["capabilities"] = json!({"detection":null,"answers":[],"composer":{
                "rule_ids":["synthetic-service-composer"],"fixture_ids":[crate::coding_agent::catalog::test_support::FIXTURE],
                "submission":{
                    "prompt":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]},
                    "exec":{"literal_framing":"plain","plain_literal_limit":null,"prefix_hex":"2120","submit_hex":"0d","fixture_ids":["synthetic-service-submission"]}
                }
            }});
            let coverage = if detecting {
                let fixture_id = crate::coding_agent::catalog::test_support::FIXTURE;
                catalog["codex"]["profile"]["rules"]
                    .as_array_mut()
                    .unwrap()
                    .push(permission_rule);
                catalog["codex"]["profile"]["capabilities"]["detection"] =
                    json!({"rule_ids":["synthetic-permission"],"fixture_ids":[fixture_id]});
                "tested"
            } else {
                "unknown"
            };
            fixture.conn.execute("UPDATE member_runs SET native_version='0.0.1-synthetic',coverage=?2 WHERE run_id=?1",rusqlite::params![fixture.run.run_id, coverage]).unwrap();
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
                crate::runtime::service::tests::control_service::service_child_path().into(),
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
                crate::runtime::test_support::write_fixture_bytes(&mut self.stream, bytes, until);
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
                            let mut frames = crate::runtime::control::test_support::feed_values(
                                &mut self.decoder,
                                &bytes[..count],
                            )
                            .unwrap();
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
            until: Instant,
        ) -> Value {
            json!({"protocol_version":1,"kind":if kind==InputKind::Exec {"submit_exec"} else {"submit_prompt"},
        "identity":fixture.prepared.as_ref().unwrap(),"expires_at_mono_ns":expiration(until),
        "operation_key":key,"input_kind":kind,"literal":literal})
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
        /// Waits until the runner observes a non-degraded screen with no open interaction and
        /// no pending observation work, which also holds on a run without composer verification.
        pub(crate) fn input_service_settled(fixture: &mut ServiceFixture, until: Instant) {
            loop {
                assert!(Instant::now() < until);
                let health = fixture.health();
                if health.observation.as_ref().is_some_and(|observation| {
                    !observation.degraded
                        && observation.eligibility
                            != InputEligibility::Hold(GateReason::Interaction)
                        && observation.pending_count == 0
                }) {
                    return;
                }
                pause(until);
            }
        }
        fn composer_ready(fixture: &mut ServiceFixture, until: Instant) {
            input_service_composer(fixture, until);
        }
        pub(crate) fn with_input_service(body: impl FnOnce(&mut ServiceFixture, Instant)) {
            with_configured_input_service(false, 0, body);
        }

        pub(crate) fn with_input_service_catalog(
            configure_catalog: impl FnOnce(&mut Value),
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_configured_input_service_catalog(false, 0, configure_catalog, body);
        }

        pub(crate) fn with_detecting_input_service(
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_configured_input_service(true, 0, body);
        }

        /// Runs a detecting input service whose body ends the run, which then exits with
        /// `exit_code`.
        pub(crate) fn with_detecting_input_service_exiting(
            exit_code: i32,
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_configured_input_service(true, exit_code, body);
        }

        /// A sweeping input service whose catalog `configure_catalog` adjusts first.
        pub(crate) fn with_sweeping_input_service_catalog(
            lifetime: Duration,
            detecting: bool,
            configure_catalog: impl FnOnce(&mut Value),
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_input_service_lifetime(
                lifetime,
                detecting,
                0,
                |config| {
                    config["preview_sweep"] = json!(true);
                    configure_catalog(&mut config["catalog"]);
                },
                input_service_settled,
                body,
            );
        }

        fn with_configured_input_service(
            detecting: bool,
            exit_code: i32,
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_configured_input_service_catalog(detecting, exit_code, |_| {}, body);
        }

        fn with_configured_input_service_catalog(
            detecting: bool,
            exit_code: i32,
            configure_catalog: impl FnOnce(&mut Value),
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            with_input_service_lifetime(
                Duration::from_secs(10),
                detecting,
                exit_code,
                |config| configure_catalog(&mut config["catalog"]),
                composer_ready,
                body,
            );
        }

        fn with_input_service_lifetime(
            lifetime: Duration,
            detecting: bool,
            exit_code: i32,
            configure: impl FnOnce(&mut Value),
            ready: impl FnOnce(&mut ServiceFixture, Instant),
            body: impl FnOnce(&mut ServiceFixture, Instant),
        ) {
            let until = Instant::now() + lifetime;
            let mut fixture = ServiceFixture::configured_until(false, until, |fixture, spec| {
                configure_with(fixture, spec, until, detecting);
                let path = fixture._dir.path().join("observation-services.json");
                let mut config: Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                configure(&mut config);
                std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
            });
            fixture.cleanup_deadline = Some(until);
            let primary = catch_unwind(AssertUnwindSafe(|| {
                fixture.start_running();
                ready(&mut fixture, until);
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
                    Some(exit_code),
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
            use crate::runtime::service::test_support::input_service::*;
            use nix::fcntl::FcntlArg;
            use nix::fcntl::OFlag;
            use nix::fcntl::fcntl;
            use nix::sys::termios::LocalFlags;
            use nix::sys::termios::SetArg;
            use nix::sys::termios::tcgetattr;
            use nix::sys::termios::tcsetattr;
            use std::fs::OpenOptions;
            use std::os::unix::fs::OpenOptionsExt;

            pub(crate) fn receipt_inputs(
                conn: &rusqlite::Connection,
                run_id: i64,
                expires: u64,
            ) -> rusqlite::Result<Vec<Value>> {
                loop {
                    assert!(
                        crate::time::Clock::monotonic_sample(&Clock)
                            .unwrap()
                            .monotonic_ns
                            < expires,
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
                            let now = crate::time::Clock::monotonic_sample(&Clock)
                                .unwrap()
                                .monotonic_ns;
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
                terminal
                    .local_flags
                    .remove(LocalFlags::ECHO | LocalFlags::ICANON);
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
                let mut form_mid_write_shown = false;
                loop {
                    assert!(
                        crate::time::Clock::monotonic_sample(&Clock)
                            .unwrap()
                            .monotonic_ns
                            < expires,
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
                    if !form_mid_write_shown
                        && !pending.contains(&b'\n')
                        && pending
                            .windows(FORM_MID_WRITE.len())
                            .any(|window| window == FORM_MID_WRITE.as_bytes())
                    {
                        form_mid_write_shown = true;
                        output.write_all(FORM).unwrap();
                        output.flush().unwrap();
                    }
                    while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                        let mut line = pending.drain(..=end).collect::<Vec<_>>();
                        assert_eq!(line.pop(), Some(b'\n'));
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        let line = String::from_utf8(line).unwrap();
                        if line.ends_with("finish") {
                            return;
                        }
                        if line == FORM_LINE {
                            output.write_all(FORM).unwrap();
                            output.flush().unwrap();
                            continue;
                        }
                        if line == DRAFT_LINE {
                            output.write_all(DRAFT).unwrap();
                            output.flush().unwrap();
                            continue;
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
        use crate::error::CafleetError;
        use crate::runtime::service::test_support::*;
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;

        pub(crate) fn synthetic(
            fixture: &mut crate::runtime::test_support::Fixture,
            spec: &mut crate::runtime::SpawnSpec,
        ) {
            spec.environment.push((
                "CAFLEET_REVIEW_NATIVE_START".into(),
                fixture
                    ._dir
                    .path()
                    .join("native-started")
                    .to_str()
                    .unwrap()
                    .into(),
            ));
            fixture.conn.execute("UPDATE member_runs SET native_version='0.0.1-synthetic',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
            fixture.run =
                crate::broker::runtime::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let script = fixture._dir.path().join("owned-observation-process");
            private_write(&script,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf '0.0.1-synthetic\\n'; exit 0; fi\n/usr/bin/touch \"$CAFLEET_REVIEW_NATIVE_START\"\nprintf 'review-native-ready\\n'\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; form*) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write %s.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: ' \"$line\" ;; transient) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write transient.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \\033[2J\\033[Hworking' ;; *) printf 'review-answer=%s\\n' \"$line\" ;; esac; done\n");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            spec.argv = vec![
                script.to_str().unwrap().into(),
                "--no-daemon".into(),
                "--approve-for-me".into(),
            ];
            let definitions = crate::coding_agent::catalog::test_support::synthetic_definitions();
            private_write(
                &fixture._dir.path().join("observation-services.json"),
                &serde_json::to_vec(&json!({"catalog":definitions})).unwrap(),
            );
        }

        pub(crate) struct Hooks {
            pub(crate) directory: std::path::PathBuf,
        }
        impl WriterHooks for Hooks {
            fn before(
                &self,
                boundary: WriterBoundary,
                bound: &crate::runtime::writer::StepBound,
            ) -> Result<(), CafleetError> {
                if matches!(boundary, WriterBoundary::Flush)
                    && self.directory.join("fail-flush").exists()
                {
                    return Err(CafleetError::DatabaseBusy(
                        "owned synthetic flush busy".into(),
                    ));
                }
                if matches!(boundary, WriterBoundary::Persist)
                    && self.directory.join("fail-persist").exists()
                {
                    return Err(CafleetError::DatabaseBusy(
                        "owned synthetic persist busy".into(),
                    ));
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
                        if bound.check().is_err() {
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
            let catalog = crate::coding_agent::catalog::test_support::load(&value["catalog"])?;
            private_write(&config.parent().unwrap().join("config-selected.json"),&serde_json::to_vec(&json!({"configuration":config,"sha256":sha256_hex(&std::fs::read(config).map_err(io_error)?)})).map_err(io_error)?);
            let diagnostics = crate::runtime::diagnostics::RunnerDiagnostics::default();
            if let Some(seeded) = value.get("seed_diagnostics") {
                let stamp = crate::time::Clock::wall_stamp(&crate::time::SystemClock);
                for index in 0..seeded.as_u64().expect("seed_diagnostics is an entry count") {
                    diagnostics.record(
                        crate::broker::runtime::RunnerDiagnosticSource::QueueHousekeeping,
                        &format!("label: seeded {index:02}"),
                        &stamp,
                    );
                }
            }
            let services = crate::runtime::service::RunnerDependencies {
                catalog: Box::leak(Box::new(catalog)),
                clock: Arc::new(crate::time::SystemClock),
                diagnostics,
                writer_hooks: Arc::new(Hooks {
                    directory: config.parent().unwrap().to_path_buf(),
                }),
                capture: None,
                observation_hooks: crate::runtime::service::test_support::injected_failures::hooks(
                    &value,
                    config.parent().unwrap(),
                )?,
                preview_sweep: match value.get("preview_sweep") {
                    None => false,
                    Some(enabled) => enabled.as_bool().expect("preview_sweep is a boolean"),
                },
            };
            crate::runtime::service::run_service(settings, descriptor, services)
        }
        pub(crate) use crate::runtime::observation::native_capture::test_support::private_write;
        pub(crate) fn wait_until(ready: impl FnMut() -> bool) {
            crate::runtime::writer::test_support::wait_until_within(Duration::from_secs(3), ready);
        }
        /// Every persisted event kind of `run_id`, in event-sequence order.
        pub(crate) fn kinds(conn: &rusqlite::Connection, run_id: i64) -> Vec<String> {
            conn.prepare(
                "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
            )
            .unwrap()
            .query_map([run_id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
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
                .capabilities(crate::coding_agent::CodingAgent::Codex);
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET coverage=?1 WHERE run_id=?2",
                    rusqlite::params![bundled.detection.as_str(), fixture.run.run_id],
                )
                .unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            let evidence = fixture._dir.path().join("synthetic-source.json");
            private_write(
                &evidence,
                b"{\"kind\":\"synthetic\",\"version\":\"0.0.1-synthetic\"}",
            );
            let hash = |path: &Path| sha256_hex(&std::fs::read(path).unwrap());
            let current = std::env::current_exe().unwrap();
            let executable = Path::new(&spec.argv[0]);
            let producer = json!({"schema_version":1,"kind":"synthetic","source_manifest_ref":evidence,"source_manifest_sha256":hash(&evidence),"libtest_executable_path":current,"libtest_executable_sha256":crate::runtime::observation::native_capture::test_support::libtest_digest(),"parser_version":"0.18.1","collector_version":"1","catalog_origin":"bundled_unknown","native_executable":{"path":executable,"sha256":hash(executable),"version":"0.0.1-synthetic","version_evidence_ref":evidence},"launch_evidence_ref":evidence,"environment_evidence_ref":evidence});
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
            let config = crate::runtime::observation::native_capture::NativeCaptureConfig {
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

    pub(crate) mod injected_failures {
        use crate::error::CafleetError;
        use crate::runtime::service::test_support::observation_services::count;
        use crate::runtime::service::test_support::observation_services::health;
        use crate::runtime::service::test_support::observation_services::private_write;
        use crate::runtime::service::test_support::observation_services::synthetic;
        use crate::runtime::service::test_support::*;
        use crate::runtime::writer::test_support::wait_until_within;
        use std::sync::Arc;
        use std::sync::OnceLock;

        pub(crate) const CANARY: &str = "private-observation-fault-canary";

        pub(crate) struct Hooks {
            pub(crate) site: ObservationPhase,
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
                site: ObservationPhase,
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
                Some("child_output") => ObservationPhase::ChildOutput,
                Some("quiet_timer") => ObservationPhase::QuietTimer,
                Some("resize") => ObservationPhase::Resize,
                Some("shutdown_drain") => ObservationPhase::ShutdownDrain,
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
            wait_until_within(Duration::from_secs(30), || path.exists());
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
            wait_until_within(Duration::from_secs(30), || {
                count(fixture, "runner_degraded") >= 2
            });
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

    /// The runtime scenario harness: the runner's transport, observation, event writer and input
    /// coordinator over a scripted native child, fake terminal and temporary database, driven in
    /// the runner loop's turn order on one virtual [`TestClock`].
    pub(crate) mod scenario {
        use std::io;
        use std::sync::Arc;
        use std::time::Duration;
        use std::time::Instant;

        use crate::broker::runtime as storage;
        use crate::coding_agent::CodingAgent;
        use crate::coding_agent::ComposerOperation;
        use crate::coding_agent::ProfileCatalog;
        use crate::coding_agent::catalog::LiteralFraming;
        use crate::db::test_support::FaultPlan;
        use crate::error::CafleetError;
        use crate::runtime::service::InputTurnIo;
        use crate::runtime::service::InputTurnOutcome;
        use crate::runtime::service::InputTurns;
        use crate::runtime::service::SystemRequestTokens;
        use crate::runtime::service::input_deadline;
        use crate::runtime::service::tick_sleep;
        use crate::runtime::test_support::Fixture;
        use crate::runtime::transport::test_support::FakeDisplay;
        use crate::runtime::transport::test_support::FakeInput;
        use crate::runtime::writer::WriterBoundary;
        use crate::runtime::writer::WriterHooks;
        use crate::runtime::writer::test_support::WriterFixture;
        use crate::runtime::writer::test_support::YieldingClock;
        use crate::runtime::*;
        use crate::time::Clock;
        use crate::time::test_support::TestClock;

        /// Every scenario's shutdown unit, so a storage deadline lies 400 ms of virtual time out.
        const SHUTDOWN_UNIT: Duration = Duration::from_millis(100);
        /// The real-time bound on waiting for the event writer's thread.
        const WRITER_WAIT: Duration = Duration::from_secs(10);
        const TURN_LIMIT: usize = 100_000;

        /// One scripted step of a scenario.
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum Step {
            /// The native child draws the bundled fixture screen with this id.
            Screen(&'static str),
            /// The human types these bytes into the outer terminal.
            Type(&'static [u8]),
            /// Runner turns until the virtual clock has advanced by at least this much.
            Run(Duration),
            /// A control client submits this prompt under the operation key.
            Prompt {
                key: &'static str,
                literal: &'static str,
            },
            /// A control client submits this exec under the operation key.
            Exec {
                key: &'static str,
                literal: &'static str,
            },
            /// A control client reconciles the uncertain receipt of the operation key as
            /// submitted.
            Reconcile(&'static str),
            /// The Director sends the member a message with this body.
            Message(&'static str),
            /// The Director sends the member a message the member has already consumed.
            Consumed(&'static str),
            /// Another writer takes the member's pane input lock.
            HoldPane,
            ReleasePane,
            Arm(FaultPlan),
            /// Every event writer persist fails with a transient busy cause while set.
            FailPersist(bool),
        }

        /// A scenario as data: the native screen the child starts on, the screen it redraws after
        /// each submit key, the screen it draws once the bytes written to it contain a marker,
        /// the steps, and the runtime event kinds the run records, in order.
        pub(crate) struct Scenario {
            pub(crate) name: &'static str,
            pub(crate) screen: &'static str,
            pub(crate) redraw: Option<&'static str>,
            pub(crate) interrupt: Option<Interrupt>,
            pub(crate) steps: &'static [Step],
            pub(crate) events: &'static [&'static str],
        }

        /// The native child draws `screen` once, as soon as the bytes written to it contain
        /// `marker`.
        #[derive(Clone, Copy)]
        pub(crate) struct Interrupt {
            pub(crate) marker: &'static str,
            pub(crate) screen: &'static str,
        }

        impl Scenario {
            /// Runs every step and asserts the recorded event kinds, returning the finished runner
            /// for further row and byte assertions.
            pub(crate) fn run(&self) -> Runner {
                self.run_with(|_| {})
            }

            /// Like [`Scenario::run`], with `configure` applied to the started runner first.
            pub(crate) fn run_with(&self, configure: impl FnOnce(&mut Runner)) -> Runner {
                let mut runner = Runner::new(self.screen, self.redraw);
                runner.child.interrupt = self.interrupt.map(|interrupt| {
                    (
                        interrupt.marker.as_bytes().to_vec(),
                        screen_bytes(interrupt.screen),
                    )
                });
                configure(&mut runner);
                runner.settle();
                for step in self.steps {
                    runner.step(*step);
                }
                runner.settle();
                assert_eq!(runner.event_kinds(), self.events, "{}", self.name);
                runner
            }
        }

        #[derive(Default)]
        struct ScenarioWriterHooks {
            fail_persist: std::sync::atomic::AtomicBool,
        }

        impl WriterHooks for ScenarioWriterHooks {
            fn before(
                &self,
                boundary: WriterBoundary,
                _: &crate::runtime::writer::StepBound,
            ) -> Result<()> {
                if matches!(boundary, WriterBoundary::Persist)
                    && self.fail_persist.load(std::sync::atomic::Ordering::Acquire)
                {
                    return Err(CafleetError::DatabaseBusy("scenario persist busy".into()));
                }
                Ok(())
            }
        }

        /// The native child behind the PTY: it draws queued screens, records every byte the
        /// runner writes to it, and draws the `redraw` screen after each write that ends with
        /// its submit key.
        pub(crate) struct NativeChild {
            screens: FakeInput,
            pub(crate) written: Vec<u8>,
            redraw: Option<(Vec<u8>, Vec<u8>)>,
            interrupt: Option<(Vec<u8>, Vec<u8>)>,
        }

        impl NativeChild {
            fn draw(&mut self, screen: Vec<u8>) {
                assert!(!screen.is_empty(), "an empty read is end of file");
                self.screens.chunks.push_back(screen);
            }
        }

        impl ByteInput for NativeChild {
            fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.screens.try_read(bytes)
            }
        }

        impl PtyIo for NativeChild {
            fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.written.extend_from_slice(bytes);
                if self.interrupt.as_ref().is_some_and(|(marker, _)| {
                    self.written
                        .windows(marker.len())
                        .any(|window| window == marker.as_slice())
                }) {
                    let (_, screen) = self.interrupt.take().expect("the armed interrupt");
                    self.screens.chunks.push_back(screen);
                } else if let Some((submit, screen)) = &self.redraw
                    && self.written.ends_with(submit)
                {
                    self.screens.chunks.push_back(screen.clone());
                }
                Ok(bytes.len())
            }
            fn resize(&mut self, _: TerminalSize) -> Result<()> {
                Ok(())
            }
            fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }

        pub(crate) fn screen_bytes(id: &str) -> Vec<u8> {
            use crate::coding_agent::detector::test_support::bundled_screen;
            use crate::coding_agent::detector::test_support::native_fixture_bytes;
            native_fixture_bytes(&bundled_screen(id))
        }

        struct NativeScreen {
            backend: CodingAgent,
            version: String,
            size: TerminalSize,
        }

        fn native_screen(id: &str) -> NativeScreen {
            let (backend, file) = crate::coding_agent::catalog::test_support::bundled_file(id);
            let screen = &file["screens"][id];
            let dimension = |key: &str| u16::try_from(screen[key].as_u64().unwrap()).unwrap();
            NativeScreen {
                backend,
                version: screen["native_version"].as_str().unwrap().into(),
                size: TerminalSize {
                    rows: dimension("rows"),
                    cols: dimension("cols"),
                },
            }
        }

        /// The runner I/O the production input turns drive: the scenario's fake terminal,
        /// scripted child and observation, with no termination, resize or native exit.
        struct ScenarioIo<'a> {
            clock: &'a TestClock,
            controller: &'a mut ObservationController,
            writer: &'a EventWriter,
            transport: &'a mut Transport,
            human: &'a mut FakeInput,
            child: &'a mut NativeChild,
            display: &'a mut FakeDisplay,
        }

        impl InputTurnIo for ScenarioIo<'_> {
            fn clock(&self) -> &dyn Clock {
                self.clock
            }
            fn controller(&self) -> &ObservationController {
                self.controller
            }
            fn quiesce(
                &mut self,
                coordinator: &mut InputCoordinator,
                deadline: Instant,
            ) -> std::result::Result<InputDrain, InputFailure> {
                coordinator.progress_with_manual(self.transport, |transport| {
                    transport.quiesce_for_input(
                        self.human,
                        self.child,
                        self.display,
                        &mut self.controller.observer(self.writer, self.clock),
                        deadline,
                    )
                })
            }
            fn advance(
                &mut self,
                coordinator: &mut InputCoordinator,
                conn: &mut rusqlite::Connection,
                deadline: Instant,
            ) -> Result<Option<InputControlReply>> {
                coordinator
                    .advance(conn, self.transport, self.child, self.controller, deadline)
                    .map_err(CafleetError::from)
            }
            fn terminate_requested(&self) -> bool {
                false
            }
            fn resize_requested(&self) -> bool {
                false
            }
            fn shutdown_deadlines(&self) -> ShutdownDeadlines {
                ShutdownDeadlines::starting_now(self.clock)
            }
            fn observe_exit(
                &mut self,
                _: &mut InputCoordinator,
                _: &mut rusqlite::Connection,
            ) -> Result<Option<ShutdownDeadlines>> {
                Ok(None)
            }
        }

        pub(crate) struct Runner {
            pub(crate) clock: Arc<TestClock>,
            pub(crate) fixture: Fixture,
            _artifacts: RunArtifacts,
            pub(crate) writer: EventWriter,
            pub(crate) controller: ObservationController,
            pub(crate) transport: Transport,
            pub(crate) coordinator: InputCoordinator,
            pub(crate) human: FakeInput,
            pub(crate) child: NativeChild,
            pub(crate) display: FakeDisplay,
            pub(crate) replies: Vec<std::result::Result<InputControlReply, CafleetError>>,
            pub(crate) held_pane: Option<crate::pane_lock::PaneLock>,
            writer_hooks: Arc<ScenarioWriterHooks>,
            turns: InputTurns,
            finished: bool,
        }

        impl Runner {
            /// A started run of the backend and version that recorded `screen`, at its size, with
            /// the child showing `screen`.
            pub(crate) fn new(screen: &str, redraw: Option<&str>) -> Self {
                let catalog = ProfileCatalog::bundled();
                let native = native_screen(screen);
                let clock = Arc::new(
                    TestClock::starting_at(
                        Instant::now() + crate::runtime::test_support::VIRTUAL_LEAD,
                    )
                    .with_shutdown_unit(SHUTDOWN_UNIT),
                );
                let WriterFixture {
                    mut fixture,
                    artifacts,
                    lock,
                } = WriterFixture::new();
                let coverage = catalog.capabilities(native.backend).detection;
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET backend=?1,native_version=?2,coverage=?3 WHERE run_id=?4",
                        rusqlite::params![
                            native.backend.name(),
                            native.version,
                            coverage.as_str(),
                            fixture.run.run_id
                        ],
                    )
                    .unwrap();
                fixture
                    .conn
                    .execute(
                        "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                        rusqlite::params![native.backend.name(), fixture.run.member_id],
                    )
                    .unwrap();
                fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                let mut controller = ObservationController::new(&fixture.run, catalog).unwrap();
                let writer_hooks = Arc::new(ScenarioWriterHooks::default());
                let writer = EventWriter::start_with_hooks(
                    format!("sqlite:///{}", fixture.conn.path().unwrap()),
                    fixture.run.clone(),
                    lock.clone(),
                    Arc::new(YieldingClock(clock.clone())),
                    writer_hooks.clone(),
                    crate::runtime::diagnostics::RunnerDiagnostics::default(),
                )
                .unwrap();
                let child = ProcessIdentity::current().unwrap();
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",
                        rusqlite::params![
                            child.pid,
                            child.start_identity,
                            clock.wall_stamp(),
                            fixture.run.run_id
                        ],
                    )
                    .unwrap();
                fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
                let transport = Transport::new(
                    native.size,
                    TransportLimits {
                        input_bytes: 65536,
                        display_bytes: 262144,
                        bytes_per_tick: 16384,
                    },
                    clock.clone(),
                )
                .unwrap();
                controller
                    .started(&fixture.run, &transport.snapshot(), &writer, clock.as_ref())
                    .unwrap();
                let coordinator = InputCoordinator::new(
                    &mut fixture.conn,
                    &fixture.run,
                    lock,
                    InputServices {
                        catalog: catalog.clone(),
                        token_source: Box::new(SystemRequestTokens),
                        clock: clock.clone(),
                        stage_settle: crate::runtime::input::STAGE_SETTLE,
                        hooks: crate::runtime::input::test_support::default_input_hooks(),
                    },
                    clock.now() + Duration::from_secs(10),
                )
                .unwrap();
                let redraw = redraw.map(|id| {
                    let plan = catalog
                        .composer_plan(native.backend, ComposerOperation::Prompt)
                        .expect("a redraw after submit needs the profile's prompt plan");
                    let submit = plan
                        .stages("x", LiteralFraming::Plain)
                        .unwrap()
                        .into_iter()
                        .last()
                        .expect("a prompt plan ends with its submit key");
                    (submit, screen_bytes(id))
                });
                let mut child = NativeChild {
                    screens: FakeInput::default(),
                    written: Vec::new(),
                    redraw,
                    interrupt: None,
                };
                child.draw(screen_bytes(screen));
                Self {
                    clock,
                    fixture,
                    _artifacts: artifacts,
                    writer,
                    controller,
                    transport,
                    coordinator,
                    human: FakeInput::default(),
                    child,
                    display: FakeDisplay::default(),
                    replies: Vec::new(),
                    held_pane: None,
                    writer_hooks,
                    turns: InputTurns::new(),
                    finished: false,
                }
            }

            pub(crate) fn step(&mut self, step: Step) {
                match step {
                    Step::Screen(id) => self.child.draw(screen_bytes(id)),
                    Step::Type(bytes) => {
                        assert!(!bytes.is_empty(), "an empty read is end of file");
                        self.human.chunks.push_back(bytes.to_vec());
                    }
                    Step::Run(duration) => self.run_for(duration),
                    Step::Prompt { key, literal } => {
                        self.submit(InputControlOperation::SubmitPrompt(SubmitInput {
                            operation_key: key.into(),
                            input_kind: storage::InputKind::Prompt,
                            literal: literal.into(),
                        }))
                    }
                    Step::Exec { key, literal } => {
                        self.submit(InputControlOperation::SubmitExec(SubmitInput {
                            operation_key: key.into(),
                            input_kind: storage::InputKind::Exec,
                            literal: literal.into(),
                        }))
                    }
                    Step::Reconcile(key) => {
                        let input =
                            storage::find_input(&self.fixture.conn, self.fixture.run.run_id, key)
                                .unwrap()
                                .unwrap_or_else(|| panic!("{key} has a receipt"));
                        self.submit(InputControlOperation::Receipt(ReceiptAction::Reconcile {
                            input_id: input.input_id,
                            expected: storage::InputState::Uncertain,
                            disposition: storage::InputDisposition::Submitted,
                            evidence_ref: "audit:pane-shows-the-input".into(),
                        }))
                    }
                    Step::Message(body) => {
                        self.send(body);
                    }
                    Step::Consumed(body) => {
                        let message = self.send(body);
                        self.fixture
                            .conn
                            .execute(
                                "UPDATE messages SET status_state='completed' WHERE message_id=?1",
                                [message.message_id],
                            )
                            .unwrap();
                    }
                    Step::HoldPane => {
                        assert!(self.held_pane.is_none(), "one held pane lock");
                        let url = format!("sqlite:///{}", self.fixture.conn.path().unwrap());
                        let placement = crate::broker::get_member(
                            &self.fixture.conn,
                            self.fixture.run.member_id,
                            self.fixture.run.fleet_id,
                        )
                        .unwrap()
                        .unwrap()
                        .placement;
                        self.held_pane = Some(
                            crate::pane_lock::PaneLock::try_acquire(&url, &placement)
                                .unwrap()
                                .expect("the member's pane input lock is free"),
                        );
                    }
                    Step::ReleasePane => {
                        self.held_pane.take().expect("a held pane lock");
                    }
                    Step::Arm(plan) => plan.arm(&mut self.fixture.conn),
                    Step::FailPersist(failing) => self
                        .writer_hooks
                        .fail_persist
                        .store(failing, std::sync::atomic::Ordering::Release),
                }
            }

            /// Turns until every queued screen and typed byte is consumed and nothing more moves.
            pub(crate) fn settle(&mut self) {
                for _ in 0..TURN_LIMIT {
                    let start = self.clock.elapsed();
                    self.turn();
                    if self.child.screens.chunks.is_empty()
                        && self.human.chunks.is_empty()
                        && self.turns.idle()
                        && self.clock.elapsed() > start
                    {
                        return;
                    }
                }
                panic!("the scenario settles within {TURN_LIMIT} turns");
            }

            pub(crate) fn run_for(&mut self, duration: Duration) {
                let until = self.clock.elapsed() + duration;
                for _ in 0..TURN_LIMIT {
                    if self.clock.elapsed() >= until {
                        return;
                    }
                    self.skip_idle_turns(self.clock.now() + (until - self.clock.elapsed()));
                    self.turn();
                }
                panic!("{duration:?} of virtual time passes within {TURN_LIMIT} turns");
            }

            /// One runner loop turn of a started runner: the transport tick and observation poll,
            /// the production input turns, then the loop's idle sleep.
            pub(crate) fn turn(&mut self) {
                self.await_writer();
                let clock = self.clock.clone();
                let progress = self
                    .coordinator
                    .progress_with_manual(&mut self.transport, |transport| {
                        transport.tick(
                            &mut self.human,
                            &mut self.child,
                            &mut self.display,
                            &mut self.controller.observer(&self.writer, clock.as_ref()),
                        )
                    })
                    .unwrap();
                self.controller
                    .poll(&self.transport.snapshot(), &self.writer, clock.as_ref())
                    .unwrap();
                let mut io = ScenarioIo {
                    clock: clock.as_ref(),
                    controller: &mut self.controller,
                    writer: &self.writer,
                    transport: &mut self.transport,
                    human: &mut self.human,
                    child: &mut self.child,
                    display: &mut self.display,
                };
                match self
                    .turns
                    .turn(
                        &mut self.coordinator,
                        &mut self.fixture.conn,
                        &self.fixture.run,
                        &mut io,
                    )
                    .unwrap()
                {
                    InputTurnOutcome::Continue => {}
                    InputTurnOutcome::Reply { result, .. } => self.replies.push(result),
                    InputTurnOutcome::Shutdown(_) => {
                        unreachable!("a scenario runner requests no shutdown")
                    }
                }
                self.await_writer();
                let pause = tick_sleep(
                    &progress,
                    self.controller.status().next_deadline,
                    clock.now(),
                );
                if !pause.is_zero() {
                    clock.sleep(pause);
                }
            }

            /// Moves the clock to the next turn that can do work, or to `limit`, while nothing is
            /// in flight: the turns in between read no bytes and reach no timer.
            fn skip_idle_turns(&mut self, limit: Instant) {
                let idle = self.turns.idle()
                    && !self.coordinator.has_deferred_work()
                    && self.child.screens.chunks.is_empty()
                    && self.human.chunks.is_empty()
                    && self.transport.pending_input_bytes() == 0
                    && !self.controller.has_pending();
                if !idle {
                    return;
                }
                let now = self.clock.now();
                let wake = self
                    .turns
                    .wakes(now)
                    .into_iter()
                    .chain([self.controller.status().next_deadline])
                    .flatten()
                    .chain([limit])
                    .min()
                    .expect("the limit bounds every wake");
                if wake > now {
                    self.clock.advance(wake - now);
                }
            }

            /// Waits in real time until the writer has persisted every accepted event or reported
            /// a failure, so each turn observes a settled sink.
            fn await_writer(&self) {
                let until = Instant::now() + WRITER_WAIT;
                loop {
                    let status = self.writer.status();
                    if status.pending_count == 0 || status.diagnostic.is_some() {
                        return;
                    }
                    assert!(
                        Instant::now() < until,
                        "the event writer settles: {status:?}"
                    );
                    std::thread::yield_now();
                }
            }

            /// Admits a control request the way the runner serves an input client.
            pub(crate) fn submit(&mut self, operation: InputControlOperation) {
                let expires = self.clock.now() + Duration::from_secs(30);
                let request = InputControlRequest {
                    identity: ControlIdentity::from_run(&self.fixture.run),
                    expires_at_mono_ns: InputExpiration::capture(expires, self.clock.as_ref())
                        .unwrap()
                        .monotonic_ns(),
                    operation,
                };
                let deadline =
                    input_deadline(&request, self.clock.now(), self.clock.as_ref()).unwrap();
                match self.coordinator.handle_received(
                    &mut self.fixture.conn,
                    request,
                    &self.controller,
                    deadline,
                ) {
                    Ok(Some(reply)) => self.replies.push(Ok(reply)),
                    Ok(None) => self.turns.admit_client(0, deadline).unwrap(),
                    Err(failure) => self.replies.push(Err(CafleetError::from(failure))),
                }
            }

            fn send(&mut self, body: &str) -> crate::broker::MessageRecord {
                let run = &self.fixture.run;
                let (director, member) = (run.director_member_id, run.member_id);
                crate::broker::send_message(&mut self.fixture.conn, director, member, body).unwrap()
            }

            /// The preview line the sweep types for the message with this body.
            pub(crate) fn preview_line(&self, body: &str) -> String {
                let message = self.message(body);
                crate::runtime::preview::canonical_preview_line(&message)
            }

            pub(crate) fn message(&self, body: &str) -> crate::broker::MessageRecord {
                let id: i64 = self
                    .fixture
                    .conn
                    .query_row(
                        "SELECT message_id FROM messages WHERE text=?1",
                        [body],
                        |row| row.get(0),
                    )
                    .unwrap_or_else(|cause| panic!("message {body:?}: {cause}"));
                crate::broker::get_message(&self.fixture.conn, id).unwrap()
            }

            /// How many times the child received `text`.
            pub(crate) fn typed(&self, text: &str) -> usize {
                String::from_utf8_lossy(&self.child.written)
                    .matches(text)
                    .count()
            }

            /// The byte offset of the first `text` the child received.
            pub(crate) fn typed_at(&self, text: &str) -> usize {
                String::from_utf8_lossy(&self.child.written)
                    .find(text)
                    .unwrap_or_else(|| panic!("the child received {text:?}"))
            }

            pub(crate) fn notified(&self, body: &str) -> bool {
                self.fixture
                    .conn
                    .query_row(
                        "SELECT notified_at IS NOT NULL FROM messages WHERE text=?1",
                        [body],
                        |row| row.get(0),
                    )
                    .unwrap()
            }

            pub(crate) fn payloads(&self, kind: &str) -> Vec<serde_json::Value> {
                self.fixture
                    .conn
                    .prepare(
                        "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind=?2 ORDER BY event_seq",
                    )
                    .unwrap()
                    .query_map(rusqlite::params![self.fixture.run.run_id, kind], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap()
                    .map(|text| serde_json::from_str(&text.unwrap()).unwrap())
                    .collect()
            }

            pub(crate) fn event_kinds(&self) -> Vec<String> {
                self.fixture
                    .conn
                    .prepare(
                        "SELECT kind FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
                    )
                    .unwrap()
                    .query_map([self.fixture.run.run_id], |row| row.get(0))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap()
            }

            /// When the run-wide sweep backoff next allows a sweep, once a failed attempt armed it.
            pub(crate) fn sweep_backoff_until(&self) -> Option<Instant> {
                self.turns.preview_sweep.backoff_until
            }

            pub(crate) fn eligibility(&self) -> InputEligibility {
                self.controller.health(&self.writer).eligibility
            }

            /// Stops the writer at the shutdown storage deadline and returns its unsaved
            /// `(pending, gap)` counts; a writer that cannot persist is abandoned at that deadline.
            pub(crate) fn finish(&mut self) -> (u64, u64) {
                self.finished = true;
                self.await_writer();
                let until = ShutdownDeadlines::starting_now(self.clock.as_ref()).storage_deadline();
                self.writer.begin_shutdown(until);
                if self.writer.status().pending_count > 0 {
                    self.clock
                        .advance(until.saturating_duration_since(self.clock.now()));
                }
                self.writer.finish(until)
            }
        }

        impl Drop for Runner {
            fn drop(&mut self) {
                if !self.finished && !std::thread::panicking() {
                    self.finish();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::Shutdown;
    use std::panic::AssertUnwindSafe;
    use std::panic::catch_unwind;
    use std::panic::resume_unwind;
    use std::path::Path;
    use std::time::Duration;
    use std::time::Instant;

    use serde_json::Value;
    use serde_json::json;

    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::config::Settings;
    use crate::runtime::service::test_support::*;
    use crate::runtime::*;

    pub(super) mod control_service {
        use crate::runtime::service::tests::*;
        pub(crate) fn service_child_path() -> &'static str {
            test_path!(service_child)
        }

        #[test]
        fn service_child() {
            if let Some(config) =
                std::env::var_os(crate::runtime::service::test_support::input_service::NATIVE_ENV)
            {
                crate::runtime::service::test_support::input_service::native(Path::new(&config));
                std::process::exit(0);
            }
            let Some(descriptor) = std::env::var_os(DESCRIPTOR_ENV) else {
                return;
            };
            if let Some(configured) = std::env::var_os(SERVICES_ENV) {
                let directory = Path::new(&configured).parent().unwrap();
                if directory.join("service-input-native.json").exists() {
                    crate::runtime::service::test_support::input_service::install_panic_record(
                        directory, "runner",
                    );
                }
            }
            let database_url = std::env::var(DATABASE_ENV).unwrap();
            let settings =
                Settings::for_database(Path::new(crate::db::database_path(&database_url).unwrap()));
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
                    eprintln!("service child runner failed: {cause}");
                    std::process::exit(1);
                }
            }
        }
        #[test]
        fn control_invalid_protocol_versions_leave_the_running_child_available() {
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
            for stale in ["nonce", "epoch"] {
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
                assert_eq!(
                    fixture.health().child,
                    Some(started.child.clone()),
                    "{stale}"
                );
            }
            assert_rejected(&fixture.start());
            assert_eq!(fixture.health().child, Some(started.child));
            fixture.assert_manual_io();
            fixture.finish();
        }

        #[test]
        fn control_noncreator_start_is_isolated_before_and_after_spawn() {
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
        fn control_disconnected_reply_preserves_health_and_manual_io() {
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
        fn control_creator_disconnect_before_start_exits_without_spawning() {
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
        fn control_accepted_start_spawn_and_storage_failures_compensate() {
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

        #[test]
        fn control_shutdown_outcome_diagnostics_are_stored_as_shutdown_rows() {
            let mut fixture = ServiceFixture::new(false);
            fixture.prepare();
            fixture.publish();
            fixture.fixture.conn.execute_batch("CREATE TRIGGER reject_review_running BEFORE UPDATE OF status ON member_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'fixture running persistence blocked'); END").unwrap();
            assert_rejected(&fixture.start());
            assert_eq!(fixture.wait().code(), Some(1));
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            let outcome: Vec<String> =
                serde_json::from_value(run.cleanup_diagnostics_json.clone().unwrap()).unwrap();
            assert!(!outcome.is_empty(), "the failed start reports its cause");
            let rows: Vec<(String, String, i64)> = fixture
                .fixture
                .conn
                .prepare("SELECT source,message,count FROM member_runner_diagnostics WHERE run_id=?1 AND source='shutdown' ORDER BY diagnostic_id")
                .unwrap()
                .query_map([run.run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            for diagnostic in &outcome {
                let sanitized = crate::broker::runtime::diagnostic_text(diagnostic);
                assert!(
                    rows.iter()
                        .any(|(_, message, count)| message.contains(&sanitized) && *count > 0),
                    "{diagnostic:?} has a shutdown row: {rows:?}"
                );
            }
        }

        const SEEDED: u64 = 40;

        fn seeded_unflushable(
            fixture: &mut crate::runtime::test_support::Fixture,
            spec: &mut crate::runtime::SpawnSpec,
        ) {
            crate::runtime::service::test_support::observation_services::synthetic(fixture, spec);
            let path = fixture._dir.path().join("observation-services.json");
            let mut config: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            config["seed_diagnostics"] = serde_json::json!(SEEDED);
            std::fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
            crate::runtime::service::test_support::observation_services::private_write(
                &fixture._dir.path().join("fail-flush"),
                b"owned test gate",
            );
        }

        #[test]
        fn control_shutdown_writes_still_buffered_diagnostics_and_the_overflow_row() {
            let mut fixture = ServiceFixture::configured(false, seeded_unflushable);
            fixture.prepare();
            fixture.publish();
            fixture.fixture.conn.execute_batch("CREATE TRIGGER reject_review_running BEFORE UPDATE OF status ON member_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'fixture running persistence blocked'); END").unwrap();
            assert_rejected(&fixture.start());
            assert_eq!(fixture.wait().code(), Some(1));
            let run_id = fixture.fixture.run.run_id;
            let rows: Vec<(String, String, i64)> = fixture
                .fixture
                .conn
                .prepare("SELECT source,message,count FROM member_runner_diagnostics WHERE run_id=?1 ORDER BY diagnostic_id")
                .unwrap()
                .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let seeded = |index: u64| format!("label: seeded {index:02}");
            assert!(
                rows.iter()
                    .any(|(source, message, _)| source == "queue_housekeeping"
                        && *message == seeded(SEEDED - 1)),
                "the newest buffered entry is written at shutdown: {rows:?}"
            );
            for evicted in 0..8 {
                assert!(
                    !rows
                        .iter()
                        .any(|(_, message, _)| *message == seeded(evicted)),
                    "seeded entry {evicted} was evicted from the 32-entry buffer: {rows:?}"
                );
            }
            let overflow: Vec<u64> = rows
                .iter()
                .filter(|(source, _, _)| source == "event_writer")
                .filter_map(|(_, message, _)| {
                    message
                        .strip_prefix("diagnostic buffer overflow: ")?
                        .strip_suffix(" entries dropped")?
                        .parse()
                        .ok()
                })
                .collect();
            let [dropped] = overflow.as_slice() else {
                panic!("one overflow row: {rows:?}");
            };
            assert!(*dropped >= 8, "{rows:?}");
            assert!(
                rows.iter().any(|(source, _, _)| source == "shutdown"),
                "the outcome diagnostics are written alongside: {rows:?}"
            );
            assert!(rows.len() <= 256);
        }
    }

    mod input_service {
        use crate::runtime::service::test_support::input_service::*;
        use crate::runtime::service::tests::*;
        #[test]
        fn service_input_socket_submission_persists_typed_outcome_and_matching_key_never_replays() {
            with_input_service(|fixture, until| {
                let key = "prompt:service:single";
                let literal = "socket prompt λ";
                let value = request(
                    fixture,
                    key,
                    literal,
                    InputKind::Prompt,
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
        fn service_input_queued_expiration_keeps_manual_progress_and_uses_fresh_output_for_exec() {
            with_input_service(|fixture, until| {
                let before = input_service_composer(fixture, until).output_seq;
                let expires = Instant::now() + Duration::from_millis(800);
                let value = request(
                    fixture,
                    "exec:service:expired",
                    "expired command",
                    InputKind::Exec,
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
        fn service_input_expired_unreserved_attempt_releases_pending_work_for_a_fresh_key() {
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
                    until.min(Instant::now() + Duration::from_secs(3)),
                );
                let reply = exchange(fixture, &fresh, until);
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
        fn service_input_post_write_failure_preserves_known_identity_and_original_history() {
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
                assert_eq!(original.state, InputState::Uncertain);
                assert_eq!(original.stop_cause, Some(StopCause::WriteFailed));
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

        fn queue_behind_the_form(fixture: &mut ServiceFixture, until: Instant) -> i64 {
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            manual(fixture, format!("{FORM_LINE}\r").as_bytes(), until);
            loop {
                assert!(Instant::now() < until, "the detected interaction opens");
                let health = NativeInputHost.health(&run, until).unwrap();
                let observation = health.observation.as_ref().unwrap();
                if observation.eligibility == InputEligibility::Hold(GateReason::Interaction)
                    && observation.pending_count == 0
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let value = request(
                fixture,
                "prompt:service:stopped",
                "queued at shutdown",
                InputKind::Prompt,
                until.min(Instant::now() + Duration::from_secs(3)),
            );
            let reply = exchange(fixture, &value, until);
            assert!(reply.get("error").is_none(), "{reply}");
            let result: SubmissionResult =
                serde_json::from_value(reply["result"]["submission"].clone()).unwrap();
            assert_eq!(result.status, SubmissionStatus::Queued);
            assert_eq!(result.stage, SubmissionStage::ZeroBytes);
            result.input_id.expect("queued work has a durable input ID")
        }

        /// The run's result events for `input_id` and its exit event, in event order.
        fn results_then_exit(
            fixture: &ServiceFixture,
            input_id: i64,
        ) -> Vec<(String, Option<String>)> {
            fixture
                .fixture
                .conn
                .prepare(
                    "SELECT kind,json_extract(payload_json,'$.delivery.reason') FROM member_runtime_events WHERE run_id=?1 AND (kind='runner_exited' OR (kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2)) ORDER BY event_seq",
                )
                .unwrap()
                .query_map(
                    rusqlite::params![fixture.fixture.run.run_id, input_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }

        fn assert_stopped_result_before_exit(fixture: &ServiceFixture, input_id: i64) {
            assert_eq!(
                results_then_exit(fixture, input_id),
                [
                    (
                        "input_delivery_result".to_owned(),
                        Some("runner_stopped".to_owned())
                    ),
                    ("runner_exited".to_owned(), None),
                ]
            );
            let input = storage::get_input(&fixture.fixture.conn, input_id).unwrap();
            assert_eq!(input.disposition, Some(InputDisposition::NotSubmitted));
            assert_eq!(input.delivery_reason.as_deref(), Some("runner_stopped"));
            assert_eq!(input.stop_cause, None);
            let published_cause: Option<String> = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT json_extract(payload_json,'$.cause') FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2",
                    rusqlite::params![fixture.fixture.run.run_id, input_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                published_cause, None,
                "a not-submitted result publishes no uncertain cause"
            );
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            assert_eq!(run.status, RunStatus::Exited);
            assert_eq!(
                run.cleanup_state,
                CleanupState::Complete,
                "{:?}",
                run.cleanup_diagnostics_json
            );
        }

        #[test]
        fn service_native_exit_publishes_queued_results_as_runner_stopped_before_runner_exited() {
            with_detecting_input_service(|fixture, until| {
                let id = queue_behind_the_form(fixture, until);
                manual(fixture, b"finish\r", until);
                assert_eq!(fixture.wait().code(), Some(0));
                assert_stopped_result_before_exit(fixture, id);
            });
        }

        #[test]
        fn service_shutdown_publishes_queued_results_as_runner_stopped_before_runner_exited() {
            with_detecting_input_service_exiting(143, |fixture, until| {
                let id = queue_behind_the_form(fixture, until);
                let outcome =
                    crate::runtime::service::test_support::injected_failures::shutdown_reply(
                        fixture,
                    );
                assert_eq!(
                    outcome.cleanup_state,
                    CleanupState::Complete,
                    "{:?}",
                    outcome.diagnostics
                );
                assert_eq!(
                    fixture.wait().code(),
                    Some(143),
                    "the terminated native exit is preserved"
                );
                assert_stopped_result_before_exit(fixture, id);
            });
        }
    }

    mod preview_sweep {
        use crate::broker::MessageRecord;
        use crate::runtime::service::SweepBackoff;
        use crate::runtime::service::test_support::input_service::*;
        use crate::runtime::service::tests::*;

        #[test]
        fn the_run_wide_backoff_starts_at_one_second_doubles_to_thirty_and_resets() {
            let mut backoff = SweepBackoff::default();
            assert_eq!(backoff.delay(), Duration::ZERO);
            let mut delays = Vec::new();
            for _ in 0..8 {
                backoff.transient();
                delays.push(backoff.delay().as_secs());
            }
            assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
            backoff.reset();
            assert_eq!(backoff.delay(), Duration::ZERO);
            backoff.transient();
            assert_eq!(backoff.delay(), Duration::from_secs(1));
        }

        #[test]
        fn the_sweep_interval_is_one_second() {
            assert_eq!(
                crate::runtime::service::PREVIEW_SWEEP_INTERVAL,
                Duration::from_secs(1)
            );
        }

        const RESUME_CLAUSE: &str = "[cafleet] Resume your work if something was still running.";
        /// Longer than two sweep intervals, so a sweep that could deliver would have done so.
        const SWEEP_WINDOW: Duration = Duration::from_millis(2500);

        fn send(fixture: &mut ServiceFixture, body: &str) -> MessageRecord {
            let run = &fixture.fixture.run;
            let (director, member) = (run.director_member_id, run.member_id);
            crate::broker::send_message(&mut fixture.fixture.conn, director, member, body).unwrap()
        }

        /// The canonical preview line of a message whose body needs no folding.
        fn line(message: &MessageRecord) -> String {
            format!(
                "[cafleet msg {} from {} {}] {} {RESUME_CLAUSE}",
                message.message_id, message.from_member_id, message.created_at, message.text
            )
        }

        fn notified_at(fixture: &ServiceFixture, message_id: i64) -> Option<String> {
            fixture
                .fixture
                .conn
                .query_row(
                    "SELECT notified_at FROM messages WHERE message_id=?1",
                    [message_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        fn lines(fixture: &ServiceFixture) -> Vec<String> {
            input_service_records(fixture)
                .iter()
                .map(|record| record["line"].as_str().unwrap().to_owned())
                .collect()
        }

        fn typed_count(fixture: &ServiceFixture, text: &str) -> usize {
            lines(fixture)
                .iter()
                .filter(|typed| typed.contains(text))
                .count()
        }

        fn wait_for(until: Instant, what: &str, mut done: impl FnMut() -> bool) {
            while !done() {
                assert!(Instant::now() < until, "{what}");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// The receipt-free delivery results the run published for `message_id`.
        fn preview_results(fixture: &ServiceFixture, message_id: i64) -> Vec<Value> {
            fixture
                .fixture
                .conn
                .prepare(
                    "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.message_id')=?2 AND json_type(payload_json,'$.input_id')='null' ORDER BY event_seq",
                )
                .unwrap()
                .query_map(
                    rusqlite::params![fixture.fixture.run.run_id, message_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
                .map(|text| serde_json::from_str(&text.unwrap()).unwrap())
                .collect()
        }

        /// Types `barrier` and waits for its journal line, which also carries any literal the
        /// native child still holds unsubmitted; the child then restores the empty composer.
        fn clear_with_barrier(fixture: &mut ServiceFixture, until: Instant) {
            manual(fixture, b"barrier\r", until);
            wait_for(until, "the barrier line is journaled", || {
                lines(fixture)
                    .iter()
                    .any(|typed| typed.ends_with("barrier"))
            });
            input_service_settled(fixture, until);
        }

        fn submit(
            fixture: &ServiceFixture,
            key: &str,
            literal: &str,
            kind: InputKind,
            until: Instant,
        ) -> (SubmissionResult, Value) {
            let value = request(fixture, key, literal, kind, until);
            let reply = exchange(fixture, &value, until);
            assert!(reply.get("error").is_none(), "{reply}");
            let result = serde_json::from_value(reply["result"]["submission"].clone()).unwrap();
            (result, reply)
        }

        fn reconcile(fixture: &ServiceFixture, input_id: i64, until: Instant) {
            let value = json!({"protocol_version":1,"kind":"receipt",
                "identity":fixture.prepared.as_ref().unwrap(),"expires_at_mono_ns":expiration(until),
                "action":"reconcile","input_id":input_id,"expected_state":"uncertain",
                "disposition":"submitted","evidence_ref":"audit:pane-shows-the-input"});
            let reply = exchange(fixture, &value, until);
            assert!(reply.get("error").is_none(), "{reply}");
        }

        fn without_composer_verification(catalog: &mut Value) {
            catalog["codex"]["profile"]["capabilities"]["composer"] = Value::Null;
        }

        #[test]
        fn an_unverified_run_stamps_an_uncertain_preview_before_publishing_it_and_never_retypes_it()
        {
            with_sweeping_input_service_catalog(
                Duration::from_secs(30),
                true,
                without_composer_verification,
                |fixture, until| {
                    let message = send(fixture, &format!("unverified preview {FORM_MID_WRITE}"));
                    wait_for(until, "the stopped preview publishes its result", || {
                        !preview_results(fixture, message.message_id).is_empty()
                    });
                    assert!(
                        notified_at(fixture, message.message_id).is_some(),
                        "the stamp precedes the published result"
                    );
                    clear_with_barrier(fixture, until);
                    std::thread::sleep(SWEEP_WINDOW);
                    assert_eq!(
                        typed_count(fixture, &line(&message)),
                        1,
                        "{:?}",
                        lines(fixture)
                    );
                    assert_eq!(preview_results(fixture, message.message_id).len(), 1);
                },
            );
        }

        /// Stops a prompt mid-write with a detected form, clears the form, then checks that
        /// neither a queued exec nor a sweep preview writes a byte until the prompt is
        /// reconciled.
        fn uncertain_prompt_stops_the_fifo_and_the_sweep(
            fixture: &mut ServiceFixture,
            until: Instant,
        ) {
            let (stopped, stopped_reply) = submit(
                fixture,
                "prompt:stopped-mid-write",
                &format!("stopped prompt {FORM_MID_WRITE}"),
                InputKind::Prompt,
                until.min(Instant::now() + Duration::from_secs(10)),
            );
            assert_eq!(
                stopped.status,
                SubmissionStatus::Uncertain,
                "{stopped_reply}"
            );
            clear_with_barrier(fixture, until);
            let (queued, queued_reply) = submit(
                fixture,
                "exec:behind-the-uncertain-prompt",
                "printf behind",
                InputKind::Exec,
                until.min(Instant::now() + Duration::from_secs(10)),
            );
            assert_eq!(queued.status, SubmissionStatus::Queued, "{queued_reply}");
            let message = send(fixture, "preview behind the uncertain prompt");
            std::thread::sleep(SWEEP_WINDOW);
            let before = lines(fixture);
            assert_eq!(
                before.len(),
                1,
                "only the stopped prompt's line: {before:?}"
            );
            assert_eq!(notified_at(fixture, message.message_id), None);
            reconcile(
                fixture,
                stopped.input_id.expect("the stopped prompt has a receipt"),
                until,
            );
            wait_for(until, "the queued exec and the preview drain", || {
                let typed = lines(fixture);
                typed.iter().any(|typed| typed.ends_with("printf behind"))
                    && typed.iter().any(|typed| typed.contains(&line(&message)))
            });
            let typed = lines(fixture);
            let exec = typed
                .iter()
                .position(|typed| typed.ends_with("printf behind"))
                .unwrap();
            let preview = typed
                .iter()
                .position(|typed| typed.contains(&line(&message)))
                .unwrap();
            assert!(
                exec < preview,
                "the FIFO drains before the sweep: {typed:?}"
            );
        }

        #[test]
        fn an_uncertain_prompt_stops_queued_input_and_the_sweep_until_reconciled_on_an_unverified_run()
         {
            with_sweeping_input_service_catalog(
                Duration::from_secs(40),
                true,
                without_composer_verification,
                uncertain_prompt_stops_the_fifo_and_the_sweep,
            );
        }
    }

    mod observation_contract {
        use crate::runtime::service::test_support::observation_services::count;
        use crate::runtime::service::test_support::observation_services::health;
        use crate::runtime::service::test_support::observation_services::private_write;
        use crate::runtime::service::test_support::observation_services::synthetic;
        use crate::runtime::service::test_support::observation_services::wait_until;
        use crate::runtime::service::tests::*;
        #[test]
        fn service_sqlite_and_writer_failures_keep_health_manual_display_and_resize_live() {
            for boundary in ["fail-persist", "block-persist"] {
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
                assert!(fixture.process.as_ref().unwrap().matches().unwrap());
                fixture.resize(25, 90);
                wait_until(|| health(&mut fixture)["output_seq"].as_u64().unwrap() > sequence);
                fixture.fixture.conn.execute_batch("ROLLBACK").unwrap();
                std::fs::remove_file(hook).unwrap();
                wait_until(|| count(&fixture, "interaction_opened") == 1);
                fixture.finish();
            }
        }
        #[test]
        fn service_each_durable_history_table_is_checked_before_native_spawn() {
            for interaction in [false, true] {
                let mut seeded = None;
                let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
                    synthetic(fixture, spec);
                    let run = &fixture.run;
                    let evidence =
                        crate::broker::runtime::run_evidence(run, "runner-lifecycle", 1).unwrap();
                    if interaction {
                        seeded = Some(evidence.to_string());
                        fixture.conn.execute("INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json) VALUES(?1,1,'permission_request',?2,'2026-10-26T17:00:00Z',?3)",rusqlite::params![run.run_id,evidence["evidence_digest"].as_str().unwrap(),evidence.to_string()]).unwrap();
                    } else {
                        let payload = json!({"event_id":1,"event_seq":1,"run_id":run.run_id,"member_id":run.member_id,"fleet_id":run.fleet_id,"director_member_id":run.director_member_id,"kind":"coverage_unknown","interaction_id":null,"classification":null,"observed_at":"2026-10-26T17:00:00Z","source":"runner_lifecycle","decision_outcome":"unknown","director_action":null,"evidence":evidence,"cause":{"kind":"coverage_unknown"}});
                        seeded = Some(payload.to_string());
                        fixture.conn.execute("INSERT INTO member_runtime_events(run_id,event_seq,kind,observed_at,payload_json,notification_state,archive_reason) VALUES(?1,1,'coverage_unknown','2026-10-26T17:00:00Z',?2,'archived','member_inactive')",rusqlite::params![run.run_id,payload.to_string()]).unwrap();
                    }
                });
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                assert!(run.child_pid.is_none());
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
    }

    mod injected_failures {
        use crate::runtime::service::test_support::injected_failures::*;
        use crate::runtime::service::test_support::observation_services::count;
        use crate::runtime::service::test_support::observation_services::synthetic;
        use crate::runtime::service::tests::*;
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
            fixture.resize(0, 0);
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
        fn shutdown_detector_failure_records_gap_before_truthful_native_exit() {
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
            assert_durable_gap(&fixture);
            assert_eq!(fixture.wait().code(), Some(0));
            assert_eq!(count(&fixture, "runner_exited"), 1);
            assert_private_cause_stays_local(&fixture);
        }
    }
    mod pacing_tests {
        use crate::runtime::service::*;
        use crate::runtime::transport::TransportProgress;

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
            let idle = progress(0, 0);
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
    mod input_admission_tests {
        use crate::runtime::control::AdmittedControlFrame;
        use crate::runtime::input::receive_input_expiration;
        use crate::runtime::service::*;
        use std::net::Shutdown;

        use crate::runtime::control::test_support::independent_frame as frame;
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
        use crate::runtime::test_support::write_fixture_bytes as send;
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
                frames.extend(client.poll(&crate::time::SystemClock).unwrap());
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
            assert!(client.poll(&crate::time::SystemClock).unwrap().is_empty());
            let after = Instant::now();
            assert!(
                client.decoder.finish().is_err(),
                "this actual poll consumed the first header byte"
            );
            (before, after)
        }
        #[test]
        fn client_empty_poll_supplies_no_admission_time() {
            let (mut client, mut peer) = pair();
            let deadline = Instant::now() + Duration::from_secs(1);
            client.decoder.finish().unwrap();
            let empty_at = Instant::now();
            assert!(client.poll(&crate::time::SystemClock).unwrap().is_empty());
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
        #[test]
        fn client_frames_keep_first_time_through_coalescence_queue_and_conversion() {
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
                let clock = crate::runtime::input::test_support::ScriptClock::one(
                    crate::time::MonotonicSample {
                        before: now,
                        monotonic_ns: 1_000_000_000,
                        after: now,
                    },
                );
                let converted =
                    receive_input_expiration(11_000_000_000, actual.admitted_at(), &clock).unwrap();
                assert_eq!(converted, actual.admitted_at() + Duration::from_secs(5));
                clock.consumed(1, 0);
            }
            assert!(queue.is_empty());
            client.decoder.finish().unwrap();
        }
        #[test]
        fn client_rejects_partial_eof() {
            let deadline = Instant::now() + Duration::from_secs(1);
            let (mut partial, mut peer) = pair();
            first(&mut partial, &mut peer, 0, deadline);
            peer.shutdown(Shutdown::Write).unwrap();
            assert!(partial.poll(&crate::time::SystemClock).is_err());
        }
    }

    mod scenarios {
        use std::time::Duration;

        use crate::time::Clock;

        use crate::coding_agent::ComposerOperation;
        use crate::coding_agent::ProfileCatalog;
        use crate::coding_agent::catalog::LiteralFraming;
        use crate::db::test_support::FaultPlan;
        use crate::runtime::service::test_support::scenario::*;
        use crate::runtime::*;

        const IDLE: &str = "codex-0.167.0-composer-idle-empty-tmux-120-1";
        const APPROVAL: &str = "codex-0.167.0-allow-pre-stage-tmux-120-1";
        const STATUS: &str = "codex-0.167.0-auto-status-tmux-120-1";
        const REJECT_OPENING: FaultPlan = FaultPlan::Abort {
            trigger: "scenario_reject_opening",
            on: "BEFORE INSERT ON member_runtime_events WHEN NEW.kind='interaction_opened'",
            message: "scenario opening rejection",
        };

        fn prompt_program(literal: &str) -> Vec<u8> {
            ProfileCatalog::bundled()
                .composer_plan(
                    crate::coding_agent::CodingAgent::Codex,
                    ComposerOperation::Prompt,
                )
                .unwrap()
                .stages(literal, LiteralFraming::Plain)
                .unwrap()
                .concat()
        }

        #[test]
        fn screen_scenarios_record_their_event_sequences() {
            const SCENARIOS: &[Scenario] = &[
                Scenario {
                    name: "an idle composer records nothing",
                    screen: IDLE,
                    redraw: None,
                    interrupt: None,
                    steps: &[Step::Run(Duration::from_secs(2))],
                    events: &[],
                },
                Scenario {
                    name: "an approval form opens an interaction that a later screen closes",
                    screen: STATUS,
                    redraw: None,
                    interrupt: None,
                    steps: &[
                        Step::Screen(APPROVAL),
                        Step::Run(Duration::from_secs(1)),
                        Step::Screen(STATUS),
                        Step::Run(Duration::from_secs(1)),
                    ],
                    events: &["interaction_opened", "interaction_closed"],
                },
            ];
            for scenario in SCENARIOS {
                let mut runner = scenario.run();
                assert_eq!(runner.finish(), (0, 0), "{}", scenario.name);
            }
        }

        #[test]
        fn human_typing_reaches_the_child_and_the_child_screen_reaches_the_display() {
            const TYPING: Scenario = Scenario {
                name: "typing",
                screen: IDLE,
                redraw: None,
                interrupt: None,
                steps: &[Step::Type(b"typed by hand")],
                events: &[],
            };
            let mut runner = TYPING.run();
            assert_eq!(runner.child.written, b"typed by hand");
            assert_eq!(runner.display.bytes, screen_bytes(IDLE));
            assert_eq!(runner.finish(), (0, 0));
        }

        #[test]
        fn a_prompt_into_a_verified_empty_composer_writes_its_plan_and_is_accepted() {
            const LITERAL: &str = "scenario prompt";
            const PROMPT: Scenario = Scenario {
                name: "prompt",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::Prompt {
                        key: "scenario:prompt",
                        literal: LITERAL,
                    },
                    Step::Run(Duration::from_secs(5)),
                ],
                events: &[],
            };
            let mut runner = PROMPT.run();
            assert_eq!(runner.child.written, prompt_program(LITERAL));
            let [Ok(InputControlReply::Submission(reply))] = runner.replies.as_slice() else {
                panic!("one submission reply, got {}", runner.replies.len());
            };
            assert_eq!(reply.status, SubmissionStatus::Accepted);
            assert_eq!(reply.stage, SubmissionStage::Submitted);
            assert!(
                runner.clock.sleeps().iter().sum::<Duration>()
                    >= crate::runtime::input::STAGE_SETTLE,
                "the staged write settles on the virtual clock"
            );
            assert_eq!(runner.finish(), (0, 0));
        }

        const DRAFT: &str = "codex-0.167.0-composer-prompt-plain-41-literal-tmux-120-1";
        const MID_WRITE: Interrupt = Interrupt {
            marker: "FORM_MID_WRITE",
            screen: APPROVAL,
        };
        /// Longer than two sweep intervals, so a sweep that could deliver would have done so.
        const SWEEP_WINDOW: Duration = Duration::from_millis(2500);

        fn submission(reply: &Result<InputControlReply>) -> &SubmissionResult {
            match reply {
                Ok(InputControlReply::Submission(submission)) => submission,
                Ok(_) => panic!("a submission reply"),
                Err(cause) => panic!("a submission reply, got {cause}"),
            }
        }

        #[test]
        fn the_sweep_types_each_pending_preview_once_oldest_first_and_stamps_it() {
            const SWEEP: Scenario = Scenario {
                name: "oldest first",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::Consumed("consumed before any sweep"),
                    Step::Message("first swept preview"),
                    Step::Message("second swept preview"),
                    Step::Run(Duration::from_secs(3)),
                ],
                events: &[],
            };
            let mut runner = SWEEP.run();
            let first = runner.preview_line("first swept preview");
            let second = runner.preview_line("second swept preview");
            assert_eq!(runner.typed(&first), 1);
            assert_eq!(runner.typed(&second), 1);
            assert!(runner.typed_at(&first) < runner.typed_at(&second));
            assert!(runner.notified("first swept preview"));
            assert!(runner.notified("second swept preview"));
            assert_eq!(runner.typed("consumed before any sweep"), 0);
            assert!(!runner.notified("consumed before any sweep"));
            assert_eq!(runner.finish(), (0, 0));
        }

        #[test]
        fn an_obstacle_defers_the_sweep_with_zero_bytes_until_it_clears() {
            const CASES: &[Scenario] = &[
                Scenario {
                    name: "a non-empty composer blocks delivery until it verifies empty",
                    screen: IDLE,
                    redraw: Some(IDLE),
                    interrupt: None,
                    steps: &[
                        Step::Screen(DRAFT),
                        Step::Run(Duration::from_millis(100)),
                        Step::Message("blocked preview"),
                        Step::Run(SWEEP_WINDOW),
                    ],
                    events: &[],
                },
                Scenario {
                    name: "a held pane lock defers the sweep until it is free",
                    screen: IDLE,
                    redraw: Some(IDLE),
                    interrupt: None,
                    steps: &[
                        Step::HoldPane,
                        Step::Message("blocked preview"),
                        Step::Run(SWEEP_WINDOW),
                    ],
                    events: &[],
                },
            ];
            for scenario in CASES {
                let name = scenario.name;
                let mut runner = scenario.run();
                assert!(runner.child.written.is_empty(), "{name}");
                assert!(!runner.notified("blocked preview"), "{name}");
                match runner.held_pane {
                    Some(_) => runner.step(Step::ReleasePane),
                    None => runner.step(Step::Screen(IDLE)),
                }
                runner.step(Step::Run(Duration::from_secs(10)));
                let line = runner.preview_line("blocked preview");
                assert_eq!(runner.typed(&line), 1, "{name}");
                assert!(runner.notified("blocked preview"), "{name}");
                assert_eq!(runner.finish(), (0, 0), "{name}");
            }
        }

        #[test]
        fn a_retried_head_waits_out_the_run_wide_backoff_and_precedes_a_newer_message() {
            const RETRY: Scenario = Scenario {
                name: "retried head",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::HoldPane,
                    Step::Message("head behind a busy pane"),
                    Step::Message("newer message"),
                    Step::Run(SWEEP_WINDOW),
                    Step::ReleasePane,
                ],
                events: &[],
            };
            let mut runner = RETRY.run();
            let released = runner.clock.now();
            let backoff = runner
                .sweep_backoff_until()
                .expect("a pane_input_busy hold arms the run-wide backoff");
            assert!(
                backoff >= released,
                "the backoff outlasts the release: {:?}",
                backoff.duration_since(released)
            );
            runner.step(Step::Run(
                backoff
                    .duration_since(released)
                    .saturating_sub(Duration::from_millis(10)),
            ));
            assert!(
                runner.child.written.is_empty(),
                "no preview byte is written while the backoff is armed"
            );
            runner.step(Step::Run(Duration::from_secs(10)));
            let head = runner.preview_line("head behind a busy pane");
            let newer = runner.preview_line("newer message");
            assert_eq!(runner.typed(&head), 1);
            assert_eq!(runner.typed(&newer), 1);
            assert!(runner.typed_at(&head) < runner.typed_at(&newer));
            assert!(
                runner.payloads("input_delivery_result").is_empty(),
                "a held and retried preview publishes no delivery result"
            );
            assert_eq!(runner.finish(), (0, 0));
        }

        #[test]
        fn a_control_prompt_queues_behind_an_in_flight_sweep_or_an_open_interaction() {
            const CASES: &[Scenario] = &[
                Scenario {
                    name: "a prompt arriving during a sweep attempt queues behind it",
                    screen: IDLE,
                    redraw: Some(IDLE),
                    interrupt: None,
                    steps: &[
                        Step::Message("ahead of the prompt"),
                        Step::Run(Duration::from_millis(1250)),
                        Step::Prompt {
                            key: "prompt:queued",
                            literal: "queued control prompt",
                        },
                    ],
                    events: &[],
                },
                Scenario {
                    name: "a prompt queued behind a form is written before the sweep",
                    screen: IDLE,
                    redraw: Some(IDLE),
                    interrupt: None,
                    steps: &[
                        Step::Screen(APPROVAL),
                        Step::Run(Duration::from_secs(1)),
                        Step::Prompt {
                            key: "prompt:queued",
                            literal: "queued control prompt",
                        },
                        Step::Message("behind the prompt"),
                        Step::Run(SWEEP_WINDOW),
                    ],
                    events: &["interaction_opened"],
                },
            ];
            for (index, scenario) in CASES.iter().enumerate() {
                let name = scenario.name;
                let mut runner = scenario.run();
                let reply = runner.replies.last().expect("the prompt replies");
                assert_eq!(submission(reply).status, SubmissionStatus::Queued, "{name}");
                assert_eq!(runner.typed("queued control prompt"), 0, "{name}");
                if index == 1 {
                    runner.step(Step::Screen(IDLE));
                }
                runner.step(Step::Run(Duration::from_secs(5)));
                assert_eq!(runner.typed("queued control prompt"), 1, "{name}");
                let (first, second) = if index == 0 {
                    (
                        runner.preview_line("ahead of the prompt"),
                        "queued control prompt".to_owned(),
                    )
                } else {
                    (
                        "queued control prompt".to_owned(),
                        runner.preview_line("behind the prompt"),
                    )
                };
                assert_eq!(runner.typed(&first), 1, "{name}");
                assert_eq!(runner.typed(&second), 1, "{name}");
                assert!(runner.typed_at(&first) < runner.typed_at(&second), "{name}");
                runner.finish();
            }
        }

        #[test]
        fn an_uncertain_preview_publishes_one_receipt_free_result_and_is_typed_again_once_verified_empty()
         {
            const UNCERTAIN: Scenario = Scenario {
                name: "uncertain preview",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: Some(MID_WRITE),
                steps: &[
                    Step::Message("retried preview FORM_MID_WRITE"),
                    Step::Run(Duration::from_secs(3)),
                ],
                events: &["input_delivery_result", "interaction_opened"],
            };
            let mut runner = UNCERTAIN.run();
            let body = "retried preview FORM_MID_WRITE";
            let line = runner.preview_line(body);
            let message_id = runner.message(body).message_id;
            let results: Vec<_> = runner
                .payloads("input_delivery_result")
                .into_iter()
                .filter(|result| result["message_id"] == message_id)
                .collect();
            assert_eq!(results.len(), 1, "{results:?}");
            assert_eq!(results[0]["input_id"], serde_json::Value::Null);
            assert_eq!(results[0]["delivery"]["status"], "uncertain");
            assert_eq!(results[0]["director_action"], serde_json::Value::Null);
            assert!(!runner.notified(body));
            runner.step(Step::Screen(IDLE));
            runner.step(Step::Run(Duration::from_secs(5)));
            assert_eq!(runner.typed(&line), 2, "the stopped literal and its retry");
            assert!(runner.notified(body));
            assert_eq!(
                runner
                    .payloads("input_delivery_result")
                    .iter()
                    .filter(|result| result["message_id"] == message_id)
                    .count(),
                1,
                "the retry publishes no further result"
            );
            runner.finish();
        }

        #[test]
        fn a_preview_behind_a_non_empty_composer_publishes_one_stall_event_at_the_stall_bound() {
            const BOUND: Duration = Duration::from_secs(2);
            const STALLED: Scenario = Scenario {
                name: "stalled preview",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::Screen(DRAFT),
                    Step::Run(Duration::from_millis(100)),
                    Step::Message("stalled behind a human draft"),
                    Step::Run(Duration::from_secs(1)),
                ],
                events: &[],
            };
            let mut runner = STALLED.run_with(|runner| runner.coordinator.set_stall_bound(BOUND));
            assert!(runner.payloads("input_delivery_stalled").is_empty());
            runner.step(Step::Run(BOUND + SWEEP_WINDOW));
            let stalls = runner.payloads("input_delivery_stalled");
            assert_eq!(
                stalls.len(),
                1,
                "published once per run and message: {stalls:?}"
            );
            let stall = &stalls[0];
            assert_eq!(
                stall["message_id"],
                runner.message("stalled behind a human draft").message_id
            );
            assert_eq!(
                stall["blocked_by_input_id"],
                serde_json::Value::Null,
                "{stall}"
            );
            assert!(stall["waiting_since"].as_str().is_some(), "{stall}");
            assert!(
                stall["director_action"]["instruction"]
                    .as_str()
                    .is_some_and(|instruction| !instruction.trim().is_empty()),
                "one instruction: {stall}"
            );
            assert!(runner.child.written.is_empty());
            runner.step(Step::Screen(IDLE));
            runner.step(Step::Run(Duration::from_secs(5)));
            assert!(runner.notified("stalled behind a human draft"));
            assert_eq!(runner.payloads("input_delivery_stalled").len(), 1);
            runner.finish();
        }

        #[test]
        fn a_head_that_stalls_again_after_an_interaction_archives_the_repeat_and_later_events_persist()
         {
            const BOUND: Duration = Duration::from_secs(2);
            const RESTARTED: Scenario = Scenario {
                name: "restarted head stall",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::Screen(APPROVAL),
                    Step::Run(Duration::from_secs(1)),
                    Step::Prompt {
                        key: "prompt:restarted-stall",
                        literal: "head that stalls twice",
                    },
                    Step::HoldPane,
                    Step::Screen(IDLE),
                    Step::Run(Duration::from_secs(1)),
                    Step::Screen(DRAFT),
                    Step::Run(Duration::from_millis(500)),
                    Step::ReleasePane,
                    Step::Run(Duration::from_secs(4)),
                    Step::Screen(APPROVAL),
                    Step::Run(Duration::from_secs(1)),
                    Step::HoldPane,
                    Step::Screen(IDLE),
                    Step::Run(Duration::from_secs(1)),
                    Step::Screen(DRAFT),
                    Step::Run(Duration::from_millis(500)),
                    Step::ReleasePane,
                    Step::Run(Duration::from_secs(4)),
                    Step::Screen(APPROVAL),
                    Step::Run(Duration::from_secs(1)),
                ],
                events: &[
                    "interaction_opened",
                    "interaction_closed",
                    "input_delivery_stalled",
                    "interaction_opened",
                    "interaction_closed",
                    "input_delivery_stalled",
                    "interaction_opened",
                ],
            };
            let mut runner = RESTARTED.run_with(|runner| runner.coordinator.set_stall_bound(BOUND));
            assert_eq!(runner.typed("head that stalls twice"), 0);
            let head = crate::broker::runtime::find_input(
                &runner.fixture.conn,
                runner.fixture.run.run_id,
                "prompt:restarted-stall",
            )
            .unwrap()
            .expect("the queued head has a receipt");
            let rows: Vec<(String, Option<i64>, Option<String>, serde_json::Value)> = runner
                .fixture
                .conn
                .prepare(
                    "SELECT notification_state,message_id,archive_reason,payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_stalled' ORDER BY event_seq",
                )
                .unwrap()
                .query_map([runner.fixture.run.run_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let [first, repeat] = rows.as_slice() else {
                panic!("two stalled records, got {rows:?}");
            };
            assert_eq!(first.0, "inbox", "{first:?}");
            assert!(first.1.is_some(), "{first:?}");
            assert_eq!(first.2, None, "{first:?}");
            assert_eq!(repeat.0, "archived", "{repeat:?}");
            assert_eq!(repeat.1, None, "{repeat:?}");
            assert_eq!(
                repeat.2.as_deref(),
                Some("duplicate_delivery"),
                "{repeat:?}"
            );
            for (_, _, _, payload) in &rows {
                assert_eq!(payload["input_id"], head.input_id, "{payload}");
            }
            assert!(
                first.3["waiting_since"] != repeat.3["waiting_since"],
                "the restarted watch carries its own wait start: {rows:?}"
            );
            let status = runner.writer.status();
            assert_eq!(status.diagnostic, None, "{status:?}");
            runner.finish();
        }

        #[test]
        fn an_uncertain_prompt_stops_queued_input_and_the_sweep_until_reconciled() {
            const STOPPED: Scenario = Scenario {
                name: "uncertain prompt",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: Some(MID_WRITE),
                steps: &[
                    Step::Prompt {
                        key: "prompt:stopped-mid-write",
                        literal: "stopped prompt FORM_MID_WRITE",
                    },
                    Step::Run(Duration::from_secs(3)),
                    Step::Screen(IDLE),
                    Step::Run(Duration::from_secs(1)),
                    Step::Exec {
                        key: "exec:behind-the-uncertain-prompt",
                        literal: "printf behind",
                    },
                    Step::Message("preview behind the uncertain prompt"),
                    Step::Run(SWEEP_WINDOW),
                ],
                events: &[
                    "input_delivery_result",
                    "interaction_opened",
                    "interaction_closed",
                ],
            };
            let mut runner = STOPPED.run();
            assert_eq!(
                submission(&runner.replies[0]).status,
                SubmissionStatus::Uncertain
            );
            assert_eq!(
                submission(&runner.replies[1]).status,
                SubmissionStatus::Queued
            );
            assert_eq!(runner.typed("printf behind"), 0);
            let line = runner.preview_line("preview behind the uncertain prompt");
            assert_eq!(runner.typed(&line), 0);
            assert!(!runner.notified("preview behind the uncertain prompt"));
            runner.step(Step::Reconcile("prompt:stopped-mid-write"));
            runner.step(Step::Run(Duration::from_secs(10)));
            assert_eq!(runner.typed("printf behind"), 1);
            assert_eq!(runner.typed(&line), 1);
            assert!(
                runner.typed_at("printf behind") < runner.typed_at(&line),
                "the FIFO drains before the sweep"
            );
            runner.finish();
        }

        #[test]
        fn an_event_storage_fault_holds_input_until_the_writer_persists_again() {
            const FAULTED: Scenario = Scenario {
                name: "event fault",
                screen: STATUS,
                redraw: None,
                interrupt: None,
                steps: &[
                    Step::FailPersist(true),
                    Step::Screen(APPROVAL),
                    Step::Run(Duration::from_secs(1)),
                ],
                events: &[],
            };
            let mut runner = FAULTED.run();
            assert!(runner.writer.status().diagnostic.is_some());
            assert_eq!(
                runner.eligibility(),
                InputEligibility::Hold(GateReason::Gap)
            );
            runner.step(Step::FailPersist(false));
            runner.step(Step::Run(Duration::from_secs(5)));
            assert!(
                runner
                    .event_kinds()
                    .contains(&"interaction_opened".to_owned()),
                "{:?}",
                runner.event_kinds()
            );
            runner.finish();
        }

        #[test]
        fn a_rejected_event_degrades_without_a_gap_and_the_held_preview_is_delivered() {
            const REJECTED: Scenario = Scenario {
                name: "rejected opening",
                screen: IDLE,
                redraw: Some(IDLE),
                interrupt: None,
                steps: &[
                    Step::Arm(REJECT_OPENING),
                    Step::Screen(APPROVAL),
                    Step::Run(Duration::from_secs(1)),
                    Step::Message("preview held by the rejected form"),
                    Step::Run(Duration::from_secs(1)),
                    Step::Screen(IDLE),
                    Step::Run(Duration::from_secs(5)),
                ],
                events: &["runner_degraded", "runner_degraded"],
            };
            let mut runner = REJECTED.run();
            let body = "preview held by the rejected form";
            let line = runner.preview_line(body);
            let rejected: Vec<_> = runner
                .payloads("runner_degraded")
                .into_iter()
                .map(|payload| {
                    assert_eq!(payload["cause"]["kind"], "event_rejected", "{payload}");
                    payload["cause"]["rejected_kind"].clone()
                })
                .collect();
            assert_eq!(rejected, ["interaction_opened", "interaction_closed"]);
            let status = runner.writer.status();
            assert_eq!(status.diagnostic, None, "{status:?}");
            assert_eq!(status.gap_count, 0, "{status:?}");
            assert_eq!(runner.controller.status().gap_count, 0);
            assert_eq!(runner.typed(&line), 1, "the held preview is typed once");
            assert!(runner.notified(body));
            runner.finish();
        }
    }
}
