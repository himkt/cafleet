use crate::broker::runtime as storage;
use crate::broker::runtime::OwnerKind;
use crate::broker::runtime::RunOwner;
use crate::broker::runtime::RunRecord;
use crate::digest::canonical_json;
use crate::runtime::*;
use rusqlite::Connection;
use rusqlite::params;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use std::cell::RefCell;
use std::fs;
use std::fs::DirBuilder;
use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

#[cfg(test)]
use std::time::Duration;

fn namespace(identity: &Value) -> String {
    let digest = Sha256::digest(canonical_json(identity).as_bytes());
    let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut bits = 0u32;
    let mut count = 0;
    for byte in digest {
        bits = (bits << 8) | u32::from(byte);
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(alphabet[((bits >> count) & 31) as usize] as char);
            if out.len() == 26 {
                return out;
            }
        }
    }
    unreachable!("SHA-256 contains enough namespace bits")
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    pub(crate) namespace_dir: PathBuf,
    pub(crate) namespace_manifest: PathBuf,
    pub(crate) run_dir: PathBuf,
    pub(crate) descriptor: PathBuf,
    pub(crate) control_socket: PathBuf,
    pub(crate) owner_lock: PathBuf,
    pub(crate) outbox: PathBuf,
}
impl RuntimePaths {
    pub(crate) fn for_run(identity: &Value, run_id: i64) -> Result<Self> {
        ensure(run_id > 0, "runtime run ID must be positive")?;
        let path = identity
            .get("canonical_path")
            .and_then(Value::as_str)
            .ok_or_else(|| error("canonical database path required"))?;
        let path = Path::new(path);
        ensure(
            path.is_absolute(),
            "absolute canonical database path required",
        )?;
        let parent = path
            .parent()
            .ok_or_else(|| error("database parent required"))?;
        let namespace_dir = parent.join(".cafleet-runs").join(namespace(identity));
        let run_dir = namespace_dir.join(run_id.to_string());
        let control_socket = run_dir.join("control.sock");
        let maximum = if cfg!(target_os = "macos") { 103 } else { 107 };
        ensure(
            control_socket.as_os_str().as_encoded_bytes().len() <= maximum,
            "complete runtime socket path exceeds platform limit",
        )?;
        Ok(Self {
            namespace_manifest: namespace_dir.join("identity.json"),
            descriptor: run_dir.join("launch.json"),
            owner_lock: run_dir.join("owner.lock"),
            outbox: run_dir.join("events.outbox"),
            control_socket,
            run_dir,
            namespace_dir,
        })
    }
}
fn uid() -> u32 {
    nix::unistd::geteuid().as_raw()
}
fn identity(metadata: &fs::Metadata) -> Value {
    json!({"device":metadata.dev(),"inode":metadata.ino()})
}
pub(super) fn private_metadata(
    path: &Path,
    expected_uid: u32,
    mode: u32,
    directory: bool,
) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path).map_err(io_error)?;
    ensure(
        !meta.file_type().is_symlink()
            && if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            },
        "runtime artifact type mismatch",
    )?;
    ensure(
        meta.uid() == expected_uid && meta.mode() & 0o777 == mode,
        "runtime artifact ownership or permissions mismatch",
    )?;
    Ok(meta)
}
pub(crate) fn validate_private_file(path: &Path, expected_uid: u32, mode: u32) -> Result<()> {
    private_metadata(path, expected_uid, mode, false).map(|_| ())
}
fn present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(cause) => Err(io_error(cause)),
    }
}
fn read_manifest(path: &Path) -> Result<Value> {
    let mut text = String::new();
    open_private(path, false)?
        .take(65537)
        .read_to_string(&mut text)
        .map_err(io_error)?;
    serde_json::from_str(&text).map_err(io_error)
}
fn directory(path: &Path) -> Result<()> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(cause) => return Err(io_error(cause)),
    }
    private_metadata(path, uid(), 0o700, true)?;
    Ok(())
}
pub(super) fn open_private(path: &Path, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC);
    if create {
        options.create_new(true).mode(0o600);
    }
    let file = options.open(path).map_err(io_error)?;
    let named = private_metadata(path, uid(), 0o600, false)?;
    ensure(
        identity(&file.metadata().map_err(io_error)?) == identity(&named),
        "runtime artifact changed while opening",
    )?;
    Ok(file)
}
pub(crate) fn ensure_same_database(conn: &Connection, expected: &Value) -> Result<()> {
    ensure(
        storage::database_identity(conn)? == *expected,
        "runtime database identity changed",
    )
}

pub(super) fn placement_of(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<crate::broker::Placement>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        &format!(
            "SELECT {} FROM member_placements p WHERE member_id=?1",
            crate::broker::PLACEMENT_COLUMNS
        ),
        [member_id],
        |row| crate::broker::Placement::from_row(row, 0),
    )
    .optional()
    .map_err(CafleetError::from)
}
pub(crate) fn validate_run_paths(conn: &Connection, run: &RunRecord) -> Result<RuntimePaths> {
    ensure_same_database(conn, &run.database_identity_json)?;
    let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id)?;
    ensure(
        paths.run_dir == Path::new(&run.runtime_path),
        "runtime registry path mismatch",
    )?;
    private_metadata(
        paths.namespace_dir.parent().expect("namespace parent"),
        uid(),
        0o700,
        true,
    )?;
    private_metadata(&paths.namespace_dir, uid(), 0o700, true)?;
    let dir = private_metadata(&paths.run_dir, uid(), 0o700, true)?;
    let lock = private_metadata(&paths.owner_lock, uid(), 0o600, false)?;
    ensure(
        run.runtime_dir_identity_json.as_ref() == Some(&identity(&dir))
            && run.run_lock_identity_json.as_ref() == Some(&identity(&lock)),
        "runtime directory or owner lock identity changed; explicit recovery required",
    )?;
    ensure(
        read_manifest(&paths.namespace_manifest)? == run.database_identity_json,
        "runtime namespace identity collision",
    )?;
    Ok(paths)
}

