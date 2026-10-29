use super::*;
use crate::broker::records::Placement;
use crate::coding_agent::{ProfileCatalog, ScreenRegion};
use crate::runtime::runner_tests::owned_child::OwnedChild;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) fn private_write(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
pub(super) fn digest(path: &Path) -> String {
    sha256_hex(&std::fs::read(path).unwrap())
}
pub(super) fn catalog() -> ProfileCatalog {
    ProfileCatalog::from_definitions(
        &serde_json::from_str(include_str!("../fixtures/synthetic-profile.json")).unwrap(),
    )
    .unwrap()
}
pub(super) fn redacted() -> Value {
    json!({"mode":"redacted","config":{"schema_version":1,"patterns":["PRIVATE-[A-Z]+"],"sensitive_rules":[]}})
}
pub(super) fn mask(top: u16, left: u16, bottom: u16, right: u16) -> CaptureMask {
    CaptureMask {
        screen: CaptureScreen::Either,
        rows: 24,
        cols: 80,
        region: ScreenRegion {
            top,
            left,
            bottom,
            right,
        },
        reason: CaptureMaskReason::ReviewedPrivacy,
    }
}

pub(super) struct Clock {
    origin: Instant,
    millis: AtomicU64,
}
impl Clock {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
            millis: AtomicU64::new(0),
        }
    }
    pub(super) fn set(&self, millis: u64) {
        self.millis.store(millis, Ordering::Release);
    }
}
impl ObservationClock for Clock {
    fn sample(&self) -> ObservationMoment {
        ObservationMoment {
            monotonic: self.origin + Duration::from_millis(self.millis.load(Ordering::Acquire)),
            observed_at: "2026-10-26T17:00:00.000000+00:00".into(),
        }
    }
    fn kind(&self) -> ObservationClockKind {
        ObservationClockKind::Deterministic
    }
}

