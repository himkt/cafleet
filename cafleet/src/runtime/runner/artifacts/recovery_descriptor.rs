use super::*;
use std::io::{Seek, SeekFrom};

const DESCRIPTOR_LIMIT: u64 = 1024 * 1024;

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
            super::super::recovery::stable_run_identity(&current)
                == super::super::recovery::stable_run_identity(run),
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
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { path, pinned: None });
            }
            Err(cause) => return Err(io_error(cause)),
        }
        let mut file = open_private(&path, false)?;
        let file_identity = identity(&file.metadata().map_err(io_error)?);
        let bytes = read_bounded(&mut file, until)?;
        let descriptor: LaunchDescriptor =
            serde_json::from_slice(&bytes).map_err(|_| error("malformed launch descriptor"))?;
        ensure(
            descriptor.schema_version == 1,
            "unsupported launch descriptor schema",
        )?;
        ensure(
            descriptor.database_identity_json == run.database_identity_json
                && descriptor.fleet_id == run.fleet_id
                && descriptor.member_id == run.member_id
                && descriptor.director_member_id == run.director_member_id
                && descriptor.run_id == run.run_id
                && descriptor.launch_nonce == run.launch_nonce
                && descriptor.owner_epoch > 0
                && descriptor.owner_epoch <= run.owner_epoch,
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
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                self.pinned = None;
                return Ok(());
            }
            Err(cause) => return Err(io_error(cause)),
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
        match fs::symlink_metadata(&self.path) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => return Err(io_error(cause)),
            Ok(_) => return Err(error("descriptor remains after unlink")),
        }
        check_deadline(Instant::now(), until)?;
        self.pinned = None;
        Ok(())
    }
}
