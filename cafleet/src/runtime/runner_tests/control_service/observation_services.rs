use super::*;
use crate::coding_agent::ProfileCatalog;
use crate::error::CafleetError;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::Arc;

struct Clock;
impl ObservationClock for Clock {
    fn sample(&self) -> ObservationMoment {
        ObservationMoment {
            monotonic: Instant::now(),
            observed_at: crate::time::format_utc(crate::time::now_utc()),
        }
    }
    fn kind(&self) -> ObservationClockKind {
        ObservationClockKind::Real
    }
}
struct Hooks {
    directory: std::path::PathBuf,
}
impl WriterHooks for Hooks {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> Result<(), CafleetError> {
        if matches!(boundary, WriterBoundary::Sync) && self.directory.join("fail-sync").exists() {
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
pub(super) fn run_configured(
    settings: &Settings,
    descriptor: &Path,
    config: &Path,
) -> Result<i32, CafleetError> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(config).map_err(io_error)?).map_err(io_error)?;
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
pub(super) fn private_write(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
pub(in crate::runtime) fn synthetic(fixture: &mut Fixture, spec: &mut SpawnSpec) {
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
    fixture.conn.execute("UPDATE member_runs SET native_version='synthetic-0.0.1',profile_version='synthetic-profile-1',profile_match='exact',coverage='tested' WHERE run_id=?1",[fixture.run.run_id]).unwrap();
    fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
    let script = fixture._dir.path().join("owned-observation-process");
    private_write(&script,b"#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'synthetic-0.0.1\\n'; exit 0; fi\n/usr/bin/touch \"$CAFLEET_REVIEW_NATIVE_START\"\nprintf 'review-native-ready\\n'\nwhile IFS= read -r line; do case \"$line\" in finish) exit 0 ;; form*) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write %s.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: ' \"$line\" ;; transient) printf '\\033[2J\\033[19;1HSynthetic permission request\\r\\nOperation: write transient.txt\\r\\n> Allow once\\r\\n  Deny once\\r\\nRequest form: synthetic-only\\r\\nSelection: \\033[2J\\033[Hworking' ;; *) printf 'review-answer=%s\\n' \"$line\" ;; esac; done\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    spec.argv = vec![
        script.to_str().unwrap().into(),
        "--approve-for-me".into(),
        "--sandbox".into(),
        "workspace-write".into(),
    ];
    let definitions: Value =
        serde_json::from_str(include_str!("../fixtures/synthetic-profile.json")).unwrap();
    private_write(
        &fixture._dir.path().join("observation-services.json"),
        &serde_json::to_vec(&json!({"catalog":definitions})).unwrap(),
    );
}
pub(super) fn wait_until(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < until, "bounded actual service condition");
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub(super) fn count(fixture: &ServiceFixture, kind: &str) -> i64 {
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
pub(super) fn health(fixture: &mut ServiceFixture) -> Value {
    let mut client = fixture.connect("observation status");
    let reply = client
        .exchange(&fixture.request("health", fixture.prepared.as_ref().unwrap()))
        .unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

pub(super) fn capture(fixture: &mut Fixture, spec: &mut SpawnSpec) {
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
        RuntimePaths::for_run(&fixture.run.database_identity_json, fixture.run.run_id).unwrap();
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
