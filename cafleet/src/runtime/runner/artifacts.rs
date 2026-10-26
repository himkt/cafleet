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
