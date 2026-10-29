use super::observation_services::{capture, private_write};
use super::*;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct ServiceReturn {
    at: Instant,
    deadline: Option<Instant>,
    status: Option<i32>,
    error: Option<String>,
}

struct ExportHooks {
    directory: PathBuf,
    stall: Option<NativeCaptureExportBoundary>,
    returned: Mutex<Option<ServiceReturn>>,
}

fn write_record(path: &Path, value: &Value) {
    let pending = path.with_extension("pending");
    private_write(&pending, &serde_json::to_vec(value).unwrap());
    std::fs::rename(pending, path).unwrap();
}

impl NativeCaptureExportHooks for ExportHooks {
    fn service_returned(
        &self,
        storage_deadline: Option<Instant>,
        result: &Result<i32, crate::error::CafleetError>,
    ) {
        let at = Instant::now();
        let (status, error) = match result {
            Ok(status) => (Some(*status), None),
            Err(cause) => (None, Some(cause.message().into_owned())),
        };
        let mut observed = self.returned.lock().unwrap();
        assert!(observed.is_none(), "one actual service-return notification");
        *observed = Some(ServiceReturn {
            at,
            deadline: storage_deadline,
            status,
            error,
        });
    }
    fn before(
        &self,
        boundary: NativeCaptureExportBoundary,
        deadline: Instant,
    ) -> Result<(), crate::error::CafleetError> {
        private_write(
            &self.directory.join(format!("export-entered-{boundary:?}")),
            &serde_json::to_vec(&json!({
                "boundary": format!("{boundary:?}"),
                "remaining_ms": deadline.saturating_duration_since(Instant::now()).as_millis(),
            }))
            .unwrap(),
        );
        if self.stall == Some(boundary) {
            let emergency = Instant::now() + Duration::from_secs(10);
            while !self
                .directory
                .join("export-release")
                .try_exists()
                .map_err(io_error)?
            {
                ensure(
                    Instant::now() < emergency,
                    "owned export stall exceeded its emergency bound",
                )?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }
}

impl Drop for ExportHooks {
    fn drop(&mut self) {
        if let Err(cause) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.directory.join("export-hooks-dropped"))
        {
            eprintln!("owned exporter completion marker failed: {cause}");
        }
    }
}

pub(crate) fn runner_collector_export_child_path() -> &'static str {
    test_path!(runner_collector_export_child)
}

