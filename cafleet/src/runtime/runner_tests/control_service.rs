use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(crate) mod input_service;
mod observation_contract;
pub(in crate::runtime) mod observation_services;
mod review_collector;
mod review_export;
mod review_failures;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use serde_json::Value;

use super::owned_child::OwnedChild;
use super::*;
use crate::config::Settings;

const CHILD_TEST: &str = "runtime::runner_tests::control_service::runner_review_service_child";
const DESCRIPTOR_ENV: &str = "CAFLEET_REVIEW_RUNNER_DESCRIPTOR";
const SERVICES_ENV: &str = "CAFLEET_REVIEW_TEST_SERVICES";

#[test]
fn runner_review_service_child() {
    if let Some(config) = std::env::var_os(input_service::NATIVE_ENV) {
        input_service::native(Path::new(&config));
        std::process::exit(0);
    }
    let Some(descriptor) = std::env::var_os(DESCRIPTOR_ENV) else {
        return;
    };
    if let Some(configured) = std::env::var_os(SERVICES_ENV) {
        let directory = Path::new(&configured).parent().unwrap();
        if directory.join("service-input-native.json").exists() {
            input_service::install_panic_record(directory, "runner");
        }
    }
    let settings = Settings {
        database_url: std::env::var("CAFLEET_DATABASE_URL").unwrap(),
        max_text_len: 200,
        multiplexer: Some("tmux".into()),
    };
    let outcome = match std::env::var_os(SERVICES_ENV) {
        Some(configured) => {
            let configured = Path::new(&configured);
            assert!(
                configured.is_absolute(),
                "explicit service configuration is absolute"
            );
            observation_services::run_configured(&settings, Path::new(&descriptor), configured)
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

fn wire_frame(value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    assert!(body.len() <= 65536);
    let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

struct Wire(UnixStream);

impl Wire {
    fn connect(path: &Path) -> std::io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        Ok(Self(stream))
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

pub(crate) struct ServiceFixture {
    pub(crate) fixture: Fixture,
    _artifacts: RunArtifacts,
    child: OwnedChild,
    process: Option<ProcessIdentity>,
    pub(in crate::runtime) master: File,
    _slave: File,
    creator: Option<Wire>,
    prepared: Option<ControlIdentity>,
    pub(in crate::runtime) output: Vec<u8>,
    stopped: bool,
    cleanup_deadline: Option<Instant>,
    startup_deadline: Instant,
}

impl ServiceFixture {
    fn assert_selected_config(&self, path: &Path) {
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
    fn connection_diagnostic(&mut self, stage: &str) -> String {
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

    fn connect(&mut self, stage: &str) -> Wire {
        match Wire::connect(&self._artifacts.paths.control_socket) {
            Ok(wire) => wire,
            Err(cause) => panic!(
                "owned runner connection failed: {cause}; {}; backtrace={}",
                self.connection_diagnostic(stage),
                std::backtrace::Backtrace::force_capture(),
            ),
        }
    }

    fn new(missing_executable: bool) -> Self {
        Self::configured(missing_executable, |_, _| {})
    }

    pub(in crate::runtime) fn configured(
        missing_executable: bool,
        configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
    ) -> Self {
        Self::launch(missing_executable, configure, false, None)
    }

    fn configured_until(
        missing_executable: bool,
        until: Instant,
        configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
    ) -> Self {
        Self::launch(missing_executable, configure, false, Some(until))
    }

    fn startup_refusal(configure: impl FnOnce(&mut Fixture, &mut SpawnSpec)) -> Self {
        Self::launch(false, configure, true, None)
    }

    fn launch(
        missing_executable: bool,
        configure: impl FnOnce(&mut Fixture, &mut SpawnSpec),
        refusal: bool,
        caller_deadline: Option<Instant>,
    ) -> Self {
        let startup_deadline = Instant::now() + Duration::from_secs(10);
        let until = caller_deadline.map_or(startup_deadline, |caller| caller.min(startup_deadline));
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
                    "resize" => {
                        "runtime::runner::native_capture::review_service_tests::runner_collector_resize_child"
                    }
                    "export" => {
                        "runtime::runner_tests::control_service::review_export::runner_collector_export_child"
                    }
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
                "runtime::runner_tests::native_capture::native_capture_service_child"
            } else {
                CHILD_TEST
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
        let child = OwnedChild::new(
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

    fn request(&self, kind: &str, identity: &ControlIdentity) -> Value {
        json!({"protocol_version":1,"kind":kind,"identity":identity})
    }

    fn prepare(&mut self) {
        let mut creator = self
            .creator
            .take()
            .expect("connected initial creator stream");
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

    pub(in crate::runtime) fn start_running(&mut self) -> Started {
        self.prepare();
        self.publish();
        let reply = self.start().unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let started: Started = serde_json::from_value(reply["result"].clone()).unwrap();
        self.read_until("review-native-ready");
        started
    }

    fn health(&mut self) -> HealthSnapshot {
        let mut client = self.connect("health");
        let reply = client
            .exchange(&self.request("health", self.prepared.as_ref().unwrap()))
            .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        serde_json::from_value(reply["result"].clone()).unwrap()
    }

    pub(in crate::runtime) fn read_until(&mut self, needle: &str) {
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

    pub(in crate::runtime) fn finish(&mut self) {
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
            RunLock::acquire_released(&fixture.fixture.conn, &run.owner())
                .unwrap()
                .is_some()
        );
    }
}