pub(super) struct CaptureFixture {
    pub fixture: Fixture,
    pub artifacts: RunArtifacts,
    pub spec: SpawnSpec,
    pub placement: Placement,
    pub producer_path: PathBuf,
    pub producer_json: Value,
    pub clock: Clock,
    _lock: RunLock,
    child: Option<OwnedChild>,
}
impl CaptureFixture {
    pub(super) fn new(policy: Value) -> Self {
        let mut fixture = Fixture::new();
        std::fs::set_permissions(fixture._dir.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let artifacts = fixture.artifacts();
        let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
            .unwrap()
            .unwrap();
        held.transfer_to_runner(
            &mut fixture.conn,
            &fixture.run.owner(),
            ProcessIdentity::current().unwrap(),
        )
        .unwrap();
        fixture.conn.execute("UPDATE member_runs SET runner_pid=owner_pid,runner_start_identity=owner_start_identity,native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',profile_match='exact',coverage='tested',event_evidence=?1 WHERE run_id=?2",rusqlite::params![policy["mode"].as_str().unwrap(),fixture.run.run_id]).unwrap();
        fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        let executable = fixture._dir.path().join("owned-synthetic-process");
        private_write(&executable,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'synthetic-0.0.1\\n'; exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; *) printf '%s\\n' \"$line\" ;; esac; done\n");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut spec = fixture.spec();
        spec.evidence_policy = policy;
        spec.argv = vec![
            executable.to_str().unwrap().into(),
            "--approve-for-me".into(),
            "private assignment canary".into(),
        ];
        artifacts
            .write_descriptor(&LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap())
            .unwrap();
        let source = fixture._dir.path().join("source-evidence.json");
        private_write(
            &source,
            b"{\"kind\":\"synthetic\",\"version\":\"synthetic-0.0.1\"}",
        );
        let current = std::env::current_exe().unwrap();
        let producer_json = json!({"schema_version":1,"kind":"synthetic","source_manifest_ref":source,"source_manifest_sha256":digest(&source),"libtest_executable_path":current,"libtest_executable_sha256":digest(&current),"parser_version":"0.18.1","collector_version":"1","catalog_origin":"synthetic","native_executable":{"path":executable,"sha256":digest(&executable),"version":"synthetic-0.0.1","version_evidence_ref":source},"launch_evidence_ref":source,"environment_evidence_ref":source});
        let producer_path = fixture._dir.path().join("producer.json");
        private_write(&producer_path, &serde_json::to_vec(&producer_json).unwrap());
        let placement =
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
        Self {
            fixture,
            artifacts,
            spec,
            placement,
            producer_path,
            producer_json,
            clock: Clock::new(),
            _lock: held,
            child: None,
        }
    }
    pub(super) fn channel(
        &self,
        limits: NativeCaptureLimits,
        masks: &[CaptureMask],
    ) -> (NativeCapturePort, NativeCaptureReceiver) {
        let policy = NativeCapturePolicy::from_launch(&self.spec, masks, &catalog()).unwrap();
        native_capture_channel(&self.fixture.run, policy, limits).unwrap()
    }
    pub(super) fn prepare(&mut self, port: &mut NativeCapturePort) {
        self.fixture
            .conn
            .execute(
                "UPDATE member_runs SET status='prepared' WHERE run_id=?1",
                [self.fixture.run.run_id],
            )
            .unwrap();
        self.fixture.run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
        let producer =
            read_native_capture_producer(&self.producer_path, &NativeCaptureLimits::default())
                .unwrap();
        port.bind_prepared(
            &self.fixture.run,
            &self.placement,
            &self.spec,
            producer,
            &self.clock,
        )
        .unwrap();
    }
    pub(super) fn finalize(&mut self, port: &mut NativeCapturePort) {
        let child = OwnedChild::new(
            Command::new(&self.spec.argv[0])
                .args(&self.spec.argv[1..])
                .current_dir(&self.spec.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        self.child = Some(child);
        let child = self
            .child
            .as_ref()
            .expect("owned child guard before identity validation");
        let identity = ProcessIdentity::read(i64::from(child.id()))
            .unwrap()
            .unwrap();
        self.fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at='2026-10-26T17:00:00Z' WHERE run_id=?3",rusqlite::params![identity.pid,identity.start_identity,self.fixture.run.run_id]).unwrap();
        self.fixture.run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
        port.finalize_source(&self.fixture.run, &self.placement, &identity)
            .unwrap();
    }
    pub(super) fn ready(
        &mut self,
        limits: NativeCaptureLimits,
        masks: &[CaptureMask],
    ) -> (NativeCapturePort, NativeCaptureReceiver) {
        let (mut port, receiver) = self.channel(limits, masks);
        self.prepare(&mut port);
        self.finalize(&mut port);
        (port, receiver)
    }
    pub(super) fn request(&self, id: &str) -> NativeCaptureRequest {
        NativeCaptureRequest {
            capture_id: id.into(),
            run_id: self.fixture.run.run_id,
            owner_epoch: self.fixture.run.owner_epoch,
            minimum_interval: Duration::from_millis(100),
            deadline: self.clock.sample().monotonic + Duration::from_secs(1),
        }
    }
    pub(super) fn pair(
        &mut self,
        port: &mut NativeCapturePort,
        receiver: &NativeCaptureReceiver,
        bytes: &[u8],
    ) -> NativeCaptureLease {
        let mut terminal = TerminalState::new(self.spec.size).unwrap();
        terminal.feed(bytes).unwrap();
        receiver.request(self.request("pair")).unwrap();
        port.observe(
            &terminal.snapshot(),
            CaptureTrigger::ChildOutput,
            &self.clock,
        )
        .unwrap();
        self.clock.set(100);
        port.observe(&terminal.snapshot(), CaptureTrigger::Timer, &self.clock)
            .unwrap();
        receiver
            .try_take()
            .unwrap()
            .expect("completed post-policy pair")
    }
}

pub(super) fn value(lease: &NativeCaptureLease) -> Value {
    serde_json::to_value(lease.result()).unwrap()
}
impl Drop for CaptureFixture {
    fn drop(&mut self) {
        self.child.take();
    }
}
pub(super) fn objects<'a>(value: &'a Value, key: &str, result: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map.contains_key(key) {
                result.push(value);
            }
            for child in map.values() {
                objects(child, key, result);
            }
        }
        Value::Array(values) => {
            for child in values {
                objects(child, key, result);
            }
        }
        _ => {}
    }
}
pub(super) fn assert_keys(value: &Value, keys: &[&str]) {
    let actual: std::collections::BTreeSet<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(actual, keys.iter().copied().collect());
}
