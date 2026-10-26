use super::*;
use crate::broker::runtime::{self as storage, ObservedTransition, RunOwner, RunRecord};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::sync::Arc;
use std::time::Instant;

const OUTBOX_LIMIT: u64 = 16 * 1024 * 1024;
pub(crate) trait OutboxIo: Read + Write + Seek + Send {
    fn sync_all(&mut self) -> std::io::Result<()>;
    fn byte_len(&self) -> std::io::Result<u64>;
    fn set_len(&mut self, length: u64) -> std::io::Result<()>;
}
impl OutboxIo for File {
    fn sync_all(&mut self) -> std::io::Result<()> {
        File::sync_all(self)
    }
    fn byte_len(&self) -> std::io::Result<u64> {
        Ok(self.metadata()?.len())
    }
    fn set_len(&mut self, length: u64) -> std::io::Result<()> {
        File::set_len(self, length)
    }
}
pub(crate) struct OutboxFrame {
    pub transition: ObservedTransition,
    pub start: u64,
    pub end: u64,
}
pub(crate) struct OutboxScan {
    pub frames: Vec<OutboxFrame>,
    pub incomplete_tail: Option<String>,
    pub maximum_event_seq: Option<i64>,
}
pub(crate) struct Outbox<S: OutboxIo> {
    io: S,
    conn: Option<Connection>,
    run: RunRecord,
    lock: Arc<RunLock>,
    known: BTreeMap<i64, [u8; 32]>,
    end: u64,
    dirty: bool,
    path_identity: Option<(u64, u64)>,
    hooks: Arc<dyn WriterHooks>,
}
impl Outbox<File> {
    pub(crate) fn open_recovery(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<Self> {
        check_deadline(Instant::now(), until)?;
        lock.validate(conn, &run.owner())?;
        let paths = validate_run_paths(conn, run)?;
        validate_private_file(&paths.outbox, nix::unistd::geteuid().as_raw(), 0o600)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&paths.outbox)
            .map_err(io_error)?;
        let meta = file.metadata().map_err(io_error)?;
        let named = std::fs::symlink_metadata(&paths.outbox).map_err(io_error)?;
        ensure(
            (meta.dev(), meta.ino()) == (named.dev(), named.ino()),
            "outbox pathname changed during open",
        )?;
        let mut value = Self::state(run, lock, file, None);
        value.path_identity = Some((meta.dev(), meta.ino()));
        value.scan_recovery(conn, until)?;
        Ok(value)
    }
    pub(crate) fn open_owned(
        conn: &Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<Self> {
        check_deadline(Instant::now(), until)?;
        lock.validate(conn, &run.owner())?;
        let paths = validate_run_paths(conn, run)?;
        validate_private_file(&paths.outbox, nix::unistd::geteuid().as_raw(), 0o600)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&paths.outbox)
            .map_err(io_error)?;
        let meta = file.metadata().map_err(io_error)?;
        let named = std::fs::symlink_metadata(&paths.outbox).map_err(io_error)?;
        ensure(
            meta.dev() == named.dev() && meta.ino() == named.ino(),
            "outbox pathname changed during open",
        )?;
        let mut value = Self::initialize(conn, run, lock, file, until)?;
        value.path_identity = Some((meta.dev(), meta.ino()));
        Ok(value)
    }
}
impl<S: OutboxIo> Outbox<S> {
    fn state(run: &RunRecord, lock: Arc<RunLock>, io: S, conn: Option<Connection>) -> Self {
        Self {
            io,
            conn,
            run: run.clone(),
            lock,
            known: BTreeMap::new(),
            end: 0,
            dirty: true,
            path_identity: None,
            hooks: Arc::new(InertWriterHooks),
        }
    }
    fn initialize(
        conn: &Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        io: S,
        until: Instant,
    ) -> Result<Self> {
        check_deadline(Instant::now(), until)?;
        lock.validate(conn, &run.owner())?;
        let path = conn
            .path()
            .ok_or_else(|| error("outbox requires a file-backed registry"))?;
        let validation = crate::db::connect_read_only(&format!("sqlite:///{path}"))?;
        let mut value = Self::state(run, lock, io, Some(validation));
        value.scan(until)?;
        Ok(value)
    }
    #[cfg(test)]
    pub(crate) fn from_io(
        conn: &Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        io: S,
        until: Instant,
    ) -> Result<Self> {
        Self::initialize(conn, run, lock, io, until)
    }
    pub(super) fn set_hooks(&mut self, hooks: Arc<dyn WriterHooks>) {
        self.hooks = hooks;
    }
    fn validate(&mut self, until: Instant) -> Result<RunRecord> {
        check_deadline(Instant::now(), until)?;
        let owner = self.run.owner();
        let lock = &self.lock;
        let run = with_database_deadline(
            self.conn.as_mut().expect("worker validation connection"),
            until,
            |conn| {
                lock.validate(conn, &owner)?;
                storage::get_run(conn, owner.run_id)
            },
        )?;
        self.validate_named(&run)?;
        Ok(run)
    }
    fn validate_on(&self, conn: &mut Connection, until: Instant) -> Result<RunRecord> {
        let run = with_database_deadline(conn, until, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            storage::get_run(conn, self.run.run_id)
        })?;
        self.validate_named(&run)?;
        Ok(run)
    }
    fn validate_named(&self, run: &RunRecord) -> Result<()> {
        if let Some((dev, ino)) = self.path_identity {
            let meta =
                std::fs::symlink_metadata(PathBuf::from(&run.runtime_path).join("events.outbox"))
                    .map_err(io_error)?;
            ensure(
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.uid() == nix::unistd::geteuid().as_raw()
                    && meta.mode() & 0o777 == 0o600
                    && meta.dev() == dev
                    && meta.ino() == ino,
                "owned outbox inode changed",
            )?;
        }
        Ok(())
    }
    pub(crate) fn scan(&mut self, until: Instant) -> Result<OutboxScan> {
        let run = self.validate(until)?;
        self.scan_validated(run, until)
    }
    pub(crate) fn scan_recovery(
        &mut self,
        conn: &mut Connection,
        until: Instant,
    ) -> Result<OutboxScan> {
        let run = self.validate_on(conn, until)?;
        self.scan_validated(run, until)
    }
    fn scan_validated(&mut self, run: RunRecord, until: Instant) -> Result<OutboxScan> {
        let length = self.io.byte_len().map_err(io_error)?;
        ensure(length <= OUTBOX_LIMIT, "runtime outbox exceeds capacity")?;
        self.io.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut frames = Vec::new();
        let mut known = BTreeMap::new();
        let mut position = 0;
        let mut tail = None;
        let mut previous = 0;
        while position < length {
            check_deadline(Instant::now(), until)?;
            if length - position < 4 {
                tail = Some("incomplete outbox length".into());
                break;
            }
            let mut header = [0u8; 4];
            self.io.read_exact(&mut header).map_err(io_error)?;
            let bytes = u64::from(u32::from_be_bytes(header));
            ensure(
                bytes > 0 && bytes <= OUTBOX_LIMIT - 36,
                "invalid outbox frame length",
            )?;
            if length - position < 36 + bytes {
                tail = Some("incomplete outbox frame".into());
                break;
            }
            let mut checksum = [0u8; 32];
            self.io.read_exact(&mut checksum).map_err(io_error)?;
            let mut body = vec![0; usize::try_from(bytes).map_err(io_error)?];
            self.io.read_exact(&mut body).map_err(io_error)?;
            let digest: [u8; 32] = Sha256::digest(&body).into();
            ensure(digest == checksum, "outbox frame checksum mismatch")?;
            let transition: ObservedTransition = serde_json::from_slice(&body).map_err(io_error)?;
            ensure(
                serde_json::from_slice::<Value>(&body).map_err(io_error)?
                    == serde_json::to_value(&transition).map_err(io_error)?,
                "unknown outbox transition fields",
            )?;
            storage::validate_observed_transition(&run, &transition)?;
            ensure(
                transition.event_seq >= previous,
                "outbox sequences regressed",
            )?;
            if let Some(old) = known.insert(transition.event_seq, digest) {
                ensure(old == digest, "conflicting outbox original sequence")?;
            }
            previous = transition.event_seq;
            let end = position + 36 + bytes;
            frames.push(OutboxFrame {
                transition,
                start: position,
                end,
            });
            position = end;
        }
        self.end = position;
        self.dirty = tail.is_some();
        self.known = known;
        Ok(OutboxScan {
            maximum_event_seq: frames.last().map(|frame| frame.transition.event_seq),
            frames,
            incomplete_tail: tail,
        })
    }
    pub(crate) fn append_synced(
        &mut self,
        value: &ObservedTransition,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate(until)?;
        self.append_validated(value, run, until)
    }
    pub(crate) fn append_recovery(
        &mut self,
        conn: &mut Connection,
        value: &ObservedTransition,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate_on(conn, until)?;
        self.append_validated(value, run, until)
    }
    fn append_validated(
        &mut self,
        value: &ObservedTransition,
        run: RunRecord,
        until: Instant,
    ) -> Result<()> {
        storage::validate_observed_transition(&run, value)?;
        if self.dirty || self.io.byte_len().map_err(io_error)? != self.end {
            let scan = self.scan_validated(run.clone(), until)?;
            if scan.incomplete_tail.is_some() {
                self.io.set_len(self.end).map_err(io_error)?;
                self.io.sync_all().map_err(io_error)?;
                self.dirty = false;
            }
        }
        let body = serde_json::to_vec(value).map_err(io_error)?;
        let digest: [u8; 32] = Sha256::digest(&body).into();
        if let Some(old) = self.known.get(&value.event_seq) {
            ensure(*old == digest, "conflicting outbox sequence retry")?;
        } else {
            ensure(
                self.known
                    .last_key_value()
                    .is_none_or(|(seq, _)| value.event_seq > *seq),
                "outbox append sequence regressed",
            )?;
            let end = self
                .end
                .checked_add(body.len() as u64 + 36)
                .ok_or_else(|| error("outbox size overflow"))?;
            ensure(
                end <= OUTBOX_LIMIT,
                "runtime outbox reached 16 MiB pressure limit",
            )?;
            self.hooks.before(WriterBoundary::Append, until)?;
            self.io.seek(SeekFrom::Start(self.end)).map_err(io_error)?;
            self.dirty = true;
            self.io
                .write_all(&u32::try_from(body.len()).map_err(io_error)?.to_be_bytes())
                .map_err(io_error)?;
            self.io.write_all(&digest).map_err(io_error)?;
            self.io.write_all(&body).map_err(io_error)?;
            self.known.insert(value.event_seq, digest);
            self.end = end;
            self.dirty = false;
        }
        self.dirty = true;
        self.hooks.before(WriterBoundary::Sync, until)?;
        check_deadline(Instant::now(), until)?;
        self.io.sync_all().map_err(io_error)?;
        self.dirty = false;
        Ok(())
    }
    pub(crate) fn compact_committed(
        &mut self,
        conn: &mut Connection,
        owner: &RunOwner,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate_on(conn, until)?;
        ensure(*owner == self.run.owner(), "compaction owner mismatch")?;
        let length = self.io.byte_len().map_err(io_error)?;
        let scan = self.scan_validated(run, until)?;
        let all = if scan.maximum_event_seq.is_some() {
            with_database_deadline(conn, until, |conn| {
                self.lock.validate(conn, owner)?;
                let mut all = true;
                for frame in &scan.frames {
                    check_deadline(Instant::now(), until)?;
                    let id: Option<i64> = conn
                .query_row(
                    "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2",
                    rusqlite::params![owner.run_id, frame.transition.event_seq],
                    |row| row.get(0),
                )
                .optional()
                .map_err(crate::broker::members::db_err)?;
                    if let Some(id) = id {
                        let view = storage::get_event(conn, id)?;
                        ensure(
                            storage::original_transition(&view)? == frame.transition,
                            &format!(
                                "committed outbox frame {}..{} original mismatch",
                                frame.start, frame.end
                            ),
                        )?;
                    } else {
                        all = false;
                    }
                }
                Ok(all)
            })?
        } else {
            true
        };
        if all {
            self.hooks.before(WriterBoundary::Compact, until)?;
            self.validate_on(conn, until)?;
            ensure(
                length == self.io.byte_len().map_err(io_error)?,
                "outbox size changed before compaction",
            )?;
            check_deadline(Instant::now(), until)?;
            self.dirty = true;
            self.io.set_len(0).map_err(io_error)?;
            check_deadline(Instant::now(), until)?;
            self.io.sync_all().map_err(io_error)?;
            self.end = 0;
            self.known.clear();
            self.dirty = false;
        }
        Ok(())
    }
}
