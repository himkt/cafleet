use super::*;
use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, CleanupState, RunRecord};
use crate::multiplexer::{AnyMultiplexer, MultiplexerContext};
use serde_json::json;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) fn pause(deadline: Instant) -> Result<()> {
    check_deadline(Instant::now(), deadline)?;
    std::thread::sleep(
        Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
    );
    Ok(())
}
pub(super) struct ControlClient {
    pub stream: UnixStream,
    decoder: ControlDecoder,
}

#[derive(Clone, Copy)]
enum EndpointPhase {
    Starting,
    Ready,
}

impl ControlClient {
    pub(super) fn connect(path: &Path, deadline: Instant) -> Result<Self> {
        Self::connect_in_phase(path, deadline, EndpointPhase::Ready)
    }

    fn connect_in_phase(path: &Path, deadline: Instant, phase: EndpointPhase) -> Result<Self> {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
        loop {
            check_deadline(Instant::now(), deadline)?;
            let meta = match std::fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(cause)
                    if cause.kind() == ErrorKind::NotFound
                        && matches!(phase, EndpointPhase::Starting) =>
                {
                    pause(deadline)?;
                    continue;
                }
                Err(cause) => return Err(io_error(cause)),
            };
            ensure(
                meta.file_type().is_socket() && meta.uid() == nix::unistd::geteuid().as_raw(),
                "private control socket identity required",
            )?;
            if meta.mode() & 0o777 != 0o600 {
                if matches!(phase, EndpointPhase::Starting) {
                    pause(deadline)?;
                    continue;
                }
                return Err(error("private control socket identity required"));
            }
            let fd = socket(
                AddressFamily::Unix,
                SockType::Stream,
                SockFlag::empty(),
                None,
            )
            .map_err(io_error)?;
            nix::fcntl::fcntl(
                &fd,
                nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
            )
            .map_err(io_error)?;
            nix::fcntl::fcntl(
                &fd,
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
            .map_err(io_error)?;
            match connect(fd.as_raw_fd(), &UnixAddr::new(path).map_err(io_error)?) {
                Ok(()) | Err(nix::errno::Errno::EINPROGRESS) => {
                    return Ok(Self {
                        stream: UnixStream::from(fd),
                        decoder: ControlDecoder::new(),
                    });
                }
                Err(nix::errno::Errno::EAGAIN) => pause(deadline)?,
                Err(nix::errno::Errno::ECONNREFUSED | nix::errno::Errno::ENOENT)
                    if matches!(phase, EndpointPhase::Starting) =>
                {
                    pause(deadline)?;
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
    }
    pub(super) fn exchange(&mut self, request: &Value, deadline: Instant) -> Result<Value> {
        let bytes = encode_control_frame(request)?;
        let mut offset = 0;
        while offset < bytes.len() {
            check_deadline(Instant::now(), deadline)?;
            match self.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(error("control endpoint disconnected during write")),
                Ok(count) => offset += count,
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(deadline)?
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
        loop {
            check_deadline(Instant::now(), deadline)?;
            let mut bytes = [0; 4096];
            match self.stream.read(&mut bytes) {
                Ok(0) => {
                    self.decoder.finish()?;
                    return Err(error("control endpoint disconnected before reply"));
                }
                Ok(count) => {
                    let frames = self.decoder.feed(&bytes[..count])?;
                    if !frames.is_empty() {
                        ensure(frames.len() == 1, "unexpected multiple control replies")?;
                        self.decoder.finish()?;
                        let value = frames.into_iter().next().expect("one control reply");
                        ensure(
                            value.get("result").is_some() ^ value.get("error").is_some(),
                            "control reply requires one result or error",
                        )?;
                        if let Some(cause) = value.get("error") {
                            let cause = cause
                                .as_str()
                                .ok_or_else(|| error("control error requires text"))?;
                            return Err(error(format!("runner control: {cause}")));
                        }
                        return Ok(value
                            .get("result")
                            .expect("validated result envelope")
                            .clone());
                    }
                }
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                {
                    pause(deadline)?
                }
                Err(cause) => return Err(io_error(cause)),
            }
        }
    }
}
pub(super) fn request(run: &RunRecord, kind: &str, deadline: Instant) -> Result<Value> {
    let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
    let mut client = ControlClient::connect(&paths.control_socket, deadline)?;
    client.exchange(
        &json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"kind":kind,"identity":ControlIdentity::from_run(run)}),
        deadline,
    )
}

pub(crate) struct NativeLaunchHost<'a> {
    mux: &'a AnyMultiplexer,
    context: &'a MultiplexerContext,
    client: Option<ControlClient>,
}
impl<'a> NativeLaunchHost<'a> {
    pub(crate) fn new(mux: &'a AnyMultiplexer, context: &'a MultiplexerContext) -> Self {
        Self {
            mux,
            context,
            client: None,
        }
    }
}
impl LaunchHost for NativeLaunchHost<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn current_executable(&self) -> Result<std::path::PathBuf> {
        std::env::current_exe().map_err(io_error)
    }
    fn split_runner(
        &mut self,
        argv: &[String],
        environment: &[(String, String)],
        deadline: Instant,
    ) -> Result<String> {
        self.mux
            .bounded(crate::input::Deadline::until(deadline))
            .split_window(self.context, environment, argv)
            .map_err(|cause| error(format!("runner pane spawn failed: {cause}")))
    }
    fn wait_prepared(
        &mut self,
        descriptor: &LaunchDescriptor,
        deadline: Instant,
    ) -> Result<Prepared> {
        let paths = RuntimePaths::for_run(&descriptor.database_identity_json, descriptor.run_id)?;
        let mut client = ControlClient::connect_in_phase(
            &paths.control_socket,
            deadline,
            EndpointPhase::Starting,
        )?;
        let identity = ControlIdentity {
            database_identity_json: descriptor.database_identity_json.clone(),
            fleet_id: descriptor.fleet_id,
            member_id: descriptor.member_id,
            director_member_id: descriptor.director_member_id,
            run_id: descriptor.run_id,
            owner_epoch: descriptor.owner_epoch,
            launch_nonce: descriptor.launch_nonce.clone(),
        };
        let result=client.exchange(&json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"kind":"prepare","identity":identity,"remaining_ms":deadline.saturating_duration_since(Instant::now()).as_millis().min(10000)as u64}),deadline)?;
        let prepared = serde_json::from_value(result).map_err(io_error)?;
        self.client = Some(client);
        Ok(prepared)
    }
    fn start(
        &mut self,
        prepared: &Prepared,
        placement: &Placement,
        deadline: Instant,
    ) -> Result<Started> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| error("prepared control connection required"))?;
        let value = client.exchange(
            &json!({"protocol_version":CONTROL_PROTOCOL_VERSION,"kind":"start","identity":prepared.identity,"pane_id":placement.mux_pane_id}),
            deadline,
        )?;
        serde_json::from_value(value).map_err(io_error)
    }
    fn shutdown_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome> {
        if let Ok(value) = request(run, "shutdown", clock.storage_deadline()) {
            return serde_json::from_value(value).map_err(io_error);
        }
        let runner = ProcessIdentity {
            pid: run.owner_pid,
            start_identity: run.owner_start_identity.clone(),
        };
        if runner.matches()? {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(runner.pid).map_err(io_error)?),
                nix::sys::signal::Signal::SIGTERM,
            )
            .map_err(io_error)?;
            while runner.matches()? && Instant::now() < clock.storage_deadline() {
                pause(clock.storage_deadline())?;
            }
        }
        let mut notes=vec!["runner control shutdown unavailable; descendant cleanup requires explicit reconciliation".into()];
        if runner.matches()? {
            notes.push("owned runner termination remains unconfirmed".into());
        }
        Ok(ShutdownOutcome {
            native_exit: None,
            cleanup_state: CleanupState::Incomplete,
            diagnostics: notes,
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        })
    }
    fn close_pane(&mut self, pane: &str, deadline: Instant) -> Result<()> {
        self.mux
            .bounded(crate::input::Deadline::until(deadline))
            .kill_pane(pane, true)
            .map_err(|cause| error(format!("runner pane cleanup failed: {cause}")))
    }
}

pub(crate) fn request_shutdown(
    conn: &mut rusqlite::Connection,
    run: &RunRecord,
    deadline: Instant,
) -> Result<ShutdownOutcome> {
    validate_run_paths(conn, run)?;
    let result = request(run, "shutdown", deadline)?;
    let outcome: ShutdownOutcome = serde_json::from_value(result).map_err(io_error)?;
    let current = storage::get_run(conn, run.run_id)?;
    ensure(
        current.owner_epoch == run.owner_epoch,
        "shutdown ownership changed",
    )?;
    Ok(outcome)
}
