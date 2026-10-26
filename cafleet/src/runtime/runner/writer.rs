use super::*;
use crate::broker::runtime::{self as storage, ObservedTransition, PersistenceMode, RunRecord};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[cfg(test)]
mod contract_tests;

pub(super) struct EventWriter {
    sender: Option<mpsc::SyncSender<ObservedTransition>>,
    stop: Arc<AtomicBool>,
    deadline: Arc<Mutex<Option<Instant>>>,
    pending: Arc<AtomicU64>,
    gap: Arc<AtomicU64>,
    diagnostic: Arc<Mutex<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl EventWriter {
    pub(super) fn start(
        database_url: String,
        run: RunRecord,
        lock: Arc<RunLock>,
        outbox: PathBuf,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<ObservedTransition>(64);
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = Arc::new(Mutex::new(None::<Instant>));
        let pending = Arc::new(AtomicU64::new(0));
        let gap = Arc::new(AtomicU64::new(0));
        let diagnostic = Arc::new(Mutex::new(None));
        let observed_stop = stop.clone();
        let observed_deadline = deadline.clone();
        let observed_pending = pending.clone();
        let observed_gap = gap.clone();
        let observed_diagnostic = diagnostic.clone();
        let thread = std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut conn = crate::db::connect(&database_url)?;
                let mut spool = std::fs::OpenOptions::new()
                    .read(true)
                    .append(true)
                    .custom_flags(nix::libc::O_NOFOLLOW)
                    .open(&outbox)
                    .map_err(io_error)?;
                ensure(
                    spool.metadata().map_err(io_error)?.len() == 0,
                    "new runner outbox must start empty",
                )?;
                let mut queue = VecDeque::<ObservedTransition>::new();
                let mut stopping = false;
                let mut retry = Duration::from_millis(100);
                let mut next_attempt = Instant::now();
                loop {
                    let until = *observed_deadline.lock().expect("writer deadline mutex");
                    if until.is_some_and(|until| Instant::now() >= until) {
                        break;
                    }
                    while queue.len() < 64 {
                        let event = match receiver.try_recv() {
                            Ok(event) => event,
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                stopping = true;
                                break;
                            }
                        };
                        let bytes = serde_json::to_vec(&event).map_err(io_error)?;
                        ensure(
                            spool.metadata().map_err(io_error)?.len() + bytes.len() as u64 + 36
                                <= 16 * 1024 * 1024,
                            "runtime outbox reached 16 MiB pressure limit",
                        )?;
                        let length = u32::try_from(bytes.len()).map_err(io_error)?;
                        spool.write_all(&length.to_be_bytes()).map_err(io_error)?;
                        spool.write_all(&Sha256::digest(&bytes)).map_err(io_error)?;
                        spool.write_all(&bytes).map_err(io_error)?;
                        spool.sync_all().map_err(io_error)?;
                        queue.push_back(event);
                    }
                    stopping |= observed_stop.load(Ordering::Acquire);
                    if queue.is_empty() {
                        if stopping {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    if Instant::now() < next_attempt {
                        std::thread::sleep(
                            Duration::from_millis(5)
                                .min(next_attempt.saturating_duration_since(Instant::now())),
                        );
                        continue;
                    }
                    let work_until = until
                        .map(|until| until.min(Instant::now() + Duration::from_millis(100)))
                        .unwrap_or_else(|| Instant::now() + Duration::from_millis(100));
                    let event = queue.front().expect("nonempty event queue");
                    match with_database_deadline(&mut conn, work_until, |conn| {
                        lock.validate(conn, &run.owner())?;
                        storage::persist_transition(
                            conn,
                            &run.owner(),
                            event,
                            PersistenceMode::Live,
                        )
                        .map(|_| ())
                    }) {
                        Ok(()) => {
                            queue.pop_front();
                            observed_pending.fetch_sub(1, Ordering::AcqRel);
                            retry = Duration::from_millis(100);
                            next_attempt = Instant::now();
                            if queue.is_empty() {
                                spool.set_len(0).map_err(io_error)?;
                                spool.sync_all().map_err(io_error)?;
                            }
                            *observed_diagnostic.lock().expect("writer diagnostic mutex") = None;
                        }
                        Err(cause) => {
                            *observed_diagnostic.lock().expect("writer diagnostic mutex") =
                                Some(cause.message().into_owned());
                            next_attempt = Instant::now() + retry;
                            retry = (retry * 2).min(Duration::from_secs(2));
                        }
                    }
                }
                while let Ok(event) = receiver.try_recv() {
                    if observed_deadline
                        .lock()
                        .expect("writer deadline mutex")
                        .is_some_and(|until| Instant::now() >= until)
                    {
                        observed_gap.fetch_add(1, Ordering::AcqRel);
                        continue;
                    }
                    let bytes = serde_json::to_vec(&event).map_err(io_error)?;
                    ensure(
                        spool.metadata().map_err(io_error)?.len() + bytes.len() as u64 + 36
                            <= 16 * 1024 * 1024,
                        "runtime outbox pressure during shutdown",
                    )?;
                    spool
                        .write_all(&u32::try_from(bytes.len()).map_err(io_error)?.to_be_bytes())
                        .map_err(io_error)?;
                    spool.write_all(&Sha256::digest(&bytes)).map_err(io_error)?;
                    spool.write_all(&bytes).map_err(io_error)?;
                    spool.sync_all().map_err(io_error)?;
                }
                Ok(())
            })();
            if let Err(cause) = result {
                observed_gap.fetch_add(1, Ordering::AcqRel);
                *observed_diagnostic.lock().expect("writer diagnostic mutex") =
                    Some(cause.message().into_owned());
            }
        });
        Ok(Self {
            sender: Some(sender),
            stop,
            deadline,
            pending,
            gap,
            diagnostic,
            thread: Some(thread),
        })
    }
    pub(super) fn submit(&self, event: ObservedTransition) -> Result<()> {
        self.pending.fetch_add(1, Ordering::AcqRel);
        match self
            .sender
            .as_ref()
            .ok_or_else(|| error("event writer is stopped"))?
            .try_send(event)
        {
            Ok(()) => Ok(()),
            Err(_) => {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                self.gap.fetch_add(1, Ordering::AcqRel);
                Err(error("runtime observation channel is full or unavailable"))
            }
        }
    }
    pub(super) fn diagnostic(&self) -> Option<String> {
        self.diagnostic
            .lock()
            .expect("writer diagnostic mutex")
            .clone()
    }
    pub(super) fn set_deadline(&mut self, until: Instant) {
        let mut deadline = self.deadline.lock().expect("writer deadline mutex");
        if deadline.is_none() {
            *deadline = Some(until);
        }
    }
    pub(super) fn begin_shutdown(&mut self, until: Instant) {
        let mut deadline = self.deadline.lock().expect("writer deadline mutex");
        if deadline.is_none() {
            *deadline = Some(until);
        }
        self.stop.store(true, Ordering::Release);
        self.sender.take();
    }
    pub(super) fn finish(&mut self, until: Instant) -> (u64, u64) {
        self.begin_shutdown(until);
        if let Some(thread) = self.thread.as_ref() {
            while !thread.is_finished() && Instant::now() < until {
                std::thread::sleep(
                    Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
                );
            }
        }
        if self
            .thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            self.thread
                .take()
                .expect("writer thread exists")
                .join()
                .expect("writer event processing is panic-free");
        } else if self.thread.is_some() {
            self.gap.fetch_add(1, Ordering::AcqRel);
        }
        (
            self.pending.load(Ordering::Acquire),
            self.gap.load(Ordering::Acquire),
        )
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        let until = self
            .deadline
            .lock()
            .expect("writer deadline mutex")
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(4));
        self.finish(until);
    }
}
