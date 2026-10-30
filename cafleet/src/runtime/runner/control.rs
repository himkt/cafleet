use super::codec::{AdmittedControlFrame, decode_legacy_control_body};
use super::input_protocol::{decode_input_reply, encode_input_request};
use super::*;

use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, RunRecord};
use crate::multiplexer::{AnyMultiplexer, MultiplexerContext};
use serde_json::json;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

pub(super) fn pause(deadline: Instant) -> Result<()> {
    check_deadline(Instant::now(), deadline)?;
    nap_until(deadline);
    Ok(())
}
pub(super) struct ControlClient {
    pub(crate) stream: UnixStream,
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
        let identity = ControlIdentity::from_descriptor(descriptor);
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
        Ok(ShutdownOutcome::new(None, notes))
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
mod tests {
    use super::*;
    use std::time::Duration;

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
        use crate::multiplexer::{
            AnyMultiplexer, MultiplexerContext, TmuxMultiplexer, test_support,
        };
        use crate::runtime::runner::test_support::{
            Fixture, read_fixture_bytes, write_fixture_bytes,
        };

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
            let runner = test_support::FakeRunner::without_binaries();
            let mux =
                AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()));
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

        use crate::runtime::runner::codec::test_support::independent_frame as raw_frame;

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
                        StartupInterval::Bound => {
                            listen(&listener, Backlog::new(8).unwrap()).unwrap()
                        }
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
            let result =
                with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
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
                    StartupInterval::BeforePrivateMode => {
                        bound_socket(endpoint.path(), 0o644, true)
                    }
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
    mod input_client_tests {
        use super::*;
        use crate::broker::runtime::InputKind;
        use crate::runtime::runner::input_protocol::{
            InputControlOperation, InputControlReply, InputControlRequest, SubmissionStage,
            SubmissionStatus, SubmitInput,
        };
        use crate::runtime::runner::test_support::Fixture;
        use serde_json::json;
        use std::net::Shutdown;
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::thread::JoinHandle;

        type PeerResult<T> = std::result::Result<T, String>;
        use crate::runtime::runner::codec::test_support::independent_frame as frame;
        fn request(fixture: &Fixture) -> InputControlRequest {
            InputControlRequest {
                identity: ControlIdentity::from_run(&fixture.run),
                expires_at_mono_ns: 50_000_000_000,
                operation: InputControlOperation::SubmitPrompt(SubmitInput {
                    operation_key: "fixture:typed-client".into(),
                    input_kind: InputKind::Prompt,
                    message_id: None,
                    literal: "literal 世界 '$x'".into(),
                }),
            }
        }
        fn expected(fixture: &Fixture) -> Value {
            let run = &fixture.run;
            json!({"protocol_version":1,"kind":"submit_prompt","identity":{"database_identity_json":run.database_identity_json,"fleet_id":run.fleet_id,"member_id":run.member_id,"director_member_id":run.director_member_id,"run_id":run.run_id,"owner_epoch":run.owner_epoch,"launch_nonce":run.launch_nonce},"expires_at_mono_ns":50_000_000_000u64,"operation_key":"fixture:typed-client","input_kind":"prompt","message_id":null,"literal":"literal 世界 '$x'"})
        }
        fn reply(status: &str, stage: &str) -> Vec<u8> {
            let reason = if status == "accepted" {
                Value::Null
            } else {
                json!("fixture uncertainty")
            };
            frame(
            &serde_json::to_vec(
                &json!({"protocol_version":1,"result":{"kind":"submission","submission":{
                    "status":status,"input_id":13,"stage":stage,"output_seq":null,"reason":reason
                }}}),
            )
            .unwrap(),
        )
        }
        fn pair() -> (ControlClient, UnixStream) {
            let (stream, peer) = UnixStream::pair().unwrap();
            stream.set_nonblocking(true).unwrap();
            peer.set_nonblocking(true).unwrap();
            (
                ControlClient {
                    stream,
                    decoder: ControlDecoder::new(),
                },
                peer,
            )
        }
        fn active(stop: &AtomicBool, deadline: Instant) -> PeerResult<()> {
            if stop.load(Ordering::Acquire) {
                return Err("owned peer cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("owned peer deadline expired".into());
            }
            Ok(())
        }
        fn write_peer(
            stream: &mut UnixStream,
            mut bytes: &[u8],
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<()> {
            while !bytes.is_empty() {
                active(stop, deadline)?;
                match stream.write(bytes) {
                    Ok(0) => return Err("peer write made no progress".into()),
                    Ok(count) => bytes = &bytes[count..],
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(format!("peer write: {error}")),
                }
            }
            Ok(())
        }
        fn read_peer(
            stream: &mut UnixStream,
            mut bytes: &mut [u8],
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<()> {
            while !bytes.is_empty() {
                active(stop, deadline)?;
                match stream.read(bytes) {
                    Ok(0) => return Err("peer received truncated request".into()),
                    Ok(count) => bytes = &mut bytes[count..],
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::Interrupted
                        ) =>
                    {
                        std::thread::yield_now()
                    }
                    Err(error) => return Err(format!("peer read: {error}")),
                }
            }
            Ok(())
        }
        fn read_request(
            stream: &mut UnixStream,
            stop: &AtomicBool,
            deadline: Instant,
        ) -> PeerResult<Value> {
            let mut header = [0; 4];
            read_peer(stream, &mut header, stop, deadline)?;
            let size =
                usize::try_from(u32::from_be_bytes(header)).map_err(|error| error.to_string())?;
            if size > 65536 {
                return Err("peer received oversized request".into());
            }
            let mut body = vec![0; size];
            read_peer(stream, &mut body, stop, deadline)?;
            serde_json::from_slice(&body).map_err(|error| error.to_string())
        }
        struct Observed {
            request: Value,
            at: Instant,
            fragments_at: Option<(Instant, Instant)>,
            drained_after_return_at: Option<Instant>,
            extra_connections: usize,
            extra_request_bytes: usize,
        }
        static UNCONFIRMED_PEERS: Mutex<Vec<JoinHandle<PeerResult<Observed>>>> =
            Mutex::new(Vec::new());
        struct Peer {
            stop: Arc<AtomicBool>,
            handle: Option<JoinHandle<PeerResult<Observed>>>,
        }
        impl Peer {
            fn spawn(
                work: impl FnOnce(Arc<AtomicBool>, Instant) -> PeerResult<Observed> + Send + 'static,
            ) -> Self {
                let stop = Arc::new(AtomicBool::new(false));
                let control = stop.clone();
                let until = Instant::now() + Duration::from_secs(2);
                let handle = std::thread::spawn(move || work(control, until));
                Self {
                    stop,
                    handle: Some(handle),
                }
            }
            fn finish(&mut self) -> PeerResult<Observed> {
                self.finish_until(Instant::now() + Duration::from_secs(2))
            }
            fn finish_until(&mut self, until: Instant) -> PeerResult<Observed> {
                self.stop.store(true, Ordering::Release);
                while !self
                    .handle
                    .as_ref()
                    .expect("owned peer finishes once")
                    .is_finished()
                {
                    if Instant::now() >= until {
                        return Err(
                            "owned peer cleanup unconfirmed: deadline expired; handle retained"
                                .into(),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                self.handle
                    .take()
                    .expect("completed peer remains owned until join")
                    .join()
                    .map_err(|_| "owned peer terminated with a panic; join failed".to_owned())?
            }
        }
        impl Drop for Peer {
            fn drop(&mut self) {
                if self.handle.is_some()
                    && let Err(error) = self.finish()
                {
                    if let Some(handle) = self.handle.take() {
                        let mut retained = match UNCONFIRMED_PEERS.lock() {
                            Ok(retained) => retained,
                            Err(poisoned) => {
                                eprintln!(
                                    "recovering poisoned unconfirmed-peer ownership registry"
                                );
                                poisoned.into_inner()
                            }
                        };
                        retained.push(handle);
                        eprintln!(
                            "unconfirmed peer handle retained until process exit; thread termination remains unconfirmed"
                        );
                    }
                    if std::thread::panicking() {
                        eprintln!("owned peer cleanup: {error}");
                    } else {
                        panic!("owned peer cleanup: {error}");
                    }
                }
            }
        }
        fn finish_exchange(peer: &mut Peer, result: &Result<InputControlReply>) -> Observed {
            match peer.finish() {
                Ok(observed) => observed,
                Err(cleanup) => {
                    let primary = match result {
                        Err(error) => format!("Err({error:?})"),
                        Ok(InputControlReply::Submission(value)) => format!(
                            "Ok(Submission status={:?}, input_id={:?}, stage={:?}, output_seq={:?}, reason={:?})",
                            value.status,
                            value.input_id,
                            value.stage,
                            value.output_seq,
                            value.reason
                        ),
                        Ok(InputControlReply::Inspection(_)) => "Ok(Inspection)".into(),
                        Ok(InputControlReply::PermissionResponse(_)) => {
                            "Ok(PermissionResponse)".into()
                        }
                        Ok(InputControlReply::Receipt(_)) => "Ok(Receipt)".into(),
                    };
                    panic!("primary exchange result: {primary}; owned peer cleanup: {cleanup}");
                }
            }
        }
        fn accepted(result: Result<InputControlReply>) {
            match result.unwrap() {
                InputControlReply::Submission(submission) => {
                    assert_eq!(submission.status, SubmissionStatus::Accepted);
                    assert_eq!(submission.input_id, Some(13));
                    assert_eq!(submission.stage, SubmissionStage::Submitted);
                    assert_eq!(submission.output_seq, None);
                    assert_eq!(submission.reason, None);
                }
                _ => panic!("actual typed exchange returns submission"),
            }
        }
        fn preloaded(fixture: &Fixture, response: &[u8]) -> (Result<InputControlReply>, Value) {
            let (mut client, mut peer) = pair();
            let until = Instant::now() + Duration::from_secs(1);
            let stop = AtomicBool::new(false);
            write_peer(&mut peer, response, &stop, until).unwrap();
            let result = client.exchange_input(&request(fixture), until);
            let actual = read_request(&mut peer, &stop, until).unwrap();
            (result, actual)
        }
        fn baseline(fixture: &Fixture) {
            let (result, actual) = preloaded(fixture, &reply("accepted", "submitted"));
            accepted(result);
            assert_eq!(actual, expected(fixture));
        }

        #[test]
        fn step5_wire_x1_actual_typed_exchange_sends_independent_flat_request_and_original_expiry()
        {
            let fixture = Fixture::new();
            baseline(&fixture);
        }
        #[test]
        fn step5_wire_x2_fragmented_reply_and_raw_duplicate_or_wrong_kind_validation() {
            let fixture = Fixture::new();
            baseline(&fixture);
            let (mut client, mut stream) = pair();
            let response = reply("accepted", "submitted");
            let mut peer = Peer::spawn(move |stop, until| {
                let request = read_request(&mut stream, &stop, until)?;
                let at = Instant::now();
                for chunk in response.chunks(3) {
                    write_peer(&mut stream, chunk, &stop, until)?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(Observed {
                    request,
                    at,
                    fragments_at: None,
                    drained_after_return_at: None,
                    extra_connections: 0,
                    extra_request_bytes: 0,
                })
            });
            let result =
                client.exchange_input(&request(&fixture), Instant::now() + Duration::from_secs(1));
            let observed = finish_exchange(&mut peer, &result);
            accepted(result);
            assert_eq!(observed.request, expected(&fixture));
            let duplicate=frame(br#"{"protocol_version":1,"result":{"kind":"submission","submission":{"status":"accepted","input_id":13,"input_id":13,"stage":"submitted","output_seq":null,"reason":null}}}"#);
            let wrong=frame(br#"{"protocol_version":1,"result":{"kind":"receipt","receipt":{"status":"missing","input_id":null,"submission":null}}}"#);
            for response in [duplicate, wrong] {
                let (result, actual) = preloaded(&fixture, &response);
                assert!(result.is_err());
                assert_eq!(actual, expected(&fixture));
            }
        }
        #[test]
        fn step5_wire_x3_extra_reply_trailing_partial_and_premature_eof_reject() {
            let fixture = Fixture::new();
            baseline(&fixture);
            let good = reply("accepted", "submitted");
            let mut extra = good.clone();
            extra.extend_from_slice(&good);
            let mut partial = good.clone();
            partial.extend_from_slice(&good[..2]);
            for response in [extra, partial] {
                let (result, actual) = preloaded(&fixture, &response);
                assert!(result.is_err());
                assert_eq!(actual, expected(&fixture));
            }
            let (mut client, mut peer) = pair();
            let until = Instant::now() + Duration::from_secs(1);
            let stop = AtomicBool::new(false);
            write_peer(&mut peer, &good[..good.len() - 1], &stop, until).unwrap();
            peer.shutdown(Shutdown::Write).unwrap();
            let result = client.exchange_input(&request(&fixture), until);
            let actual = read_request(&mut peer, &stop, until).unwrap();
            assert!(result.is_err());
            assert_eq!(actual, expected(&fixture));
        }
        #[test]
        fn step5_wire_x4_expired_and_stalled_actual_exchange_keep_original_deadline() {
            let fixture = Fixture::new();
            baseline(&fixture);
            let (mut client, mut peer) = pair();
            let expired = Instant::now()
                .checked_sub(Duration::from_millis(1))
                .unwrap();
            assert!(client.exchange_input(&request(&fixture), expired).is_err());
            let mut byte = [0];
            let observation_deadline = Instant::now() + Duration::from_millis(100);
            loop {
                assert!(Instant::now() < observation_deadline);
                match peer.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => panic!("an expired invocation sends zero request bytes"),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => panic!("observe expired exchange bytes: {error}"),
                }
            }
            let (mut client, mut stream) = pair();
            let budget = Duration::from_millis(800);
            let fragment_delay = Duration::from_millis(350);
            let return_tolerance = Duration::from_millis(100);
            let started = Instant::now();
            let deadline = started + budget;
            let mut peer = Peer::spawn(move |stop, until| {
                let request = read_request(&mut stream, &stop, until)?;
                let at = Instant::now();
                write_peer(&mut stream, &[0, 0], &stop, until)?;
                let first_fragment_at = Instant::now();
                let next_fragment_at = first_fragment_at + fragment_delay;
                while Instant::now() < next_fragment_at {
                    active(&stop, until)?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                write_peer(&mut stream, &[0, 20], &stop, until)?;
                let later_fragment_at = Instant::now();
                while !stop.load(Ordering::Acquire) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(Observed {
                    request,
                    at,
                    fragments_at: Some((first_fragment_at, later_fragment_at)),
                    drained_after_return_at: None,
                    extra_connections: 0,
                    extra_request_bytes: 0,
                })
            });
            let result = client.exchange_input(&request(&fixture), deadline);
            let returned = Instant::now();
            let observed = finish_exchange(&mut peer, &result);
            let (first_fragment_at, later_fragment_at) = observed.fragments_at.unwrap();
            eprintln!(
                "X4 original budget={budget:?}; request={:?}; first fragment={:?}; later fragment={:?}; return={:?}; declared tolerance={return_tolerance:?}",
                observed.at.duration_since(started),
                first_fragment_at.duration_since(started),
                later_fragment_at.duration_since(started),
                returned.duration_since(started)
            );
            assert!(result.is_err());
            assert_eq!(observed.request, expected(&fixture));
            assert!(
                observed.at < deadline,
                "peer observed the actual request before the tested expiry"
            );
            assert!(observed.at <= first_fragment_at);
            assert!(first_fragment_at < later_fragment_at);
            assert!(later_fragment_at < deadline);
            assert!(later_fragment_at.duration_since(first_fragment_at) >= fragment_delay);
            assert!(later_fragment_at.duration_since(started) >= fragment_delay);
            assert!(return_tolerance < later_fragment_at.duration_since(first_fragment_at));
            assert!(returned >= deadline);
            assert!(
                returned < deadline + return_tolerance,
                "actual exchange shares its original budget"
            );
            assert!(returned < later_fragment_at + budget);
        }
        #[test]
        fn step5_wire_x5_disconnect_after_real_request_has_no_reconnect_or_second_request() {
            let fixture = Fixture::new();
            baseline(&fixture);
            let directory = tempfile::Builder::new()
                .prefix(".pwire-")
                .tempdir_in("/tmp")
                .unwrap();
            let path = directory.path().join("control.sock");
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let exchange_returned = Arc::new(AtomicBool::new(false));
            let returned_signal = exchange_returned.clone();
            let mut peer = Peer::spawn(move |stop, until| {
                let (mut stream, _) = loop {
                    active(&stop, until)?;
                    match listener.accept() {
                        Ok(value) => break value,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            std::thread::yield_now()
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                };
                stream
                    .set_nonblocking(true)
                    .map_err(|error| error.to_string())?;
                let request = read_request(&mut stream, &stop, until)?;
                let at = Instant::now();
                let mut trailing = [0; 4096];
                let mut extra_request_bytes = 0;
                stream
                    .shutdown(Shutdown::Write)
                    .map_err(|error| error.to_string())?;
                let mut extra_connections = 0;
                let drained_after_return_at = loop {
                    if Instant::now() >= until {
                        return Err("disconnect observer expired before final drain".into());
                    }
                    let after_return = exchange_returned.load(Ordering::Acquire);
                    loop {
                        if Instant::now() >= until {
                            return Err("disconnect request drain exceeded its bound".into());
                        }
                        match stream.read(&mut trailing) {
                            Ok(0) => break,
                            Ok(count) => extra_request_bytes += count,
                            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                            Err(error) if error.kind() == ErrorKind::Interrupted => {}
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    loop {
                        if Instant::now() >= until {
                            return Err("disconnect listener drain exceeded its bound".into());
                        }
                        match listener.accept() {
                            Ok((extra, _)) => {
                                extra_connections += 1;
                                drop(extra);
                            }
                            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                            Err(error) if error.kind() == ErrorKind::Interrupted => {}
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    if after_return {
                        break Instant::now();
                    }
                    if stop.load(Ordering::Acquire) && !exchange_returned.load(Ordering::Acquire) {
                        return Err(
                            "disconnect observer cancelled before actual exchange return".into(),
                        );
                    }
                    std::thread::yield_now();
                };
                Ok(Observed {
                    request,
                    at,
                    fragments_at: None,
                    drained_after_return_at: Some(drained_after_return_at),
                    extra_connections,
                    extra_request_bytes,
                })
            });
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut client = ControlClient::connect(&path, deadline).unwrap();
            let result = client.exchange_input(&request(&fixture), deadline);
            let returned_at = Instant::now();
            returned_signal.store(true, Ordering::Release);
            let observed = finish_exchange(&mut peer, &result);
            assert!(result.is_err());
            assert_eq!(observed.request, expected(&fixture));
            assert_eq!(observed.extra_connections, 0);
            assert_eq!(observed.extra_request_bytes, 0);
            assert!(observed.drained_after_return_at.unwrap() >= returned_at);
        }

        #[test]
        fn step5_wire_peer_cleanup_timeout_retains_ownership_until_release_and_confirmed_join() {
            let released = Arc::new(AtomicBool::new(false));
            let release = released.clone();
            let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
            let mut peer = Peer::spawn(move |_, until| {
                entered_tx.send(()).map_err(|error| error.to_string())?;
                while !released.load(Ordering::Acquire) {
                    if Instant::now() >= until {
                        return Err("ownership control release deadline expired".into());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(Observed {
                    request: json!({"fixture":"ownership-control"}),
                    at: Instant::now(),
                    fragments_at: None,
                    drained_after_return_at: None,
                    extra_connections: 0,
                    extra_request_bytes: 0,
                })
            });
            let entered = entered_rx.recv_timeout(Duration::from_millis(500));
            let owner_id = peer.handle.as_ref().unwrap().thread().id();
            let timeout = peer.finish_until(Instant::now() + Duration::from_millis(20));
            let retained_id = peer.handle.as_ref().map(|handle| handle.thread().id());
            let remained_live = peer
                .handle
                .as_ref()
                .is_some_and(|handle| !handle.is_finished());
            release.store(true, Ordering::Release);
            let joined = if peer.handle.is_some() {
                Some(peer.finish())
            } else {
                None
            };
            let first_outcome = match &timeout {
                Ok(observed) => format!("Ok(request={:?})", observed.request),
                Err(error) => format!("Err({error:?})"),
            };
            assert!(
                entered.is_ok(),
                "worker entry was acknowledged: {entered:?}; first finish: {first_outcome}"
            );
            match timeout {
                Err(error) => assert_eq!(
                    error, "owned peer cleanup unconfirmed: deadline expired; handle retained",
                    "timeout scenario was established; first finish: {first_outcome}"
                ),
                Ok(_) => panic!(
                    "unreleased worker must exceed the first cleanup deadline; first finish: {first_outcome}"
                ),
            }
            assert_eq!(retained_id, Some(owner_id), "first finish: {first_outcome}");
            assert!(remained_live, "first finish: {first_outcome}");
            match joined {
                Some(Ok(observed)) => {
                    assert_eq!(observed.request, json!({"fixture":"ownership-control"}))
                }
                Some(Err(error)) => panic!(
                    "released worker cleanup must confirm join: {error}; first finish: {first_outcome}"
                ),
                None => {
                    panic!(
                        "timeout retains an owned worker for final join; first finish: {first_outcome}"
                    )
                }
            }
            assert!(peer.handle.is_none(), "released worker has been joined");
        }
        #[test]
        fn step5_wire_x6_textual_error_remains_error_and_structured_uncertainty_remains_result() {
            let fixture = Fixture::new();
            baseline(&fixture);
            let error = frame(br#"{"protocol_version":1,"error":"fixture failure"}"#);
            let (result, actual) = preloaded(&fixture, &error);
            assert_eq!(actual, expected(&fixture));
            match result {
                Err(CafleetError::App(message)) => {
                    assert_eq!(message, "runner control: fixture failure")
                }
                Err(other) => panic!("ordinary application error expected: {other}"),
                Ok(_) => panic!("textual server failure is an error"),
            }
            for stage in ["partial", "unknown"] {
                let (result, actual) = preloaded(&fixture, &reply("uncertain", stage));
                assert_eq!(actual, expected(&fixture));
                match result.unwrap() {
                    InputControlReply::Submission(submission) => {
                        assert_eq!(submission.status, SubmissionStatus::Uncertain);
                        assert_eq!(submission.stage.as_str(), stage);
                        assert_eq!(submission.input_id, Some(13));
                    }
                    _ => panic!("structured uncertainty remains a submission result"),
                }
            }
        }
    }
}
