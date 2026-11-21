use crate::broker::runtime as storage;
use crate::broker::runtime::ObservedTransition;
use crate::broker::runtime::PersistenceMode;
use crate::broker::runtime::RunOwner;
use crate::broker::runtime::RunRecord;
use crate::runtime::*;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum WriterBoundary {
    Receive,
    Append,
    Sync,
    Persist,
    Compact,
}

pub(in crate::runtime) const WORK_SLICE: Duration = Duration::from_millis(100);

#[cfg(test)]
pub(crate) trait WriterHooks: Send + Sync {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> Result<()>;
    fn work_slice(&self) -> Duration {
        WORK_SLICE
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WriterStatus {
    pub(crate) pending_count: u64,
    pub(crate) gap_count: u64,
    pub(crate) diagnostic: Option<String>,
}
#[derive(Debug)]
pub(crate) struct RetainedTransition {
    pub(crate) transition: ObservedTransition,
    pub(crate) cause: CafleetError,
}
#[derive(Default)]
struct Mailbox {
    pending: u64,
    gaps: u64,
    diagnostic: Option<String>,
    last_sequence: i64,
    refill: bool,
    grant: Option<std::ops::RangeInclusive<i64>>,
    stop: bool,
    deadline: Option<Instant>,
}
impl Mailbox {
    fn failure(&mut self, cause: &CafleetError) {
        self.gaps = self.gaps.checked_add(1).expect("writer gap count overflow");
        self.diagnostic = Some(cause.message().into_owned());
    }
    fn work_deadline(&self, slice: Duration) -> Instant {
        let local = Instant::now() + slice;
        self.deadline.map_or(local, |until| until.min(local))
    }
    // Compaction validates every retained frame in one attempt and each retry repeats that
    // whole pass, so it gets a budget wider than one work slice; otherwise an outbox whose
    // validation outgrows the slice could never compact.
    fn compaction_deadline(&self) -> Instant {
        let local = Instant::now() + Duration::from_secs(1);
        self.deadline.map_or(local, |until| until.min(local))
    }
    fn success(&mut self) {
        self.diagnostic = None;
    }
}
pub(crate) struct EventWriter {
    sender: mpsc::SyncSender<Box<ObservedTransition>>,
    mailbox: Arc<Mutex<Mailbox>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
struct Pending {
    transition: Box<ObservedTransition>,
    synced: bool,
}
impl EventWriter {
    pub(crate) fn start(database_url: String, run: RunRecord, lock: Arc<RunLock>) -> Result<Self> {
        Self::start_worker(
            database_url,
            run,
            lock,
            #[cfg(test)]
            crate::runtime::writer::test_support::default_writer_hooks(),
        )
    }

    #[cfg(test)]
    pub(crate) fn start_with_hooks(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        Self::start_worker(database_url, run, lock, hooks)
    }

    fn start_worker(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        #[cfg(test)] hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        let mut conn = crate::db::connect(&database_url)?;
        let spool = Outbox::open_owned(
            &mut conn,
            &run,
            lock.clone(),
            Instant::now() + Duration::from_secs(1),
        )?;
        ensure(
            spool.io.byte_len().map_err(io_error)? == 0,
            "new runner outbox must start empty",
        )?;
        #[cfg(test)]
        let mut spool = spool;
        #[cfg(test)]
        spool.set_hooks(hooks.clone());

        let (sender, receiver) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        #[cfg(not(test))]
        let work_slice = WORK_SLICE;
        #[cfg(test)]
        let work_slice = hooks.work_slice();
        let worker = Worker {
            conn,
            spool,
            run,
            lock,
            mailbox: mailbox.clone(),
            receiver,
            queue: VecDeque::new(),
            committed: 0,
            compact: false,
            work_slice,
            #[cfg(test)]
            hooks,
        };
        let thread = std::thread::Builder::new()
            .name("cafleet-events".into())
            .spawn(move || worker.serve())
            .map_err(io_error)?;
        Ok(Self {
            sender,
            mailbox,
            thread: Some(thread),
        })
    }
    fn send(&self, transition: Box<ObservedTransition>) -> Result<()> {
        self.sender
            .try_send(transition)
            .map_err(|_| error("runtime observation channel is full or unavailable"))
    }
    pub(crate) fn try_submit(
        &self,
        transition: ObservedTransition,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            ensure(!state.stop, "event writer is stopped")?;
            ensure(
                transition.event_seq > state.last_sequence,
                "accepted event sequence regressed",
            )?;
            storage::validate_evidence(&transition.evidence_json)?;
            self.send(Box::new(transition.clone()))
        })();
        match result {
            Ok(()) => {
                state.pending = state
                    .pending
                    .checked_add(1)
                    .expect("writer pending overflow");
                assert!(state.pending <= 128, "bounded accepted writer capacity");
                state.last_sequence = transition.event_seq;
                Ok(())
            }
            Err(cause) => {
                state.failure(&cause);
                Err(Box::new(RetainedTransition { transition, cause }))
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn submit(&self, transition: ObservedTransition) -> Result<()> {
        self.try_submit(transition)
            .map_err(|retained| retained.cause)
    }
    #[cfg(test)]
    pub(crate) fn joined(&self) -> bool {
        self.thread.is_none()
    }
    pub(crate) fn request_sequence_range(&self) -> Result<()> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        ensure(!state.stop, "writer stopped before sequence refill")?;
        if state.grant.is_none() {
            state.refill = true;
        }
        Ok(())
    }
    pub(crate) fn take_sequence_range(&self) -> Option<std::ops::RangeInclusive<i64>> {
        self.mailbox.lock().expect("writer mailbox").grant.take()
    }
    pub(crate) fn status(&self) -> WriterStatus {
        let state = self.mailbox.lock().expect("writer mailbox");
        WriterStatus {
            pending_count: state.pending,
            gap_count: state.gaps,
            diagnostic: state.diagnostic.clone(),
        }
    }
    pub(crate) fn diagnostic(&self) -> Option<String> {
        self.status().diagnostic
    }
    pub(crate) fn set_deadline(&mut self, until: Instant) {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.deadline = Some(state.deadline.map_or(until, |old| old.min(until)));
    }
    pub(crate) fn begin_shutdown(&mut self, until: Instant) {
        self.set_deadline(until);
        self.mailbox.lock().expect("writer mailbox").stop = true;
    }
    pub(crate) fn finish(&mut self, until: Instant) -> (u64, u64) {
        self.begin_shutdown(until);
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline
            .expect("shutdown deadline");
        if let Some(thread) = self.thread.as_ref() {
            while !thread.is_finished() && Instant::now() < until {
                nap_until(until);
            }
        }
        if self
            .thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            self.thread
                .take()
                .expect("writer thread")
                .join()
                .expect("writer processing is panic-free");
        }
        let mut state = self.mailbox.lock().expect("writer mailbox");
        if self.thread.is_some() && state.diagnostic.is_none() {
            state.failure(&CafleetError::DeadlineExceeded);
        }
        let unsaved = if state.diagnostic.is_some() {
            state.gaps
        } else {
            0
        };
        (state.pending, unsaved)
    }
}
impl Drop for EventWriter {
    fn drop(&mut self) {
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .deadline
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(4));
        self.finish(until);
    }
}

struct Worker {
    conn: Connection,
    spool: Outbox<File>,
    run: RunRecord,
    lock: Arc<RunLock>,
    mailbox: Arc<Mutex<Mailbox>>,
    receiver: mpsc::Receiver<Box<ObservedTransition>>,
    queue: VecDeque<Pending>,
    committed: i64,
    compact: bool,
    work_slice: Duration,
    #[cfg(test)]
    hooks: Arc<dyn WriterHooks>,
}
impl Worker {
    fn serve(mut self) {
        let mut retry = Duration::from_millis(100);
        let mut next = Instant::now();
        loop {
            {
                let state = self.mailbox.lock().expect("writer mailbox");
                if state.deadline.is_some_and(|end| Instant::now() >= end) {
                    break;
                }
                if state.stop && state.pending == 0 && !self.compact {
                    break;
                }
            }
            if Instant::now() < next {
                nap_until(next);
                continue;
            }
            match self.step() {
                Ok(()) => {
                    retry = Duration::from_millis(100);
                    next = Instant::now();
                }
                Err(cause) => {
                    self.mailbox.lock().expect("writer mailbox").failure(&cause);
                    next = Instant::now() + retry;
                    retry = (retry * 2).min(Duration::from_secs(2));
                }
            }
            if self.mailbox.lock().expect("writer mailbox").pending == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    fn step(&mut self) -> Result<()> {
        if !self.receive()? {
            return Ok(());
        }
        self.sync_pending()?;
        self.persist_front()?;
        self.compact_if_idle()?;
        self.refill_sequences()
    }
    fn work_deadline(&self) -> Instant {
        self.mailbox
            .lock()
            .expect("writer mailbox")
            .work_deadline(self.work_slice)
    }
    /// Returns `false` when a test hook ends this pass before any persistence.
    fn receive(&mut self) -> Result<bool> {
        while self.queue.len() < 64 {
            let until = self.work_deadline();
            #[cfg(test)]
            match self.hooks.before(WriterBoundary::Receive, until) {
                Ok(()) => {}
                Err(CafleetError::DeadlineExceeded) => return Ok(false),
                Err(cause) => return Err(cause),
            }

            check_deadline(Instant::now(), until)?;
            match self.receiver.try_recv() {
                Ok(transition) => self.queue.push_back(Pending {
                    transition,
                    synced: false,
                }),
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Ok(true)
    }
    fn sync_pending(&mut self) -> Result<()> {
        for pending in &mut self.queue {
            if !pending.synced {
                let until = self
                    .mailbox
                    .lock()
                    .expect("writer mailbox")
                    .work_deadline(self.work_slice);
                self.spool
                    .append_synced(&mut self.conn, &pending.transition, until)?;
                pending.synced = true;
            }
        }
        Ok(())
    }
    fn persist_front(&mut self) -> Result<()> {
        let Some(pending) = self.queue.front() else {
            return Ok(());
        };
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .work_deadline(self.work_slice);
        let transition = &pending.transition;
        #[cfg(test)]
        self.hooks.before(WriterBoundary::Persist, until)?;

        with_database_deadline(&mut self.conn, until, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            storage::persist_transition(conn, &self.run.owner(), transition, PersistenceMode::Live)
                .map(|_| ())
        })?;
        self.committed = transition.event_seq;
        self.compact = true;
        self.queue.pop_front();
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.pending = state.pending.checked_sub(1).expect("accepted writer count");
        state.success();
        Ok(())
    }
    fn compact_if_idle(&mut self) -> Result<()> {
        if !self.compact || !self.queue.is_empty() {
            return Ok(());
        }
        let until = self
            .mailbox
            .lock()
            .expect("writer mailbox")
            .compaction_deadline();
        self.spool
            .compact_committed(&mut self.conn, &self.run.owner(), until)?;
        self.compact = false;
        self.mailbox.lock().expect("writer mailbox").success();
        Ok(())
    }
    fn refill_sequences(&mut self) -> Result<()> {
        let refill = {
            let state = self.mailbox.lock().expect("writer mailbox");
            state.refill && state.grant.is_none()
        };
        if !refill {
            return Ok(());
        }
        let until = self.work_deadline();
        let grant = with_database_deadline(&mut self.conn, until, |conn| {
            self.lock.validate(conn, &self.run.owner())?;
            storage::reserve_event_sequences(conn, &self.run.owner())
        })?;
        let mut state = self.mailbox.lock().expect("writer mailbox");
        state.grant = Some(grant);
        state.refill = false;
        state.success();
        Ok(())
    }
}

const OUTBOX_LIMIT: u64 = 16 * 1024 * 1024;
/// The big-endian body length and the body's SHA-256 digest.
const FRAME_HEADER: u64 = 4 + 32;
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
    pub(crate) transition: ObservedTransition,
    pub(crate) start: u64,
    pub(crate) end: u64,
}
fn parse_frame_body(body: &[u8]) -> Result<ObservedTransition> {
    let value = serde_json::from_slice::<Value>(body).map_err(io_error)?;
    let transition: ObservedTransition = serde_json::from_value(value.clone()).map_err(io_error)?;
    ensure(
        value == serde_json::to_value(&transition).map_err(io_error)?,
        "unknown outbox transition fields",
    )?;
    Ok(transition)
}
pub(crate) struct OutboxScan {
    pub(crate) frames: Vec<OutboxFrame>,
    pub(crate) incomplete_tail: Option<String>,
}
pub(crate) struct Outbox<S: OutboxIo> {
    io: S,
    run: RunRecord,
    lock: Arc<RunLock>,
    known: BTreeMap<i64, [u8; 32]>,
    end: u64,
    dirty: bool,
    path_identity: Option<OutboxIdentity>,
    #[cfg(test)]
    hooks: Arc<dyn WriterHooks>,
}
/// The validated outbox path and the inode it named when opened.
struct OutboxIdentity {
    path: PathBuf,
    dev: u64,
    ino: u64,
}
fn open_outbox(
    conn: &Connection,
    run: &RunRecord,
    lock: &RunLock,
    until: Instant,
) -> Result<(File, OutboxIdentity)> {
    check_deadline(Instant::now(), until)?;
    lock.validate(conn, &run.owner())?;
    let paths = validate_run_paths(conn, run)?;
    let file = crate::runtime::artifacts::open_private(&paths.outbox, false)?;
    let meta = file.metadata().map_err(io_error)?;
    Ok((
        file,
        OutboxIdentity {
            path: paths.outbox,
            dev: meta.dev(),
            ino: meta.ino(),
        },
    ))
}
fn validate_with(
    conn: &mut Connection,
    lock: &RunLock,
    current: &RunRecord,
    path_identity: Option<&OutboxIdentity>,
    until: Instant,
) -> Result<RunRecord> {
    let run = with_database_deadline(conn, until, |conn| {
        lock.validate(conn, &current.owner())?;
        storage::get_run(conn, current.run_id)
    })?;
    if let Some(identity) = path_identity {
        let meta = std::fs::symlink_metadata(&identity.path).map_err(io_error)?;
        ensure(
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.uid() == nix::unistd::geteuid().as_raw()
                && meta.mode() & 0o777 == 0o600
                && meta.dev() == identity.dev
                && meta.ino() == identity.ino,
            "owned outbox inode changed",
        )?;
    }
    Ok(run)
}
impl Outbox<File> {
    pub(crate) fn open_recovery(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<(Self, OutboxScan)> {
        let (file, identity) = open_outbox(conn, run, &lock, until)?;
        let mut value = Self::state(run, lock, file);
        value.path_identity = Some(identity);
        let scan = value.scan(conn, until)?;
        Ok((value, scan))
    }
    pub(crate) fn open_owned(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<Self> {
        let (file, identity) = open_outbox(conn, run, &lock, until)?;
        let mut value = Self::initialize(conn, run, lock, file, until)?;
        value.path_identity = Some(identity);
        Ok(value)
    }
}
impl<S: OutboxIo> Outbox<S> {
    fn state(run: &RunRecord, lock: Arc<RunLock>, io: S) -> Self {
        Self {
            io,
            run: run.clone(),
            lock,
            known: BTreeMap::new(),
            end: 0,
            dirty: true,
            path_identity: None,
            #[cfg(test)]
            hooks: crate::runtime::writer::test_support::default_writer_hooks(),
        }
    }
    fn initialize(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        io: S,
        until: Instant,
    ) -> Result<Self> {
        let mut value = Self::state(run, lock, io);
        value.scan(conn, until)?;
        Ok(value)
    }
    #[cfg(test)]
    pub(super) fn set_hooks(&mut self, hooks: Arc<dyn WriterHooks>) {
        self.hooks = hooks;
    }

    fn validate(&self, conn: &mut Connection, until: Instant) -> Result<RunRecord> {
        check_deadline(Instant::now(), until)?;
        validate_with(
            conn,
            &self.lock,
            &self.run,
            self.path_identity.as_ref(),
            until,
        )
    }
    pub(crate) fn scan(&mut self, conn: &mut Connection, until: Instant) -> Result<OutboxScan> {
        let run = self.validate(conn, until)?;
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
                bytes > 0 && bytes <= OUTBOX_LIMIT - FRAME_HEADER,
                "invalid outbox frame length",
            )?;
            if length - position < FRAME_HEADER + bytes {
                tail = Some("incomplete outbox frame".into());
                break;
            }
            let mut checksum = [0u8; 32];
            self.io.read_exact(&mut checksum).map_err(io_error)?;
            let mut body = vec![0; usize::try_from(bytes).map_err(io_error)?];
            self.io.read_exact(&mut body).map_err(io_error)?;
            let digest: [u8; 32] = Sha256::digest(&body).into();
            ensure(digest == checksum, "outbox frame checksum mismatch")?;
            let transition = parse_frame_body(&body)?;
            storage::validate_transition(&run, &transition)?;
            ensure(
                transition.event_seq >= previous,
                "outbox sequences regressed",
            )?;
            if let Some(old) = known.insert(transition.event_seq, digest) {
                ensure(old == digest, "conflicting outbox original sequence")?;
            }
            previous = transition.event_seq;
            let end = position + FRAME_HEADER + bytes;
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
            frames,
            incomplete_tail: tail,
        })
    }
    pub(crate) fn append_synced(
        &mut self,
        conn: &mut Connection,
        value: &ObservedTransition,
        until: Instant,
    ) -> Result<()> {
        let run = self.validate(conn, until)?;
        storage::validate_transition(&run, value)?;
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
                .checked_add(body.len() as u64 + FRAME_HEADER)
                .ok_or_else(|| error("outbox size overflow"))?;
            ensure(
                end <= OUTBOX_LIMIT,
                "runtime outbox reached 16 MiB pressure limit",
            )?;
            #[cfg(test)]
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
        #[cfg(test)]
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
        let run = self.validate(conn, until)?;
        ensure(*owner == self.run.owner(), "compaction owner mismatch")?;
        let length = self.io.byte_len().map_err(io_error)?;
        let scan = self.scan_validated(run, until)?;
        let all = if !scan.frames.is_empty() {
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
                .optional()?;
                    if let Some(id) = id {
                        let view = storage::get_event(conn, id)?;
                        ensure(
                            storage::original(&view)? == frame.transition,
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
            #[cfg(test)]
            self.hooks.before(WriterBoundary::Compact, until)?;

            self.validate(conn, until)?;
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
    use crate::runtime::writer::test_support::WriterFixture;
    use crate::runtime::writer::test_support::deadline;
    use crate::runtime::writer::test_support::transition;
    use crate::runtime::writer::*;
    use std::time::Duration;

    mod supplied_connection_compaction {}

    mod terminal_tail {}

    use crate::runtime::writer::test_support::*;

    mod refresh {}

    mod worker {
        use crate::runtime::writer::tests::*;
        #[test]
        fn writer_worker_receive_stall_pins_64_channel_slots_and_returns_the_failed_original() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            for sequence in 1..=64 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            let original = transition(&fixture, 65);
            let begin = Instant::now();
            let rejected = writer
                .try_submit(original.clone())
                .expect_err("channel pressure returns the original");
            assert!(begin.elapsed() < Duration::from_millis(250));
            assert_eq!(rejected.transition, original);
            let status = writer.status();
            assert_eq!(status.pending_count, 64);
            assert!(status.diagnostic.is_some());
            assert!(status.gap_count > 0);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), 64);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_worker_append_and_database_failures_retain_the_same_transition_for_retry() {
            for boundary in [WriterBoundary::Append, WriterBoundary::Persist] {
                let fixture = WriterFixture::new();
                let hooks = Arc::new(Hooks::default());
                hooks.fail_once(boundary);
                let mut writer = start(&fixture, hooks.clone());
                let _release = Release(hooks);
                let original = transition(&fixture, 1);
                assert!(writer.try_submit(original.clone()).is_ok());
                wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
                assert_eq!(
                    storage::get_event(&fixture.fixture.conn, 1)
                        .unwrap()
                        .event
                        .payload_json["evidence"],
                    original.evidence_json
                );
                assert_eq!(committed(&fixture), 1);
                assert_eq!(writer.finish(deadline()).0, 0);
            }
        }

        #[test]
        fn writer_commits_each_submitted_transition_directly_with_its_event_sequence() {
            let fixture = WriterFixture::new();
            let mut writer = start(&fixture, Arc::new(Hooks::default()));
            for sequence in 1..=3 {
                assert!(writer.try_submit(transition(&fixture, sequence)).is_ok());
            }
            wait_until(|| committed(&fixture) == 3 && writer.status().pending_count == 0);
            let sequences: Vec<i64> = fixture
                .fixture
                .conn
                .prepare(
                    "SELECT event_seq FROM member_runtime_events WHERE run_id=?1 ORDER BY event_id",
                )
                .unwrap()
                .query_map([fixture.fixture.run.run_id], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(sequences, [1, 2, 3]);
            assert!(writer.status().diagnostic.is_none());
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_retry_exhaustion_reports_a_degraded_status_and_keeps_the_transition() {
            let fixture = WriterFixture::new();
            fixture
                .fixture
                .conn
                .execute_batch(
                    "CREATE TRIGGER injected_event_failure BEFORE INSERT ON member_runtime_events BEGIN SELECT RAISE(ABORT,'injected event failure'); END",
                )
                .unwrap();
            let writer = start(&fixture, Arc::new(Hooks::default()));
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| writer.status().diagnostic.is_some());
            let status = writer.status();
            assert_eq!(status.pending_count, 1, "{status:?}");
            assert_eq!(committed(&fixture), 0);
        }

        #[test]
        fn writer_worker_pending_capacity_counts_channel_and_worker_queue_without_growth() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let mut accepted = 1;
            for sequence in 2..=256 {
                match writer.try_submit(transition(&fixture, sequence)) {
                    Ok(()) => accepted += 1,
                    Err(retained) => {
                        assert_eq!(retained.transition.event_seq, sequence);
                        break;
                    }
                }
            }
            assert!((64..=128).contains(&accepted));
            assert_eq!(writer.status().pending_count, accepted);
            assert!(writer.status().diagnostic.is_some());
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
            assert_eq!(writer.finish(deadline()).0, 0);
        }
    }

    fn assert_start_rejected(fixture: &WriterFixture) {
        let result = fixture.start();
        let rejected = result.is_err();
        if let Ok(mut writer) = result {
            writer.finish(Instant::now() + Duration::from_millis(100));
        }
        assert!(
            rejected,
            "writer startup validates the recorded owned outbox before accepting work"
        );
    }

    #[test]
    fn writer_stale_owner_is_rejected_before_worker_start() {
        let mut fixture = WriterFixture::new();
        fixture.fixture.run.owner_epoch += 1;
        assert_start_rejected(&fixture);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::error::Result;
    use crate::runtime::writer::{WriterBoundary, WriterHooks};
    use std::sync::Arc;
    use std::time::Instant;
    struct DefaultWriterHooks;
    impl WriterHooks for DefaultWriterHooks {
        fn before(&self, _: WriterBoundary, _: Instant) -> Result<()> {
            Ok(())
        }
    }
    pub(in crate::runtime) fn default_writer_hooks() -> Arc<dyn WriterHooks> {
        Arc::new(DefaultWriterHooks)
    }

    use crate::broker::runtime::*;
    use crate::runtime::test_support::Fixture;
    use crate::runtime::writer::*;
    use std::sync::Condvar;
    pub(crate) fn index(boundary: WriterBoundary) -> usize {
        match boundary {
            WriterBoundary::Receive => 0,
            WriterBoundary::Append => 1,
            WriterBoundary::Sync => 2,
            WriterBoundary::Persist => 3,
            WriterBoundary::Compact => 4,
        }
    }
    #[derive(Debug, Default)]
    pub(crate) struct HookState {
        pub(crate) paused: [bool; 5],
        pub(crate) failures: [u64; 5],
        pub(crate) calls: [u64; 5],
        pub(crate) first: [Option<u64>; 5],
        pub(crate) ordinal: u64,
    }
    #[derive(Default)]
    pub(crate) struct Hooks {
        pub(crate) state: Mutex<HookState>,
        pub(crate) changed: Condvar,
    }
    impl Hooks {
        pub(crate) fn pause(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().paused[index(boundary)] = true;
        }
        pub(crate) fn release_for_cleanup(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.paused = [false; 5];
            drop(state);
            self.changed.notify_all();
        }
        pub(crate) fn fail_once(&self, boundary: WriterBoundary) {
            self.state.lock().unwrap().failures[index(boundary)] += 1;
        }
        pub(crate) fn wait(&self, boundary: WriterBoundary, count: u64) {
            let slot = index(boundary);
            let until = deadline();
            let mut state = self.state.lock().unwrap();
            while state.calls[slot] < count {
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    let calls = state.calls;
                    drop(state);
                    panic!("worker boundary {slot} was not reached; calls={calls:?}");
                }
                state = self.changed.wait_timeout(state, remaining).unwrap().0;
            }
        }
    }
    impl WriterHooks for Hooks {
        fn before(&self, boundary: WriterBoundary, until: Instant) -> Result<()> {
            let slot = index(boundary);
            let mut state = self.state.lock().unwrap();
            state.ordinal = state.ordinal.checked_add(1).unwrap();
            state.calls[slot] = state.calls[slot].checked_add(1).unwrap();
            let ordinal = state.ordinal;
            state.first[slot].get_or_insert(ordinal);
            self.changed.notify_all();
            while state.paused[slot] {
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(CafleetError::DeadlineExceeded);
                }
                state = self.changed.wait_timeout(state, remaining).unwrap().0;
            }
            if state.failures[slot] > 0 {
                state.failures[slot] -= 1;
                return Err(error("fixture writer boundary failure"));
            }
            Ok(())
        }
    }
    pub(crate) struct Release(pub(crate) Arc<Hooks>);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.release_for_cleanup();
        }
    }

