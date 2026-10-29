use super::*;
use crate::broker::runtime::{
    self as storage, EffectivePolicyObservation, ObservedTransition, RunOwner, RunRecord,
};
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
    pub effective_policy: Option<EffectivePolicyObservation>,
    pub start: u64,
    pub end: u64,
}
const EFFECTIVE_POLICY_KEY: &str = "effective_policy";
fn frame_body(
    transition: &ObservedTransition,
    effective_policy: Option<&EffectivePolicyObservation>,
) -> Result<Vec<u8>> {
    let Some(observation) = effective_policy else {
        return serde_json::to_vec(transition).map_err(io_error);
    };
    let mut body = serde_json::to_value(transition).map_err(io_error)?;
    body.as_object_mut()
        .expect("a transition serializes as an object")
        .insert(
            EFFECTIVE_POLICY_KEY.into(),
            serde_json::to_value(observation).map_err(io_error)?,
        );
    serde_json::to_vec(&body).map_err(io_error)
}
fn parse_frame_body(
    body: &[u8],
) -> Result<(ObservedTransition, Option<EffectivePolicyObservation>)> {
    let mut value = serde_json::from_slice::<Value>(body).map_err(io_error)?;
    let effective_policy = value
        .as_object_mut()
        .ok_or_else(|| error("outbox frame body must be an object"))?
        .remove(EFFECTIVE_POLICY_KEY)
        .map(serde_json::from_value::<EffectivePolicyObservation>)
        .transpose()
        .map_err(io_error)?;
    let transition: ObservedTransition = serde_json::from_value(value.clone()).map_err(io_error)?;
    ensure(
        value == serde_json::to_value(&transition).map_err(io_error)?,
        "unknown outbox transition fields",
    )?;
    ensure(
        effective_policy.is_none()
            || transition.kind == storage::RuntimeEventKind::AutomaticReviewUnavailable,
        "only an unavailable review frame carries an effective policy",
    )?;
    Ok((transition, effective_policy))
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
            let (transition, effective_policy) = parse_frame_body(&body)?;
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
                effective_policy,
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
    #[cfg(test)]
    pub(crate) fn append_synced(
        &mut self,
        value: &ObservedTransition,
        until: Instant,
    ) -> Result<()> {
        self.append_synced_with_policy(value, None, until)
    }
    pub(crate) fn append_synced_with_policy(
        &mut self,
        value: &ObservedTransition,
        effective_policy: Option<&EffectivePolicyObservation>,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate(until)?;
        self.append_validated(value, effective_policy, run, until)
    }
    pub(crate) fn append_recovery(
        &mut self,
        conn: &mut Connection,
        value: &ObservedTransition,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate_on(conn, until)?;
        self.append_validated(value, None, run, until)
    }
    fn append_validated(
        &mut self,
        value: &ObservedTransition,
        effective_policy: Option<&EffectivePolicyObservation>,
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
        ensure(
            effective_policy.is_none()
                || value.kind == storage::RuntimeEventKind::AutomaticReviewUnavailable,
            "only an unavailable review frame carries an effective policy",
        )?;
        let body = frame_body(value, effective_policy)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::runtime::*;
    use crate::runtime::runner::writer::test_support::{
        WriterFixture, deadline, frame, independent_frames, transition,
    };
    use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    mod review {
        use super::*;

        fn ordinary_connection_work(conn: &rusqlite::Connection, timeout: i64) {
            assert_eq!(
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                timeout
            );
            assert_eq!(
                conn.query_row(
                    "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
                50005000
            );
        }

        #[test]
        fn runner_review_r2_compaction_cancels_supplied_connection_sql_preserves_bytes_and_retries()
        {
            let mut fixture = WriterFixture::new();
            let original = transition(&fixture, 1);
            storage::persist_transition(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                &original,
                PersistenceMode::Live,
            )
            .unwrap();
            let io = FaultIo::from_bytes(frame(&original));
            let mut outbox = open(&fixture, io.clone());
            let before = io.snapshot_bytes();
            fixture
                .fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            fixture.fixture.conn.execute_batch(
                "CREATE TEMP VIEW member_runtime_events AS SELECT original.* FROM main.member_runtime_events original WHERE (WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter)>0",
            ).unwrap();
            let interrupt = fixture.fixture.conn.get_interrupt_handle();
            let emergency = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let fired = emergency.clone();
            let (stop, receive) = std::sync::mpsc::channel();
            let begin = Instant::now();
            let until = begin + Duration::from_millis(60);
            let (result, elapsed) = std::thread::scope(|scope| {
                let watchdog = scope.spawn(move || {
                    if matches!(
                        receive.recv_timeout(Duration::from_millis(650)),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        fired.store(true, Ordering::Release);
                        interrupt.interrupt();
                    }
                });
                let result = outbox.compact_committed(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    until,
                );
                let elapsed = begin.elapsed();
                let _ = stop.send(());
                watchdog.join().unwrap();
                (result, elapsed)
            });
            assert!(
                result.is_err(),
                "expensive supplied-connection SQL must be cancelled"
            );
            assert_eq!(io.snapshot_bytes(), before);
            fixture
                .fixture
                .conn
                .execute_batch("DROP VIEW temp.member_runtime_events")
                .unwrap();
            std::thread::sleep(
                until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            ordinary_connection_work(&fixture.fixture.conn, 137);
            outbox
                .compact_committed(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    deadline(),
                )
                .unwrap();
            assert!(io.snapshot_bytes().is_empty());
            ordinary_connection_work(&fixture.fixture.conn, 137);
            assert!(
                elapsed < Duration::from_millis(400) && !emergency.load(Ordering::Acquire),
                "compaction exceeded its 60 ms SQL budget: {elapsed:?}; emergency interrupt fired={} ",
                emergency.load(Ordering::Acquire)
            );
        }

        struct PanicAtCompaction;
        impl WriterHooks for PanicAtCompaction {
            fn before(&self, boundary: WriterBoundary, _: Instant) -> Result<()> {
                if matches!(boundary, WriterBoundary::Compact) {
                    panic!("owned compaction unwind");
                }
                Ok(())
            }
        }

        #[test]
        fn runner_review_r2_compaction_restores_supplied_connection_after_success_error_and_unwind()
        {
            for outcome in ["success", "error", "unwind"] {
                let mut fixture = WriterFixture::new();
                let original = transition(&fixture, 1);
                storage::persist_transition(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    &original,
                    PersistenceMode::Live,
                )
                .unwrap();
                let io = FaultIo::from_bytes(frame(&original));
                let mut outbox = open(&fixture, io.clone());
                let before = io.snapshot_bytes();
                fixture
                    .fixture
                    .conn
                    .busy_timeout(Duration::from_millis(173))
                    .unwrap();
                if outcome == "error" {
                    fixture
                        .fixture
                        .conn
                        .execute("UPDATE messages SET text='owned invalid association'", [])
                        .unwrap();
                }
                if outcome == "unwind" {
                    outbox.set_hooks(Arc::new(PanicAtCompaction));
                }
                let until = Instant::now() + Duration::from_millis(80);
                let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    outbox.compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        until,
                    )
                }));
                match outcome {
                    "success" => assert!(attempt.unwrap().is_ok()),
                    "error" => assert!(attempt.unwrap().is_err()),
                    "unwind" => {
                        let payload =
                            attempt.expect_err("compaction preserves the original unwind");
                        assert_eq!(
                            payload.downcast_ref::<&str>(),
                            Some(&"owned compaction unwind")
                        );
                    }
                    _ => unreachable!(),
                }
                if outcome == "success" {
                    assert!(io.snapshot_bytes().is_empty());
                } else {
                    assert_eq!(io.snapshot_bytes(), before);
                }
                std::thread::sleep(
                    until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
                );
                ordinary_connection_work(&fixture.fixture.conn, 173);
            }
        }
    }

    mod terminal_tail {
        use super::*;

        fn terminal_spool(committed: bool) -> (WriterFixture, FaultIo, Vec<u8>) {
            let mut fixture = WriterFixture::new();
            let mut bytes = Vec::new();
            if committed {
                let original = ObservedTransition {
                    kind: RuntimeEventKind::RunnerExited,
                    cause_json: Some(
                        serde_json::json!({"kind":"native_exit","exit_code":7,"exit_signal":null}),
                    ),
                    ..transition(&fixture, 1)
                };
                storage::persist_transition(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    &original,
                    PersistenceMode::Live,
                )
                .unwrap();
                bytes.extend(frame(&original));
            } else {
                fixture
                    .fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET status='exited',exit_code=7 WHERE run_id=?1",
                        [fixture.fixture.run.run_id],
                    )
                    .unwrap();
            }
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            bytes.extend([0, 0, 0, 40, 1, 2, 3]);
            let io = FaultIo::from_bytes(bytes.clone());
            (fixture, io, bytes)
        }

        #[test]
        fn runner_recovery_terminal_tail_compaction_truncates_and_syncs_empty_or_committed_originals()
         {
            for committed in [false, true] {
                let (mut fixture, io, _) = terminal_spool(committed);
                let mut outbox = open(&fixture, io.clone());
                let scan = outbox.scan(deadline()).unwrap();
                assert!(scan.incomplete_tail.is_some());
                assert_eq!(scan.frames.len(), usize::from(committed));
                let before =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                let syncs = io.0.lock().unwrap().syncs;
                outbox
                    .compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        deadline(),
                    )
                    .unwrap();
                assert!(io.snapshot_bytes().is_empty());
                let state = io.0.lock().unwrap();
                assert_eq!(state.truncations.last(), Some(&0));
                assert!(
                    state.syncs > syncs,
                    "successful tail retirement includes fsync"
                );
                drop(state);
                assert_eq!(
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
                    before
                );
            }
        }

        #[test]
        fn runner_recovery_terminal_tail_truncate_failure_retains_bytes_and_retries_the_same_owner()
        {
            for committed in [false, true] {
                let (mut fixture, io, bytes) = terminal_spool(committed);
                let mut outbox = open(&fixture, io.clone());
                let before =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                io.0.lock().unwrap().fail_truncate = true;
                let error = outbox
                    .compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        deadline(),
                    )
                    .expect_err("incomplete tail truncate failure remains explicit");
                assert!(error.message().contains("fixture frame truncate failure"));
                assert_eq!(io.snapshot_bytes(), bytes);
                assert_eq!(
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
                    before
                );
                let syncs = io.0.lock().unwrap().syncs;
                io.0.lock().unwrap().fail_truncate = false;
                outbox
                    .compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        deadline(),
                    )
                    .unwrap();
                assert!(io.snapshot_bytes().is_empty());
                assert!(io.0.lock().unwrap().syncs > syncs);
            }
        }

        #[test]
        fn runner_recovery_terminal_tail_sync_failure_requires_successful_sync_on_retry_even_after_truncation()
         {
            for committed in [false, true] {
                let (mut fixture, io, bytes) = terminal_spool(committed);
                let mut outbox = open(&fixture, io.clone());
                let before =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
                io.0.lock().unwrap().fail_sync = true;
                let error = outbox
                    .compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        deadline(),
                    )
                    .expect_err("incomplete tail sync failure remains explicit");
                assert!(error.message().contains("fixture frame sync failure"));
                let retained = io.snapshot_bytes();
                assert!(
                    retained == bytes
                        || retained.is_empty()
                        || retained == bytes[..bytes.len() - 7],
                    "failed synchronization preserves a valid original prefix"
                );
                assert_eq!(
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap(),
                    before
                );
                let failed_syncs = io.0.lock().unwrap().syncs;
                assert!(failed_syncs > 0);
                io.0.lock().unwrap().fail_sync = false;
                outbox
                    .compact_committed(
                        &mut fixture.fixture.conn,
                        &fixture.fixture.run.owner(),
                        deadline(),
                    )
                    .unwrap();
                assert!(io.snapshot_bytes().is_empty());
                assert!(
                    io.0.lock().unwrap().syncs > failed_syncs,
                    "a retry completes fsync even when the failed attempt already truncated the file"
                );
            }
        }
    }

    #[derive(Default)]
    struct FaultState {
        data: Cursor<Vec<u8>>,
        short_write: Option<usize>,
        fail_after: Option<u64>,
        fail_sync: bool,
        fail_truncate: bool,
        syncs: usize,
        largest_read: usize,
        truncations: Vec<u64>,
    }
    #[derive(Clone, Default)]
    struct FaultIo(Arc<Mutex<FaultState>>);
    impl FaultIo {
        fn snapshot_bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().data.get_ref().clone()
        }
        fn from_bytes(bytes: Vec<u8>) -> Self {
            let value = Self::default();
            value.0.lock().unwrap().data = Cursor::new(bytes);
            value
        }
    }
    impl Read for FaultIo {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let mut state = self.0.lock().unwrap();
            state.largest_read = state.largest_read.max(bytes.len());
            state.data.read(bytes)
        }
    }
    impl Write for FaultIo {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut state = self.0.lock().unwrap();
            let position = state.data.position();
            if state.fail_after.is_some_and(|limit| position >= limit) {
                return Err(io::Error::other("fixture frame write failure"));
            }
            let mut limit = bytes.len();
            if let Some(short) = state.short_write {
                limit = limit.min(short);
            }
            if let Some(end) = state.fail_after {
                limit = limit.min(usize::try_from(end - position).unwrap());
            }
            state.data.write(&bytes[..limit])
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for FaultIo {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.0.lock().unwrap().data.seek(position)
        }
    }
    impl OutboxIo for FaultIo {
        fn sync_all(&mut self) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            state.syncs += 1;
            if state.fail_sync {
                Err(io::Error::other("fixture frame sync failure"))
            } else {
                Ok(())
            }
        }
        fn byte_len(&self) -> io::Result<u64> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .data
                .get_ref()
                .len()
                .try_into()
                .unwrap())
        }
        fn set_len(&mut self, length: u64) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            if state.fail_truncate {
                return Err(io::Error::other("fixture frame truncate failure"));
            }
            state.truncations.push(length);
            state.data.get_mut().resize(length.try_into().unwrap(), 0);
            Ok(())
        }
    }

    fn open(fixture: &WriterFixture, io: FaultIo) -> Outbox<FaultIo> {
        Outbox::from_io(
            &fixture.fixture.conn,
            &fixture.fixture.run,
            fixture.lock.clone(),
            io,
            deadline(),
        )
        .unwrap()
    }

    mod effective_policy_frames {
        use super::*;
        use crate::runtime::runner::writer::test_support::{
            frame_bodies, observation, unavailable,
        };

        fn framed(body: &Value) -> Vec<u8> {
            let body = serde_json::to_vec(body).unwrap();
            let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
            bytes.extend_from_slice(&Sha256::digest(&body));
            bytes.extend(body);
            bytes
        }

        #[test]
        fn runner_outbox_unavailable_frame_body_adds_only_the_effective_policy_key() {
            let fixture = WriterFixture::new();
            let io = FaultIo::default();
            let mut outbox = open(&fixture, io.clone());
            let plain = transition(&fixture, 1);
            outbox
                .append_synced_with_policy(&plain, None, deadline())
                .unwrap();
            assert_eq!(io.snapshot_bytes(), frame(&plain), "unchanged frame bytes");
            let value = unavailable(&fixture, 2);
            let lost = observation(AutomaticReview::Unavailable);
            outbox
                .append_synced_with_policy(&value, Some(&lost), deadline())
                .unwrap();
            let mut expected = serde_json::to_value(&value).unwrap();
            expected["effective_policy"] = serde_json::to_value(&lost).unwrap();
            assert_eq!(
                frame_bodies(&io.snapshot_bytes()),
                vec![serde_json::to_value(&plain).unwrap(), expected]
            );
            let scan = outbox.scan(deadline()).unwrap();
            assert_eq!(scan.frames.len(), 2);
            assert!(scan.frames[0].effective_policy.is_none());
            assert_eq!(scan.frames[1].transition, value);
            assert_eq!(
                serde_json::to_value(scan.frames[1].effective_policy.as_ref().unwrap()).unwrap(),
                serde_json::to_value(&lost).unwrap()
            );
            outbox
                .append_synced_with_policy(&value, Some(&lost), deadline())
                .unwrap();
            assert_eq!(
                frame_bodies(&io.snapshot_bytes()).len(),
                2,
                "identical retry"
            );
            assert!(
                outbox
                    .append_synced_with_policy(&value, None, deadline())
                    .is_err(),
                "a retry without its observation conflicts with the original frame"
            );
        }

        #[test]
        fn runner_outbox_rejects_an_effective_policy_on_any_other_frame() {
            let fixture = WriterFixture::new();
            let io = FaultIo::default();
            let mut outbox = open(&fixture, io.clone());
            let plain = transition(&fixture, 1);
            let lost = observation(AutomaticReview::Unavailable);
            assert!(
                outbox
                    .append_synced_with_policy(&plain, Some(&lost), deadline())
                    .is_err()
            );
            assert!(io.snapshot_bytes().is_empty());
            let mut forged = serde_json::to_value(&plain).unwrap();
            forged["effective_policy"] = serde_json::to_value(&lost).unwrap();
            let on_disk = FaultIo::from_bytes(framed(&forged));
            assert!(
                Outbox::from_io(
                    &fixture.fixture.conn,
                    &fixture.fixture.run,
                    fixture.lock.clone(),
                    on_disk,
                    deadline(),
                )
                .is_err()
            );
            let mut malformed = serde_json::to_value(unavailable(&fixture, 1)).unwrap();
            malformed["effective_policy"] = serde_json::json!({"automatic_review":"available"});
            assert!(
                Outbox::from_io(
                    &fixture.fixture.conn,
                    &fixture.fixture.run,
                    fixture.lock.clone(),
                    FaultIo::from_bytes(framed(&malformed)),
                    deadline(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn runner_batch2_outbox_frames_pin_big_endian_length_checksum_and_original_identity() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        let mut outbox = open(&fixture, io.clone());
        let value = transition(&fixture, 1);
        let syncs = io.0.lock().unwrap().syncs;
        outbox.append_synced(&value, deadline()).unwrap();
        assert_eq!(io.snapshot_bytes(), frame(&value));
        assert!(io.0.lock().unwrap().syncs > syncs);
        let scan = outbox.scan(deadline()).unwrap();
        assert_eq!(scan.frames.len(), 1);
        assert_eq!(scan.frames[0].transition, value);
        assert_eq!(scan.frames[0].start, 0);
        assert_eq!(scan.frames[0].end, io.snapshot_bytes().len() as u64);
        assert!(scan.incomplete_tail.is_none());
        assert_eq!(scan.maximum_event_seq, Some(1));
    }

    #[test]
    fn runner_batch2_outbox_matching_duplicates_reconcile_but_conflicting_sequence_bytes_fail() {
        let fixture = WriterFixture::new();
        let original = transition(&fixture, 1);
        let mut duplicate = frame(&original);
        duplicate.extend(frame(&original));
        let io = FaultIo::from_bytes(duplicate);
        let mut outbox = open(&fixture, io);
        assert!(outbox.scan(deadline()).is_ok());
        let mut changed = original.clone();
        changed.observed_at = "2026-10-26T16:41:00.000000+00:00".into();
        let mut bytes = frame(&original);
        bytes.extend(frame(&changed));
        let io = FaultIo::from_bytes(bytes.clone());
        let result = Outbox::from_io(
            &fixture.fixture.conn,
            &fixture.fixture.run,
            fixture.lock.clone(),
            io.clone(),
            deadline(),
        )
        .and_then(|mut outbox| outbox.scan(deadline()));
        assert!(result.is_err());
        assert_eq!(io.snapshot_bytes(), bytes);
    }

    #[test]
    fn runner_batch2_outbox_total_capacity_includes_frame_overhead_and_rejects_before_append() {
        let mut fixture = WriterFixture::new();
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                [fixture.fixture.run.run_id],
            )
            .unwrap();
        for _ in 1..16 {
            storage::reserve_event_sequences(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
            )
            .unwrap();
        }
        fixture.fixture.run =
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        let io = FaultIo::default();
        let mut outbox = open(&fixture, io.clone());
        let policy=storage::evidence::EvidencePolicy::redacted(&serde_json::json!({"schema_version":1,"patterns":["absent-fixture-canary"],"sensitive_rules":[]}),&["runner-lifecycle"]).unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        let mut used = 0;
        for sequence in 1..=4096 {
            assert!(
                Instant::now() < until,
                "bounded outbox-capacity fixture exceeded its budget"
            );
            let mut value = transition(&fixture, sequence);
            let mut raw = value.evidence_json.clone();
            raw["sensitive"] = serde_json::json!(false);
            raw["excerpt"] = serde_json::json!("x".repeat(6000));
            value.evidence_json = policy.apply(&raw).unwrap();
            value.kind = RuntimeEventKind::PermissionDeniedObserved;
            value.cause_json = None;
            let size = frame(&value).len();
            if used + size > 16 * 1024 * 1024 {
                let before = io.snapshot_bytes();
                assert!(outbox.append_synced(&value, until).is_err());
                assert_eq!(io.snapshot_bytes(), before);
                assert!(used <= 16 * 1024 * 1024);
                return;
            }
            outbox.append_synced(&value, until).unwrap();
            used += size;
            assert_eq!(io.byte_len().unwrap(), used as u64);
        }
        panic!("fixture must reach the fixed outbox capacity");
    }

    #[test]
    fn runner_batch2_outbox_short_writes_preserve_the_complete_frame() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        io.0.lock().unwrap().short_write = Some(1);
        let mut outbox = open(&fixture, io.clone());
        let value = transition(&fixture, 1);
        outbox.append_synced(&value, deadline()).unwrap();
        assert_eq!(independent_frames(&io.snapshot_bytes()), vec![value]);
    }

    #[test]
    fn runner_batch2_outbox_reopen_recovers_complete_original_frames_in_order() {
        let fixture = WriterFixture::new();
        let values = [
            transition(&fixture, 1),
            transition(&fixture, 2),
            transition(&fixture, 3),
        ];
        let bytes = values.iter().flat_map(frame).collect();
        let io = FaultIo::from_bytes(bytes);
        let mut outbox = open(&fixture, io.clone());
        let scan = outbox.scan(deadline()).unwrap();
        assert_eq!(
            scan.frames
                .iter()
                .map(|entry| entry.transition.clone())
                .collect::<Vec<_>>(),
            values
        );
        assert_eq!(scan.maximum_event_seq, Some(3));
        assert!(scan.incomplete_tail.is_none());
        assert_eq!(independent_frames(&io.snapshot_bytes()), values);
    }

    #[test]
    fn runner_batch2_outbox_each_incomplete_final_component_retains_the_valid_prefix() {
        let fixture = WriterFixture::new();
        let first = frame(&transition(&fixture, 1));
        let second = frame(&transition(&fixture, 2));
        for cut in [1, 3, 4, 20, 35, 36, second.len() - 1] {
            let mut bytes = first.clone();
            bytes.extend_from_slice(&second[..cut]);
            let io = FaultIo::from_bytes(bytes.clone());
            let mut outbox = open(&fixture, io.clone());
            let scan = outbox.scan(deadline()).unwrap();
            assert_eq!(scan.frames.len(), 1, "cut={cut}");
            assert_eq!(scan.frames[0].transition, transition(&fixture, 1));
            assert!(scan.incomplete_tail.is_some(), "cut={cut}");
            assert_eq!(io.snapshot_bytes(), bytes);
        }
    }

    #[test]
    fn runner_batch2_outbox_append_diagnoses_and_truncates_only_an_incomplete_tail() {
        let fixture = WriterFixture::new();
        let first = frame(&transition(&fixture, 1));
        let mut initial = first.clone();
        initial.extend_from_slice(&[0, 0]);
        let io = FaultIo::from_bytes(initial);
        let mut outbox = open(&fixture, io.clone());
        assert!(outbox.scan(deadline()).unwrap().incomplete_tail.is_some());
        outbox
            .append_synced(&transition(&fixture, 2), deadline())
            .unwrap();
        assert_eq!(
            independent_frames(&io.snapshot_bytes()),
            vec![transition(&fixture, 1), transition(&fixture, 2)]
        );
        assert!(
            io.0.lock()
                .unwrap()
                .truncations
                .contains(&(first.len() as u64))
        );
    }

    #[test]
    fn runner_batch2_outbox_completed_corruption_preserves_all_original_bytes() {
        let fixture = WriterFixture::new();
        let valid = frame(&transition(&fixture, 1));
        for corrupt in ["checksum", "json", "run", "sequence"] {
            let mut value = transition(&fixture, 1);
            if corrupt == "run" {
                value.run_id += 1;
            }
            if corrupt == "sequence" {
                value.event_seq = 0;
            }
            let mut bytes = if matches!(corrupt, "run" | "sequence") {
                frame(&value)
            } else {
                valid.clone()
            };
            if corrupt == "checksum" {
                bytes[4] ^= 1;
            }
            if corrupt == "json" {
                bytes[36] = b'!';
                let digest = Sha256::digest(&bytes[36..]);
                bytes[4..36].copy_from_slice(&digest);
            }
            let io = FaultIo::from_bytes(bytes.clone());
            let result = Outbox::from_io(
                &fixture.fixture.conn,
                &fixture.fixture.run,
                fixture.lock.clone(),
                io.clone(),
                deadline(),
            )
            .and_then(|mut outbox| outbox.scan(deadline()));
            assert!(result.is_err(), "{corrupt}");
            assert_eq!(io.snapshot_bytes(), bytes);
        }
    }

    #[test]
    fn runner_batch2_outbox_oversized_declared_length_fails_before_large_reads() {
        let fixture = WriterFixture::new();
        let io = FaultIo::from_bytes(u32::MAX.to_be_bytes().to_vec());
        let result = Outbox::from_io(
            &fixture.fixture.conn,
            &fixture.fixture.run,
            fixture.lock.clone(),
            io.clone(),
            deadline(),
        )
        .and_then(|mut outbox| outbox.scan(deadline()));
        assert!(result.is_err());
        assert!(io.0.lock().unwrap().largest_read <= 36);
        assert_eq!(io.snapshot_bytes(), u32::MAX.to_be_bytes());
    }

    #[test]
    fn runner_batch2_outbox_sync_failure_leaves_a_complete_recoverable_pending_frame() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        let mut outbox = open(&fixture, io.clone());
        io.0.lock().unwrap().fail_sync = true;
        assert!(
            outbox
                .append_synced(&transition(&fixture, 1), deadline())
                .is_err()
        );
        assert_eq!(
            independent_frames(&io.snapshot_bytes()),
            vec![transition(&fixture, 1)]
        );
        io.0.lock().unwrap().fail_sync = false;
        assert_eq!(outbox.scan(deadline()).unwrap().frames.len(), 1);
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }

    #[test]
    fn runner_batch2_outbox_partial_write_failure_is_an_explicit_tail_without_a_commit() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        let mut outbox = open(&fixture, io.clone());
        io.0.lock().unwrap().fail_after = Some(40);
        assert!(
            outbox
                .append_synced(&transition(&fixture, 1), deadline())
                .is_err()
        );
        let scan = outbox.scan(deadline()).unwrap();
        assert!(scan.frames.is_empty());
        assert!(scan.incomplete_tail.is_some());
        assert_eq!(io.snapshot_bytes().len(), 40);
    }

    #[test]
    fn runner_batch2_outbox_compaction_retains_committed_prefix_until_every_frame_is_durable() {
        let mut fixture = WriterFixture::new();
        let values = [transition(&fixture, 1), transition(&fixture, 2)];
        let io = FaultIo::from_bytes(values.iter().flat_map(frame).collect());
        let mut outbox = open(&fixture, io.clone());
        let original = io.snapshot_bytes();
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &values[0],
            PersistenceMode::Live,
        )
        .unwrap();
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert_eq!(io.snapshot_bytes(), original);
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &values[1],
            PersistenceMode::Live,
        )
        .unwrap();
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert!(io.snapshot_bytes().is_empty());
        assert!(io.0.lock().unwrap().syncs > 0);
    }

    #[test]
    fn runner_batch2_outbox_compaction_checks_event_message_association_before_removal() {
        let mut fixture = WriterFixture::new();
        let value = transition(&fixture, 1);
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &value,
            PersistenceMode::Live,
        )
        .unwrap();
        let io = FaultIo::from_bytes(frame(&value));
        let mut outbox = open(&fixture, io.clone());
        fixture
            .fixture
            .conn
            .execute("UPDATE messages SET text='corrupt durable association'", [])
            .unwrap();
        let before = io.snapshot_bytes();
        assert!(
            outbox
                .compact_committed(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    deadline()
                )
                .is_err()
        );
        assert_eq!(io.snapshot_bytes(), before);
    }

    #[test]
    fn runner_batch2_outbox_truncate_failure_preserves_committed_originals_for_retry() {
        let mut fixture = WriterFixture::new();
        let value = transition(&fixture, 1);
        storage::persist_transition(
            &mut fixture.fixture.conn,
            &fixture.fixture.run.owner(),
            &value,
            PersistenceMode::Live,
        )
        .unwrap();
        let io = FaultIo::from_bytes(frame(&value));
        let mut outbox = open(&fixture, io.clone());
        io.0.lock().unwrap().fail_truncate = true;
        let before = io.snapshot_bytes();
        assert!(
            outbox
                .compact_committed(
                    &mut fixture.fixture.conn,
                    &fixture.fixture.run.owner(),
                    deadline()
                )
                .is_err()
        );
        assert_eq!(io.snapshot_bytes(), before);
        io.0.lock().unwrap().fail_truncate = false;
        outbox
            .compact_committed(
                &mut fixture.fixture.conn,
                &fixture.fixture.run.owner(),
                deadline(),
            )
            .unwrap();
        assert!(io.snapshot_bytes().is_empty());
    }

    #[test]
    fn runner_batch2_outbox_expired_deadline_and_stale_epoch_have_zero_content_effect() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        let mut outbox = open(&fixture, io.clone());
        assert!(
            outbox
                .append_synced(
                    &transition(&fixture, 1),
                    Instant::now() - Duration::from_millis(1)
                )
                .is_err()
        );
        assert!(io.snapshot_bytes().is_empty());
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [fixture.fixture.run.run_id],
            )
            .unwrap();
        assert!(
            outbox
                .append_synced(&transition(&fixture, 1), deadline())
                .is_err()
        );
        assert!(io.snapshot_bytes().is_empty());
    }

    #[test]
    fn runner_batch2_outbox_holds_the_original_lock_for_its_entire_content_lifetime() {
        let WriterFixture {
            fixture,
            artifacts: _artifacts,
            lock,
        } = WriterFixture::new();
        let outbox =
            Outbox::open_owned(&fixture.conn, &fixture.run, lock.clone(), deadline()).unwrap();
        drop(lock);
        assert!(
            RunLock::acquire(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .is_none()
        );
        drop(outbox);
        assert!(
            RunLock::acquire_released(&fixture.conn, &fixture.run.owner())
                .unwrap()
                .is_some()
        );
    }
}