pub(crate) struct RunArtifacts {
    pub(crate) paths: RuntimePaths,
    descriptor_identity: RefCell<Option<Value>>,
}
impl RunArtifacts {
    #[cfg(test)]
    pub(crate) fn create(conn: &mut Connection, run_id: i64) -> Result<Self> {
        Self::create_with_deadline(conn, run_id, Instant::now() + Duration::from_secs(5))
    }
    pub(crate) fn create_with_deadline(
        conn: &mut Connection,
        run_id: i64,
        deadline: Instant,
    ) -> Result<Self> {
        let run = storage::get_run(conn, run_id)?;
        ensure_same_database(conn, &run.database_identity_json)?;
        ensure(
            run.owner_kind == OwnerKind::Creator && !run.status.terminal(),
            "artifact allocation requires the pending creator",
        )?;
        let paths = RuntimePaths::for_run(&run.database_identity_json, run_id)?;
        directory(paths.namespace_dir.parent().expect("namespace parent"))?;
        directory(&paths.namespace_dir)?;
        match open_private(&paths.namespace_manifest, true) {
            Ok(mut file) => {
                file.write_all(canonical_json(&run.database_identity_json).as_bytes())
                    .map_err(io_error)?;
                file.sync_all().map_err(io_error)?;
            }
            Err(cause) => {
                if !paths.namespace_manifest.exists() {
                    return Err(cause);
                }
                ensure(
                    read_manifest(&paths.namespace_manifest)? == run.database_identity_json,
                    "runtime namespace identity collision",
                )?;
            }
        }
        DirBuilder::new()
            .mode(0o700)
            .create(&paths.run_dir)
            .map_err(|cause| {
                error(format!(
                    "runtime directory requires exclusive allocation or explicit recovery: {cause}"
                ))
            })?;
        let lock = open_private(&paths.owner_lock, true)?;
        open_private(&paths.outbox, true)?;
        let dir_id = identity(&private_metadata(&paths.run_dir, uid(), 0o700, true)?);
        let lock_id = identity(&lock.metadata().map_err(io_error)?);
        with_database_deadline(conn, deadline, |conn| {
            ensure_same_database(conn, &run.database_identity_json)?;
            let changed=conn.execute("UPDATE member_runs SET runtime_path=?1,runtime_dir_identity_json=?2,run_lock_identity_json=?3 WHERE run_id=?4 AND owner_epoch=?5 AND owner_kind='creator' AND runtime_dir_identity_json IS NULL",params![paths.run_dir.to_string_lossy(),dir_id.to_string(),lock_id.to_string(),run_id,run.owner_epoch])?;
            ensure(changed == 1, "artifact ownership changed during allocation")
        })?;
        Ok(Self {
            paths,
            descriptor_identity: RefCell::new(None),
        })
    }
    pub(crate) fn from_run(conn: &Connection, run: &RunRecord) -> Result<Self> {
        Ok(Self {
            paths: validate_run_paths(conn, run)?,
            descriptor_identity: RefCell::new(None),
        })
    }
    pub(crate) fn write_descriptor(&self, descriptor: &LaunchDescriptor) -> Result<()> {
        ensure(
            self.paths.run_dir
                == RuntimePaths::for_run(
                    &descriptor.identity.database_identity_json,
                    descriptor.identity.run_id,
                )?
                .run_dir,
            "descriptor path mismatch",
        )?;
        descriptor.spawn.validate()?;
        let mut file = open_private(&self.paths.descriptor, true)?;
        *self.descriptor_identity.borrow_mut() =
            Some(identity(&file.metadata().map_err(io_error)?));
        let data = serde_json::to_vec(descriptor).map_err(io_error)?;
        ensure(
            data.len() as u64 <= DESCRIPTOR_LIMIT,
            "launch descriptor exceeds private size limit",
        )?;
        file.write_all(&data).map_err(io_error)?;
        file.sync_all().map_err(io_error)
    }
    pub(crate) fn remove_descriptor(&self) -> Result<()> {
        let meta = match fs::symlink_metadata(&self.paths.descriptor) {
            Ok(meta) => meta,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(io_error(cause)),
        };
        validate_private_file(&self.paths.descriptor, uid(), 0o600)?;
        if let Some(expected) = self.descriptor_identity.borrow().as_ref() {
            ensure(
                identity(&meta) == *expected,
                "descriptor identity changed before unlink",
            )?;
        }
        fs::remove_file(&self.paths.descriptor)
            .map_err(|cause| error(format!("descriptor unlink failed: {cause}")))
    }
}
impl Drop for RunArtifacts {
    fn drop(&mut self) {
        if self.descriptor_identity.borrow().is_some()
            && let Err(cause) = self.remove_descriptor()
        {
            eprintln!("{cause}");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchDescriptor {
    pub(crate) schema_version: u32,
    pub(crate) identity: ControlIdentity,
    pub(crate) spawn: SpawnSpec,
}
impl LaunchDescriptor {
    pub(crate) fn new(run: &RunRecord, spawn: SpawnSpec) -> Result<Self> {
        spawn.validate()?;
        Ok(Self {
            schema_version: 1,
            identity: ControlIdentity::from_run(run),
            spawn,
        })
    }
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let descriptor: Self =
            serde_json::from_slice(bytes).map_err(|_| error("malformed launch descriptor"))?;
        ensure(
            descriptor.schema_version == 1,
            "unsupported launch descriptor schema",
        )?;
        Ok(descriptor)
    }
}
pub(crate) const DESCRIPTOR_LIMIT: u64 = 1024 * 1024;
pub(crate) fn load_descriptor(conn: &Connection, path: &Path) -> Result<LaunchDescriptor> {
    ensure(
        path.is_absolute(),
        "absolute runner descriptor path required",
    )?;
    let mut file = open_private(path, false)?;
    ensure(
        file.metadata().map_err(io_error)?.len() <= DESCRIPTOR_LIMIT,
        "launch descriptor exceeds private size limit",
    )?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let descriptor = LaunchDescriptor::parse(&bytes)?;
    descriptor.spawn.validate()?;
    let run = storage::get_run(conn, descriptor.identity.run_id)?;
    let paths = validate_run_paths(conn, &run)?;
    ensure(
        path == paths.descriptor,
        "runner descriptor was not allocated for this run",
    )?;
    ensure(
        run.owner_kind == OwnerKind::Creator
            && descriptor.identity == ControlIdentity::from_run(&run),
        "launch descriptor identity mismatch",
    )?;
    Ok(descriptor)
}

pub(crate) struct RunLock {
    file: File,
}
impl RunLock {
    #[cfg(test)]
    pub(crate) fn transfer_to_reconciler_with_deadline(
        &self,
        conn: &mut Connection,
        expected: &RunRecord,
        process: ProcessIdentity,
        deadline: Instant,
    ) -> Result<RunOwner> {
        self.transfer_to_reconciler_checked(conn, expected, process, deadline, |_| Ok(()))
            .map(|run| run.owner())
    }
    pub(super) fn transfer_to_reconciler_checked(
        &self,
        conn: &mut Connection,
        expected: &RunRecord,
        process: ProcessIdentity,
        deadline: Instant,
        check: impl FnOnce(&Connection) -> Result<()>,
    ) -> Result<RunRecord> {
        with_database_deadline(conn, deadline, |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check(&tx)?;
            let current = storage::get_run(&tx, expected.run_id)?;
            ensure_same_database(&tx, &expected.database_identity_json)?;
            ensure(
                crate::runtime::recovery::stable_run_identity(&current)
                    == crate::runtime::recovery::stable_run_identity(expected),
                "recovery ownership changed before CAS",
            )?;
            let next = if current.is_settled() {
                current
            } else {
                self.validate(&tx, &expected.owner())?;
                crate::runtime::recovery::advance_recovery_owner(&tx, expected, &process)?
            };
            check_deadline(Instant::now(), deadline)?;
            tx.commit()?;
            Ok(next)
        })
    }
    pub(crate) fn acquire(conn: &Connection, owner: &RunOwner) -> Result<Option<Self>> {
        let run = storage::get_run(conn, owner.run_id)?;
        ensure(run.owner() == *owner, "stale runtime owner")?;
        let paths = validate_run_paths(conn, &run)?;
        let file = open_private(&paths.owner_lock, false)?;
        match file.try_lock() {
            Ok(()) => {
                let lock = Self { file };
                lock.validate(conn, owner)?;
                Ok(Some(lock))
            }
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(cause)) => Err(io_error(cause)),
        }
    }
    /// The run-lock counterpart of [`crate::pane_lock::PaneLock::acquire_released`].
    #[cfg(test)]
    pub(crate) fn acquire_released(conn: &Connection, owner: &RunOwner) -> Result<Option<Self>> {
        let until = Instant::now() + crate::pane_lock::PaneLock::RELEASED_LOCK_GRACE;
        loop {
            if let Some(lock) = Self::acquire(conn, owner)? {
                return Ok(Some(lock));
            }
            if Instant::now() >= until {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    pub(crate) fn validate(&self, conn: &Connection, owner: &RunOwner) -> Result<()> {
        let run = storage::get_run(conn, owner.run_id)?;
        ensure(run.owner() == *owner, "stale runtime owner")?;
        validate_run_paths(conn, &run)?;
        ensure(
            run.run_lock_identity_json.as_ref()
                == Some(&identity(&self.file.metadata().map_err(io_error)?)),
            "open owner lock identity mismatch",
        )
    }
    #[cfg(test)]
    pub(crate) fn transfer_to_runner(
        &self,
        conn: &mut Connection,
        owner: &RunOwner,
        process: ProcessIdentity,
    ) -> Result<RunOwner> {
        self.transfer_to_runner_with_deadline(
            conn,
            owner,
            process,
            Instant::now() + Duration::from_secs(5),
        )
    }
    pub(crate) fn transfer_to_runner_with_deadline(
        &self,
        conn: &mut Connection,
        owner: &RunOwner,
        process: ProcessIdentity,
        deadline: Instant,
    ) -> Result<RunOwner> {
        ensure(
            owner.owner_kind == OwnerKind::Creator,
            "runner transfer requires creator ownership",
        )?;
        with_database_deadline(conn, deadline, |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            self.validate(&tx, owner)?;
            let epoch = owner
                .owner_epoch
                .checked_add(1)
                .ok_or_else(|| error("owner epoch exhausted"))?;
            let changed=tx.execute("UPDATE member_runs SET owner_epoch=?1,owner_kind='runner',owner_pid=?2,owner_start_identity=?3,runner_pid=?2,runner_start_identity=?3,last_health_at=?4 WHERE run_id=?5 AND owner_epoch=?6 AND owner_kind='creator'",params![epoch,process.pid,process.start_identity,crate::time::now_stamp(),owner.run_id,owner.owner_epoch])?;
            ensure(changed == 1, "creator ownership transfer lost CAS")?;
            let next = storage::get_run(&tx, owner.run_id)?.owner();
            tx.commit()?;
            Ok(next)
        })
    }
}

struct PinnedFile {
    file: File,
    identity: Value,
    digest: [u8; 32],
}

pub(crate) struct RecoveryDescriptor {
    path: PathBuf,
    pinned: Option<PinnedFile>,
}

fn validated_paths(
    conn: &mut Connection,
    run: &RunRecord,
    lock: &RunLock,
    until: Instant,
) -> Result<RuntimePaths> {
    let paths = with_database_deadline(conn, until, |conn| {
        let current = storage::get_run(conn, run.run_id)?;
        ensure(
            crate::runtime::recovery::stable_run_identity(&current)
                == crate::runtime::recovery::stable_run_identity(run),
            "descriptor runtime ownership changed",
        )?;
        lock.validate(conn, &run.owner())?;
        validate_run_paths(conn, run)
    })?;
    let process = ProcessIdentity::current()?;
    ensure(
        run.owner_pid == process.pid && run.owner_start_identity == process.start_identity,
        "descriptor access requires the current runtime owner",
    )?;
    check_deadline(Instant::now(), until)?;
    Ok(paths)
}

fn read_bounded(file: &mut File, until: Instant) -> Result<Vec<u8>> {
    check_deadline(Instant::now(), until)?;
    ensure(
        file.metadata().map_err(io_error)?.len() <= DESCRIPTOR_LIMIT,
        "launch descriptor exceeds private size limit",
    )?;
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut bytes = Vec::new();
    Read::by_ref(file)
        .take(DESCRIPTOR_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    ensure(
        bytes.len() as u64 <= DESCRIPTOR_LIMIT,
        "launch descriptor exceeds private size limit",
    )?;
    check_deadline(Instant::now(), until)?;
    Ok(bytes)
}

fn validate_named(path: &Path, pinned: &PinnedFile) -> Result<()> {
    let named = private_metadata(path, uid(), 0o600, false)?;
    ensure(
        identity(&named) == pinned.identity
            && identity(&pinned.file.metadata().map_err(io_error)?) == pinned.identity,
        "descriptor identity changed before unlink",
    )
}

impl RecoveryDescriptor {
    pub(crate) fn pin(
        conn: &mut Connection,
        run: &RunRecord,
        lock: &RunLock,
        until: Instant,
    ) -> Result<Self> {
        let path = validated_paths(conn, run, lock, until)?.descriptor;
        if !present(&path)? {
            return Ok(Self { path, pinned: None });
        }
        let mut file = open_private(&path, false)?;
        let file_identity = identity(&file.metadata().map_err(io_error)?);
        let bytes = read_bounded(&mut file, until)?;
        let descriptor = LaunchDescriptor::parse(&bytes)?;
        let recorded = &descriptor.identity;
        ensure(
            recorded.owner_epoch > 0
                && recorded.owner_epoch <= run.owner_epoch
                && ControlIdentity {
                    owner_epoch: run.owner_epoch,
                    ..recorded.clone()
                } == ControlIdentity::from_run(run),
            "launch descriptor identity mismatch",
        )?;
        let pinned = PinnedFile {
            file,
            identity: file_identity,
            digest: Sha256::digest(&bytes).into(),
        };
        validate_named(&path, &pinned)?;
        check_deadline(Instant::now(), until)?;
        Ok(Self {
            path,
            pinned: Some(pinned),
        })
    }

    pub(crate) fn remove(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        lock: &RunLock,
        until: Instant,
        unlink: impl FnOnce(&Path, Instant) -> Result<()>,
    ) -> Result<()> {
        let paths = validated_paths(conn, run, lock, until)?;
        ensure(paths.descriptor == self.path, "descriptor pathname changed")?;
        if !present(&self.path)? {
            self.pinned = None;
            return Ok(());
        }
        let pinned = self
            .pinned
            .as_mut()
            .ok_or_else(|| error("descriptor appeared after absence was pinned"))?;
        validate_named(&self.path, pinned)?;
        let bytes = read_bounded(&mut pinned.file, until)?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        ensure(
            digest == pinned.digest,
            "descriptor content changed before unlink",
        )?;
        validate_named(&self.path, pinned)?;
        check_deadline(Instant::now(), until)?;
        unlink(&self.path, until)?;
        ensure(!present(&self.path)?, "descriptor remains after unlink")?;
        check_deadline(Instant::now(), until)?;
        self.pinned = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime as storage;
    use crate::error::CafleetError;
    use crate::runtime::recovery::test_support::descriptor::described_creator;
    use crate::runtime::recovery::test_support::descriptor::identity as descriptor_identity;
    use crate::runtime::recovery::test_support::descriptor::owned_bytes;
    use crate::runtime::recovery::test_support::descriptor::secret_free;
    use crate::runtime::recovery::test_support::*;
    use std::time::Duration;
    use std::time::Instant;
    #[test]
    fn runner_recovery_historical_descriptor_pin_remove_uses_the_supplied_connection_budget() {
        for takeovers in [1, 2] {
            let mut case = described_creator();
            let lock = case.held.as_ref().unwrap().clone();
            for _ in 0..takeovers {
                lock.transfer_to_reconciler_with_deadline(
                    &mut case.fixture.conn,
                    &case.fixture.run,
                    ProcessIdentity::current().unwrap(),
                    Instant::now() + Duration::from_secs(1),
                )
                .unwrap();
                case.refresh();
            }
            assert!(case.fixture.run.owner_epoch > 1);
            let before = case.fixture.run.clone();
            let path = case.artifacts.paths.descriptor.clone();
            let until = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            with_database_deadline(&mut case.fixture.conn, until, |conn| {
                let mut pinned = RecoveryDescriptor::pin(conn, &before, &lock, until)?;
                assert!(
                    load_descriptor(conn, &path).is_err(),
                    "launch-time reading still requires exact creator ownership"
                );
                pinned.remove(conn, &before, &lock, until, |selected, deadline| {
                    attempts += 1;
                    assert_eq!(selected, path.as_path());
                    assert_eq!(deadline, until);
                    assert!(Instant::now() < deadline);
                    std::fs::remove_file(selected)
                        .map_err(|_| CafleetError::App("owned descriptor removal failed".into()))
                })
            })
            .unwrap();
            assert_eq!(attempts, 1);
            assert!(!path.exists());
            assert_eq!(
                storage::get_run(&case.fixture.conn, before.run_id).unwrap(),
                before
            );
        }
    }

    #[test]
    fn runner_recovery_descriptor_pin_drop_preserves_path_for_explicit_completion() {
        let mut case = described_creator();
        let bytes = owned_bytes(&case);
        let lock = case.held.as_ref().unwrap().clone();
        let pinned = RecoveryDescriptor::pin(
            &mut case.fixture.conn,
            &case.fixture.run,
            &lock,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        drop(pinned);
        assert!(
            owned_bytes(&case) == bytes,
            "pin ownership alone schedules no pathname removal"
        );
    }

    #[test]
    fn runner_recovery_descriptor_remove_revalidates_owner_and_expired_deadline_before_dispatch() {
        for changed in ["epoch", "deadline"] {
            let mut case = described_creator();
            let before = descriptor_identity(&case.artifacts.paths.descriptor);
            let lock = case.held.as_ref().unwrap().clone();
            let until = Instant::now() + Duration::from_secs(1);
            let mut pinned =
                RecoveryDescriptor::pin(&mut case.fixture.conn, &case.fixture.run, &lock, until)
                    .unwrap();
            if changed == "epoch" {
                bump_epoch(&case.fixture.conn, case.fixture.run.run_id);
            }
            let deadline = if changed == "deadline" {
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .unwrap()
            } else {
                until
            };
            let mut dispatched = false;
            let result: TestResult<()> = pinned.remove(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                deadline,
                |_, _| {
                    dispatched = true;
                    Ok(())
                },
            );
            let error: CafleetError = match result {
                Err(error) => error,
                Ok(()) => panic!("changed descriptor removal precondition fails before dispatch"),
            };
            secret_free(&error.message());
            assert!(!dispatched);
            assert_eq!(
                descriptor_identity(&case.artifacts.paths.descriptor),
                before
            );
        }
    }

    #[test]
    fn runner_recovery_actual_descriptor_unlink_closure_failure_preserves_file_and_retries() {
        let mut case = described_creator();
        let bytes = owned_bytes(&case);
        let lock = case.held.as_ref().unwrap().clone();
        let until = Instant::now() + Duration::from_secs(1);
        let mut pinned =
            RecoveryDescriptor::pin(&mut case.fixture.conn, &case.fixture.run, &lock, until)
                .unwrap();
        let mut attempts = 0;
        let result: TestResult<()> = pinned.remove(
            &mut case.fixture.conn,
            &case.fixture.run,
            &lock,
            until,
            |_, deadline| {
                attempts += 1;
                assert_eq!(deadline, until);
                Err(CafleetError::App(
                    "owned descriptor unlink-dispatch failure".into(),
                ))
            },
        );
        let error: CafleetError = match result {
            Err(error) => error,
            Ok(()) => panic!("unlink failure remains explicit"),
        };
        assert!(
            error
                .message()
                .contains("owned descriptor unlink-dispatch failure")
        );
        secret_free(&error.message());
        assert_eq!(attempts, 1);
        assert!(
            owned_bytes(&case) == bytes,
            "a failed unlink leaves its selected descriptor"
        );
        pinned
            .remove(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                until,
                |path, deadline| {
                    attempts += 1;
                    assert_eq!(deadline, until);
                    std::fs::remove_file(path)
                        .map_err(|_| CafleetError::App("owned descriptor retry failed".into()))
                },
            )
            .unwrap();
        assert_eq!(attempts, 2);
        assert!(!case.artifacts.paths.descriptor.exists());
    }

    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    use crate::runtime::artifacts::*;
    use crate::runtime::test_support::Fixture;
    use rusqlite::params;

    #[test]
    fn runner_artifact_namespace_is_database_adjacent_and_separates_sibling_databases() {
        let first = Fixture::new();
        let mut second = Connection::open(first._dir.path().join("sibling.db")).unwrap();
        crate::db::initialize(&mut second).unwrap();
        let a =
            RuntimePaths::for_run(&storage::database_identity(&first.conn).unwrap(), 1).unwrap();
        let b = RuntimePaths::for_run(&storage::database_identity(&second).unwrap(), 1).unwrap();
        assert_ne!(a.namespace_dir, b.namespace_dir);
        assert_ne!(a.run_dir, b.run_dir);
        assert_ne!(a.control_socket, b.control_socket);
        assert_ne!(a.outbox, b.outbox);
        assert_eq!(
            a.namespace_dir.parent().unwrap(),
            std::fs::canonicalize(first._dir.path())
                .unwrap()
                .join(".cafleet-runs")
        );
        assert_eq!(
            a.namespace_dir.file_name().unwrap().to_str().unwrap().len(),
            26
        );
        assert_eq!(a.run_dir.file_name().unwrap(), "1");
    }

    #[test]
    fn runner_artifact_identity_changes_for_copied_or_replaced_database_files() {
        let fixture = Fixture::new();
        let original = storage::database_identity(&fixture.conn).unwrap();
        let copy = fixture._dir.path().join("copy.db");
        std::fs::copy(fixture.conn.path().unwrap(), &copy).unwrap();
        let copied = crate::db::connect(&format!("sqlite:///{}", copy.display())).unwrap();
        let copied_identity = storage::database_identity(&copied).unwrap();
        assert_eq!(original["instance_id"], copied_identity["instance_id"]);
        assert_ne!(
            RuntimePaths::for_run(&original, 1).unwrap().namespace_dir,
            RuntimePaths::for_run(&copied_identity, 1)
                .unwrap()
                .namespace_dir
        );
        let selected = std::path::PathBuf::from(fixture.conn.path().unwrap());
        std::fs::rename(&selected, fixture._dir.path().join("old.db")).unwrap();
        std::fs::copy(&copy, &selected).unwrap();
        let replacement = crate::db::connect(&format!("sqlite:///{}", selected.display())).unwrap();
        let replacement_identity = storage::database_identity(&replacement).unwrap();
        assert_eq!(original["instance_id"], replacement_identity["instance_id"]);
        assert_ne!(original["inode"], replacement_identity["inode"]);
        assert_ne!(
            RuntimePaths::for_run(&original, 1).unwrap().namespace_dir,
            RuntimePaths::for_run(&replacement_identity, 1)
                .unwrap()
                .namespace_dir
        );
    }

    #[test]
    fn runner_artifact_allocation_pins_private_directory_and_lock_inodes() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        for directory in [&artifacts.paths.namespace_dir, &artifacts.paths.run_dir] {
            assert_eq!(
                std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let lock = std::fs::metadata(&artifacts.paths.owner_lock).unwrap();
        assert_eq!(lock.permissions().mode() & 0o777, 0o600);
        assert!(fixture.run.runtime_dir_identity_json.is_some());
        assert!(fixture.run.run_lock_identity_json.is_some());
        assert_eq!(
            fixture.run.runtime_path,
            artifacts.paths.run_dir.to_str().unwrap()
        );
    }

    #[test]
    fn runner_artifact_existing_directory_and_conflicting_namespace_manifest_are_preserved() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let sentinel = artifacts.paths.run_dir.join("retained");
        std::fs::write(&sentinel, b"existing-owner-data").unwrap();
        assert!(RunArtifacts::create(&mut fixture.conn, fixture.run.run_id).is_err());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"existing-owner-data");
        let mut other = Fixture::new();
        let paths =
            RuntimePaths::for_run(&other.run.database_identity_json, other.run.run_id).unwrap();
        std::fs::create_dir_all(&paths.namespace_dir).unwrap();
        std::fs::set_permissions(
            paths.namespace_dir.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::set_permissions(&paths.namespace_dir, std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let mut foreign = other.run.database_identity_json.clone();
        foreign["instance_id"] = json!("0".repeat(32));
        std::fs::write(
            &paths.namespace_manifest,
            serde_json::to_vec(&foreign).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(
            &paths.namespace_manifest,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let before = std::fs::read(&paths.namespace_manifest).unwrap();
        assert!(RunArtifacts::create(&mut other.conn, other.run.run_id).is_err());
        assert_eq!(std::fs::read(&paths.namespace_manifest).unwrap(), before);
        assert!(!paths.run_dir.exists());
    }

    #[test]
    fn runner_artifact_allocation_rejects_a_symlinked_runtime_namespace() {
        let mut fixture = Fixture::new();
        let paths =
            RuntimePaths::for_run(&fixture.run.database_identity_json, fixture.run.run_id).unwrap();
        let target = fixture._dir.path().join("foreign");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(paths.namespace_dir.parent().unwrap()).unwrap();
        symlink(&target, &paths.namespace_dir).unwrap();
        assert!(RunArtifacts::create(&mut fixture.conn, fixture.run.run_id).is_err());
        assert!(std::fs::read_dir(&target).unwrap().next().is_none());
    }

    #[test]
    fn runner_descriptor_round_trips_literal_argv_cwd_environment_and_private_permissions() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let spec = fixture.spec();
        let descriptor = LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap();
        artifacts.write_descriptor(&descriptor).unwrap();
        assert_eq!(
            std::fs::metadata(&artifacts.paths.descriptor)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let read = load_descriptor(&fixture.conn, &artifacts.paths.descriptor).unwrap();
        assert_eq!(read.spawn.argv, spec.argv);
        assert_eq!(read.spawn.cwd, spec.cwd);
        assert_eq!(read.spawn.environment, spec.environment);
        assert_eq!(read.spawn.size, spec.size);
        assert_eq!(read.identity.run_id, fixture.run.run_id);
        assert_eq!(read.identity.launch_nonce, fixture.run.launch_nonce);
        assert_eq!(read.identity.owner_epoch, fixture.run.owner_epoch);
        artifacts.remove_descriptor().unwrap();
        artifacts.remove_descriptor().unwrap();
        assert!(!artifacts.paths.descriptor.exists());
    }

    #[test]
    fn runner_descriptor_records_the_forwarded_config_location_in_order() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let mut spec = fixture.spec();
        for &agent in crate::coding_agent::CodingAgent::all() {
            spec.environment = agent
                .forwarded_config(&|name| {
                    Ok((name == agent.config_variable()).then(|| format!("/cfg/{}", agent.name())))
                })
                .unwrap()
                .into_iter()
                .collect();
            let descriptor = LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap();
            artifacts.write_descriptor(&descriptor).unwrap();
            let read = load_descriptor(&fixture.conn, &artifacts.paths.descriptor).unwrap();
            assert_eq!(
                read.spawn.environment,
                vec![(
                    agent.config_variable().to_owned(),
                    format!("/cfg/{}", agent.name())
                )]
            );
            artifacts.remove_descriptor().unwrap();
        }
    }

    #[test]
    fn runner_descriptor_load_rejects_malformed_or_identity_mismatched_input() {
        for damage in ["json", "nonce", "epoch", "run", "database"] {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let descriptor = LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap();
            artifacts.write_descriptor(&descriptor).unwrap();
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&artifacts.paths.descriptor).unwrap())
                    .unwrap();
            match damage {
                "json" => {
                    std::fs::write(&artifacts.paths.descriptor, b"{").unwrap();
                }
                "nonce" => {
                    value["launch_nonce"] = json!("b".repeat(64));
                }
                "epoch" => {
                    value["owner_epoch"] = json!(fixture.run.owner_epoch + 1);
                }
                "run" => {
                    value["run_id"] = json!(fixture.run.run_id + 1);
                }
                "database" => {
                    value["database_identity_json"]["instance_id"] = json!("0".repeat(32));
                }
                _ => unreachable!(),
            }
            if damage != "json" {
                std::fs::write(
                    &artifacts.paths.descriptor,
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
            }
            assert!(
                load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err(),
                "{damage}"
            );
        }
    }

    #[test]
    fn runner_descriptor_load_rejects_symlinks_and_world_readable_launch_input() {
        for symlinked in [false, true] {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            artifacts
                .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
                .unwrap();
            if symlinked {
                let moved = artifacts.paths.run_dir.join("moved-descriptor");
                std::fs::rename(&artifacts.paths.descriptor, &moved).unwrap();
                symlink(&moved, &artifacts.paths.descriptor).unwrap();
            } else {
                std::fs::set_permissions(
                    &artifacts.paths.descriptor,
                    std::fs::Permissions::from_mode(0o644),
                )
                .unwrap();
            }
            assert!(load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err());
        }
    }

    #[test]
    fn runner_private_metadata_validation_rejects_a_different_owner_without_privileged_chown() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let metadata = std::fs::metadata(&artifacts.paths.owner_lock).unwrap();
        validate_private_file(&artifacts.paths.owner_lock, metadata.uid(), 0o600).unwrap();
        assert!(
            validate_private_file(
                &artifacts.paths.owner_lock,
                metadata.uid().checked_add(1).unwrap(),
                0o600
            )
            .is_err()
        );
    }

    #[test]
    fn runner_lock_keeps_the_original_inode_exclusive_even_if_its_path_is_unlinked() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let owner = fixture.run.owner();
        let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
        assert!(RunLock::acquire(&fixture.conn, &owner).unwrap().is_none());
        let original = std::fs::metadata(&artifacts.paths.owner_lock)
            .unwrap()
            .ino();
        std::fs::remove_file(&artifacts.paths.owner_lock).unwrap();
        std::fs::write(&artifacts.paths.owner_lock, b"").unwrap();
        std::fs::set_permissions(
            &artifacts.paths.owner_lock,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_ne!(
            std::fs::metadata(&artifacts.paths.owner_lock)
                .unwrap()
                .ino(),
            original
        );
        assert!(RunLock::acquire(&fixture.conn, &owner).is_err());
        assert!(held.validate(&fixture.conn, &owner).is_err());
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .owner_epoch,
            owner.owner_epoch
        );
    }

    #[test]
    fn runner_lock_transfer_advances_epoch_and_fences_the_old_owner() {
        let mut fixture = Fixture::new();
        fixture.artifacts();
        let owner = fixture.run.owner();
        let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
        let next = held
            .transfer_to_runner(
                &mut fixture.conn,
                &owner,
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
        assert_eq!(next.owner_epoch, owner.owner_epoch + 1);
        assert_eq!(next.owner_kind, OwnerKind::Runner);
        assert!(held.validate(&fixture.conn, &owner).is_err());
        held.validate(&fixture.conn, &next).unwrap();
        assert!(RunLock::acquire(&fixture.conn, &next).unwrap().is_none());
        drop(held);
        assert!(
            RunLock::acquire_released(&fixture.conn, &next)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn runner_artifact_paths_validate_complete_socket_length_before_creation() {
        let fixture = Fixture::new();
        let mut identity = fixture.run.database_identity_json.clone();
        identity["canonical_path"] = json!(format!("/private/tmp/{}/registry.db", "x".repeat(240)));
        assert!(RuntimePaths::for_run(&identity, 1).is_err());
        for run in [0, -1] {
            assert!(RuntimePaths::for_run(&fixture.run.database_identity_json, run).is_err());
        }
    }

    #[test]
    fn runner_database_replacement_fences_descriptors_and_owned_lock_checks() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        artifacts
            .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
            .unwrap();
        let owner = fixture.run.owner();
        let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
        let selected = std::path::PathBuf::from(fixture.conn.path().unwrap());
        let copy = fixture._dir.path().join("replacement.db");
        std::fs::copy(&selected, &copy).unwrap();
        std::fs::rename(&selected, fixture._dir.path().join("old.db")).unwrap();
        std::fs::rename(&copy, &selected).unwrap();
        assert!(held.validate(&fixture.conn, &owner).is_err());
        assert!(load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err());
        assert!(artifacts.paths.run_dir.exists());
        assert_eq!(
            fixture
                .conn
                .query_row(
                    "SELECT owner_epoch FROM member_runs WHERE run_id=?1",
                    params![owner.run_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            owner.owner_epoch
        );
    }

    #[test]
    fn runner_artifact_canonical_aliases_share_one_database_namespace() {
        let fixture = Fixture::new();
        let alias = fixture._dir.path().join("database-alias.db");
        symlink(fixture.conn.path().unwrap(), &alias).unwrap();
        let conn = crate::db::connect(&format!("sqlite:///{}", alias.display())).unwrap();
        let identity = storage::database_identity(&conn).unwrap();
        assert_eq!(identity, fixture.run.database_identity_json);
        assert_eq!(
            RuntimePaths::for_run(&identity, 1).unwrap().run_dir,
            RuntimePaths::for_run(&fixture.run.database_identity_json, 1)
                .unwrap()
                .run_dir
        );
    }

    #[test]
    fn runner_descriptor_removal_reports_an_unlink_failure_without_claiming_cleanup() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        artifacts
            .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
            .unwrap();
        let retained = artifacts.paths.run_dir.join("retained-descriptor");
        std::fs::rename(&artifacts.paths.descriptor, &retained).unwrap();
        std::fs::create_dir(&artifacts.paths.descriptor).unwrap();
        assert!(artifacts.remove_descriptor().is_err());
        assert!(retained.is_file());
        assert!(artifacts.paths.descriptor.is_dir());
    }

    #[test]
    fn runner_descriptor_construction_validates_required_spawn_inputs_before_writing() {
        for missing in ["argv", "cwd", "rows", "cols"] {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let mut spec = fixture.spec();
            match missing {
                "argv" => spec.argv.clear(),
                "cwd" => spec.cwd = "relative".into(),
                "rows" => spec.size.rows = 0,
                "cols" => spec.size.cols = 0,
                _ => unreachable!(),
            }
            assert!(
                LaunchDescriptor::new(&fixture.run, spec).is_err(),
                "{missing}"
            );
            assert!(!artifacts.paths.descriptor.exists());
        }
    }

    mod lock_transfer {
        use crate::broker::runtime::RunStatus;
        use crate::runtime::artifacts::tests::*;

        fn locked() -> (Fixture, RunArtifacts, RunLock) {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
            (fixture, artifacts, lock)
        }

        #[test]
        fn runner_recovery_lock_transfer_commits_fresh_epochs_and_preserves_first_origin_status() {
            let (mut fixture, _artifacts, lock) = locked();
            let original = fixture.run.clone();
            let process = ProcessIdentity::current().unwrap();
            for offset in 1..=2 {
                let owner = lock
                    .transfer_to_reconciler_with_deadline(
                        &mut fixture.conn,
                        &fixture.run,
                        process.clone(),
                        Instant::now() + Duration::from_secs(1),
                    )
                    .unwrap();
                fixture.run = storage::get_run(&fixture.conn, original.run_id).unwrap();
                assert_eq!(owner, fixture.run.owner());
                assert_eq!(fixture.run.owner_epoch, original.owner_epoch + offset);
                assert_eq!(fixture.run.owner_kind, OwnerKind::Reconciler);
                assert_eq!(fixture.run.status, RunStatus::Recovering);
                assert_eq!(fixture.run.recovery_origin_status, Some(original.status));
                assert_eq!(fixture.run.owner_pid, process.pid);
                assert_eq!(fixture.run.owner_start_identity, process.start_identity);
                assert_eq!(
                    fixture.run.runtime_dir_identity_json,
                    original.runtime_dir_identity_json
                );
                assert_eq!(
                    fixture.run.run_lock_identity_json,
                    original.run_lock_identity_json
                );
                lock.validate(&fixture.conn, &owner).unwrap();
            }
            assert!(
                storage::reserve_event_sequences(&mut fixture.conn, &original.owner()).is_err()
            );
        }

        #[test]
        fn runner_recovery_lock_transfer_rejects_every_changed_stable_expected_identity() {
            let (mut fixture, _artifacts, lock) = locked();
            let before = fixture.run.clone();
            let mut expected = before.clone();
            expected.launch_nonce = "b".repeat(64);
            assert!(
                lock.transfer_to_reconciler_with_deadline(
                    &mut fixture.conn,
                    &expected,
                    ProcessIdentity::current().unwrap(),
                    Instant::now() + Duration::from_secs(1),
                )
                .is_err()
            );
            assert_eq!(
                storage::get_run(&fixture.conn, before.run_id).unwrap(),
                before
            );
            lock.validate(&fixture.conn, &before.owner()).unwrap();
        }

        #[test]
        fn runner_recovery_lock_transfer_commit_failure_returns_no_uncommitted_owner() {
            let (mut fixture, _artifacts, lock) = locked();
            fixture.conn.commit_hook(Some(|| true)).unwrap();
            let result = lock.transfer_to_reconciler_with_deadline(
                &mut fixture.conn,
                &fixture.run,
                ProcessIdentity::current().unwrap(),
                Instant::now() + Duration::from_secs(1),
            );
            fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
            assert!(result.is_err());
            assert_eq!(
                storage::get_run(&fixture.conn, fixture.run.run_id).unwrap(),
                fixture.run
            );
            lock.validate(&fixture.conn, &fixture.run.owner()).unwrap();
        }

        #[test]
        fn runner_recovery_lock_transfer_contention_uses_the_original_absolute_deadline() {
            let (mut fixture, _artifacts, lock) = locked();
            let writer = Connection::open(fixture.conn.path().unwrap()).unwrap();
            writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            let started = Instant::now();
            let result = lock.transfer_to_reconciler_with_deadline(
                &mut fixture.conn,
                &fixture.run,
                ProcessIdentity::current().unwrap(),
                started + Duration::from_millis(80),
            );
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_millis(750));
            writer.execute_batch("ROLLBACK").unwrap();
            assert_eq!(
                storage::get_run(&fixture.conn, fixture.run.run_id).unwrap(),
                fixture.run
            );
        }
    }

    mod recovery_descriptor {
        use crate::error::CafleetError;
        use crate::runtime::artifacts::*;
        use crate::runtime::recovery::test_support::Case;
        use crate::runtime::recovery::test_support::TestResult;
        use crate::runtime::recovery::test_support::descriptor::described_creator;
        use crate::runtime::recovery::test_support::descriptor::identity;
        use crate::runtime::recovery::test_support::descriptor::owned_bytes;
        use crate::runtime::recovery::test_support::descriptor::secret_free;
        use crate::runtime::recovery::test_support::descriptor::write_private;
        use serde_json::Value;
        use serde_json::json;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        use std::time::Instant;

        fn overwrite(path: &std::path::Path, bytes: &[u8]) {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }

        fn pin(case: &mut Case) -> TestResult<RecoveryDescriptor> {
            let lock = case.held.as_ref().unwrap().clone();
            RecoveryDescriptor::pin(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                Instant::now() + Duration::from_secs(1),
            )
        }

        fn remove(case: &mut Case, pinned: &mut RecoveryDescriptor) -> (TestResult<()>, bool) {
            let lock = case.held.as_ref().unwrap().clone();
            let mut dispatched = false;
            let result = pinned.remove(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                Instant::now() + Duration::from_secs(1),
                |path, _| {
                    dispatched = true;
                    std::fs::remove_file(path)
                        .map_err(|_| CafleetError::App("owned descriptor removal failed".into()))
                },
            );
            (result, dispatched)
        }

        #[derive(Clone, Copy, Debug)]
        enum Change {
            Replacement,
            Symlink,
            Content,
            Appeared,
        }

        #[test]
        fn recovery_descriptor_remove_preserves_a_descriptor_changed_after_its_pin() {
            for change in [
                Change::Replacement,
                Change::Symlink,
                Change::Content,
                Change::Appeared,
            ] {
                let mut case = described_creator();
                let original = owned_bytes(&case);
                let path = case.artifacts.paths.descriptor.clone();
                let retained = case.artifacts.paths.run_dir.join("retained-launch.json");
                if matches!(change, Change::Appeared) {
                    case.artifacts.remove_descriptor().unwrap();
                }
                let mut pinned = pin(&mut case).unwrap();
                let expected = match change {
                    Change::Replacement => {
                        std::fs::rename(&path, &retained).unwrap();
                        write_private(&path, &original);
                        assert_ne!(identity(&path).1, identity(&retained).1);
                        original.clone()
                    }
                    Change::Symlink => {
                        std::fs::rename(&path, &retained).unwrap();
                        std::os::unix::fs::symlink(&retained, &path).unwrap();
                        original.clone()
                    }
                    Change::Content => {
                        let mut value: Value = serde_json::from_slice(&original).unwrap();
                        value["spawn"]["argv"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!("owned-in-place-change"));
                        let bytes = serde_json::to_vec(&value).unwrap();
                        let before = identity(&path);
                        overwrite(&path, &bytes);
                        assert_eq!((identity(&path).0, identity(&path).1), (before.0, before.1));
                        bytes
                    }
                    Change::Appeared => {
                        write_private(&path, &original);
                        original.clone()
                    }
                };
                let named = identity(&path);
                let (result, dispatched) = remove(&mut case, &mut pinned);
                let error = result.expect_err("a descriptor changed after its pin is refused");
                secret_free(&error.message());
                assert!(!dispatched, "{change:?}");
                assert_eq!(identity(&path), named, "{change:?}");
                if matches!(change, Change::Symlink) {
                    assert!(
                        std::fs::symlink_metadata(&path)
                            .unwrap()
                            .file_type()
                            .is_symlink()
                    );
                    assert_eq!(std::fs::read(&retained).unwrap(), expected);
                } else {
                    assert_eq!(std::fs::read(&path).unwrap(), expected, "{change:?}");
                }
            }
        }

        #[test]
        fn recovery_descriptor_disappearing_after_its_pin_is_a_safe_absence() {
            let mut case = described_creator();
            let mut pinned = pin(&mut case).unwrap();
            std::fs::remove_file(&case.artifacts.paths.descriptor).unwrap();
            let (result, dispatched) = remove(&mut case, &mut pinned);
            result.unwrap();
            assert!(!dispatched);
            assert!(!case.artifacts.paths.descriptor.exists());
        }

        #[test]
        fn recovery_descriptor_pin_refuses_an_invalid_descriptor_privately() {
            for damage in ["size", "json", "schema", "member", "permissions", "symlink"] {
                let mut case = described_creator();
                let path = case.artifacts.paths.descriptor.clone();
                let mut value: Value = serde_json::from_slice(&owned_bytes(&case)).unwrap();
                match damage {
                    "size" => overwrite(&path, &vec![b' '; 1024 * 1024 + 1]),
                    "json" => overwrite(&path, b"{"),
                    "schema" => {
                        value["schema_version"] = json!(2);
                        overwrite(&path, &serde_json::to_vec(&value).unwrap());
                    }
                    "member" => {
                        value["member_id"] = json!(case.fixture.run.member_id + 1);
                        overwrite(&path, &serde_json::to_vec(&value).unwrap());
                    }
                    "permissions" => {
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                            .unwrap();
                    }
                    "symlink" => {
                        let retained = case.artifacts.paths.run_dir.join("invalid-target.json");
                        std::fs::rename(&path, &retained).unwrap();
                        std::os::unix::fs::symlink(&retained, &path).unwrap();
                    }
                    _ => unreachable!(),
                }
                let before = identity(&path);
                let error = match pin(&mut case) {
                    Err(error) => error,
                    Ok(_) => panic!("an invalid descriptor is refused at pin: {damage}"),
                };
                secret_free(&error.message());
                assert_eq!(identity(&path), before, "{damage}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use tempfile::TempDir;

    /// A database directory under `/tmp`, because `RuntimePaths::for_run` places the control
    /// socket beneath the database parent and refuses socket paths past the platform limit.
    pub(crate) fn short_tempdir() -> TempDir {
        tempfile::Builder::new()
            .prefix(".cafleet-test-")
            .tempdir_in("/tmp")
            .unwrap()
    }
}
