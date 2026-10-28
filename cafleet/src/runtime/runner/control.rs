use super::codec::{AdmittedControlFrame, decode_legacy_control_body};
use super::input_protocol::{decode_input_reply, encode_input_request};
use super::*;

#[cfg(test)]
mod input_client_tests;
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
        let frame = self.exchange_bytes(&bytes, deadline)?;
        let value = decode_legacy_control_body(frame.body())?;
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
        Ok(value
            .get("result")
            .expect("validated result envelope")
            .clone())
    }
    pub(super) fn exchange_input(
        &mut self,
        request: &InputControlRequest,
        deadline: Instant,
    ) -> Result<InputControlReply> {
        check_deadline(Instant::now(), deadline)?;
        let bytes = encode_input_request(request)?;
        let frame = self.exchange_bytes(&bytes, deadline)?;
        let reply = decode_input_reply(frame.body(), request)?;
        check_deadline(Instant::now(), deadline)?;
        Ok(reply)
    }
    fn exchange_bytes(&mut self, bytes: &[u8], deadline: Instant) -> Result<AdmittedControlFrame> {
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
                    let observed_at = Instant::now();
                    let frames = self.decoder.feed_admitted(&bytes[..count], observed_at)?;
                    if !frames.is_empty() {
                        ensure(frames.len() == 1, "unexpected multiple control replies")?;
                        self.decoder.finish()?;
                        return Ok(frames.into_iter().next().expect("one control reply"));
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

#[cfg(test)]
mod launch_readiness_tests {
    use std::io::ErrorKind;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use nix::sys::socket::{
        AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
    };
    use serde_json::Value;

    use super::*;
    use crate::error::CafleetError;
    use crate::multiplexer::{AnyMultiplexer, MultiplexerContext, TmuxMultiplexer, recording};
    use crate::runtime::runner_tests::{Fixture, read_fixture_bytes, write_fixture_bytes};

    struct Endpoint {
        fixture: Fixture,
        artifacts: RunArtifacts,
        descriptor: LaunchDescriptor,
    }

    impl Endpoint {
        fn new() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let descriptor = LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap();
            Self {
                fixture,
                artifacts,
                descriptor,
            }
        }

        fn path(&self) -> &Path {
            &self.artifacts.paths.control_socket
        }

        fn prepared(&self) -> Prepared {
            let mut identity = ControlIdentity::from_run(&self.fixture.run);
            identity.owner_epoch += 1;
            Prepared {
                identity,
                runner: ProcessIdentity::current().unwrap(),
            }
        }
    }

    fn with_native_host<T>(operation: impl FnOnce(&mut NativeLaunchHost<'_>) -> T) -> T {
        let runner = recording::FakeRunner::without_binaries();
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()));
        let context = MultiplexerContext {
            session: "readiness-fixture".into(),
            window_id: "@1".into(),
            pane_id: "%1".into(),
        };
        let result = operation(&mut NativeLaunchHost::new(&mux, &context));
        assert!(runner.events().is_empty(), "{:?}", runner.events());
        result
    }

    fn bound_socket(path: &Path, mode: u32, listening: bool) -> UnixListener {
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .unwrap();
        bind(fd.as_raw_fd(), &UnixAddr::new(path).unwrap()).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        let listener = UnixListener::from(fd);
        listener.set_nonblocking(true).unwrap();
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_socket()
        );
        if listening {
            listen(&listener, Backlog::new(8).unwrap()).unwrap();
        }
        listener
    }

    fn pause_until(deadline: Instant) {
        std::thread::sleep(
            Duration::from_millis(2).min(deadline.saturating_duration_since(Instant::now())),
        );
    }

    fn accept_before(
        listener: &UnixListener,
        finished: &AtomicBool,
        deadline: Instant,
    ) -> Option<UnixStream> {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true).unwrap();
                    return Some(stream);
                }
                Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                Err(cause) => panic!("fixture accept failed: {cause}"),
            }
            if finished.load(Ordering::Acquire) || Instant::now() >= deadline {
                return None;
            }
            pause_until(deadline);
        }
    }

    fn read_prepare(
        stream: &mut UnixStream,
        identity: &ControlIdentity,
        deadline: Instant,
    ) -> Value {
        let mut header = [0; 4];
        read_fixture_bytes(stream, &mut header, deadline);
        let length = usize::try_from(u32::from_be_bytes(header)).unwrap();
        assert!(
            length > 0 && length <= 65536,
            "invalid request length {length}"
        );
        let mut bytes = vec![0; length];
        read_fixture_bytes(stream, &mut bytes, deadline);
        let request: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request["protocol_version"], 1);
        assert_eq!(request["kind"], "prepare");
        assert_eq!(request["identity"], serde_json::to_value(identity).unwrap());
        assert!(request["remaining_ms"].as_u64().unwrap() > 0);
        request
    }

    fn raw_frame(body: &[u8]) -> Vec<u8> {
        let mut frame = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        frame.extend_from_slice(body);
        frame
    }

    #[derive(Clone, Copy)]
    enum StartupInterval {
        Bound,
        BeforePrivateMode,
    }

    fn readiness_success(interval: StartupInterval) {
        let endpoint = Endpoint::new();
        let listener = match interval {
            StartupInterval::Bound => {
                let listener = bound_socket(endpoint.path(), 0o600, false);
                assert_eq!(
                    UnixStream::connect(endpoint.path()).unwrap_err().kind(),
                    ErrorKind::ConnectionRefused
                );
                listener
            }
            StartupInterval::BeforePrivateMode => bound_socket(endpoint.path(), 0o644, true),
        };
        let finished = AtomicBool::new(false);
        let started = Instant::now();
        let deadline = started + Duration::from_secs(1);
        let ready_at = started + Duration::from_millis(100);
        let path = endpoint.path().to_path_buf();
        let expected_identity = ControlIdentity::from_run(&endpoint.fixture.run);
        let reply = json!({"protocol_version":1,"result":endpoint.prepared()});
        let (result, request, premature_connections) = std::thread::scope(|scope| {
            let server = scope.spawn(|| {
                let mut premature_connections = 0;
                while Instant::now() < ready_at {
                    if matches!(interval, StartupInterval::BeforePrivateMode) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                premature_connections += 1;
                                drop(stream);
                            }
                            Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                            Err(cause) => panic!("pre-private fixture accept failed: {cause}"),
                        }
                    }
                    if finished.load(Ordering::Acquire) {
                        return (None, premature_connections);
                    }
                    pause_until(ready_at);
                }
                match interval {
                    StartupInterval::Bound => listen(&listener, Backlog::new(8).unwrap()).unwrap(),
                    StartupInterval::BeforePrivateMode => {
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                            .unwrap()
                    }
                }
                let Some(mut stream) = accept_before(&listener, &finished, deadline) else {
                    return (None, premature_connections);
                };
                let request = read_prepare(&mut stream, &expected_identity, deadline);
                write_fixture_bytes(
                    &mut stream,
                    &raw_frame(&serde_json::to_vec(&reply).unwrap()),
                    deadline,
                );
                (Some(request), premature_connections)
            });
            let result =
                with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
            finished.store(true, Ordering::Release);
            let (request, premature_connections) = server.join().unwrap();
            (result, request, premature_connections)
        });
        assert_eq!(premature_connections, 0);
        let prepared =
            result.expect("startup interval must become ready within its original deadline");
        assert_eq!(prepared.identity, endpoint.prepared().identity);
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(Instant::now() < deadline);
        assert!(request.unwrap()["remaining_ms"].as_u64().unwrap() <= 900);
    }

    #[test]
    fn runner_readiness_bound_socket_becomes_listening_within_the_original_deadline() {
        readiness_success(StartupInterval::Bound);
    }

    #[test]
    fn runner_readiness_pre_private_socket_waits_without_sending_prepare() {
        readiness_success(StartupInterval::BeforePrivateMode);
    }

    fn readiness_expiry(interval: StartupInterval) {
        let endpoint = Endpoint::new();
        let listener = match interval {
            StartupInterval::Bound => bound_socket(endpoint.path(), 0o600, false),
            StartupInterval::BeforePrivateMode => bound_socket(endpoint.path(), 0o644, true),
        };
        let started = Instant::now();
        let deadline = started + Duration::from_millis(120);
        let result = with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
        assert!(
            matches!(result, Err(CafleetError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(Instant::now() >= deadline);
        assert!(started.elapsed() < Duration::from_secs(1));
        if matches!(interval, StartupInterval::BeforePrivateMode) {
            assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
        }
    }

    #[test]
    fn runner_readiness_bound_unready_socket_expires_at_the_supplied_deadline() {
        readiness_expiry(StartupInterval::Bound);
    }

    #[test]
    fn runner_readiness_nonprivate_socket_expires_without_a_prepare_connection() {
        readiness_expiry(StartupInterval::BeforePrivateMode);
    }

    fn assert_identity_rejected(endpoint: &Endpoint) {
        let started = Instant::now();
        let result = with_native_host(|host| {
            host.wait_prepared(&endpoint.descriptor, started + Duration::from_secs(1))
        });
        let cause = result.expect_err("invalid endpoint identity must fail explicitly");
        assert!(
            cause
                .to_string()
                .contains("private control socket identity required"),
            "{cause}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn runner_readiness_regular_file_is_an_identity_failure() {
        let endpoint = Endpoint::new();
        std::fs::write(endpoint.path(), b"fixture is a regular file").unwrap();
        assert_identity_rejected(&endpoint);
    }

    #[test]
    fn runner_readiness_symlink_to_a_private_socket_is_an_identity_failure() {
        let endpoint = Endpoint::new();
        let target = endpoint.fixture._dir.path().join("linked.sock");
        let listener = bound_socket(&target, 0o600, true);
        symlink(target, endpoint.path()).unwrap();
        assert_identity_rejected(&endpoint);
        assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    }

    #[test]
    fn runner_readiness_dangling_symlink_is_an_identity_failure() {
        let endpoint = Endpoint::new();
        symlink(
            endpoint.fixture._dir.path().join("absent.sock"),
            endpoint.path(),
        )
        .unwrap();
        assert_identity_rejected(&endpoint);
    }

    fn assert_prepare_not_reconnected(reply_body: Option<&[u8]>) {
        let endpoint = Endpoint::new();
        let listener = bound_socket(endpoint.path(), 0o600, true);
        let finished = AtomicBool::new(false);
        let started = Instant::now();
        let deadline = started + Duration::from_secs(1);
        let expected_identity = ControlIdentity::from_run(&endpoint.fixture.run);
        let (result, connections) = std::thread::scope(|scope| {
            let server = scope.spawn(|| {
                let mut stream = accept_before(&listener, &finished, deadline)
                    .expect("actual startup client must connect");
                read_prepare(&mut stream, &expected_identity, deadline);
                if let Some(body) = reply_body {
                    write_fixture_bytes(&mut stream, &raw_frame(body), deadline);
                }
                drop(stream);
                let mut connections = 1;
                while let Some(stream) = accept_before(&listener, &finished, deadline) {
                    connections += 1;
                    drop(stream);
                }
                connections
            });
            let result =
                with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
            finished.store(true, Ordering::Release);
            (result, server.join().unwrap())
        });
        assert!(result.is_err(), "{result:?}");
        assert!(
            !matches!(&result, Err(CafleetError::DeadlineExceeded)),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(connections, 1, "a written prepare must not be reconnected");
    }

    #[test]
    fn runner_readiness_lost_reply_after_written_prepare_is_not_reconnected() {
        assert_prepare_not_reconnected(None);
    }

    #[test]
    fn runner_readiness_malformed_reply_after_written_prepare_is_not_reconnected() {
        for body in [
            b"{".as_slice(),
            br#"{"protocol_version":2,"result":null}"#,
            br#"{"protocol_version":1,"result":{"identity":null}}"#,
        ] {
            assert_prepare_not_reconnected(Some(body));
        }
    }

    #[test]
    fn runner_readiness_post_start_health_preserves_strict_endpoint_failures() {
        for interval in [StartupInterval::Bound, StartupInterval::BeforePrivateMode] {
            let mut endpoint = Endpoint::new();
            let held = RunLock::acquire(&endpoint.fixture.conn, &endpoint.fixture.run.owner())
                .unwrap()
                .unwrap();
            let process = ProcessIdentity::current().unwrap();
            held.transfer_to_runner(
                &mut endpoint.fixture.conn,
                &endpoint.fixture.run.owner(),
                process.clone(),
            )
            .unwrap();
            endpoint.fixture.conn.execute(
                "UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2 WHERE run_id=?3",
                rusqlite::params![process.pid,process.start_identity,endpoint.fixture.run.run_id],
            ).unwrap();
            endpoint.fixture.run =
                storage::get_run(&endpoint.fixture.conn, endpoint.fixture.run.run_id).unwrap();
            let listener = match interval {
                StartupInterval::Bound => bound_socket(endpoint.path(), 0o600, false),
                StartupInterval::BeforePrivateMode => bound_socket(endpoint.path(), 0o644, true),
            };
            let started = Instant::now();
            let result = probe_health(&endpoint.fixture.run, started + Duration::from_secs(1));
            assert!(result.is_err(), "{result:?}");
            assert!(started.elapsed() < Duration::from_millis(500));
            if matches!(interval, StartupInterval::BeforePrivateMode) {
                assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
            }
        }
    }
}
