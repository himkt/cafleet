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