#[test]
#[ignore = "Owned synthetic exporter fixture selects this private entry"]
fn runner_collector_export_child() {
    let path =
        std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG").expect("owned exporter configuration");
    let path = Path::new(&path);
    let directory = path.parent().expect("owned exporter directory");
    let plan: Value =
        serde_json::from_slice(&std::fs::read(directory.join("export-test-plan.json")).unwrap())
            .unwrap();
    let stall = match plan["stall"].as_str() {
        Some("encode") => Some(NativeCaptureExportBoundary::Encode),
        Some("manifest") => Some(NativeCaptureExportBoundary::ManifestPublish),
        None if plan["stall"].is_null() => None,
        other => panic!("owned export stall selection required: {other:?}"),
    };
    let hooks = Arc::new(ExportHooks {
        directory: directory.to_path_buf(),
        stall,
        returned: Mutex::new(None),
    });
    let begin = Instant::now();
    let result = run_native_capture_verification_with_export_hooks(path, hooks.clone());
    let completed = Instant::now();
    let service = match hooks.returned.lock().unwrap().as_ref() {
        Some(observed) => json!({
            "returned_ns":observed.at.duration_since(begin).as_nanos(),
            "storage_deadline_ns":observed.deadline.map(|deadline|deadline.duration_since(begin).as_nanos()),
            "deadline_exhausted":observed.deadline.map(|deadline|observed.at >= deadline),
            "status":observed.status,
            "error":observed.error,
            "completion_after_service_ns":completed.duration_since(observed.at).as_nanos(),
        }),
        None => Value::Null,
    };
    let mut record = match &result {
        Ok(status) => {
            json!({"result":"success", "status":status, "elapsed_ms":begin.elapsed().as_millis()})
        }
        Err(cause) => {
            json!({"result":"error", "message":cause.message(), "elapsed_ms":begin.elapsed().as_millis()})
        }
    };
    record["completed_ns"] = json!(completed.duration_since(begin).as_nanos());
    record["service"] = service;
    write_record(&directory.join("export-helper-returned.json"), &record);
    drop(hooks);
    let until = Instant::now() + Duration::from_secs(10);
    while !directory.join("export-hooks-dropped").exists() {
        assert!(
            Instant::now() < until,
            "owned exporter did not release its final hook reference"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    if plan["startup_refusal"] == true {
        assert!(
            result.is_err(),
            "invalid producer fails before native startup"
        );
        std::process::exit(1);
    }
}

struct ReleaseExport {
    path: PathBuf,
    released: bool,
}
impl ReleaseExport {
    fn new(fixture: &ServiceFixture) -> Self {
        Self {
            path: fixture.fixture._dir.path().join("export-release"),
            released: false,
        }
    }
    fn release(&mut self) {
        private_write(&self.path, b"owned exporter release after helper return");
        self.released = true;
    }
}
impl Drop for ReleaseExport {
    fn drop(&mut self) {
        if !self.released
            && let Err(cause) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&self.path)
        {
            eprintln!("owned export emergency release failed: {cause}");
        }
    }
}

struct StorageBlocker(rusqlite::Connection);
impl StorageBlocker {
    fn new(fixture: &ServiceFixture) -> Self {
        let connection = crate::db::connect(&format!(
            "sqlite:///{}",
            fixture.fixture.conn.path().unwrap()
        ))
        .unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        Self(connection)
    }
}
impl Drop for StorageBlocker {
    fn drop(&mut self) {
        if let Err(cause) = self.0.execute_batch("ROLLBACK") {
            eprintln!("owned storage lock release failed: {cause}");
        }
    }
}

fn configure(
    fixture: &mut Fixture,
    spec: &mut SpawnSpec,
    stall: Option<&str>,
    startup_refusal: bool,
) {
    capture(fixture, spec);
    private_write(
        &fixture._dir.path().join("collector-review-entry"),
        b"export",
    );
    private_write(
        &fixture._dir.path().join("export-test-plan.json"),
        &serde_json::to_vec(&json!({"stall":stall,"startup_refusal":startup_refusal})).unwrap(),
    );
}

fn wait_path(path: &Path, until: Instant) {
    while !path.exists() {
        assert!(
            Instant::now() < until,
            "owned exporter condition missed its deadline: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn arm_capture(fixture: &ServiceFixture, started: &Started) {
    private_write(
        &fixture.fixture._dir.path().join("capture-arm.json"),
        &serde_json::to_vec(&NativeCaptureArm {
            schema_version: 1,
            capture_id: "export-pair".into(),
            run_id: started.identity.run_id,
            owner_epoch: started.identity.owner_epoch,
            minimum_interval_ms: 100,
            timeout_ms: 30000,
        })
        .unwrap(),
    );
}

fn helper_record(fixture: &ServiceFixture) -> Value {
    serde_json::from_slice(
        &std::fs::read(
            fixture
                .fixture
                ._dir
                .path()
                .join("export-helper-returned.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn assert_unfinished(record: &Value) {
    assert_eq!(record["result"], "error", "{record}");
    let message = record["message"]
        .as_str()
        .expect("explicit unfinished exporter error")
        .to_ascii_lowercase();
    assert!(
        message.contains("unfinished")
            && (message.contains("export") || message.contains("capture")),
        "{record}"
    );
}

fn assert_original_completion_bound(record: &Value) {
    let service = record["service"]
        .as_object()
        .expect("actual service-return observation");
    let deadline = service["storage_deadline_ns"]
        .as_u64()
        .expect("original service storage deadline");
    let completed = record["completed_ns"].as_u64().unwrap();
    assert!(
        completed <= deadline + 100_000_000,
        "helper completion stays within original deadline plus 100 ms scheduling allowance: {record}"
    );
}

fn assert_expired_service_failure(record: &Value) {
    assert_original_completion_bound(record);
    let service = &record["service"];
    assert_eq!(
        service["deadline_exhausted"], true,
        "actual service return exhausts its own storage deadline: {record}"
    );
    assert_eq!(
        service["status"], 1,
        "native zero plus blocked storage yields actual incomplete-service status: {record}"
    );
    assert!(
        service["error"].is_null(),
        "record the uncombined actual Ok(1) service result: {record}"
    );
    assert!(
        service["returned_ns"].as_u64().unwrap()
            >= service["storage_deadline_ns"].as_u64().unwrap()
    );
    assert!(
        service["completion_after_service_ns"].as_u64().unwrap() <= 100_000_000,
        "expired original deadline permits only 100 ms scheduling allowance after actual service return: {record}"
    );
    assert_unfinished(record);
    let message = record["message"].as_str().unwrap().to_ascii_lowercase();
    assert!(
        message.lines().any(|line| line.contains("service")
            && line
                .split(|character: char| !character.is_ascii_digit())
                .any(|number| number == "1")),
        "combined helper result retains the independently observed service status alongside unfinished export: {record}"
    );
}

fn final_manifest(fixture: &ServiceFixture) -> PathBuf {
    fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/export-pair.manifest.json")
}

fn artifact_path(fixture: &ServiceFixture) -> PathBuf {
    fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/export-pair.complete.json")
}

struct OutputFile {
    path: PathBuf,
    device: u64,
    inode: u64,
    bytes: Vec<u8>,
    _identity_pin: File,
}
impl OutputFile {
    fn capture(path: PathBuf) -> Self {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&path)
            .unwrap();
        let metadata = file.metadata().unwrap();
        assert!(
            metadata.is_file(),
            "owned publication snapshot is a regular file"
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes,
            _identity_pin: file,
        }
    }
    fn assert_unchanged(&self) {
        let metadata = std::fs::symlink_metadata(&self.path).unwrap();
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (self.device, self.inode),
            "preserved identity: {}",
            self.path.display()
        );
        assert_eq!(
            std::fs::read(&self.path).unwrap(),
            self.bytes,
            "preserved bytes: {}",
            self.path.display()
        );
    }
}

fn seed_unrelated(fixture: &ServiceFixture) -> OutputFile {
    let path = fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/unrelated-output.txt");
    private_write(&path, b"pre-existing unrelated output identity");
    OutputFile::capture(path)
}

fn output_paths(fixture: &ServiceFixture) -> std::collections::BTreeSet<PathBuf> {
    std::fs::read_dir(fixture.fixture._dir.path().join("capture-output"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

fn temporary_outputs(fixture: &ServiceFixture, preserved: &[&OutputFile]) -> Vec<OutputFile> {
    let kept: std::collections::BTreeSet<_> =
        preserved.iter().map(|file| file.path.clone()).collect();
    output_paths(fixture)
        .difference(&kept)
        .cloned()
        .map(OutputFile::capture)
        .collect()
}

fn assert_removed(temporaries: &[OutputFile]) {
    for file in temporaries {
        assert!(
            matches!(std::fs::symlink_metadata(&file.path),Err(cause) if cause.kind() == ErrorKind::NotFound),
            "worker removes its still-matching temporary before fixture cleanup: {}",
            file.path.display()
        );
    }
}

fn assert_only_preserved(fixture: &ServiceFixture, preserved: &[&OutputFile]) {
    for file in preserved {
        file.assert_unchanged();
    }
    let kept: std::collections::BTreeSet<_> =
        preserved.iter().map(|file| file.path.clone()).collect();
    assert_eq!(
        output_paths(fixture),
        kept,
        "worker leaves no task-created temporary or late output before TempDir cleanup"
    );
}

#[test]
fn runner_collector_t1_encode_and_prepermit_manifest_stalls_return_unfinished_and_revoke_late_publication()
 {
    for (selected, boundary) in [("encode", "Encode"), ("manifest", "ManifestPublish")] {
        let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
            configure(fixture, spec, Some(selected), false)
        });
        let mut release = ReleaseExport::new(&fixture);
        let unrelated = seed_unrelated(&fixture);
        let started = fixture.start_running();
        arm_capture(&fixture, &started);
        wait_path(
            &fixture
                .fixture
                ._dir
                .path()
                .join(format!("export-entered-{boundary}")),
            Instant::now() + Duration::from_secs(3),
        );
        assert!(!final_manifest(&fixture).exists());
        let published_artifact =
            (selected == "manifest").then(|| OutputFile::capture(artifact_path(&fixture)));
        let mut preserved = vec![&unrelated];
        if let Some(artifact) = &published_artifact {
            preserved.push(artifact);
        }
        let temporaries = temporary_outputs(&fixture, &preserved);
        assert!(started.child.matches().unwrap());
        fixture.assert_manual_io();
        let shutdown = Instant::now();
        fixture.master.write_all(b"finish\r").unwrap();
        wait_path(
            &fixture
                .fixture
                ._dir
                .path()
                .join("export-helper-returned.json"),
            shutdown + Duration::from_secs(5),
        );
        assert_unfinished(&helper_record(&fixture));
        assert_original_completion_bound(&helper_record(&fixture));
        assert!(!started.child.matches().unwrap());
        assert!(!final_manifest(&fixture).exists());
        release.release();
        wait_path(
            &fixture.fixture._dir.path().join("export-hooks-dropped"),
            Instant::now() + Duration::from_secs(2),
        );
        assert_eq!(fixture.wait().code(), Some(0));
        assert_removed(&temporaries);
        assert_only_preserved(&fixture, &preserved);
        assert!(!final_manifest(&fixture).exists());
        if selected == "encode" {
            assert!(
                !fixture
                    .fixture
                    ._dir
                    .path()
                    .join("export-entered-ArtifactPublish")
                    .exists()
            );
        }
        let run = storage::get_run(&fixture.fixture.conn, started.identity.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Exited);
        assert_eq!(run.exit_code, Some(0));
    }
}

#[test]
fn runner_collector_t1_confirmed_pair_remains_accepted_while_native_work_continues_and_after_exit()
{
    let mut fixture =
        ServiceFixture::configured(false, |fixture, spec| configure(fixture, spec, None, false));
    let started = fixture.start_running();
    arm_capture(&fixture, &started);
    let manifest_path = final_manifest(&fixture);
    wait_path(&manifest_path, Instant::now() + Duration::from_secs(3));
    let original_manifest = std::fs::read(&manifest_path).unwrap();
    let accepted_manifest = OutputFile::capture(manifest_path.clone());
    let manifest: Value = serde_json::from_slice(&original_manifest).unwrap();
    let artifact_path = fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/export-pair.complete.json");
    let original_artifact = std::fs::read(&artifact_path).unwrap();
    let accepted_artifact = OutputFile::capture(artifact_path.clone());
    assert_eq!(manifest["publication_status"], "complete");
    assert_eq!(manifest["artifact_sha256"], sha256_hex(&original_artifact));
    assert_eq!(manifest["artifact_bytes"], original_artifact.len() as u64);
    assert!(started.child.matches().unwrap());
    assert!(
        !fixture
            .fixture
            ._dir
            .path()
            .join("export-helper-returned.json")
            .exists()
    );
    fixture.health();
    fixture.assert_manual_io();
    fixture.finish();
    let record = helper_record(&fixture);
    assert_eq!(record["result"], "success", "{record}");
    assert_eq!(record["status"], 0);
    assert_eq!(std::fs::read(manifest_path).unwrap(), original_manifest);
    assert_eq!(std::fs::read(artifact_path).unwrap(), original_artifact);
    accepted_manifest.assert_unchanged();
    accepted_artifact.assert_unchanged();
}

#[test]
fn runner_collector_t1_cancelled_manifest_worker_preserves_foreign_identities_and_cleans_its_matching_temporaries()
 {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, Some("manifest"), false)
    });
    let mut release = ReleaseExport::new(&fixture);
    let unrelated = seed_unrelated(&fixture);
    let started = fixture.start_running();
    arm_capture(&fixture, &started);
    wait_path(
        &fixture
            .fixture
            ._dir
            .path()
            .join("export-entered-ManifestPublish"),
        Instant::now() + Duration::from_secs(3),
    );
    assert!(!final_manifest(&fixture).exists());
    let artifact = OutputFile::capture(artifact_path(&fixture));
    let mut temporaries = temporary_outputs(&fixture, &[&unrelated, &artifact]);
    let shutdown = Instant::now();
    fixture.master.write_all(b"finish\r").unwrap();
    wait_path(
        &fixture
            .fixture
            ._dir
            .path()
            .join("export-helper-returned.json"),
        shutdown + Duration::from_secs(5),
    );
    let original_result = helper_record(&fixture);
    assert_unfinished(&original_result);
    assert_original_completion_bound(&original_result);
    assert!(!started.child.matches().unwrap());

    let replaceable =
        temporaries
            .iter()
            .position(|file| match std::fs::symlink_metadata(&file.path) {
                Ok(metadata) => (metadata.dev(), metadata.ino()) == (file.device, file.inode),
                Err(cause) if cause.kind() == ErrorKind::NotFound => false,
                Err(cause) => panic!("owned temporary inspection failed: {cause}"),
            });
    let (foreign_temporary, displaced) = if let Some(index) = replaceable {
        let original = temporaries.remove(index);
        let displaced_path = fixture
            .fixture
            ._dir
            .path()
            .join("displaced-exporter-temporary");
        std::fs::rename(&original.path, &displaced_path).unwrap();
        let displaced = OutputFile::capture(displaced_path);
        assert_eq!(
            (displaced.device, displaced.inode),
            (original.device, original.inode)
        );
        private_write(
            &original.path,
            b"foreign replacement at an exporter temporary name",
        );
        let replacement = OutputFile::capture(original.path.clone());
        assert_ne!(
            (replacement.device, replacement.inode),
            (original.device, original.inode)
        );
        (replacement, Some((original, displaced)))
    } else {
        let path = final_manifest(&fixture).with_extension("provisional");
        assert!(
            !path.exists(),
            "no current owned temporary occupies the collision name"
        );
        private_write(&path, b"foreign file at the publisher temporary name");
        (OutputFile::capture(path), None)
    };
    let collision_path = final_manifest(&fixture);
    private_write(
        &collision_path,
        b"pre-existing collision output before revoked worker resumes",
    );
    let collision = OutputFile::capture(collision_path);
    let unrelated_temporary_path = fixture
        .fixture
        ._dir
        .path()
        .join("capture-output/foreign.provisional");
    private_write(&unrelated_temporary_path, b"unrelated provisional identity");
    let unrelated_temporary = OutputFile::capture(unrelated_temporary_path);

    release.release();
    wait_path(
        &fixture.fixture._dir.path().join("export-hooks-dropped"),
        Instant::now() + Duration::from_secs(2),
    );
    assert_eq!(fixture.wait().code(), Some(0));
    assert_removed(&temporaries);
    assert_only_preserved(
        &fixture,
        &[
            &unrelated,
            &artifact,
            &foreign_temporary,
            &collision,
            &unrelated_temporary,
        ],
    );
    assert_eq!(
        helper_record(&fixture),
        original_result,
        "later worker completion does not acknowledge the unfinished invocation"
    );
    if let Some((original, displaced)) = displaced {
        displaced.assert_unchanged();
        assert_eq!(displaced.bytes, original.bytes);
        std::fs::remove_file(&displaced.path).unwrap();
    }
}

#[test]
fn runner_collector_t1_startup_failure_has_no_export_wait_or_publication() {
    let fixture = ServiceFixture::startup_refusal(|fixture, spec| {
        configure(fixture, spec, None, true);
        let path = fixture._dir.path().join("capture-producer.json");
        let mut producer: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        producer["collector_version"] = json!("invalid-owned-version");
        std::fs::write(path, serde_json::to_vec(&producer).unwrap()).unwrap();
    });
    let record = helper_record(&fixture);
    assert_eq!(record["result"], "error", "{record}");
    assert!(
        record["service"].is_null(),
        "pre-worker refusal invokes no actual service-return callback: {record}"
    );
    assert!(
        record["elapsed_ms"].as_u64().unwrap() < 1000,
        "absent service deadline grants no new exporter budget: {record}"
    );
    assert!(!record["message"].as_str().unwrap().contains("unfinished"));
    for boundary in ["Encode", "ArtifactPublish", "ManifestPublish"] {
        assert!(
            !fixture
                .fixture
                ._dir
                .path()
                .join(format!("export-entered-{boundary}"))
                .exists()
        );
    }
    assert!(!final_manifest(&fixture).exists());
    let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    assert!(run.child_pid.is_none());
}

#[test]
fn runner_collector_t1_exhausted_service_storage_budget_adds_no_fresh_exporter_wait() {
    let mut fixture = ServiceFixture::configured(false, |fixture, spec| {
        configure(fixture, spec, Some("encode"), false)
    });
    let mut release = ReleaseExport::new(&fixture);
    let unrelated = seed_unrelated(&fixture);
    let started = fixture.start_running();
    arm_capture(&fixture, &started);
    wait_path(
        &fixture.fixture._dir.path().join("export-entered-Encode"),
        Instant::now() + Duration::from_secs(3),
    );
    let temporaries = temporary_outputs(&fixture, &[&unrelated]);
    let blocker = StorageBlocker::new(&fixture);
    let shutdown = Instant::now();
    fixture.master.write_all(b"finish\r").unwrap();
    wait_path(
        &fixture
            .fixture
            ._dir
            .path()
            .join("export-helper-returned.json"),
        shutdown + Duration::from_secs(5),
    );
    assert_expired_service_failure(&helper_record(&fixture));
    assert!(!started.child.matches().unwrap());
    assert!(!final_manifest(&fixture).exists());
    drop(blocker);
    release.release();
    wait_path(
        &fixture.fixture._dir.path().join("export-hooks-dropped"),
        Instant::now() + Duration::from_secs(2),
    );
    assert_eq!(fixture.wait().code(), Some(0));
    assert_removed(&temporaries);
    assert_only_preserved(&fixture, &[&unrelated]);
    assert!(
        !fixture
            .fixture
            ._dir
            .path()
            .join("export-entered-ArtifactPublish")
            .exists()
    );
    assert!(!final_manifest(&fixture).exists());
}
