use super::*;
use crate::broker::members::db_err;
use crate::broker::runtime::{self as storage, OwnerKind, RunOwner, RunRecord};
use rusqlite::{Connection, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

mod recovery_descriptor;
pub(crate) use recovery_descriptor::RecoveryDescriptor;

pub(crate) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> = map.iter().collect();
            format!(
                "{{{}}}",
                sorted
                    .iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON key serializes"),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}
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
    pub namespace_dir: PathBuf,
    pub namespace_manifest: PathBuf,
    pub run_dir: PathBuf,
    pub descriptor: PathBuf,
    pub control_socket: PathBuf,
    pub owner_lock: PathBuf,
    pub outbox: PathBuf,
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
fn private_metadata(
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
fn directory(path: &Path) -> Result<()> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(cause) => return Err(io_error(cause)),
    }
    private_metadata(path, uid(), 0o700, true)?;
    Ok(())
}
fn open_private(path: &Path, create: bool) -> Result<File> {
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
pub(crate) fn validate_database(conn: &Connection, expected: &Value) -> Result<()> {
    ensure(
        storage::database_identity(conn)? == *expected,
        "runtime database identity changed",
    )
}
pub(crate) fn validate_run_paths(conn: &Connection, run: &RunRecord) -> Result<RuntimePaths> {
    validate_database(conn, &run.database_identity_json)?;
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
    let manifest = open_private(&paths.namespace_manifest, false)?;
    let mut text = String::new();
    manifest
        .take(65537)
        .read_to_string(&mut text)
        .map_err(io_error)?;
    let value: Value = serde_json::from_str(&text).map_err(io_error)?;
    ensure(
        value == run.database_identity_json,
        "runtime namespace identity collision",
    )?;
    Ok(paths)
}

pub(crate) struct RunArtifacts {
    pub paths: RuntimePaths,
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
        validate_database(conn, &run.database_identity_json)?;
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
                let mut file = open_private(&paths.namespace_manifest, false)?;
                let mut text = String::new();
                file.read_to_string(&mut text).map_err(io_error)?;
                ensure(
                    serde_json::from_str::<Value>(&text).map_err(io_error)?
                        == run.database_identity_json,
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
            validate_database(conn, &run.database_identity_json)?;
            let changed=conn.execute("UPDATE member_runs SET runtime_path=?1,runtime_dir_identity_json=?2,run_lock_identity_json=?3 WHERE run_id=?4 AND owner_epoch=?5 AND owner_kind='creator' AND runtime_dir_identity_json IS NULL",params![paths.run_dir.to_string_lossy(),dir_id.to_string(),lock_id.to_string(),run_id,run.owner_epoch]).map_err(db_err)?;
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
                == RuntimePaths::for_run(&descriptor.database_identity_json, descriptor.run_id)?
                    .run_dir,
            "descriptor path mismatch",
        )?;
        descriptor.spawn.validate()?;
        let mut file = open_private(&self.paths.descriptor, true)?;
        *self.descriptor_identity.borrow_mut() =
            Some(identity(&file.metadata().map_err(io_error)?));
        let data = serde_json::to_vec(descriptor).map_err(io_error)?;
        ensure(
            data.len() <= 1024 * 1024,
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
    pub schema_version: u32,
    pub database_identity_json: Value,
    pub fleet_id: i64,
    pub member_id: i64,
    pub director_member_id: i64,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub launch_nonce: String,
    pub spawn: SpawnSpec,
}
impl LaunchDescriptor {
    pub(crate) fn new(run: &RunRecord, spawn: SpawnSpec) -> Result<Self> {
        spawn.validate()?;
        Ok(Self {
            schema_version: 1,
            database_identity_json: run.database_identity_json.clone(),
            fleet_id: run.fleet_id,
            member_id: run.member_id,
            director_member_id: run.director_member_id,
            run_id: run.run_id,
            owner_epoch: run.owner_epoch,
            launch_nonce: run.launch_nonce.clone(),
            spawn,
        })
    }
}
pub(crate) fn load_descriptor(conn: &Connection, path: &Path) -> Result<LaunchDescriptor> {
    ensure(
        path.is_absolute(),
        "absolute runner descriptor path required",
    )?;
    let mut file = open_private(path, false)?;
    ensure(
        file.metadata().map_err(io_error)?.len() <= 1024 * 1024,
        "launch descriptor exceeds private size limit",
    )?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    let descriptor: LaunchDescriptor =
        serde_json::from_slice(&bytes).map_err(|_| error("malformed launch descriptor"))?;
    ensure(
        descriptor.schema_version == 1,
        "unsupported launch descriptor schema",
    )?;
    descriptor.spawn.validate()?;
    let run = storage::get_run(conn, descriptor.run_id)?;
    let paths = validate_run_paths(conn, &run)?;
    ensure(
        path == paths.descriptor,
        "runner descriptor was not allocated for this run",
    )?;
    ensure(
        run.owner_kind == OwnerKind::Creator
            && run.owner_epoch == descriptor.owner_epoch
            && run.launch_nonce == descriptor.launch_nonce
            && run.database_identity_json == descriptor.database_identity_json
            && run.fleet_id == descriptor.fleet_id
            && run.member_id == descriptor.member_id
            && run.director_member_id == descriptor.director_member_id,
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
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_err)?;
            check(&tx)?;
            let current = storage::get_run(&tx, expected.run_id)?;
            validate_database(&tx, &expected.database_identity_json)?;
            ensure(
                super::recovery::stable_run_identity(&current)
                    == super::recovery::stable_run_identity(expected),
                "recovery ownership changed before CAS",
            )?;
            let next = if current.status.terminal()
                && current.cleanup_state == storage::CleanupState::Complete
            {
                current
            } else {
                self.validate(&tx, &expected.owner())?;
                super::recovery::advance_recovery_owner(&tx, expected, &process)?
            };
            check_deadline(Instant::now(), deadline)?;
            tx.commit().map_err(db_err)?;
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
    /// The run-lock counterpart of [`crate::input::PaneLock::acquire_released`].
    #[cfg(test)]
    pub(crate) fn acquire_released(conn: &Connection, owner: &RunOwner) -> Result<Option<Self>> {
        let until = Instant::now() + crate::input::RELEASED_LOCK_GRACE;
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
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_err)?;
            self.validate(&tx, owner)?;
            let epoch = owner
                .owner_epoch
                .checked_add(1)
                .ok_or_else(|| error("owner epoch exhausted"))?;
            let changed=tx.execute("UPDATE member_runs SET owner_epoch=?1,owner_kind='runner',owner_pid=?2,owner_start_identity=?3,runner_pid=?2,runner_start_identity=?3,last_health_at=?4 WHERE run_id=?5 AND owner_epoch=?6 AND owner_kind='creator'",params![epoch,process.pid,process.start_identity,crate::time::format_utc(crate::time::now_utc()),owner.run_id,owner.owner_epoch]).map_err(db_err)?;
            ensure(changed == 1, "creator ownership transfer lost CAS")?;
            let next = storage::get_run(&tx, owner.run_id)?.owner();
            tx.commit().map_err(db_err)?;
            Ok(next)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;
    use crate::runtime::runner::test_support::Fixture;
    use rusqlite::params;

    #[test]
    fn runner_artifact_namespace_is_database_adjacent_and_separates_sibling_databases() {
        let first = Fixture::new();
        let mut second = Connection::open(first._dir.path().join("sibling.db")).unwrap();
        crate::db::migrate_to_head(&mut second).unwrap();
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
        assert_eq!(read.run_id, fixture.run.run_id);
        assert_eq!(read.launch_nonce, fixture.run.launch_nonce);
        assert_eq!(read.owner_epoch, fixture.run.owner_epoch);
        artifacts.remove_descriptor().unwrap();
        artifacts.remove_descriptor().unwrap();
        assert!(!artifacts.paths.descriptor.exists());
    }

    #[test]
    fn runner_descriptor_records_the_forwarded_config_location_in_order() {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let mut spec = fixture.spec();
        let database_url = spec.environment[0].1.clone();
        for &agent in crate::coding_agent::CodingAgent::all() {
            spec.environment = agent
                .spawn_environment(&database_url, &|name| {
                    (name == agent.config_variable()).then(|| format!("/cfg/{}", agent.name()))
                })
                .unwrap();
            let descriptor = LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap();
            artifacts.write_descriptor(&descriptor).unwrap();
            let read = load_descriptor(&fixture.conn, &artifacts.paths.descriptor).unwrap();
            assert_eq!(
                read.spawn.environment,
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), database_url.clone()),
                    (
                        agent.config_variable().to_owned(),
                        format!("/cfg/{}", agent.name())
                    ),
                ]
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
        use super::*;
        use crate::broker::runtime::{Reachability, RunStatus};

        #[test]
        fn runner_recovery_lock_transfer_commits_fresh_epochs_and_preserves_first_origin_status() {
            let mut fixture = Fixture::new();
            let _artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
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
            for changed in [
                "epoch",
                "owner",
                "nonce",
                "backend",
                "director",
                "runtime_path",
                "directory",
                "lock",
                "runner",
                "child",
                "database",
            ] {
                let mut fixture = Fixture::new();
                let _artifacts = fixture.artifacts();
                let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap();
                let before = fixture.run.clone();
                let mut expected = before.clone();
                match changed {
                    "epoch" => expected.owner_epoch += 1,
                    "owner" => expected.owner_start_identity.push_str("-foreign"),
                    "nonce" => expected.launch_nonce = "b".repeat(64),
                    "backend" => expected.backend = "claude".into(),
                    "director" => expected.director_member_id += 1,
                    "runtime_path" => expected.runtime_path.push_str("-foreign"),
                    "directory" => {
                        expected.runtime_dir_identity_json = Some(json!({"device":0,"inode":0}))
                    }
                    "lock" => expected.run_lock_identity_json = Some(json!({"device":0,"inode":0})),
                    "runner" => {
                        expected.runner_pid = Some(1);
                        expected.runner_start_identity = Some("foreign".into());
                    }
                    "child" => {
                        expected.child_pid = Some(1);
                        expected.child_start_identity = Some("foreign".into());
                    }
                    "database" => {
                        expected.database_identity_json["instance_id"] = json!("0".repeat(32))
                    }
                    _ => unreachable!(),
                }
                assert!(
                    lock.transfer_to_reconciler_with_deadline(
                        &mut fixture.conn,
                        &expected,
                        ProcessIdentity::current().unwrap(),
                        Instant::now() + Duration::from_secs(1),
                    )
                    .is_err(),
                    "{changed}"
                );
                assert_eq!(
                    storage::get_run(&fixture.conn, before.run_id).unwrap(),
                    before,
                    "{changed}"
                );
                lock.validate(&fixture.conn, &before.owner()).unwrap();
            }
        }

        #[test]
        fn runner_recovery_lock_transfer_ignores_mutable_health_diagnostics_and_sequence_snapshots()
        {
            let mut fixture = Fixture::new();
            let _artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
            let mut expected = fixture.run.clone();
            expected.last_health_at = Some("2000-01-01T00:00:00Z".into());
            expected.reachability = Reachability::Suspect;
            expected.cleanup_diagnostics_json = Some(json!(["old bounded diagnostic"]));
            expected.event_seq_reserved_through += 256;
            expected.observation_gap_count += 1;
            lock.transfer_to_reconciler_with_deadline(
                &mut fixture.conn,
                &expected,
                ProcessIdentity::current().unwrap(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
            let current = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            assert_eq!(
                current.event_seq_reserved_through,
                fixture.run.event_seq_reserved_through
            );
            assert_eq!(
                current.observation_gap_count,
                fixture.run.observation_gap_count
            );
        }

        #[test]
        fn runner_recovery_lock_transfer_commit_failure_returns_no_uncommitted_owner() {
            let mut fixture = Fixture::new();
            let _artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
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
            let mut fixture = Fixture::new();
            let _artifacts = fixture.artifacts();
            let lock = RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .unwrap();
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
}