    pub(crate) fn start(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
        EventWriter::start_with_hooks(
            format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
            fixture.fixture.run.clone(),
            fixture.lock.clone(),
            hooks,
        )
        .unwrap()
    }
    pub(crate) fn wait_until(condition: impl FnMut() -> bool) {
        wait_until_within(Duration::from_secs(2), condition);
    }
    pub(crate) fn wait_until_within(budget: Duration, mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + budget;
        while !condition() {
            assert!(
                Instant::now() < until,
                "bounded worker condition did not complete"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub(crate) fn committed(fixture: &WriterFixture) -> i64 {
        fixture
            .fixture
            .conn
            .query_row(
                "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                [fixture.fixture.run.run_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub(crate) struct WriterFixture {
        pub(crate) fixture: Fixture,
        pub(crate) artifacts: RunArtifacts,
        pub(crate) lock: Arc<RunLock>,
    }
    impl WriterFixture {
        pub(crate) fn new() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let lock = Arc::new(
                RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap(),
            );
            lock.transfer_to_runner(
                &mut fixture.conn,
                &fixture.run.owner(),
                ProcessIdentity::current().unwrap(),
            )
            .unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            storage::reserve_event_sequences(&mut fixture.conn, &fixture.run.owner()).unwrap();
            fixture.run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
            Self {
                fixture,
                artifacts,
                lock,
            }
        }
        pub(crate) fn start(&self) -> Result<EventWriter> {
            EventWriter::start(
                format!("sqlite:///{}", self.fixture.conn.path().unwrap()),
                self.fixture.run.clone(),
                self.lock.clone(),
            )
        }
    }

    pub(crate) fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    pub(crate) fn transition(fixture: &WriterFixture, sequence: i64) -> ObservedTransition {
        run_transition(&fixture.fixture.run, sequence)
    }

    /// The coverage-unknown transition `sequence` of `run`, usable off the fixture's thread.
    pub(crate) fn run_transition(run: &storage::RunRecord, sequence: i64) -> ObservedTransition {
        let evidence = crate::broker::runtime::run_evidence(
            run,
            "runner-lifecycle",
            u64::try_from(sequence).unwrap(),
        )
        .unwrap();
        ObservedTransition {
            run_id: run.run_id,
            event_seq: sequence,
            episode_seq: None,
            kind: RuntimeEventKind::CoverageUnknown,
            classification: None,
            observed_at: "2026-10-26T16:40:00.000000+00:00".into(),
            evidence_json: evidence,
            close_reason: None,
            cause_json: Some(serde_json::json!({"kind":"coverage_unknown"})),
        }
    }

    /// An outbox frame (length, SHA-256 of the body, body) built without the production encoder.
    pub(crate) fn outbox_frame(body: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(&Sha256::digest(body));
        bytes.extend_from_slice(body);
        bytes
    }

    pub(crate) fn frame(transition: &ObservedTransition) -> Vec<u8> {
        outbox_frame(&serde_json::to_vec(transition).unwrap())
    }
}
