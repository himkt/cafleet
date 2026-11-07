use crate::broker::runtime as storage;
use crate::broker::runtime::EffectivePolicyObservation;
use crate::broker::runtime::InteractionEvidenceUpdate;
use crate::broker::runtime::ObservedTransition;
use crate::broker::runtime::PersistenceMode;
use crate::broker::runtime::RunOwner;
use crate::broker::runtime::RunRecord;
use crate::broker::runtime::RuntimeEventKind;
use crate::runtime::*;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

#[cfg(test)]
use std::sync::atomic::AtomicU64;
#[cfg(test)]
use std::sync::atomic::Ordering;

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum WriterBoundary {
    Receive,
    Append,
    Sync,
    Persist,
    Compact,
}

#[cfg(test)]
pub(crate) trait WriterHooks: Send + Sync {
    fn before(&self, boundary: WriterBoundary, deadline: Instant) -> Result<()>;
}

#[derive(Debug, Clone)]
pub(crate) struct WriterStatus {
    pub(crate) pending_count: u64,
    pub(crate) pending_evidence: bool,
    pub(crate) gap_count: u64,
    pub(crate) degraded: bool,
    pub(crate) diagnostic: Option<String>,
    #[cfg(test)]
    pub(crate) policy_diagnostic: Option<String>,
}
#[derive(Debug)]
pub(crate) struct RetainedTransition {
    pub(crate) transition: ObservedTransition,
    pub(crate) cause: CafleetError,
}
#[derive(Debug)]
pub(crate) struct RetainedEvidenceUpdate {
    pub(crate) update: InteractionEvidenceUpdate,
    pub(crate) cause: CafleetError,
}
#[derive(Clone)]
struct Refresh {
    update: InteractionEvidenceUpdate,
    barrier: i64,
    revision: u64,
}
struct Mailbox {
    pending: u64,
    policies: u64,
    #[cfg(test)]
    policy_diagnostic: Option<String>,
    gaps: u64,
    diagnostic: Option<String>,
    last_sequence: i64,
    active: Option<i64>,
    refresh: Option<Refresh>,
    revision: u64,
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
    fn work_deadline(&self) -> Instant {
        let local = Instant::now() + Duration::from_millis(100);
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
    fn policy_dropped(&mut self, cause: &CafleetError) {
        eprintln!("member runtime: effective policy update dropped: {cause}");
        #[cfg(test)]
        {
            self.policy_diagnostic = Some(cause.message().into_owned());
        }
    }
}
pub(crate) struct EventWriter {
    sender: Option<mpsc::SyncSender<Work>>,
    mailbox: Arc<Mutex<Mailbox>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
enum Work {
    Event {
        transition: Box<ObservedTransition>,
        effective_policy: Option<EffectivePolicyObservation>,
    },
    Policy(EffectivePolicyObservation),
}
struct Pending {
    work: Work,
    synced: bool,
}
impl Pending {
    fn event_seq(&self) -> Option<i64> {
        match &self.work {
            Work::Event { transition, .. } => Some(transition.event_seq),
            Work::Policy(_) => None,
        }
    }
}
impl EventWriter {
    pub(crate) fn start(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
    ) -> Result<Self> {
        Self::start_worker(
            database_url,
            run,
            lock,
            outbox,
            #[cfg(test)]
            crate::runtime::writer::test_support::default_writer_hooks(),
        )
    }

    #[cfg(test)]
    pub(crate) fn start_with_hooks(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
        hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        Self::start_worker(database_url, run, lock, outbox, hooks)
    }

