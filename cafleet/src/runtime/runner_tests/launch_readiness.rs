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

use super::control_codec::{read_fixture_bytes, write_fixture_bytes};
use super::*;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, MultiplexerContext, TmuxMultiplexer, recording};

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

fn read_prepare(stream: &mut UnixStream, identity: &ControlIdentity, deadline: Instant) -> Value {
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
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap()
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
        let result = with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
        finished.store(true, Ordering::Release);
        let (request, premature_connections) = server.join().unwrap();
        (result, request, premature_connections)
    });
    assert_eq!(premature_connections, 0);
    let prepared = result.expect("startup interval must become ready within its original deadline");
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
        let result = with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
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
