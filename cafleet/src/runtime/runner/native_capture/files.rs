use super::*;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) fn file_digest(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes).map_err(io_error)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}
fn private_directory(path: &Path) -> Result<(u64, u64)> {
    ensure(path.is_absolute(), "private directory must be absolute")?;
    let meta = std::fs::symlink_metadata(path).map_err(io_error)?;
    ensure(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == nix::unistd::geteuid().as_raw()
            && meta.mode() & 0o777 == 0o700,
        "private owned directory required",
    )?;
    Ok((meta.dev(), meta.ino()))
}
pub(super) fn read_private(path: &Path, limit: usize) -> Result<Vec<u8>> {
    ensure(path.is_absolute(), "private file path must be absolute")?;
    validate_private_file(path, nix::unistd::geteuid().as_raw(), 0o600)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(io_error)?;
    let meta = file.metadata().map_err(io_error)?;
    let named = std::fs::symlink_metadata(path).map_err(io_error)?;
    ensure(
        meta.dev() == named.dev() && meta.ino() == named.ino() && meta.len() <= limit as u64,
        "private file identity/size mismatch",
    )?;
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).map_err(io_error)?);
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    ensure(bytes.len() <= limit, "private file exceeds budget")?;
    Ok(bytes)
}
fn reference(value: &Value) -> Result<&str> {
    let text = value
        .as_str()
        .ok_or_else(|| error("evidence reference required"))?;
    ensure(
        !text.trim().is_empty() && text.len() <= 2048 && !text.chars().any(|c| c.is_control()),
        "invalid evidence reference",
    )?;
    Ok(text)
}
fn lower_digest(value: &Value) -> Result<&str> {
    let text = value
        .as_str()
        .ok_or_else(|| error("digest string required"))?;
    ensure(
        text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "lowercase digest required",
    )?;
    Ok(text)
}
fn exact(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| error("capture object required"))?;
    ensure(
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)),
        "capture object fields mismatch",
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCaptureConfig {
    pub schema_version: u32,
    pub database_url: String,
    pub descriptor_path: PathBuf,
    pub expected_run_id: i64,
    pub expected_launch_nonce: String,
    pub arm_path: PathBuf,
    pub output_directory: PathBuf,
    pub producer_manifest_path: PathBuf,
    pub capture_policy_path: PathBuf,
}
pub(crate) fn read_native_capture_config(
    path: &Path,
    limits: &NativeCaptureLimits,
) -> Result<NativeCaptureConfig> {
    limits.validate()?;
    let value: NativeCaptureConfig =
        serde_json::from_slice(&read_private(path, limits.max_config_bytes)?).map_err(io_error)?;
    ensure(
        value.schema_version == 1 && value.expected_run_id > 0,
        "invalid capture config schema/run",
    )?;
    lower_digest(&json!(value.expected_launch_nonce))?;
    private_directory(&value.output_directory)?;
    private_directory(
        value
            .arm_path
            .parent()
            .ok_or_else(|| error("arm parent required"))?,
    )?;
    ensure(value.arm_path.is_absolute(), "absolute arm path required")?;
    for path in [
        &value.descriptor_path,
        &value.producer_manifest_path,
        &value.capture_policy_path,
    ] {
        validate_private_file(path, nix::unistd::geteuid().as_raw(), 0o600)?;
        ensure(
            path.is_absolute(),
            "absolute capture evidence path required",
        )?;
    }
    let database = value
        .database_url
        .strip_prefix("sqlite:///")
        .ok_or_else(|| error("capture database URL required"))?;
    ensure(
        Path::new(database).is_absolute() && Path::new(database).is_file(),
        "capture database file required",
    )?;
    Ok(value)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProducerNativeExecutable {
    path: String,
    sha256: String,
    version: Value,
    version_evidence_ref: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProducerDocument {
    schema_version: u32,
    kind: String,
    source_manifest_ref: String,
    source_manifest_sha256: String,
    libtest_executable_path: String,
    libtest_executable_sha256: String,
    parser_version: String,
    collector_version: String,
    catalog_origin: String,
    native_executable: ProducerNativeExecutable,
    launch_evidence_ref: String,
    environment_evidence_ref: String,
}
pub(crate) struct NativeCaptureProducer {
    pub(super) value: Value,
    pub(super) reference: PathBuf,
}
pub(crate) fn read_native_capture_producer(
    path: &Path,
    limits: &NativeCaptureLimits,
) -> Result<NativeCaptureProducer> {
    let document: ProducerDocument =
        serde_json::from_slice(&read_private(path, limits.max_producer_bytes)?)
            .map_err(io_error)?;
    let value = serde_json::to_value(document).map_err(io_error)?;
    exact(
        &value,
        &[
            "schema_version",
            "kind",
            "source_manifest_ref",
            "source_manifest_sha256",
            "libtest_executable_path",
            "libtest_executable_sha256",
            "parser_version",
            "collector_version",
            "catalog_origin",
            "native_executable",
            "launch_evidence_ref",
            "environment_evidence_ref",
        ],
    )?;
    ensure(
        value["schema_version"] == 1
            && matches!(
                value["kind"].as_str(),
                Some("synthetic" | "verification_libtest")
            )
            && matches!(
                value["catalog_origin"].as_str(),
                Some("synthetic" | "bundled_unknown")
            )
            && value["parser_version"] == "0.18.1"
            && value["collector_version"] == "1",
        "invalid capture producer identity",
    )?;
    for key in [
        "source_manifest_ref",
        "launch_evidence_ref",
        "environment_evidence_ref",
    ] {
        reference(&value[key])?;
    }
    lower_digest(&value["source_manifest_sha256"])?;
    let current = std::env::current_exe().map_err(io_error)?;
    let selected = Path::new(reference(&value["libtest_executable_path"])?);
    ensure(
        selected.is_absolute()
            && std::fs::canonicalize(selected).map_err(io_error)?
                == std::fs::canonicalize(current).map_err(io_error)?
            && lower_digest(&value["libtest_executable_sha256"])? == file_digest(selected)?,
        "capture libtest provenance mismatch",
    )?;
    let native = &value["native_executable"];
    exact(
        native,
        &["path", "sha256", "version", "version_evidence_ref"],
    )?;
    ensure(
        Path::new(reference(&native["path"])?).is_absolute(),
        "absolute native executable required",
    )?;
    lower_digest(&native["sha256"])?;
    reference(&native["version_evidence_ref"])?;
    if !native["version"].is_null() {
        reference(&native["version"])?;
    }
    let source = Path::new(reference(&value["source_manifest_ref"])?);
    if source.is_absolute() {
        ensure(
            file_digest(source)? == lower_digest(&value["source_manifest_sha256"])?,
            "capture source manifest digest mismatch",
        )?;
    }
    Ok(NativeCaptureProducer {
        value,
        reference: path.into(),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCaptureArm {
    pub schema_version: u32,
    pub capture_id: String,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub minimum_interval_ms: u64,
    pub timeout_ms: u64,
}
impl NativeCaptureArm {
    pub(crate) fn to_request(
        &self,
        identity: &ControlIdentity,
        now: Instant,
        limits: &NativeCaptureLimits,
    ) -> Result<NativeCaptureRequest> {
        limits.validate()?;
        validate_capture_id(&self.capture_id)?;
        ensure(
            self.schema_version == 1
                && self.run_id > 0
                && self.owner_epoch > 0
                && self.run_id == identity.run_id
                && self.owner_epoch == identity.owner_epoch
                && self.minimum_interval_ms >= 100
                && Duration::from_millis(self.minimum_interval_ms) <= limits.max_interval
                && self.timeout_ms > self.minimum_interval_ms
                && Duration::from_millis(self.timeout_ms) <= limits.max_request_duration,
            "invalid one-shot capture arm",
        )?;
        Ok(NativeCaptureRequest {
            capture_id: self.capture_id.clone(),
            run_id: self.run_id,
            owner_epoch: self.owner_epoch,
            minimum_interval: Duration::from_millis(self.minimum_interval_ms),
            deadline: now
                .checked_add(Duration::from_millis(self.timeout_ms))
                .ok_or_else(|| error("capture deadline overflow"))?,
        })
    }
}
pub(crate) struct NativeCaptureArmReader {
    path: PathBuf,
    parent: (u64, u64),
    identity: ControlIdentity,
    limits: NativeCaptureLimits,
    used: bool,
}
impl NativeCaptureArmReader {
    pub(crate) fn new(
        path: &Path,
        identity: ControlIdentity,
        limits: NativeCaptureLimits,
    ) -> Result<Self> {
        limits.validate()?;
        ensure(path.is_absolute(), "absolute arm path required")?;
        let parent = private_directory(path.parent().ok_or_else(|| error("arm parent required"))?)?;
        Ok(Self {
            path: path.into(),
            parent,
            identity,
            limits,
            used: false,
        })
    }
    pub(crate) fn poll(&mut self, now: Instant) -> Result<Option<NativeCaptureRequest>> {
        ensure(
            private_directory(self.path.parent().expect("validated arm parent"))? == self.parent,
            "arm parent replaced",
        )?;
        ensure(!self.used, "one-shot arm already consumed")?;
        let meta = match std::fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => return Err(io_error(cause)),
        };
        self.used = true;
        validate_private_file(&self.path, nix::unistd::geteuid().as_raw(), 0o600)?;
        let claimed = self.path.with_extension("claimed");
        std::fs::hard_link(&self.path, &claimed).map_err(io_error)?;
        let owned = std::fs::symlink_metadata(&claimed).map_err(io_error)?;
        ensure(
            owned.dev() == meta.dev() && owned.ino() == meta.ino(),
            "arm changed during claim",
        )?;
        let current = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
        ensure(
            current.dev() == owned.dev() && current.ino() == owned.ino(),
            "arm pathname replaced",
        )?;
        std::fs::remove_file(&self.path).map_err(io_error)?;
        let arm: NativeCaptureArm =
            serde_json::from_slice(&read_private(&claimed, self.limits.max_arm_bytes)?)
                .map_err(io_error)?;
        arm.to_request(&self.identity, now, &self.limits).map(Some)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapturePublicationStatus {
    Complete,
    Incomplete,
}
pub(crate) struct NativeCapturePublication {
    pub artifact_path: PathBuf,
    pub manifest_path: PathBuf,
    pub artifact_sha256: String,
    pub artifact_bytes: u64,
    pub collection_status: NativeCaptureStatus,
    pub publication_status: CapturePublicationStatus,
}
struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|end| end > self.limit)
        {
            return Err(std::io::Error::other("capture encoding capacity"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct PublicationTemporary {
    path: PathBuf,
    file: File,
    identity: (u64, u64),
    active: bool,
}
impl PublicationTemporary {
    fn create(path: &Path) -> Result<Self> {
        let path = path.with_extension("provisional");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&path)
            .map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        Ok(Self {
            path,
            file,
            identity: (metadata.dev(), metadata.ino()),
            active: true,
        })
    }
    fn validate(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
        ensure(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && (metadata.dev(), metadata.ino()) == self.identity,
            "capture temporary identity changed",
        )
    }
    fn cleanup(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        match std::fs::symlink_metadata(&self.path) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(io_error(cause)),
            Ok(_) => self.validate()?,
        }
        std::fs::remove_file(&self.path).map_err(io_error)
    }
}
impl Drop for PublicationTemporary {
    fn drop(&mut self) {
        if let Err(cause) = self.cleanup() {
            eprintln!("capture temporary cleanup: {cause}");
        }
    }
}
fn operation_permit(
    control: Option<&exporter::CaptureExportControl>,
    until: Instant,
) -> Result<Option<exporter::PublicationPermit<'_>>> {
    match control {
        Some(control) => control.acquire(until).map(Some),
        None => {
            check_deadline(Instant::now(), until)?;
            Ok(None)
        }
    }
}
fn publish_file(
    path: &Path,
    bytes: &[u8],
    until: Instant,
    control: Option<&exporter::CaptureExportControl>,
    hooks: &dyn NativeCaptureExportHooks,
    boundary: NativeCaptureExportBoundary,
) -> Result<()> {
    let directory = path.parent().expect("publication directory");
    let directory_identity = private_directory(directory)?;
    let mut temporary = {
        let _permit = operation_permit(control, until)?;
        PublicationTemporary::create(path)?
    };
    let result = (|| {
        for chunk in bytes.chunks(65536) {
            let _permit = operation_permit(control, until)?;
            temporary.file.write_all(chunk).map_err(io_error)?;
        }
        {
            let _permit = operation_permit(control, until)?;
            temporary.file.sync_all().map_err(io_error)?;
        }
        hooks.before(
            boundary,
            control.map_or(until, |control| control.deadline(until)),
        )?;
        {
            let _check = operation_permit(control, until)?;
        }
        ensure(
            private_directory(directory)? == directory_identity,
            "capture publication directory changed",
        )?;
        temporary.validate()?;
        {
            let _permit = operation_permit(control, until)?;
            std::fs::hard_link(&temporary.path, path).map_err(io_error)?;
        }
        temporary.cleanup()?;
        let directory = {
            let _permit = operation_permit(control, until)?;
            File::open(directory).map_err(io_error)?
        };
        let _permit = operation_permit(control, until)?;
        directory.sync_all().map_err(io_error)
    })();
    let cleanup = temporary.cleanup();
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Err(cleanup)) => Err(primary.with_cleanup(cleanup.message())),
        (Err(cause), _) | (_, Err(cause)) => Err(cause),
    }
}
pub(crate) fn publish_native_capture(
    directory: &Path,
    lease: &NativeCaptureLease,
    limits: &NativeCaptureLimits,
    until: Instant,
) -> Result<NativeCapturePublication> {
    publish_native_capture_with_control(
        directory,
        lease,
        limits,
        until,
        None,
        &InertCaptureExportHooks,
    )
}
pub(super) fn publish_native_capture_with_control(
    directory: &Path,
    lease: &NativeCaptureLease,
    limits: &NativeCaptureLimits,
    until: Instant,
    control: Option<&exporter::CaptureExportControl>,
    hooks: &dyn NativeCaptureExportHooks,
) -> Result<NativeCapturePublication> {
    check_deadline(Instant::now(), until)?;
    limits.validate()?;
    let directory_identity = private_directory(directory)?;
    let result = lease.result();
    validate_capture_id(&result.capture_id)?;
    let complete = result.status == NativeCaptureStatus::Complete;
    ensure(
        !complete || result.samples.len() == 2,
        "complete capture requires a pair",
    )?;
    let artifact_path = directory.join(format!(
        "{}.{}.json",
        result.capture_id,
        if complete { "complete" } else { "incomplete" }
    ));
    let manifest_path = directory.join(format!("{}.manifest.json", result.capture_id));
    ensure(
        !artifact_path.try_exists().map_err(io_error)?
            && !manifest_path.try_exists().map_err(io_error)?,
        "capture publication collision",
    )?;
    let encoded = {
        let _permit = exporter::publication_permit(
            control,
            hooks,
            NativeCaptureExportBoundary::Encode,
            until,
        )?;
        let mut encoded = BoundedBytes {
            bytes: Vec::new(),
            limit: limits.max_encoded_result_bytes,
        };
        serde_json::to_writer(&mut encoded, result).map_err(io_error)?;
        encoded.bytes
    };
    let artifact_sha256 = Sha256::digest(&encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let artifact_bytes = encoded.len() as u64;
    ensure(
        private_directory(directory)? == directory_identity,
        "capture publication directory changed",
    )?;
    publish_file(
        &artifact_path,
        &encoded,
        until,
        control,
        hooks,
        NativeCaptureExportBoundary::ArtifactPublish,
    )?;
    let publication_status = if complete {
        CapturePublicationStatus::Complete
    } else {
        CapturePublicationStatus::Incomplete
    };
    let manifest = json!({"schema_version":1,"capture_id":result.capture_id,"artifact_ref":artifact_path.file_name().expect("artifact name").to_string_lossy(),"artifact_sha256":artifact_sha256,"artifact_bytes":artifact_bytes,"collection_status":result.status,"publication_status":publication_status,"producer_ref":result.producer_ref,"diagnostic":result.diagnostic});
    let bytes = serde_json::to_vec(&manifest).map_err(io_error)?;
    ensure(
        private_directory(directory)? == directory_identity,
        "capture publication directory changed",
    )?;
    publish_file(
        &manifest_path,
        &bytes,
        until,
        control,
        hooks,
        NativeCaptureExportBoundary::ManifestPublish,
    )?;
    if let Some(control) = control {
        control.acknowledge()?;
    }
    Ok(NativeCapturePublication {
        artifact_path,
        manifest_path,
        artifact_sha256,
        artifact_bytes,
        collection_status: result.status,
        publication_status,
    })
}

mod tests {
    use serde_json::{Value, json};

    use super::super::test_support::*;
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::{Duration, Instant};
    #[test]
    fn runner_batch2_capture_arm_validates_schema_identity_duration_and_safe_capture_id() {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let identity = ControlIdentity::from_run(&fixture.fixture.run);
        let now = Instant::now();
        let limits = NativeCaptureLimits::default();
        let valid = arm(&identity);
        let request = valid.to_request(&identity, now, &limits).unwrap();
        assert_eq!(request.deadline, now + Duration::from_millis(1000));
        assert_eq!(request.minimum_interval, Duration::from_millis(100));
        for field in [
            "schema",
            "run",
            "epoch",
            "blank",
            "slash",
            "control",
            "length",
            "interval_low",
            "interval_high",
            "equal_timeout",
            "long_timeout",
            "overflow",
        ] {
            let mut invalid = arm(&identity);
            match field {
                "schema" => invalid.schema_version = 2,
                "run" => invalid.run_id += 1,
                "epoch" => invalid.owner_epoch += 1,
                "blank" => invalid.capture_id = "".into(),
                "slash" => invalid.capture_id = "a/b".into(),
                "control" => invalid.capture_id = "a\n".into(),
                "length" => invalid.capture_id = "a".repeat(129),
                "interval_low" => invalid.minimum_interval_ms = 99,
                "interval_high" => invalid.minimum_interval_ms = 5001,
                "equal_timeout" => invalid.timeout_ms = 100,
                "long_timeout" => invalid.timeout_ms = 30001,
                _ => invalid.timeout_ms = u64::MAX,
            }
            assert!(
                invalid.to_request(&identity, now, &limits).is_err(),
                "{field}"
            );
        }
        let mut value = serde_json::to_value(valid).unwrap();
        value["native_input"] = json!("forbidden private key");
        assert!(serde_json::from_value::<NativeCaptureArm>(value).is_err());
    }

    #[test]
    fn runner_batch2_capture_arm_reader_consumes_once_and_detects_replacement_after_claim() {
        let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
        let identity = ControlIdentity::from_run(&fixture.fixture.run);
        let path = fixture.fixture._dir.path().join("one-shot-arm.json");
        let mut reader =
            NativeCaptureArmReader::new(&path, identity.clone(), NativeCaptureLimits::default())
                .unwrap();
        assert!(reader.poll(Instant::now()).unwrap().is_none());
        private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
        let request = reader.poll(Instant::now()).unwrap().unwrap();
        assert_eq!(request.capture_id, "review-pair");
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
        assert!(reader.poll(Instant::now()).is_err());
    }

    #[test]
    fn runner_batch2_capture_arm_reader_claims_malformed_or_oversized_input_without_accepting_repair()
     {
        for bytes in [b"{\"schema_version\":".to_vec(), vec![b'x'; 16385]] {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let identity = ControlIdentity::from_run(&fixture.fixture.run);
            let path = fixture.fixture._dir.path().join("bad-arm.json");
            let mut reader = NativeCaptureArmReader::new(
                &path,
                identity.clone(),
                NativeCaptureLimits::default(),
            )
            .unwrap();
            private_write(&path, &bytes);
            assert!(reader.poll(Instant::now()).is_err());
            if path.exists() {
                std::fs::remove_file(&path).unwrap();
            }
            private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
            assert!(reader.poll(Instant::now()).is_err());
        }
    }

    #[test]
    fn runner_batch2_capture_arm_reader_rejects_symlinks_public_modes_and_replaced_parent() {
        for field in ["symlink", "mode", "parent"] {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let parent = fixture.fixture._dir.path().join("arm-parent");
            std::fs::create_dir(&parent).unwrap();
            std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
            let identity = ControlIdentity::from_run(&fixture.fixture.run);
            let path = parent.join("arm.json");
            let mut reader = NativeCaptureArmReader::new(
                &path,
                identity.clone(),
                NativeCaptureLimits::default(),
            )
            .unwrap();
            match field {
                "symlink" => {
                    let source = fixture.fixture._dir.path().join("source-arm.json");
                    private_write(&source, &serde_json::to_vec(&arm(&identity)).unwrap());
                    symlink(source, &path).unwrap();
                }
                "mode" => {
                    private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
                _ => {
                    std::fs::rename(&parent, fixture.fixture._dir.path().join("old-parent"))
                        .unwrap();
                    std::fs::create_dir(&parent).unwrap();
                    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    private_write(&path, &serde_json::to_vec(&arm(&identity)).unwrap());
                }
            }
            assert!(reader.poll(Instant::now()).is_err(), "{field}");
        }
    }

    #[test]
    fn runner_batch2_capture_config_requires_private_bounded_exact_files_and_existing_evidence() {
        for field in [
            "valid",
            "unknown",
            "run",
            "nonce",
            "relative",
            "missing_evidence",
            "public",
            "symlink",
            "oversized",
        ] {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let config = config(&fixture);
            let path = fixture.fixture._dir.path().join("config.json");
            let mut value = serde_json::to_value(config).unwrap();
            match field {
                "unknown" => value["raw_screen"] = json!("private"),
                "run" => value["expected_run_id"] = json!(0),
                "nonce" => value["expected_launch_nonce"] = json!("A".repeat(64)),
                "relative" => value["descriptor_path"] = json!("relative.json"),
                "missing_evidence" => {
                    value["producer_manifest_path"] =
                        json!(fixture.fixture._dir.path().join("absent.json"))
                }
                _ => {}
            }
            let bytes = if field == "oversized" {
                vec![b' '; 65537]
            } else {
                serde_json::to_vec(&value).unwrap()
            };
            private_write(&path, &bytes);
            if field == "public" {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            let selected = if field == "symlink" {
                let link = fixture.fixture._dir.path().join("config-link.json");
                symlink(&path, &link).unwrap();
                link
            } else {
                path
            };
            let result = read_native_capture_config(&selected, &NativeCaptureLimits::default());
            assert_eq!(result.is_ok(), field == "valid", "{field}");
        }
    }

    #[test]
    fn runner_batch2_capture_producer_binds_actual_libtest_and_rejects_fabricated_build_evidence() {
        for field in [
            "valid",
            "unknown",
            "libtest_hash",
            "libtest_path",
            "native_hash",
            "parser",
            "collector",
            "reference",
            "oversized",
        ] {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let mut value = fixture.producer_json.clone();
            match field {
                "unknown" => value["private_argv"] = json!([]),
                "libtest_hash" => value["libtest_executable_sha256"] = json!("0".repeat(64)),
                "libtest_path" => value["libtest_executable_path"] = json!("relative"),
                "native_hash" => value["native_executable"]["sha256"] = json!("INVALID"),
                "parser" => value["parser_version"] = json!("0.0.0"),
                "collector" => value["collector_version"] = json!("unknown"),
                "reference" => value["environment_evidence_ref"] = json!("\n"),
                _ => {}
            }
            let path = fixture.fixture._dir.path().join("producer-case.json");
            private_write(
                &path,
                &if field == "oversized" {
                    vec![b' '; 65537]
                } else {
                    serde_json::to_vec(&value).unwrap()
                },
            );
            assert_eq!(
                read_native_capture_producer(&path, &NativeCaptureLimits::default()).is_ok(),
                field == "valid",
                "{field}"
            );
        }
    }
    #[test]
    fn runner_batch2_capture_publication_accepts_only_the_final_manifest_and_matching_complete_artifact()
     {
        let mut fixture = CaptureFixture::new(redacted());
        let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
        let lease = fixture.pair(&mut port, &receiver, b"safe fixture");
        let output = fixture.fixture._dir.path().join("publication");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
        let publication = publish_native_capture(
            &output,
            &lease,
            &NativeCaptureLimits::default(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            publication.publication_status,
            CapturePublicationStatus::Complete
        );
        assert_eq!(publication.collection_status, NativeCaptureStatus::Complete);
        assert!(publication.artifact_path.ends_with("pair.complete.json"));
        assert!(publication.manifest_path.ends_with("pair.manifest.json"));
        let artifact = std::fs::read(&publication.artifact_path).unwrap();
        assert_eq!(
            publication.artifact_sha256,
            digest(&publication.artifact_path)
        );
        assert_eq!(publication.artifact_bytes, artifact.len() as u64);
        assert_eq!(
            serde_json::from_slice::<Value>(&artifact).unwrap(),
            value(&lease)
        );
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(&publication.manifest_path).unwrap()).unwrap();
        assert_keys(
            &manifest,
            &[
                "schema_version",
                "capture_id",
                "artifact_ref",
                "artifact_sha256",
                "artifact_bytes",
                "collection_status",
                "publication_status",
                "producer_ref",
                "diagnostic",
            ],
        );
        assert_eq!(manifest["artifact_sha256"], publication.artifact_sha256);
        assert_eq!(manifest["artifact_bytes"], publication.artifact_bytes);
        assert_eq!(manifest["collection_status"], "complete");
        assert_eq!(manifest["publication_status"], "complete");
        for path in [&publication.artifact_path, &publication.manifest_path] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(receiver.request(fixture.request("while-exported")).is_err());
    }

    #[test]
    fn runner_batch2_capture_publication_collision_and_expiry_preserve_existing_output_without_success()
     {
        for collision in ["artifact", "manifest", "expired"] {
            let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
            let lease = fixture.pair(&mut port, &receiver, b"private");
            let directory = fixture.fixture._dir.path();
            let existing = directory.join(if collision == "artifact" {
                "pair.complete.json"
            } else {
                "pair.manifest.json"
            });
            if collision != "expired" {
                private_write(&existing, b"owned prior output");
            }
            let until = if collision == "expired" {
                Instant::now() - Duration::from_millis(1)
            } else {
                Instant::now() + Duration::from_secs(2)
            };
            assert!(
                publish_native_capture(directory, &lease, &NativeCaptureLimits::default(), until)
                    .is_err()
            );
            if collision != "expired" {
                assert_eq!(std::fs::read(&existing).unwrap(), b"owned prior output");
            }
            if collision != "manifest" {
                assert!(!directory.join("pair.manifest.json").exists());
            }
        }
    }

    #[test]
    fn runner_batch2_capture_incomplete_publication_keeps_terminal_reason_and_incomplete_filename()
    {
        let mut fixture = CaptureFixture::new(redacted());
        let (mut port, receiver) = fixture.ready(NativeCaptureLimits::default(), &[]);
        receiver.request(fixture.request("cancelled-pair")).unwrap();
        port.cancel(NativeCaptureStatus::OwnerExit).unwrap();
        let lease = receiver.try_take().unwrap().unwrap();
        let publication = publish_native_capture(
            fixture.fixture._dir.path(),
            &lease,
            &NativeCaptureLimits::default(),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            publication.collection_status,
            NativeCaptureStatus::OwnerExit
        );
        assert_eq!(
            publication.publication_status,
            CapturePublicationStatus::Incomplete
        );
        assert!(
            publication
                .artifact_path
                .ends_with("cancelled-pair.incomplete.json")
        );
        assert!(
            !fixture
                .fixture
                ._dir
                .path()
                .join("cancelled-pair.complete.json")
                .exists()
        );
    }

    mod duplicate_fields {
        use super::*;
        fn duplicate_object(
            object: &serde_json::Map<String, Value>,
            key: &str,
            first: &Value,
            last: &Value,
        ) -> String {
            assert!(object.contains_key(key));
            let mut fields: Vec<String> = object
                .iter()
                .filter(|(name, _)| name.as_str() != key)
                .map(|(name, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(name).unwrap(),
                        serde_json::to_string(value).unwrap()
                    )
                })
                .collect();
            let name = serde_json::to_string(key).unwrap();
            fields.push(format!("{name}:{}", serde_json::to_string(first).unwrap()));
            fields.push(format!("{name}:{}", serde_json::to_string(last).unwrap()));
            format!("{{{}}}", fields.join(","))
        }
        fn with_native_document(producer: &Value, native: &str) -> String {
            let fields: Vec<String> = producer
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| {
                    let value = if name == "native_executable" {
                        native.to_owned()
                    } else {
                        serde_json::to_string(value).unwrap()
                    };
                    format!("{}:{value}", serde_json::to_string(name).unwrap())
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        fn assert_duplicate_fields_rejected(nested: bool, conflicting: bool) {
            let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            assert!(
                read_native_capture_producer(
                    &fixture.producer_path,
                    &NativeCaptureLimits::default()
                )
                .is_ok()
            );
            let object = if nested {
                &fixture.producer_json["native_executable"]
            } else {
                &fixture.producer_json
            };
            for (key, valid) in object.as_object().unwrap() {
                for invalid_last in [true, false] {
                    let invalid = if conflicting {
                        json!([])
                    } else {
                        valid.clone()
                    };
                    let (first, last) = if invalid_last {
                        (valid, &invalid)
                    } else {
                        (&invalid, valid)
                    };
                    let duplicate = duplicate_object(object.as_object().unwrap(), key, first, last);
                    let bytes = if nested {
                        with_native_document(&fixture.producer_json, &duplicate)
                    } else {
                        duplicate
                    };
                    let path = fixture
                        .fixture
                        ._dir
                        .path()
                        .join(format!("duplicate-{key}-{invalid_last}.json"));
                    private_write(&path, bytes.as_bytes());
                    assert!(
                        read_native_capture_producer(&path, &NativeCaptureLimits::default())
                            .is_err(),
                        "nested={nested}, conflicting={conflicting}, field={key}, invalid_last={invalid_last}"
                    );
                }
            }
        }

        #[test]
        fn runner_collector_f4_top_level_identical_duplicate_fields_fail_before_normalization() {
            assert_duplicate_fields_rejected(false, false);
        }
        #[test]
        fn runner_collector_f4_nested_identical_duplicate_fields_fail_before_normalization() {
            assert_duplicate_fields_rejected(true, false);
        }
        #[test]
        fn runner_collector_f4_top_level_conflicting_duplicates_reject_both_field_orders() {
            assert_duplicate_fields_rejected(false, true);
        }
        #[test]
        fn runner_collector_f4_nested_conflicting_duplicates_reject_both_field_orders() {
            assert_duplicate_fields_rejected(true, true);
        }
        #[test]
        fn runner_collector_f4_nullable_native_version_requires_presence_and_accepts_explicit_null()
        {
            let mut fixture = CaptureFixture::new(json!({"mode":"metadata"}));
            fixture.producer_json["native_executable"]["version"] = Value::Null;
            let null = fixture.fixture._dir.path().join("null-version.json");
            private_write(&null, &serde_json::to_vec(&fixture.producer_json).unwrap());
            assert!(read_native_capture_producer(&null, &NativeCaptureLimits::default()).is_ok());
            fixture.producer_json["native_executable"]
                .as_object_mut()
                .unwrap()
                .remove("version")
                .unwrap();
            let missing = fixture.fixture._dir.path().join("missing-version.json");
            private_write(
                &missing,
                &serde_json::to_vec(&fixture.producer_json).unwrap(),
            );
            assert!(
                read_native_capture_producer(&missing, &NativeCaptureLimits::default()).is_err()
            );
        }
    }
}