    fn start_worker(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
        #[cfg(test)] hooks: Arc<dyn WriterHooks>,
    ) -> Result<Self> {
        let mut conn = crate::db::connect(&database_url)?;
        let paths = validate_run_paths(&conn, &run)?;
        ensure(
            outbox == paths.outbox,
            "writer outbox path differs from recorded path",
        )?;
        let mut spool = Outbox::open_owned(
            &conn,
            &run,
            lock.clone(),
            Instant::now() + Duration::from_secs(1),
        )?;
        ensure(
            std::fs::metadata(&outbox).map_err(io_error)?.len() == 0,
            "new runner outbox must start empty",
        )?;
        #[cfg(test)]
        spool.set_hooks(hooks.clone());

        let (sender, receiver) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox {
            pending: 0,
            policies: 0,
            #[cfg(test)]
            policy_diagnostic: None,
            gaps: 0,
            diagnostic: None,
            last_sequence: 0,
            active: None,
            refresh: None,
            revision: 0,
            refill: false,
            grant: None,
            stop: false,
            deadline: None,
        }));
        let observed = mailbox.clone();
        let thread = std::thread::Builder::new()
            .name("cafleet-events".into())
            .spawn(move || {
                let mut queue = VecDeque::<Pending>::new();
                let mut committed = 0;
                let mut compact = false;
                let mut retry = Duration::from_millis(100);
                let mut next = Instant::now();
                loop {
                    {
                        let state = observed.lock().expect("writer mailbox");
                        if state.deadline.is_some_and(|end| Instant::now() >= end) {
                            break;
                        }
                        if state.stop
                            && state.pending == 0
                            && state.policies == 0
                            && state.refresh.is_none()
                            && !compact
                        {
                            break;
                        }
                    }
                    if Instant::now() < next {
                        nap_until(next);
                        continue;
                    }
                    let result = (|| -> Result<()> {
                        while queue.len() < 64 {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            #[cfg(test)]
                            match hooks.before(WriterBoundary::Receive, until) {
                                Ok(()) => {}
                                Err(CafleetError::DeadlineExceeded) => return Ok(()),
                                Err(cause) => return Err(cause),
                            }

                            check_deadline(Instant::now(), until)?;
                            match receiver.try_recv() {
                                Ok(work) => queue.push_back(Pending {
                                    synced: matches!(work, Work::Policy(_)),
                                    work,
                                }),
                                Err(
                                    mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected,
                                ) => break,
                            }
                        }
                        for pending in &mut queue {
                            if !pending.synced {
                                let Work::Event {
                                    transition,
                                    effective_policy,
                                } = &pending.work
                                else {
                                    unreachable!("effective policy updates are never spooled")
                                };
                                let until =
                                    observed.lock().expect("writer mailbox").work_deadline();
                                spool.append_synced_with_policy(
                                    transition,
                                    effective_policy.as_ref(),
                                    until,
                                )?;
                                pending.synced = true;
                            }
                        }
                        let refresh = {
                            let state = observed.lock().expect("writer mailbox");
                            state
                                .refresh
                                .as_ref()
                                .filter(|value| {
                                    value.barrier <= committed
                                        && queue.front().is_none_or(|pending| {
                                            pending
                                                .event_seq()
                                                .is_none_or(|sequence| sequence > value.barrier)
                                        })
                                })
                                .cloned()
                        };
                        if let Some(refresh) = refresh {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            #[cfg(test)]
                            hooks.before(WriterBoundary::Receive, until)?;

                            #[cfg(test)]
                            hooks.before(WriterBoundary::Persist, until)?;

                            storage::update_interaction_evidence(
                                &mut conn,
                                &run.owner(),
                                &refresh.update,
                                refresh.barrier,
                                until,
                            )?;
                            let mut state = observed.lock().expect("writer mailbox");
                            if state
                                .refresh
                                .as_ref()
                                .is_some_and(|current| current.revision == refresh.revision)
                            {
                                state.refresh = None;
                            }
                            state.success();
                        } else if let Some(pending) = queue.front() {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            match &pending.work {
                                Work::Event {
                                    transition,
                                    effective_policy,
                                } => {
                                    #[cfg(test)]
                                    hooks.before(WriterBoundary::Persist, until)?;

                                    with_database_deadline(&mut conn, until, |conn| {
                                        lock.validate(conn, &run.owner())?;
                                        match effective_policy {
                                            Some(observation) => {
                                                storage::persist_transition_with_effective_policy(
                                                    conn,
                                                    &run.owner(),
                                                    transition,
                                                    observation,
                                                    PersistenceMode::Live,
                                                )
                                            }
                                            None => storage::persist_transition(
                                                conn,
                                                &run.owner(),
                                                transition,
                                                PersistenceMode::Live,
                                            ),
                                        }
                                        .map(|_| ())
                                    })?;
                                    committed = transition.event_seq;
                                    compact = true;
                                    queue.pop_front();
                                    let mut state = observed.lock().expect("writer mailbox");
                                    state.pending = state
                                        .pending
                                        .checked_sub(1)
                                        .expect("accepted writer count");
                                    state.success();
                                }
                                Work::Policy(observation) => {
                                    let outcome = (|| -> Result<bool> {
                                        #[cfg(test)]
                                        hooks.before(WriterBoundary::Persist, until)?;

                                        lock.validate(&conn, &run.owner())?;
                                        storage::record_effective_policy(
                                            &mut conn,
                                            &run.owner(),
                                            observation,
                                            until,
                                        )
                                    })();
                                    queue.pop_front();
                                    let mut state = observed.lock().expect("writer mailbox");
                                    state.policies = state
                                        .policies
                                        .checked_sub(1)
                                        .expect("accepted policy update count");
                                    if let Err(cause) = outcome {
                                        state.policy_dropped(&cause);
                                    }
                                }
                            }
                        }
                        if compact && queue.is_empty() {
                            let until = observed
                                .lock()
                                .expect("writer mailbox")
                                .compaction_deadline();
                            spool.compact_committed(&mut conn, &run.owner(), until)?;
                            compact = false;
                            observed.lock().expect("writer mailbox").success();
                        }
                        let refill = {
                            let state = observed.lock().expect("writer mailbox");
                            state.refill && state.grant.is_none()
                        };
                        if refill {
                            let until = observed.lock().expect("writer mailbox").work_deadline();
                            let grant = with_database_deadline(&mut conn, until, |conn| {
                                lock.validate(conn, &run.owner())?;
                                storage::reserve_event_sequences(conn, &run.owner())
                            })?;
                            let mut state = observed.lock().expect("writer mailbox");
                            state.grant = Some(grant);
                            state.refill = false;
                            state.success();
                        }
                        Ok(())
                    })();
                    match result {
                        Ok(()) => {
                            retry = Duration::from_millis(100);
                            next = Instant::now();
                        }
                        Err(cause) => {
                            observed.lock().expect("writer mailbox").failure(&cause);
                            next = Instant::now() + retry;
                            retry = (retry * 2).min(Duration::from_secs(2));
                        }
                    }
                    if observed.lock().expect("writer mailbox").pending == 0 {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            })
            .map_err(io_error)?;
        Ok(Self {
            sender: Some(sender),
            mailbox,
            thread: Some(thread),
        })
    }
    #[cfg(test)]
    pub(crate) fn try_submit(
        &self,
        transition: ObservedTransition,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        self.try_submit_with_policy(transition, None)
    }
    pub(crate) fn try_submit_with_policy(
        &self,
        transition: ObservedTransition,
        effective_policy: Option<EffectivePolicyObservation>,
    ) -> std::result::Result<(), Box<RetainedTransition>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            ensure(!state.stop, "event writer is stopped")?;
            ensure(
                transition.event_seq > state.last_sequence,
                "accepted event sequence regressed",
            )?;
            ensure(
                effective_policy.is_none()
                    || transition.kind == RuntimeEventKind::AutomaticReviewUnavailable,
                "only an unavailable review event carries an effective policy",
            )?;
            crate::broker::runtime::evidence::validate(&transition.evidence_json)?;
            let sender = self
                .sender
                .as_ref()
                .ok_or_else(|| error("event writer is stopped"))?;
            match sender.try_send(Work::Event {
                transition: Box::new(transition.clone()),
                effective_policy,
            }) {
                Ok(()) => Ok(()),
                Err(_) => Err(error("runtime observation channel is full or unavailable")),
            }
        })();
        match result {
            Ok(()) => {
                state.pending = state
                    .pending
                    .checked_add(1)
                    .expect("writer pending overflow");
                assert!(state.pending <= 128, "bounded accepted writer capacity");
                state.last_sequence = transition.event_seq;
                match transition.kind {
                    RuntimeEventKind::InteractionOpened => state.active = transition.episode_seq,
                    RuntimeEventKind::InteractionClosed | RuntimeEventKind::RunnerExited => {
                        state.active = None
                    }
                    _ => {}
                }
                Ok(())
            }
            Err(cause) => {
                state.failure(&cause);
                Err(Box::new(RetainedTransition { transition, cause }))
            }
        }
    }
    pub(crate) fn try_record_policy(&self, observation: EffectivePolicyObservation) {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let sent = if state.stop {
            Err(error("event writer is stopped"))
        } else {
            self.sender
                .as_ref()
                .ok_or_else(|| error("event writer is stopped"))
                .and_then(|sender| {
                    sender
                        .try_send(Work::Policy(observation))
                        .map_err(|_| error("runtime observation channel is full or unavailable"))
                })
        };
        match sent {
            Ok(()) => {
                state.policies = state
                    .policies
                    .checked_add(1)
                    .expect("policy update count overflow");
            }
            Err(cause) => state.policy_dropped(&cause),
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
    pub(crate) fn try_refresh(
        &self,
        update: InteractionEvidenceUpdate,
        until: Instant,
    ) -> std::result::Result<(), Box<RetainedEvidenceUpdate>> {
        let mut state = self.mailbox.lock().expect("writer mailbox");
        let result = (|| -> Result<()> {
            check_deadline(Instant::now(), until)?;
            ensure(
                !state.stop && state.active == Some(update.episode_seq) && state.last_sequence > 0,
                "refresh requires current accepted open episode",
            )?;
            crate::broker::runtime::evidence::validate(&update.evidence_json)?;
            ensure(
                update.output_seq
                    == update.evidence_json["screen_seq"]
                        .as_u64()
                        .ok_or_else(|| error("refresh sequence required"))?,
                "refresh sequence mismatch",
            )?;
            if let Some(old) = &state.refresh {
                ensure(
                    old.update.run_id == update.run_id
                        && old.update.episode_seq == update.episode_seq,
                    "refresh episode collision",
                )?;
                ensure(
                    update.output_seq > old.update.output_seq || (update == old.update),
                    "refresh revision regressed or conflicts",
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .expect("refresh revision overflow");
                state.refresh = Some(Refresh {
                    update,
                    barrier: state.last_sequence,
                    revision: state.revision,
                });
                Ok(())
            }
            Err(cause) => {
                state.failure(&cause);
                Err(Box::new(RetainedEvidenceUpdate { update, cause }))
            }
        }
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
            pending_evidence: state.refresh.is_some(),
            gap_count: state.gaps,
            degraded: state.diagnostic.is_some(),
            diagnostic: state.diagnostic.clone(),
            #[cfg(test)]
            policy_diagnostic: state.policy_diagnostic.clone(),
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
        self.sender.take();
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
        (
            state.pending,
            unsaved
                .checked_add(u64::from(state.refresh.is_some()))
                .expect("unsaved writer observation count overflow"),
        )
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
    pub(crate) transition: ObservedTransition,
    pub(crate) effective_policy: Option<EffectivePolicyObservation>,
    pub(crate) start: u64,
    pub(crate) end: u64,
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
    pub(crate) frames: Vec<OutboxFrame>,
    pub(crate) incomplete_tail: Option<String>,
    pub(crate) maximum_event_seq: Option<i64>,
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
    #[cfg(test)]
    hooks: Arc<dyn WriterHooks>,
}
fn open_private(
    conn: &Connection,
    run: &RunRecord,
    lock: &RunLock,
    until: Instant,
) -> Result<(File, (u64, u64))> {
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
    Ok((file, (meta.dev(), meta.ino())))
}
fn validate_with(
    conn: &mut Connection,
    lock: &RunLock,
    current: &RunRecord,
    path_identity: Option<(u64, u64)>,
    until: Instant,
) -> Result<RunRecord> {
    let run = with_database_deadline(conn, until, |conn| {
        lock.validate(conn, &current.owner())?;
        storage::get_run(conn, current.run_id)
    })?;
    if let Some((dev, ino)) = path_identity {
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
    Ok(run)
}
impl Outbox<File> {
    pub(crate) fn open_recovery(
        conn: &mut Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<Self> {
        let (file, identity) = open_private(conn, run, &lock, until)?;
        let mut value = Self::state(run, lock, file, None);
        value.path_identity = Some(identity);
        value.scan_recovery(conn, until)?;
        Ok(value)
    }
    pub(crate) fn open_owned(
        conn: &Connection,
        run: &RunRecord,
        lock: Arc<RunLock>,
        until: Instant,
    ) -> Result<Self> {
        let (file, identity) = open_private(conn, run, &lock, until)?;
        let mut value = Self::initialize(conn, run, lock, file, until)?;
        value.path_identity = Some(identity);
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
            #[cfg(test)]
            hooks: crate::runtime::writer::test_support::default_writer_hooks(),
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
    pub(super) fn set_hooks(&mut self, hooks: Arc<dyn WriterHooks>) {
        self.hooks = hooks;
    }

    fn validate(&mut self, until: Instant) -> Result<RunRecord> {
        check_deadline(Instant::now(), until)?;
        let conn = self.conn.as_mut().expect("worker validation connection");
        validate_with(conn, &self.lock, &self.run, self.path_identity, until)
    }
    fn validate_on(&self, conn: &mut Connection, until: Instant) -> Result<RunRecord> {
        validate_with(conn, &self.lock, &self.run, self.path_identity, until)
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
                .optional()?;
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
            #[cfg(test)]
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
    use crate::broker::runtime::*;
    use crate::runtime::writer::test_support::WriterFixture;
    use crate::runtime::writer::test_support::deadline;
    use crate::runtime::writer::test_support::frame;
    use crate::runtime::writer::test_support::independent_frames;
    use crate::runtime::writer::test_support::transition;
    use crate::runtime::writer::*;
    use std::io;
    use std::io::Cursor;
    use std::io::Read;
    use std::io::Seek;
    use std::io::SeekFrom;
    use std::io::Write;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    mod supplied_connection_compaction {
        use crate::runtime::writer::tests::*;

        fn ordinary_connection_work(conn: &rusqlite::Connection, timeout: i64) {
            assert_eq!(
                conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                timeout
            );
            assert_eq!(
                crate::runtime::lifecycle::test_support::sum(
                    conn,
                    crate::runtime::lifecycle::test_support::FAST_SUM,
                )
                .unwrap(),
                50005000
            );
        }

        #[test]
        fn compaction_cancels_supplied_connection_sql_preserves_bytes_and_retries() {
            let mut fixture = WriterFixture::new();
            let original = transition(&fixture, 1);
            storage::persist_event(
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
            fixture
                .fixture
                .conn
                .execute_batch(&format!(
                    "CREATE TEMP VIEW member_runtime_events AS SELECT original.* FROM main.member_runtime_events original WHERE ({})>0",
                    crate::runtime::lifecycle::test_support::SLOW_SUM
                ))
                .unwrap();
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
        fn compaction_restores_supplied_connection_after_success_error_and_unwind() {
            for outcome in ["success", "error", "unwind"] {
                let mut fixture = WriterFixture::new();
                let original = transition(&fixture, 1);
                storage::persist_event(
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
        use crate::runtime::writer::tests::*;

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
                storage::persist_event(
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
        Outbox::initialize(
            &fixture.fixture.conn,
            &fixture.fixture.run,
            fixture.lock.clone(),
            io,
            deadline(),
        )
        .unwrap()
    }

    mod effective_policy_frames {
        use crate::runtime::writer::test_support::frame_bodies;
        use crate::runtime::writer::test_support::observation;
        use crate::runtime::writer::test_support::outbox_frame;
        use crate::runtime::writer::test_support::unavailable;
        use crate::runtime::writer::tests::*;

        fn framed(body: &Value) -> Vec<u8> {
            outbox_frame(&serde_json::to_vec(body).unwrap())
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
                Outbox::initialize(
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
                Outbox::initialize(
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
    fn outbox_frames_pin_big_endian_length_checksum_and_original_identity() {
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
    fn outbox_matching_duplicates_reconcile_but_conflicting_sequence_bytes_fail() {
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
        let result = Outbox::initialize(
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
    fn outbox_total_capacity_includes_frame_overhead_and_rejects_before_append() {
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
        let policy=storage::evidence::EvidencePolicy::redacted(&serde_json::json!({"schema_version":1,"patterns":["absent-fixture-canary"],"sensitive_rules":[]}),&["runner-lifecycle"]).unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        let large = |sequence| {
            let mut value = transition(&fixture, sequence);
            let mut raw = value.evidence_json.clone();
            raw["sensitive"] = serde_json::json!(false);
            raw["excerpt"] = serde_json::json!("x".repeat(6000));
            value.evidence_json = policy.apply(&raw).unwrap();
            value.kind = RuntimeEventKind::PermissionDeniedObserved;
            value.cause_json = None;
            value
        };
        let mut fitting = Vec::new();
        let mut used = 0;
        let overflow = (1..=4096)
            .map(large)
            .find(|value| {
                let size = frame(value).len();
                if used + size > 16 * 1024 * 1024 {
                    return true;
                }
                used += size;
                fitting.push(value.clone());
                false
            })
            .expect("fixture must reach the fixed outbox capacity");
        let last = fitting.pop().unwrap();
        let io = FaultIo::from_bytes(fitting.iter().flat_map(frame).collect());
        let mut outbox = open(&fixture, io.clone());
        assert!(outbox.scan(until).unwrap().incomplete_tail.is_none());
        outbox.append_synced(&last, until).unwrap();
        assert_eq!(io.byte_len().unwrap(), used as u64);
        let before = io.snapshot_bytes();
        assert!(outbox.append_synced(&overflow, until).is_err());
        assert_eq!(io.snapshot_bytes(), before);
        assert!(used <= 16 * 1024 * 1024);
    }

    #[test]
    fn outbox_short_writes_preserve_the_complete_frame() {
        let fixture = WriterFixture::new();
        let io = FaultIo::default();
        io.0.lock().unwrap().short_write = Some(1);
        let mut outbox = open(&fixture, io.clone());
        let value = transition(&fixture, 1);
        outbox.append_synced(&value, deadline()).unwrap();
        assert_eq!(independent_frames(&io.snapshot_bytes()), vec![value]);
    }

    #[test]
    fn outbox_reopen_recovers_complete_original_frames_in_order() {
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
    fn outbox_each_incomplete_final_component_retains_the_valid_prefix() {
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
    fn outbox_append_diagnoses_and_truncates_only_an_incomplete_tail() {
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
    fn outbox_completed_corruption_preserves_all_original_bytes() {
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
            let result = Outbox::initialize(
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
    fn outbox_oversized_declared_length_fails_before_large_reads() {
        let fixture = WriterFixture::new();
        let io = FaultIo::from_bytes(u32::MAX.to_be_bytes().to_vec());
        let result = Outbox::initialize(
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
    fn outbox_sync_failure_leaves_a_complete_recoverable_pending_frame() {
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
    fn outbox_partial_write_failure_is_an_explicit_tail_without_a_commit() {
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
    fn outbox_compaction_retains_committed_prefix_until_every_frame_is_durable() {
        let mut fixture = WriterFixture::new();
        let values = [transition(&fixture, 1), transition(&fixture, 2)];
        let io = FaultIo::from_bytes(values.iter().flat_map(frame).collect());
        let mut outbox = open(&fixture, io.clone());
        let original = io.snapshot_bytes();
        storage::persist_event(
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
        storage::persist_event(
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
    fn outbox_truncate_failure_preserves_committed_originals_for_retry() {
        let mut fixture = WriterFixture::new();
        let value = transition(&fixture, 1);
        storage::persist_event(
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
    fn outbox_expired_deadline_and_stale_epoch_have_zero_content_effect() {
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
    fn outbox_holds_the_original_lock_for_its_entire_content_lifetime() {
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

    use crate::runtime::writer::test_support::*;

    mod refresh {
        use crate::runtime::writer::tests::*;
        fn opening(fixture: &WriterFixture) -> ObservedTransition {
            ObservedTransition {
                episode_seq: Some(1),
                kind: RuntimeEventKind::InteractionOpened,
                classification: Some(InteractionClass::PermissionRequest),
                cause_json: None,
                ..transition(fixture, 1)
            }
        }
        fn update(fixture: &WriterFixture, sequence: u64) -> InteractionEvidenceUpdate {
            InteractionEvidenceUpdate {
                run_id: fixture.fixture.run.run_id,
                episode_seq: 1,
                output_seq: sequence,
                evidence_json: transition(fixture, sequence.try_into().unwrap()).evidence_json,
            }
        }
        fn latest(fixture: &WriterFixture) -> Value {
            let text: String = fixture
                .fixture
                .conn
                .query_row(
                    "SELECT evidence_json FROM member_interactions WHERE run_id=?1 AND episode_seq=1",
                    [fixture.fixture.run.run_id],
                    |row| row.get(0),
                )
                .unwrap();
            serde_json::from_str(&text).unwrap()
        }
        fn committed_open(fixture: &WriterFixture, hooks: Arc<Hooks>) -> EventWriter {
            let writer = start(fixture, hooks);
            assert!(writer.try_submit(opening(fixture)).is_ok());
            wait_until(|| committed(fixture) == 1 && writer.status().pending_count == 0);
            writer
        }

        #[test]
        fn writer_refresh_waits_for_its_opening_barrier_without_an_outbox_frame() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            assert!(writer.try_submit(opening(&fixture)).is_ok());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(committed(&fixture), 0);
            assert!(writer.status().pending_evidence);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![opening(&fixture)]
            );
            drop(release);
            wait_until(|| committed(&fixture) == 1 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 2);
            assert_eq!(writer.finish(deadline()).0, 0);
            assert_eq!(committed(&fixture), 1);
        }

        #[test]
        fn writer_refresh_newer_revision_survives_an_inflight_successful_attempt() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
            assert!(writer.status().pending_evidence);
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 3);
            assert_eq!(committed(&fixture), 1);
            writer.finish(deadline());
        }

        #[test]
        fn writer_refresh_newer_revision_survives_an_inflight_failed_attempt() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
            hooks.fail_once(WriterBoundary::Persist);
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 3);
            assert_eq!(committed(&fixture), 1);
            writer.finish(deadline());
        }

        #[test]
        fn writer_refresh_rejects_regression_conflict_and_foreign_episode_without_replacing_newer_work()
         {
            for field in ["regression", "conflict", "episode"] {
                let fixture = WriterFixture::new();
                let hooks = Arc::new(Hooks::default());
                let mut writer = committed_open(&fixture, hooks.clone());
                hooks.pause(WriterBoundary::Persist);
                let release = Release(hooks.clone());
                assert!(writer.try_refresh(update(&fixture, 3), deadline()).is_ok());
                hooks.wait(WriterBoundary::Persist, 2);
                let mut invalid = update(&fixture, if field == "regression" { 2 } else { 3 });
                if field == "episode" {
                    invalid.episode_seq = 2;
                }
                if field == "conflict" {
                    let mut raw = invalid.evidence_json.clone();
                    raw["sensitive"] = serde_json::json!(false);
                    raw["truncated"] = serde_json::json!(true);
                    invalid.evidence_json = storage::evidence::EvidencePolicy::metadata()
                        .apply(&raw)
                        .unwrap();
                }
                let returned = writer
                    .try_refresh(invalid, deadline())
                    .expect_err("invalid refresh retains caller ownership");
                assert_eq!(
                    returned.update.output_seq,
                    if field == "regression" { 2 } else { 3 }
                );
                assert!(writer.status().degraded);
                drop(release);
                wait_until(|| !writer.status().pending_evidence);
                assert_eq!(latest(&fixture)["screen_seq"], 3);
                writer.finish(deadline());
            }
        }

        #[test]
        fn writer_refresh_closure_follows_earlier_evidence_and_rejects_late_old_episode_updates() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            let close = ObservedTransition {
                event_seq: 2,
                kind: RuntimeEventKind::InteractionClosed,
                close_reason: Some(InteractionCloseReason::NotVisible),
                evidence_json: transition(&fixture, 3).evidence_json,
                ..opening(&fixture)
            };
            assert!(writer.try_submit(close.clone()).is_ok());
            assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_err());
            drop(release);
            wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture), update(&fixture, 2).evidence_json);
            writer.finish(deadline());
        }

        #[test]
        fn writer_refresh_newer_same_episode_revision_advances_its_semantic_barrier() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            hooks.wait(WriterBoundary::Receive, 1);
            assert!(writer.try_submit(opening(&fixture)).is_ok());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            let obscured = ObservedTransition {
                event_seq: 2,
                kind: RuntimeEventKind::InteractionObscured,
                evidence_json: transition(&fixture, 3).evidence_json,
                ..opening(&fixture)
            };
            assert!(writer.try_submit(obscured).is_ok());
            assert!(writer.try_refresh(update(&fixture, 4), deadline()).is_ok());
            drop(release);
            wait_until(|| committed(&fixture) == 2 && !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 4);
            writer.finish(deadline());
        }

        #[test]
        fn writer_refresh_equal_identical_requests_coalesce_and_expiry_has_no_enqueue_effect() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            let mut writer = committed_open(&fixture, hooks.clone());
            hooks.pause(WriterBoundary::Persist);
            let release = Release(hooks.clone());
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            hooks.wait(WriterBoundary::Persist, 2);
            assert!(writer.try_refresh(update(&fixture, 2), deadline()).is_ok());
            let begin = Instant::now();
            assert!(
                writer
                    .try_refresh(update(&fixture, 3), begin - Duration::from_millis(1))
                    .is_err()
            );
            assert!(begin.elapsed() < Duration::from_millis(250));
            drop(release);
            wait_until(|| !writer.status().pending_evidence);
            assert_eq!(latest(&fixture)["screen_seq"], 2);
            writer.finish(deadline());
        }
    }

    mod worker {
        use crate::runtime::writer::tests::*;
        #[test]
        fn writer_hook_timeout_releases_the_mutex_before_unwinding_through_cleanup() {
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let observed = hooks.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _release = Release(observed.clone());
                observed.wait(WriterBoundary::Persist, 1);
            }));
            assert!(result.is_err());
            assert!(!hooks.state.is_poisoned());
            assert_eq!(hooks.state.lock().unwrap().paused, [false; 5]);
        }

        #[test]
        fn writer_hook_cleanup_releases_known_state_after_a_worker_poison_without_panicking() {
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Receive);
            let observed = hooks.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _state = observed.state.lock().unwrap();
                panic!("owned fixture worker panic");
            }));
            assert!(result.is_err());
            let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                drop(Release(hooks.clone()))
            }));
            assert!(cleanup.is_ok());
            let state = hooks
                .state
                .lock()
                .expect_err("ordinary access preserves poisoned-state invariant")
                .into_inner();
            assert_eq!(state.paused, [false; 5]);
        }

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
            assert!(status.degraded);
            assert!(status.gap_count > 0);
            assert_eq!(committed(&fixture), 0);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), 64);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_worker_sync_precedes_sqlite_and_pending_delivery_is_not_an_inbox_commit() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(committed(&fixture), 0);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            let first = hooks.state.lock().unwrap().first;
            assert!(first[1].unwrap() < first[2].unwrap());
            assert!(first[2].unwrap() < first[3].unwrap());
            assert_eq!(writer.status().pending_count, 1);
            drop(release);
            wait_until(|| committed(&fixture) == 1);
            assert_eq!(writer.finish(deadline()), (0, 0));
        }

        #[test]
        fn writer_worker_ambiguous_sync_failure_rescans_and_reuses_the_complete_original() {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(Hooks::default());
            hooks.fail_once(WriterBoundary::Sync);
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            assert_eq!(hooks.calls(WriterBoundary::Append), 1);
            assert!(hooks.calls(WriterBoundary::Sync) >= 2);
            drop(release);
            wait_until(|| committed(&fixture) == 1);
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
            assert!(writer.status().degraded);
            drop(release);
            wait_until(|| writer.status().pending_count == 0);
            assert_eq!(committed(&fixture), i64::try_from(accepted).unwrap());
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn writer_worker_refill_is_nonblocking_coalesced_and_published_only_after_commit() {
            let fixture = WriterFixture::new();
            let blocking = crate::db::connect(&format!(
                "sqlite:///{}",
                fixture.fixture.conn.path().unwrap()
            ))
            .unwrap();
            blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks);
            let begin = Instant::now();
            for _ in 0..10 {
                writer.request_sequence_range().unwrap();
            }
            assert!(begin.elapsed() < Duration::from_millis(250));
            assert!(writer.take_sequence_range().is_none());
            assert_eq!(
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                    .unwrap()
                    .event_seq_reserved_through,
                256
            );
            blocking.execute_batch("ROLLBACK").unwrap();
            let mut range = None;
            wait_until(|| {
                range = writer.take_sequence_range();
                range.is_some()
            });
            assert_eq!(range.unwrap(), 257..=512);
            assert!(writer.take_sequence_range().is_none());
            assert_eq!(
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                    .unwrap()
                    .event_seq_reserved_through,
                512
            );
            writer.finish(deadline());
        }

        #[test]
        fn writer_worker_shutdown_retains_synced_uncommitted_frames_and_the_original_budget() {
            let fixture = WriterFixture::new();
            let blocking = crate::db::connect(&format!(
                "sqlite:///{}",
                fixture.fixture.conn.path().unwrap()
            ))
            .unwrap();
            blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            let value = transition(&fixture, 1);
            assert!(writer.try_submit(value.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let begin = Instant::now();
            writer.set_deadline(begin + Duration::from_millis(80));
            writer.begin_shutdown(begin + Duration::from_secs(3));
            let pending = writer.finish(begin + Duration::from_secs(3));
            assert!(begin.elapsed() < Duration::from_millis(750));
            assert_eq!(pending.0, 1);
            assert_eq!(committed(&fixture), 0);
            assert_eq!(
                independent_frames(&std::fs::read(&fixture.artifacts.paths.outbox).unwrap()),
                vec![value]
            );
            blocking.execute_batch("ROLLBACK").unwrap();
        }

        struct SlowCompaction {
            calls: AtomicU64,
        }
        impl WriterHooks for SlowCompaction {
            fn before(&self, boundary: WriterBoundary, _: Instant) -> Result<()> {
                if matches!(boundary, WriterBoundary::Compact) {
                    self.calls.fetch_add(1, Ordering::AcqRel);
                    std::thread::sleep(Duration::from_millis(250));
                }
                Ok(())
            }
        }

        #[test]
        fn writer_worker_final_compaction_slower_than_one_work_slice_commits_before_the_shutdown_deadline()
         {
            let fixture = WriterFixture::new();
            let hooks = Arc::new(SlowCompaction {
                calls: AtomicU64::new(0),
            });
            let mut writer = EventWriter::start_with_hooks(
                format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                fixture.fixture.run.clone(),
                fixture.lock.clone(),
                fixture.artifacts.paths.outbox.clone(),
                hooks.clone(),
            )
            .unwrap();
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let begin = Instant::now();
            assert_eq!(writer.finish(begin + Duration::from_secs(3)), (0, 0));
            assert!(
                writer.joined(),
                "the worker stops once its final compaction commits"
            );
            assert!(begin.elapsed() < Duration::from_secs(3));
            assert!(
                std::fs::read(&fixture.artifacts.paths.outbox)
                    .unwrap()
                    .is_empty(),
                "the committed outbox is compacted"
            );
            assert!(hooks.calls.load(Ordering::Acquire) >= 1);
        }
    }

    mod effective_policy {
        use crate::runtime::writer::tests::*;
        fn set_status(fixture: &mut WriterFixture, status: &str) {
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                    rusqlite::params![status, fixture.fixture.run.run_id],
                )
                .unwrap();
            fixture.fixture.run =
                storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
        }

        fn stored(fixture: &WriterFixture) -> Option<Value> {
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id)
                .unwrap()
                .effective_policy_json
        }

        fn spool(fixture: &WriterFixture) -> Vec<u8> {
            std::fs::read(&fixture.artifacts.paths.outbox).unwrap()
        }

        #[test]
        fn runner_policy_unavailable_frame_spools_its_observation_and_commits_it_with_the_event() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let value = unavailable(&fixture, 1);
            let lost = observation(AutomaticReview::Unavailable);
            assert!(
                writer
                    .try_submit_with_policy(value.clone(), Some(lost.clone()))
                    .is_ok()
            );
            hooks.wait(WriterBoundary::Persist, 1);
            let mut expected = serde_json::to_value(&value).unwrap();
            expected["effective_policy"] = serde_json::to_value(&lost).unwrap();
            assert_eq!(frame_bodies(&spool(&fixture)), vec![expected]);
            assert_eq!(stored(&fixture), None);
            drop(release);
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let event = storage::get_event(&fixture.fixture.conn, 1).unwrap().event;
            assert_eq!(event.kind, RuntimeEventKind::AutomaticReviewUnavailable);
            assert_eq!(stored(&fixture), Some(record(&lost, Some(event.event_id))));
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_other_frames_keep_their_bytes_and_reject_an_attached_observation() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let plain = transition(&fixture, 1);
            assert!(
                writer
                    .try_submit_with_policy(
                        plain.clone(),
                        Some(observation(AutomaticReview::Unavailable))
                    )
                    .is_err()
            );
            assert!(writer.try_submit_with_policy(plain.clone(), None).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            assert_eq!(spool(&fixture), frame(&plain));
            drop(release);
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(stored(&fixture), None);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_commits_behind_accepted_transitions_without_spooling() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            hooks.pause(WriterBoundary::Persist);
            let mut writer = start(&fixture, hooks.clone());
            let release = Release(hooks.clone());
            let earlier = transition(&fixture, 1);
            assert!(writer.try_submit(earlier.clone()).is_ok());
            hooks.wait(WriterBoundary::Persist, 1);
            let shown = observation(AutomaticReview::Available);
            writer.try_record_policy(shown.clone());
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(stored(&fixture), None, "queued behind the paused event");
            assert_eq!(
                spool(&fixture),
                frame(&earlier),
                "the update is not spooled"
            );
            drop(release);
            wait_until(|| stored(&fixture).is_some());
            assert_eq!(committed(&fixture), 1);
            assert_eq!(stored(&fixture), Some(record(&shown, None)));
            let status = writer.status();
            assert_eq!(status.policy_diagnostic, None);
            assert!(!status.degraded);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_on_a_non_live_run_is_dropped_once_and_the_worker_continues()
         {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "stopping");
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            writer.try_record_policy(observation(AutomaticReview::Available));
            wait_until(|| writer.status().policy_diagnostic.is_some());
            let persists = hooks.calls(WriterBoundary::Persist);
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                hooks.calls(WriterBoundary::Persist),
                persists,
                "a dropped update is not retried"
            );
            assert_eq!(stored(&fixture), None);
            let status = writer.status();
            assert!(!status.degraded, "{status:?}");
            assert_eq!(status.gap_count, 0);
            assert!(writer.try_submit(transition(&fixture, 1)).is_ok());
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            assert_eq!(writer.finish(deadline()).0, 0);
        }

        #[test]
        fn runner_policy_available_update_from_a_superseded_owner_is_dropped_with_a_diagnostic() {
            let mut fixture = WriterFixture::new();
            set_status(&mut fixture, "running");
            let hooks = Arc::new(Hooks::default());
            let mut writer = start(&fixture, hooks.clone());
            let _release = Release(hooks.clone());
            let lost = observation(AutomaticReview::Unavailable);
            assert!(
                writer
                    .try_submit_with_policy(unavailable(&fixture, 1), Some(lost.clone()))
                    .is_ok()
            );
            wait_until(|| committed(&fixture) == 1 && writer.status().pending_count == 0);
            let current = stored(&fixture).expect("event-carried record");
            assert_eq!(current, record(&lost, Some(1)));
            fixture
                .fixture
                .conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [fixture.fixture.run.run_id],
                )
                .unwrap();
            writer.try_record_policy(observation(AutomaticReview::Available));
            wait_until(|| writer.status().policy_diagnostic.is_some());
            assert_eq!(
                stored(&fixture),
                Some(current),
                "a superseded runner never overwrites the record"
            );
            writer.finish(deadline());
        }
    }
    fn assert_start_rejected(fixture: &WriterFixture, path: PathBuf) {
        let result = fixture.start(path);
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
    fn writer_starts_with_the_exact_private_recorded_outbox() {
        let fixture = WriterFixture::new();
        let mut writer = fixture
            .start(fixture.artifacts.paths.outbox.clone())
            .unwrap();
        assert_eq!(
            writer.finish(Instant::now() + Duration::from_secs(1)),
            (0, 0)
        );
    }

    #[test]
    fn writer_foreign_outbox_path_fails_before_worker_start() {
        let fixture = WriterFixture::new();
        let foreign = fixture.fixture._dir.path().join("foreign-outbox");
        std::fs::write(&foreign, []).unwrap();
        assert_start_rejected(&fixture, foreign.clone());
        assert_eq!(std::fs::read(foreign).unwrap(), b"");
    }

    #[test]
    fn writer_missing_or_symlinked_outbox_fails_before_worker_start() {
        for symlink in [false, true] {
            let fixture = WriterFixture::new();
            let path = &fixture.artifacts.paths.outbox;
            std::fs::remove_file(path).unwrap();
            let target = fixture.fixture._dir.path().join("untouched-target");
            std::fs::write(&target, b"original target").unwrap();
            if symlink {
                std::os::unix::fs::symlink(&target, path).unwrap();
            }
            assert_start_rejected(&fixture, path.clone());
            assert_eq!(std::fs::read(target).unwrap(), b"original target");
        }
    }

    #[test]
    fn writer_stale_owner_is_rejected_before_worker_start() {
        let mut fixture = WriterFixture::new();
        fixture.fixture.run.owner_epoch += 1;
        assert_start_rejected(&fixture, fixture.artifacts.paths.outbox.clone());
    }

    #[test]
    fn writer_spool_sync_is_pending_until_the_actual_sqlite_commit() {
        let fixture = WriterFixture::new();
        let blocking = crate::db::connect(&format!(
            "sqlite:///{}",
            fixture.fixture.conn.path().unwrap()
        ))
        .unwrap();
        blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
        let value = transition(&fixture, 1);
        let mut writer = fixture
            .start(fixture.artifacts.paths.outbox.clone())
            .unwrap();
        writer.submit(value.clone()).unwrap();
        let until = deadline();
        let bytes = loop {
            let bytes = std::fs::read(&fixture.artifacts.paths.outbox).unwrap();
            if bytes.len() == frame(&value).len() {
                break bytes;
            }
            assert!(
                Instant::now() < until,
                "complete pending frame was not written: {:?}",
                writer.diagnostic()
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(independent_frames(&bytes), vec![value]);
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
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        blocking.execute_batch("ROLLBACK").unwrap();
        let until = deadline();
        loop {
            let count = fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            if count == 1 {
                break;
            }
            assert!(
                Instant::now() < until,
                "event did not commit: {:?}",
                writer.diagnostic()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(writer.finish(deadline()), (0, 0));
        assert_eq!(
            fixture
                .fixture
                .conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
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
    pub(crate) const VERSION: &str = "0.0.1-synthetic";
    pub(crate) const NOW: &str = "2026-10-26T16:40:00.000000+00:00";
    pub(in crate::runtime) use crate::coding_agent::catalog::test_support::synthetic_catalog as catalog;
    pub(in crate::runtime) use crate::coding_agent::catalog::test_support::synthetic_definitions as definitions;
    pub(crate) fn configure(fixture: &mut WriterFixture) {
        fixture.fixture.conn.execute("UPDATE member_runs SET native_version=?1,coverage='tested',runner_pid=owner_pid,runner_start_identity=owner_start_identity WHERE run_id=?2",rusqlite::params![VERSION,fixture.fixture.run.run_id]).unwrap();
        fixture.fixture.run =
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    }
    pub(crate) fn configure_redacted(fixture: &mut WriterFixture) {
        configure(fixture);
        fixture
            .fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                [fixture.fixture.run.run_id],
            )
            .unwrap();
        fixture.fixture.run =
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    }
    pub(crate) struct Clock {
        pub(crate) origin: Instant,
        pub(crate) millis: AtomicU64,
    }
    impl Clock {
        pub(crate) fn new() -> Self {
            Self {
                origin: Instant::now(),
                millis: AtomicU64::new(0),
            }
        }
        pub(crate) fn set(&self, millis: u64) {
            self.millis.store(millis, Ordering::Release);
        }
    }
    impl ObservationClock for Clock {
        fn sample(&self) -> ObservationMoment {
            ObservationMoment {
                monotonic: self.origin + Duration::from_millis(self.millis.load(Ordering::Acquire)),
                observed_at: NOW.into(),
            }
        }
        fn kind(&self) -> ObservationClockKind {
            ObservationClockKind::Deterministic
        }
    }
    pub(crate) fn running(fixture: &mut WriterFixture, child: &ProcessIdentity) {
        fixture.fixture.conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",rusqlite::params![child.pid,child.start_identity,NOW,fixture.fixture.run.run_id]).unwrap();
        fixture.fixture.run =
            storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
    }
    pub(crate) fn form(operation: &str) -> Vec<u8> {
        format!("\x1b[2J\x1b[19;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nSelection: ").into_bytes()
    }
    pub(crate) fn detailed_form(operation: &str, detail: &str) -> Vec<u8> {
        format!("\x1b[2J\x1b[18;1HSynthetic permission request\r\nOperation: {operation}\r\n> Allow once\r\n  Deny once\r\nRequest form: synthetic-only\r\nDetail: {detail}\r\nSelection: ").into_bytes()
    }
    // Rewrites only the non-fingerprint detail row of `detailed_form` and returns the cursor to the end of `Selection: `.
    pub(crate) fn detail_rewrite(detail: &str) -> Vec<u8> {
        format!("\x1b[23;1H\x1b[2KDetail: {detail}\x1b[24;12H").into_bytes()
    }
    pub(crate) fn redacted_policy() -> storage::evidence::EvidencePolicy {
        storage::evidence::EvidencePolicy::redacted(
                &serde_json::json!({"schema_version":1,"patterns":["synthetic-redaction-canary"],"sensitive_rules":[]}),
                &["synthetic-permission"],
            )
            .unwrap()
    }
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
        pub(crate) fn calls(&self, boundary: WriterBoundary) -> u64 {
            self.state.lock().unwrap().calls[index(boundary)]
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
            fixture.artifacts.paths.outbox.clone(),
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
    pub(crate) const UNAVAILABLE_RULE: &str = "codex-auto-review-unavailable";
    pub(crate) const AVAILABLE_RULE: &str = "codex-auto-review-indicator";

    pub(crate) fn unavailable(fixture: &WriterFixture, sequence: i64) -> ObservedTransition {
        let evidence = storage::evidence::run_metadata(
            &fixture.fixture.run,
            UNAVAILABLE_RULE,
            u64::try_from(sequence).unwrap(),
        )
        .unwrap();
        ObservedTransition {
            kind: RuntimeEventKind::AutomaticReviewUnavailable,
            evidence_json: evidence,
            cause_json: None,
            ..transition(fixture, sequence)
        }
    }

    pub(crate) fn observation(automatic_review: AutomaticReview) -> EffectivePolicyObservation {
        EffectivePolicyObservation {
            automatic_review,
            observed_mode: None,
            rule_id: if automatic_review == AutomaticReview::Unavailable {
                UNAVAILABLE_RULE
            } else {
                AVAILABLE_RULE
            }
            .into(),
            observed_at: "2026-10-26T16:40:00.000000+00:00".into(),
        }
    }

    pub(crate) fn record(observation: &EffectivePolicyObservation, event_id: Option<i64>) -> Value {
        serde_json::json!({
            "schema_version":1,
            "automatic_review":observation.automatic_review.as_str(),
            "observed_mode":observation.observed_mode,
            "rule_id":observation.rule_id,
            "event_id":event_id,
            "observed_at":observation.observed_at,
        })
    }

    pub(crate) fn frame_bodies(mut bytes: &[u8]) -> Vec<Value> {
        let mut bodies = Vec::new();
        while !bytes.is_empty() {
            let length =
                usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
            let body = &bytes[36..36 + length];
            assert_eq!(&bytes[4..36], &Sha256::digest(body)[..]);
            bodies.push(serde_json::from_slice(body).unwrap());
            bytes = &bytes[36 + length..];
        }
        bodies
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
        pub(crate) fn start(&self, path: PathBuf) -> Result<EventWriter> {
            EventWriter::start(
                format!("sqlite:///{}", self.fixture.conn.path().unwrap()),
                self.fixture.run.clone(),
                self.lock.clone(),
                path,
            )
        }
    }

    pub(crate) fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    pub(crate) fn transition(fixture: &WriterFixture, sequence: i64) -> ObservedTransition {
        let evidence = storage::evidence::run_metadata(
            &fixture.fixture.run,
            "runner-lifecycle",
            u64::try_from(sequence).unwrap(),
        )
        .unwrap();
        ObservedTransition {
            run_id: fixture.fixture.run.run_id,
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

    pub(crate) fn independent_frames(mut bytes: &[u8]) -> Vec<ObservedTransition> {
        let mut values = Vec::new();
        while !bytes.is_empty() {
            assert!(bytes.len() >= 36);
            let length =
                usize::try_from(u32::from_be_bytes(bytes[..4].try_into().unwrap())).unwrap();
            assert!(length > 0 && length <= 16 * 1024 * 1024);
            assert!(bytes.len() >= 36 + length);
            let body = &bytes[36..36 + length];
            assert_eq!(&bytes[4..36], &Sha256::digest(body)[..]);
            values.push(serde_json::from_slice(body).unwrap());
            bytes = &bytes[36 + length..];
        }
        values
    }
}
