use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use serde_json::Value;

use super::*;
use crate::config::Settings;

const CHILD_TEST: &str = "runtime::runner_tests::control_service::runner_review_service_child";
const DESCRIPTOR_ENV: &str = "CAFLEET_REVIEW_RUNNER_DESCRIPTOR";

#[test]
fn runner_review_service_child() {
    let Some(descriptor) = std::env::var_os(DESCRIPTOR_ENV) else {
        return;
    };
    let settings = Settings {
        database_url: std::env::var("CAFLEET_DATABASE_URL").unwrap(),
        max_text_len: 200,
        multiplexer: Some("tmux".into()),
    };
    match run(&settings, Path::new(&descriptor)) {
        Ok(status) => std::process::exit(status),
        Err(cause) => {
            eprintln!("owned review runner failed: {cause}");
            std::process::exit(1);
        }
    }
}

fn wire_frame(value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    assert!(body.len() <= 65536);
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

struct Wire(UnixStream);

impl Wire {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        Self(stream)
    }

    fn exchange(&mut self, value: &Value) -> std::io::Result<Value> {
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

fn assert_rejected(reply: &std::io::Result<Value>) {
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

struct ServiceFixture {
    fixture: Fixture,
    _artifacts: RunArtifacts,
    child: Child,
    process: Option<ProcessIdentity>,
    master: File,
    _slave: File,
    creator: Option<Wire>,
    prepared: Option<ControlIdentity>,
    output: Vec<u8>,
    stopped: bool,
}

impl ServiceFixture {
    fn new(missing_executable: bool) -> Self {
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
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(DESCRIPTOR_ENV, &artifacts.paths.descriptor)
            .env(
                "CAFLEET_DATABASE_URL",
                format!("sqlite:///{}", fixture.conn.path().unwrap()),
            )
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
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
        };
        owned.process = Some(
            ProcessIdentity::read(i64::from(owned.child.id()))
                .unwrap()
                .unwrap(),
        );
        let until = Instant::now() + Duration::from_secs(3);
        while !owned._artifacts.paths.control_socket.exists() {
            if let Some(status) = owned.child.try_wait().unwrap() {
                let mut diagnostic = String::new();
                owned
                    .child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut diagnostic)
                    .unwrap();
                panic!("owned runner stopped before socket creation: {status}; {diagnostic}");
            }
            assert!(
                Instant::now() < until,
                "owned runner did not bind its private socket"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        owned
    }

    fn request(&self, kind: &str, identity: &ControlIdentity) -> Value {
        json!({"protocol_version":1,"kind":kind,"identity":identity})
    }

    fn prepare(&mut self) {
        let mut creator = Wire::connect(&self._artifacts.paths.control_socket);
        let mut request = self.request("prepare", &ControlIdentity::from_run(&self.fixture.run));
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

    fn publish(&mut self) {
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

    fn start(&mut self) -> std::io::Result<Value> {
        let mut request = self.request("start", self.prepared.as_ref().unwrap());
        request["pane_id"] = json!("%9");
        self.creator.as_mut().unwrap().exchange(&request)
    }

    fn start_running(&mut self) -> Started {
        self.prepare();
        self.publish();
        let reply = self.start().unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let started: Started = serde_json::from_value(reply["result"].clone()).unwrap();
        self.read_until("review-native-ready");
        started
    }

    fn health(&self) -> HealthSnapshot {
        let mut client = Wire::connect(&self._artifacts.paths.control_socket);
        let reply = client
            .exchange(&self.request("health", self.prepared.as_ref().unwrap()))
            .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        serde_json::from_value(reply["result"].clone()).unwrap()
    }

    fn read_until(&mut self, needle: &str) {
        let until = Instant::now() + Duration::from_secs(2);
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
                "missing native output {needle}: {:?}",
                self.output
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "runner stopped during manual I/O"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_manual_io(&mut self) {
        self.master.write_all(b"literal '$x' `bytes`\r").unwrap();
        self.read_until("review-answer=literal '$x' `bytes`");
    }

    fn wait(&mut self) -> ExitStatus {
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

    fn finish(&mut self) {
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

    fn stop_for_disconnect(&mut self) {
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

    fn resume(&mut self) {
        assert!(self.process.as_ref().unwrap().matches().unwrap());
        kill(
            Pid::from_raw(i32::try_from(self.child.id()).unwrap()),
            Signal::SIGCONT,
        )
        .unwrap();
        self.stopped = false;
    }

    fn cleanup_recorded_native(&self) {
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
                if let Err(cause) =
                    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGKILL)
                {
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
        let pid = Pid::from_raw(i32::try_from(self.child.id()).unwrap());
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
            Err(cause) => eprintln!("owned runner collection failed: {cause}"),
        }
        if let Err(cause) = kill(pid, Signal::SIGTERM) {
            eprintln!("owned runner termination failed: {cause}");
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.cleanup_recorded_native();
                    return;
                }
                Ok(None) => {}
                Err(cause) => eprintln!("owned runner collection failed: {cause}"),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.cleanup_recorded_native();
        if let Err(cause) = self.child.kill() {
            eprintln!("owned runner forced termination failed: {cause}");
        }
        if let Err(cause) = self.child.wait() {
            eprintln!("owned runner final collection failed: {cause}");
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
        let mut client = Wire::connect(&fixture._artifacts.paths.control_socket);
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
        let mut client = Wire::connect(&fixture._artifacts.paths.control_socket);
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
    let mut noncreator = Wire::connect(&fixture._artifacts.paths.control_socket);
    assert_rejected(&noncreator.exchange(&request));
    assert_eq!(fixture.health().child, None);
    let reply = fixture.start().unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let started: Started = serde_json::from_value(reply["result"].clone()).unwrap();
    fixture.read_until("review-native-ready");
    let mut second = Wire::connect(&fixture._artifacts.paths.control_socket);
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
    let mut disconnected = Wire::connect(&fixture._artifacts.paths.control_socket);
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
        RunLock::acquire(&fixture.fixture.conn, &run.owner())
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
        let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
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
            RunLock::acquire(&fixture.fixture.conn, &run.owner())
                .unwrap()
                .is_some()
        );
    }
}
