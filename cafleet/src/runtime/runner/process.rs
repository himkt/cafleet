use super::*;
use crate::broker::runtime::CleanupState;
use filedescriptor::FileDescriptor;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::time::{Duration, Instant};

#[cfg(test)]
mod ownership_diagnostics;
#[cfg(test)]
pub(crate) use ownership_diagnostics::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProcessIdentity {
    pub pid: i64,
    pub start_identity: String,
}
impl ProcessIdentity {
    pub(crate) fn current() -> Result<Self> {
        Self::read(i64::from(std::process::id()))?
            .ok_or_else(|| error("current process identity unavailable"))
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn read(pid: i64) -> Result<Option<Self>> {
        Ok(kernel_process(pid)?.map(|process| process.identity))
    }
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn read(pid: i64) -> Result<Option<Self>> {
        Self::from_stat(pid, std::fs::read_to_string(format!("/proc/{pid}/stat")))
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn from_stat(pid: i64, stat: io::Result<String>) -> Result<Option<Self>> {
        let Some(text) = present_stat(stat)? else {
            return Ok(None);
        };
        let (_, tail) = text
            .rsplit_once(')')
            .ok_or_else(|| error("malformed process stat"))?;
        let fields: Vec<_> = tail.split_whitespace().collect();
        let start = fields
            .get(19)
            .ok_or_else(|| error("missing process birth time"))?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(io_error)?;
        Ok(Some(Self {
            pid,
            start_identity: format!("{}:{start}", boot.trim()),
        }))
    }
    pub(crate) fn matches(&self) -> Result<bool> {
        Ok(Self::read(self.pid)?.as_ref() == Some(self))
    }
}

#[cfg(target_os = "macos")]
struct KernelProcess {
    identity: ProcessIdentity,
    zombie: bool,
}
// LP64 Darwin kinfo_proc: kp_proc.p_starttime (timeval) at 0, p_stat at 36, p_pid at 40.
#[cfg(target_os = "macos")]
const KINFO_PROC_BYTES: usize = 648;
// proc_pidinfo(PROC_PIDTBSDINFO) refuses group members with another effective UID (setuid
// helpers), while kern.proc.pid reports the same p_start for every process.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn kinfo_proc_bytes(pid: i32) -> io::Result<([u8; KINFO_PROC_BYTES], usize)> {
    let mut mib = [
        nix::libc::CTL_KERN,
        nix::libc::KERN_PROC,
        nix::libc::KERN_PROC_PID,
        pid,
    ];
    let mut buffer = [0u8; KINFO_PROC_BYTES];
    let mut length: nix::libc::size_t = KINFO_PROC_BYTES;
    // SAFETY: `mib` is a live four-element array and its length is passed exactly; `buffer` is
    // a live, writable byte array whose capacity is passed in `length`, and the kernel writes at
    // most `length` bytes and stores the written count back into `length`; no new value is set.
    let status = unsafe {
        nix::libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as nix::libc::c_uint,
            buffer.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((buffer, length))
}
#[cfg(target_os = "macos")]
fn kernel_process(pid: i64) -> Result<Option<KernelProcess>> {
    const SZOMB: u8 = 5;
    let pid32 = i32::try_from(pid).map_err(io_error)?;
    let (bytes, length) = kinfo_proc_bytes(pid32)
        .map_err(|cause| error(format!("cannot establish process start identity: {cause}")))?;
    if length == 0 {
        return Ok(None);
    }
    ensure(
        length == KINFO_PROC_BYTES,
        "kern.proc.pid returned an unexpected kinfo_proc size",
    )?;
    let reported = i32::from_ne_bytes(bytes[40..44].try_into().expect("four-byte p_pid"));
    ensure(
        i64::from(reported) == pid,
        "kern.proc.pid returned another process",
    )?;
    let seconds = i64::from_ne_bytes(bytes[0..8].try_into().expect("eight-byte tv_sec"));
    let micros = i32::from_ne_bytes(bytes[8..12].try_into().expect("four-byte tv_usec"));
    Ok(Some(KernelProcess {
        identity: ProcessIdentity {
            pid,
            start_identity: format!("{seconds}:{micros}"),
        },
        zombie: bytes[36] == SZOMB,
    }))
}
#[cfg(target_os = "macos")]
fn group_members(group: i32) -> Result<Vec<i64>> {
    // libproc interprets an empty group through errno, which preceding I/O can leave stale.
    nix::errno::Errno::clear();
    libproc::processes::pids_by_type(libproc::processes::ProcFilter::ByProgramGroup {
        pgrpid: group.try_into().map_err(io_error)?,
    })
    .map(|pids| pids.into_iter().map(i64::from).collect())
    .map_err(io_error)
}
#[cfg(not(target_os = "macos"))]
fn group_members(group: i32) -> Result<Vec<i64>> {
    let mut members = Vec::new();
    for entry in std::fs::read_dir("/proc").map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        match nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid))) {
            Ok(pgid) if pgid.as_raw() == group => members.push(i64::from(pid)),
            Ok(_) | Err(nix::errno::Errno::ESRCH) => {}
            Err(cause) => return Err(io_error(cause)),
        }
    }
    Ok(members)
}
#[cfg(target_os = "macos")]
fn stopped(pid: i64) -> Result<bool> {
    Ok(kernel_process(pid)?.is_none_or(|process| process.zombie))
}
// A task reaped after its `/proc/<pid>` entry was found fails the stat read with ESRCH rather
// than ENOENT; both mean the process is gone.
#[cfg(not(target_os = "macos"))]
fn present_stat(stat: io::Result<String>) -> Result<Option<String>> {
    match stat {
        Ok(text) => Ok(Some(text)),
        Err(cause)
            if cause.kind() == io::ErrorKind::NotFound
                || cause.raw_os_error() == Some(nix::libc::ESRCH) =>
        {
            Ok(None)
        }
        Err(cause) => Err(io_error(cause)),
    }
}
#[cfg(not(target_os = "macos"))]
fn stopped(pid: i64) -> Result<bool> {
    stopped_from_stat(std::fs::read_to_string(format!("/proc/{pid}/stat")))
}
#[cfg(not(target_os = "macos"))]
fn stopped_from_stat(stat: io::Result<String>) -> Result<bool> {
    let Some(text) = present_stat(stat)? else {
        return Ok(true);
    };
    Ok(text
        .rsplit_once(')')
        .ok_or_else(|| error("malformed process status"))?
        .1
        .trim_start()
        .starts_with('Z'))
}

trait ProcessObservations {
    fn members(&mut self, group: i32) -> Result<Vec<i64>>;
    fn identity(&mut self, pid: i64) -> Result<Option<ProcessIdentity>>;
    fn stopped(&mut self, pid: i64) -> Result<bool>;
}

struct NativeProcessObservations;

impl ProcessObservations for NativeProcessObservations {
    fn members(&mut self, group: i32) -> Result<Vec<i64>> {
        group_members(group)
    }

    fn identity(&mut self, pid: i64) -> Result<Option<ProcessIdentity>> {
        ProcessIdentity::read(pid)
    }

    fn stopped(&mut self, pid: i64) -> Result<bool> {
        stopped(pid)
    }
}

pub(super) fn recover_descendants(
    run: &crate::broker::runtime::RunRecord,
    clock: ShutdownClock,
) -> Result<ShutdownOutcome> {
    let mut outcome = ShutdownOutcome {
        native_exit: None,
        cleanup_state: CleanupState::Complete,
        diagnostics: Vec::new(),
        dropped_display_bytes: 0,
        pending_event_frames: 0,
        unsaved_observation_gap: 0,
    };
    let child = match (run.child_pid, run.child_start_identity.as_ref()) {
        (None, None) => return Ok(outcome),
        (Some(pid), Some(start)) => ProcessIdentity {
            pid,
            start_identity: start.clone(),
        },
        _ => return Err(error("recorded child generation is incomplete")),
    };
    if !child.matches()? || stopped(child.pid)? {
        if !group_members(i32::try_from(child.pid).map_err(io_error)?)?.is_empty() {
            outcome.cleanup_state = CleanupState::Incomplete;
            outcome.diagnostics.push(
                "descendant ownership cannot be anchored to the recorded child generation".into(),
            );
        }
        return Ok(outcome);
    }
    let mut owned = BTreeMap::new();
    owned.insert(child.pid, child.clone());
    for signal in [
        nix::sys::signal::Signal::SIGTERM,
        nix::sys::signal::Signal::SIGKILL,
    ] {
        let anchored = owned
            .values()
            .try_fold(false, |found, process| -> Result<bool> {
                Ok(found || process.matches()?)
            })?;
        if !anchored {
            if !group_members(i32::try_from(child.pid).map_err(io_error)?)?.is_empty() {
                outcome.cleanup_state = CleanupState::Incomplete;
                outcome
                    .diagnostics
                    .push("descendant ownership lost its recorded process generation".into());
            }
            break;
        }
        for pid in group_members(i32::try_from(child.pid).map_err(io_error)?)? {
            if let Some(process) = ProcessIdentity::read(pid)? {
                if let Some(previous) = owned.get(&pid) {
                    ensure(
                        previous == &process,
                        "recovery descendant PID generation changed",
                    )?;
                } else {
                    owned.insert(pid, process);
                }
            }
        }
        for process in owned.values() {
            if process.matches()? && !stopped(process.pid)? {
                match nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(i32::try_from(process.pid).map_err(io_error)?),
                    signal,
                ) {
                    Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                    Err(cause) => return Err(io_error(cause)),
                }
            }
        }
        let until = if signal == nix::sys::signal::Signal::SIGTERM {
            clock.child_deadline()
        } else {
            clock.storage_deadline()
        };
        while Instant::now() < until {
            let alive = owned
                .values()
                .try_fold(false, |alive, process| -> Result<bool> {
                    Ok(alive || (process.matches()? && !stopped(process.pid)?))
                })?;
            if !alive {
                break;
            }
            std::thread::sleep(
                Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
            );
        }
    }
    for process in owned.values() {
        if process.matches()? && !stopped(process.pid)? {
            outcome.cleanup_state = CleanupState::Incomplete;
            outcome
                .diagnostics
                .push("verified descendants remain after the original cleanup deadline".into());
            break;
        }
    }
    Ok(outcome)
}

struct MasterFd<'a>(&'a dyn MasterPty);
impl AsRawFd for MasterFd<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.0
            .as_raw_fd()
            .expect("Unix PTY exposes its master descriptor")
    }
}
pub(crate) struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    slave: Option<Box<dyn SlavePty + Send>>,
    io: Option<FileDescriptor>,
    child: Option<Box<dyn Child + Send + Sync>>,
    native: Option<ProcessIdentity>,
    exit: Option<ChildExit>,
    owned: BTreeMap<i64, ProcessIdentity>,
    group_closed: bool,
    shutdown_clock: Option<ShutdownClock>,
    #[cfg(test)]
    ownership_diagnostics: Option<OwnershipDiagnosticBuffer>,
    #[cfg(test)]
    pending_ownership_refresh: Option<PendingOwnershipRefresh>,
}
impl PtySession {
    pub(crate) fn has_child(&self) -> bool {
        self.child.is_some()
    }
    pub(crate) fn prepare(size: TerminalSize) -> Result<Self> {
        size.validate()?;
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_error)?;
        ensure(
            pair.master.as_raw_fd().is_some(),
            "Unix PTY master descriptor required",
        )?;
        let mut descriptor =
            FileDescriptor::dup(&MasterFd(pair.master.as_ref())).map_err(io_error)?;
        descriptor.set_non_blocking(true).map_err(io_error)?;
        Ok(Self {
            master: Some(pair.master),
            slave: Some(pair.slave),
            io: Some(descriptor),
            child: None,
            native: None,
            exit: None,
            owned: BTreeMap::new(),
            group_closed: false,
            shutdown_clock: None,
            #[cfg(test)]
            ownership_diagnostics: None,
            #[cfg(test)]
            pending_ownership_refresh: None,
        })
    }
    pub(crate) fn spawn(&mut self, spec: &SpawnSpec) -> Result<ProcessIdentity> {
        ensure(
            self.native.is_none() && self.child.is_none(),
            "native child spawn is single-use",
        )?;
        spec.validate()?;
        let mut command = CommandBuilder::new(&spec.argv[0]);
        command.args(&spec.argv[1..]);
        command.cwd(&spec.cwd);
        for (key, value) in &spec.environment {
            command.env(key, value);
        }
        let child = self
            .slave
            .as_ref()
            .ok_or_else(|| error("prepared PTY slave required"))?
            .spawn_command(command)
            .map_err(|_| error("native child spawn failed"))?;
        let pid = i64::from(
            child
                .process_id()
                .ok_or_else(|| error("Unix native child PID required"))?,
        );
        self.child = Some(child);
        self.slave.take();
        let native = ProcessIdentity::read(pid)?
            .ok_or_else(|| error("native child start identity unavailable"))?;
        self.native = Some(native.clone());
        self.owned.insert(pid, native.clone());
        #[cfg(test)]
        self.refresh_owned_traced(OwnershipRefreshPhase::Spawn)?;
        #[cfg(not(test))]
        self.refresh_owned()?;
        Ok(native)
    }
    pub(crate) fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        if self.exit.is_some() {
            return Ok(self.exit);
        }
        if self.child.is_none() {
            return Ok(None);
        }
        #[cfg(test)]
        self.refresh_owned_traced(OwnershipRefreshPhase::TryWait)?;
        #[cfg(not(test))]
        self.refresh_owned()?;
        let child = self
            .child
            .as_mut()
            .expect("checked child")
            .as_mut()
            .as_any_mut()
            .downcast_mut::<std::process::Child>()
            .ok_or_else(|| error("Unix portable-pty child type invariant violated"))?;
        if let Some(status) = child.try_wait().map_err(io_error)? {
            self.exit = Some(match status.signal() {
                Some(signal) => ChildExit::Signal(signal),
                None => ChildExit::Code(
                    status
                        .code()
                        .ok_or_else(|| error("native exit lacks code or signal"))?,
                ),
            });
            self.begin_shutdown(ShutdownClock::new(Instant::now()));
        }
        Ok(self.exit)
    }
    fn refresh_owned(&mut self) -> Result<()> {
        self.refresh_owned_with(&mut NativeProcessObservations)
    }

    fn refresh_owned_with(&mut self, observed: &mut impl ProcessObservations) -> Result<()> {
        let Some(native) = &self.native else {
            #[cfg(test)]
            self.note_ownership_outcome(OwnershipRefreshOutcome::NoNative);
            return Ok(());
        };
        if self.group_closed {
            #[cfg(test)]
            self.note_ownership_outcome(OwnershipRefreshOutcome::AlreadyClosed);
            return Ok(());
        }
        let group = i32::try_from(native.pid).map_err(io_error)?;
        let members = observed.members(group)?;
        #[cfg(test)]
        if let Some(record) = self.pending_ownership_refresh.as_mut() {
            record.sampled_member_count = Some(members.len());
        }
        if members.is_empty() {
            self.group_closed = true;
            #[cfg(test)]
            self.note_ownership_outcome(OwnershipRefreshOutcome::NoMembers);
            return Ok(());
        }
        let mut active = Vec::new();
        for pid in members {
            if let Some(process) = observed.identity(pid)?
                && !observed.stopped(pid)?
            {
                active.push(process);
            }
        }
        if active.is_empty() {
            self.group_closed = true;
            #[cfg(test)]
            if let Some(record) = self.pending_ownership_refresh.as_mut() {
                record.observe_decision(&active, &self.owned, None);
                record.outcome = OwnershipRefreshOutcome::NoActive;
            }
            return Ok(());
        }
        let anchored = self.owned.values().try_fold(
            self.child.is_some() && self.exit.is_none(),
            |found, identity| {
                let result = observed
                    .identity(identity.pid)
                    .map(|current| current.as_ref() == Some(identity));
                #[cfg(test)]
                if let Some(record) = self.pending_ownership_refresh.as_mut() {
                    record.observe_match(identity.pid, &result);
                }
                result.map(|matching| found || matching)
            },
        );
        #[cfg(test)]
        if let Some(record) = self.pending_ownership_refresh.as_mut() {
            record.observe_decision(&active, &self.owned, anchored.as_ref().ok().copied());
            record.outcome = match &anchored {
                Ok(true) => OwnershipRefreshOutcome::Anchored,
                Ok(false) => OwnershipRefreshOutcome::Unanchored,
                Err(_) => OwnershipRefreshOutcome::Error,
            };
        }
        let anchored = anchored?;
        if !anchored {
            let recheck = observed.members(group);
            #[cfg(test)]
            if let Some(record) = self.pending_ownership_refresh.as_mut() {
                record.group_recheck = Some(match &recheck {
                    Ok(members) if members.is_empty() => OwnershipGroupRecheck::Empty,
                    Ok(members) => OwnershipGroupRecheck::Nonempty {
                        members: members.len(),
                    },
                    Err(_) => OwnershipGroupRecheck::Error,
                });
                if recheck.is_err() {
                    record.outcome = OwnershipRefreshOutcome::Error;
                }
            }
            if recheck?.is_empty() {
                self.group_closed = true;
                #[cfg(test)]
                self.note_ownership_outcome(OwnershipRefreshOutcome::FreshEmpty);
                return Ok(());
            }
        }
        ensure(anchored, "owned process group lost its pinned generation")?;
        for process in active {
            if let Some(prior) = self.owned.get(&process.pid) {
                ensure(
                    prior == &process,
                    "owned PID was replaced by another generation",
                )?;
            } else {
                self.owned.insert(process.pid, process);
            }
        }
        Ok(())
    }
    pub(crate) fn signal_verified(&mut self, signal: nix::sys::signal::Signal) -> Result<()> {
        if self.native.is_none()
            && self.exit.is_none()
            && let Some(child) = self.child.as_mut()
        {
            let owned = child
                .as_mut()
                .as_any_mut()
                .downcast_mut::<std::process::Child>()
                .ok_or_else(|| error("Unix portable-pty child type invariant violated"))?;
            match nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(owned.id()).map_err(io_error)?),
                signal,
            ) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(cause) => return Err(io_error(cause)),
            }
        }
        #[cfg(test)]
        self.refresh_owned_traced(OwnershipRefreshPhase::Signal(signal))?;
        #[cfg(not(test))]
        self.refresh_owned()?;
        for process in self.owned.values() {
            if stopped(process.pid)? {
                continue;
            }
            if let Some(current) = ProcessIdentity::read(process.pid)? {
                ensure(
                    current == *process,
                    "process generation changed before termination",
                )?;
                if stopped(process.pid)? {
                    continue;
                }
                match nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(i32::try_from(process.pid).map_err(io_error)?),
                    signal,
                ) {
                    Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                    Err(cause) => return Err(io_error(cause)),
                }
            }
        }
        Ok(())
    }
    pub(crate) fn survivors(&mut self) -> Result<bool> {
        #[cfg(test)]
        self.refresh_owned_traced(OwnershipRefreshPhase::Survivors)?;
        #[cfg(not(test))]
        self.refresh_owned()?;
        for process in self.owned.values() {
            if stopped(process.pid)? {
                continue;
            }
            if let Some(current) = ProcessIdentity::read(process.pid)? {
                ensure(
                    current == *process,
                    "owned PID generation changed during cleanup",
                )?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn begin_shutdown(&mut self, clock: ShutdownClock) -> ShutdownClock {
        let first = match self.shutdown_clock {
            Some(prior) if prior.child_deadline() <= clock.child_deadline() => prior,
            _ => clock,
        };
        self.shutdown_clock = Some(first);
        first
    }
    pub(crate) fn shutdown(&mut self, clock: ShutdownClock) -> ShutdownOutcome {
        let clock = self.begin_shutdown(clock);
        let mut diagnostics = Vec::new();
        if let Err(cause) = self.signal_verified(nix::sys::signal::Signal::SIGTERM) {
            diagnostics.push(cause.message().into_owned());
        }
        while Instant::now() < clock.child_deadline() {
            if let Err(cause) = self.try_wait() {
                diagnostics.push(cause.message().into_owned());
                break;
            }
            match self.survivors() {
                Ok(false) => break,
                Ok(true) => {}
                Err(cause) => {
                    diagnostics.push(cause.message().into_owned());
                    break;
                }
            }
            let mut bytes = [0; 16384];
            match self.try_read(&mut bytes) {
                Ok(_) => {}
                Err(cause) if cause.kind() == io::ErrorKind::WouldBlock => {}
                Err(cause) => {
                    diagnostics.push(cause.to_string());
                    break;
                }
            }
            std::thread::sleep(
                Duration::from_millis(5).min(
                    clock
                        .child_deadline()
                        .saturating_duration_since(Instant::now()),
                ),
            );
        }
        if let Err(cause) = self.signal_verified(nix::sys::signal::Signal::SIGKILL) {
            diagnostics.push(cause.message().into_owned());
        }
        if let Err(cause) = self.close() {
            diagnostics.push(cause.message().into_owned());
        }
        while Instant::now() < clock.storage_deadline() {
            let waited = self.try_wait();
            let alive = self.survivors();
            match (waited, alive) {
                (Ok(status), Ok(false)) if status.is_some() || self.native.is_none() => break,
                (Err(cause), _) | (_, Err(cause)) => {
                    diagnostics.push(cause.message().into_owned());
                    break;
                }
                _ => {}
            }
            std::thread::sleep(
                Duration::from_millis(2).min(
                    clock
                        .storage_deadline()
                        .saturating_duration_since(Instant::now()),
                ),
            );
        }
        match self.survivors() {
            Ok(true) => diagnostics.push("owned descendants remain unconfirmed".into()),
            Err(cause) => diagnostics.push(cause.message().into_owned()),
            Ok(false) => {}
        }
        if self.child.is_some() && self.native.is_none() {
            diagnostics.push(
                "native start identity unavailable; descendant cleanup remains unverified".into(),
            );
        }
        if self.child.is_some() && self.exit.is_none() {
            diagnostics.push("native child reaping remains unconfirmed".into());
        }
        ShutdownOutcome {
            native_exit: self.exit,
            cleanup_state: if diagnostics.is_empty() {
                CleanupState::Complete
            } else {
                CleanupState::Incomplete
            },
            diagnostics,
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        }
    }
}
impl ByteInput for PtySession {
    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self.io.as_mut() {
            Some(file) => match file.read(bytes) {
                Err(cause) if cause.raw_os_error() == Some(nix::libc::EIO) => Ok(0),
                result => result,
            },
            None => Ok(0),
        }
    }
}
impl PtyIo for PtySession {
    fn try_write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.io
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .write(bytes)
    }
    fn resize(&mut self, size: TerminalSize) -> Result<()> {
        size.validate()?;
        self.master
            .as_ref()
            .ok_or_else(|| error("PTY is closed"))?
            .resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_error)
    }
    fn close(&mut self) -> Result<()> {
        self.io.take();
        self.master.take();
        self.slave.take();
        Ok(())
    }
}
impl Drop for PtySession {
    fn drop(&mut self) {
        if self.child.is_some() && (self.exit.is_none() || self.master.is_some()) {
            let outcome = self.shutdown(ShutdownClock::new(Instant::now()));
            for diagnostic in outcome.diagnostics {
                eprintln!("{diagnostic}");
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod setuid_member_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    const SETUID_HELPER: &str = "/usr/bin/su";

    fn read_until(session: &mut PtySession, needles: &[&str]) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        loop {
            let mut bytes = [0u8; 1024];
            match session.try_read(&mut bytes) {
                Ok(count) => output.extend_from_slice(&bytes[..count]),
                Err(cause) if cause.kind() == io::ErrorKind::WouldBlock => {}
                Err(cause) => panic!("setuid fixture PTY read failed: {cause}"),
            }
            let text = String::from_utf8_lossy(&output).into_owned();
            if needles.iter().all(|needle| text.contains(needle)) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "setuid fixture output lacks {needles:?}: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn refresh_pins_a_live_setuid_root_group_member_and_cleanup_terminates_it() {
        let helper = std::fs::metadata(SETUID_HELPER).unwrap();
        assert!(
            helper.uid() == 0 && helper.mode() & 0o4000 != 0,
            "{SETUID_HELPER} must be setuid root for this fixture"
        );
        let mut session = PtySession::prepare(TerminalSize { rows: 24, cols: 80 }).unwrap();
        let spec = SpawnSpec {
            argv: vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("{SETUID_HELPER} root </dev/tty & printf 'helper=%s\\n' \"$!\"; wait"),
            ],
            cwd: PathBuf::from("/"),
            environment: Vec::new(),
            evidence_policy: serde_json::json!({"mode":"metadata"}),
            size: TerminalSize { rows: 24, cols: 80 },
        };
        let native = session.spawn(&spec).unwrap();
        let output = read_until(&mut session, &["helper=", "Password:"]);
        let pid = output
            .lines()
            .find_map(|line| line.trim().strip_prefix("helper="))
            .expect("fixture announces the helper PID")
            .trim()
            .parse::<i64>()
            .unwrap();
        assert_eq!(
            session.try_wait().unwrap(),
            None,
            "a live setuid-root group member keeps the ownership refresh readable"
        );
        let helper = ProcessIdentity::read(pid)
            .unwrap()
            .expect("the prompting setuid helper is live");
        assert_eq!(session.owned.get(&pid), Some(&helper));
        assert_eq!(session.owned.get(&native.pid), Some(&native));
        let outcome = session.shutdown(ShutdownClock::new(Instant::now()));
        assert_eq!(outcome.cleanup_state, CleanupState::Complete, "{outcome:?}");
        assert!(outcome.native_exit.is_some(), "{outcome:?}");
        assert!(
            ProcessIdentity::read(pid).unwrap().is_none() || stopped(pid).unwrap(),
            "the setuid helper remains running after cleanup"
        );
    }
}

#[cfg(test)]
mod mechanisms_tests {
    use std::fs::File;
    use std::io::ErrorKind;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::runtime::runner::test_support::Fixture;

    struct OwnedPty {
        session: PtySession,
        clock: Option<ShutdownClock>,
        cleaned: bool,
    }

    impl OwnedPty {
        fn prepare(size: TerminalSize) -> Self {
            Self {
                session: PtySession::prepare(size).unwrap(),
                clock: None,
                cleaned: false,
            }
        }
        fn stop(&mut self) -> ShutdownOutcome {
            let clock = *self
                .clock
                .get_or_insert_with(|| ShutdownClock::new(Instant::now()));
            let outcome = self.session.shutdown(clock);
            self.cleaned = outcome.cleanup_state == CleanupState::Complete;
            outcome
        }
    }

    impl Drop for OwnedPty {
        fn drop(&mut self) {
            if !self.cleaned {
                let _ = self.stop();
            }
        }
    }

    fn collect(session: &mut PtySession) -> (Vec<u8>, ChildExit) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        let mut status = None;
        loop {
            let mut bytes = [0u8; 4096];
            match (session.try_read(&mut bytes), status) {
                (Ok(0), Some(exit)) => return (output, exit),
                (Ok(count), _) => output.extend_from_slice(&bytes[..count]),
                (Err(error), _) if error.kind() == ErrorKind::WouldBlock => {}
                (Err(error), _) => panic!("fixture PTY read failed: {error}"),
            }
            if status.is_none() {
                status = session.try_wait().unwrap();
            }
            if let Some(exit) = status {
                let mut tail = [0u8; 4096];
                match session.try_read(&mut tail) {
                    Ok(0) => return (output, exit),
                    Ok(count) => output.extend_from_slice(&tail[..count]),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                    Err(error) => panic!("fixture PTY tail failed: {error}"),
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture child did not exit/drain within its deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn runner_real_pty_prepares_without_a_child_and_spawns_literal_argv_cwd_and_environment_once() {
        let fixture = Fixture::new();
        let literal = "assignment '$x' `backticks` 世界";
        let mut spec = fixture.spec();
        spec.argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '%s\\n' \"$1\"; printf '%s\\n' \"$CAFLEET_FIXTURE_MARKER\"; if [ \"$PATH\" = \"$2\" ]; then printf 'inherited-path\\n'; else exit 11; fi; pwd; exit 7".into(),
            "cafleet-fixture".into(),
            literal.into(),
            std::env::var("PATH").expect("the fixture inherits the host execution path"),
        ];
        spec.environment
            .push(("CAFLEET_FIXTURE_MARKER".into(), "literal-marker-$x".into()));
        let mut owned = OwnedPty::prepare(spec.size);
        assert_eq!(owned.session.try_wait().unwrap(), None);
        let process = owned.session.spawn(&spec).unwrap();
        assert!(process.pid > 0);
        assert!(!process.start_identity.is_empty());
        assert!(owned.session.spawn(&spec).is_err());
        let (output, status) = collect(&mut owned.session);
        let cleanup = owned.stop();
        assert_eq!(cleanup.cleanup_state, CleanupState::Complete);
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains(literal));
        assert!(text.contains("literal-marker-$x"));
        assert!(text.contains("inherited-path"));
        assert!(text.contains(std::fs::canonicalize(&spec.cwd).unwrap().to_str().unwrap()));
        assert_eq!(status, ChildExit::Code(7));
    }

    #[test]
    fn runner_real_pty_child_receives_the_forwarded_config_location() {
        let fixture = Fixture::new();
        let mut spec = fixture.spec();
        let database_url = spec
            .environment
            .iter()
            .find(|(key, _)| key == "CAFLEET_DATABASE_URL")
            .map(|(_, value)| value.clone())
            .expect("fixture spawn environment carries the database URL");
        spec.environment = crate::coding_agent::CodingAgent::Opencode
            .spawn_environment(&database_url, &|name| {
                (name == "OPENCODE_CONFIG_DIR").then(|| "/cfg/open code".to_string())
            })
            .unwrap();
        spec.argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf 'config=%s\\n' \"$OPENCODE_CONFIG_DIR\"; printf 'database=%s\\n' \"$CAFLEET_DATABASE_URL\"".into(),
        ];
        let mut owned = OwnedPty::prepare(spec.size);
        owned.session.spawn(&spec).unwrap();
        let (output, status) = collect(&mut owned.session);
        assert_eq!(owned.stop().cleanup_state, CleanupState::Complete);
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("config=/cfg/open code"), "{text:?}");
        assert!(
            text.contains(&format!("database={database_url}")),
            "{text:?}"
        );
        assert_eq!(status, ChildExit::Code(0));
    }

    #[test]
    fn runner_real_pty_retains_the_numeric_native_signal_exit() {
        let fixture = Fixture::new();
        let mut spec = fixture.spec();
        spec.argv = vec!["/bin/sh".into(), "-c".into(), "kill -TERM $$".into()];
        let mut owned = OwnedPty::prepare(spec.size);
        owned.session.spawn(&spec).unwrap();
        let (_, status) = collect(&mut owned.session);
        assert_eq!(owned.stop().cleanup_state, CleanupState::Complete);
        assert_eq!(status, ChildExit::Signal(15));
    }

    fn assert_terminal_settings_restored(terminal: &File, expected: &nix::sys::termios::Termios) {
        let actual = nix::sys::termios::tcgetattr(terminal).unwrap();
        #[cfg(target_os = "macos")]
        {
            // macOS sets PENDIN when TCSANOW restores canonical input.
            let mut actual: nix::libc::termios = actual.into();
            let mut expected: nix::libc::termios = expected.clone().into();
            actual.c_lflag &= !nix::libc::PENDIN;
            expected.c_lflag &= !nix::libc::PENDIN;
            assert_eq!(actual, expected);
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(&actual, expected);
    }

    #[test]
    fn runner_real_raw_terminal_guard_restores_only_its_owned_fixture_terminal() {
        let pty = nix::pty::openpty(None, None).unwrap();
        let terminal = File::from(pty.slave);
        let before = nix::sys::termios::tcgetattr(&terminal).unwrap();
        let mut guard = RawTerminalGuard::enter(&terminal).unwrap();
        let raw = nix::sys::termios::tcgetattr(&terminal).unwrap();
        assert!(
            !raw.local_flags
                .contains(nix::sys::termios::LocalFlags::ICANON)
        );
        assert!(
            !raw.local_flags
                .contains(nix::sys::termios::LocalFlags::ECHO)
        );
        guard.restore().unwrap();
        assert_terminal_settings_restored(&terminal, &before);
        guard.restore().unwrap();
        assert_terminal_settings_restored(&terminal, &before);
        {
            let _guard = RawTerminalGuard::enter(&terminal).unwrap();
        }
        assert_terminal_settings_restored(&terminal, &before);
        let unwound = std::panic::catch_unwind(|| {
            let _guard = RawTerminalGuard::enter(&terminal).unwrap();
            panic!("fixture unwind");
        });
        assert!(unwound.is_err());
        assert_terminal_settings_restored(&terminal, &before);
        drop(pty.master);
    }

    fn read_until(session: &mut PtySession, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        loop {
            let mut bytes = [0u8; 1024];
            match session.try_read(&mut bytes) {
                Ok(count) => output.extend_from_slice(&bytes[..count]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("fixture PTY read failed: {error}"),
            }
            let text = String::from_utf8(output.clone()).unwrap();
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not publish {needle}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn runner_real_shutdown_kills_and_reaps_a_term_ignoring_owned_child_within_the_shared_deadline()
    {
        let fixture = Fixture::new();
        let mut spec = fixture.spec();
        spec.argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "trap '' TERM; printf 'ready\\n'; while :; do sleep 1; done".into(),
        ];
        let mut owned = OwnedPty::prepare(spec.size);
        let process = owned.session.spawn(&spec).unwrap();
        read_until(&mut owned.session, "ready");
        let start = Instant::now();
        let outcome = owned.stop();
        assert!(start.elapsed() < Duration::from_millis(4750));
        assert_eq!(outcome.cleanup_state, CleanupState::Complete, "{outcome:?}");
        assert_eq!(outcome.native_exit, Some(ChildExit::Signal(9)));
        assert_eq!(
            nix::sys::wait::waitpid(
                nix::unistd::Pid::from_raw(i32::try_from(process.pid).unwrap()),
                Some(nix::sys::wait::WaitPidFlag::WNOHANG)
            ),
            Err(nix::errno::Errno::ECHILD)
        );
    }

    fn report_fixture_process(stage: &str, pid: i64) {
        let identity = ProcessIdentity::read(pid);
        let pid32 = match i32::try_from(pid) {
            Ok(pid) => pid,
            Err(error) => {
                eprintln!(
                    "shutdown fixture stage={stage}; pid={pid}; identity={identity:?}; PID conversion={error}"
                );
                return;
            }
        };
        let group = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid32)));
        eprintln!(
            "shutdown fixture stage={stage}; pid={pid}; identity={identity:?}; group={group:?}"
        );
        #[cfg(target_os = "macos")]
        {
            use libproc::libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};
            let status = pidinfo::<BSDInfo>(pid32, 0).map(|info| {
                (
                    info.pbi_ppid,
                    info.pbi_pgid,
                    info.pbi_status,
                    info.pbi_start_tvsec,
                    info.pbi_start_tvusec,
                )
            });
            eprintln!(
                "shutdown fixture stage={stage}; pid={pid}; BSD parent/group/status/start_sec/start_usec={status:?}"
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            use std::io::Read;
            let status = File::open(format!("/proc/{pid}/stat")).and_then(|file| {
                let mut text = String::new();
                file.take(4096).read_to_string(&mut text)?;
                Ok(text)
            });
            eprintln!("shutdown fixture stage={stage}; pid={pid}; bounded process stat={status:?}");
        }
    }

    fn report_fixture_group(stage: &str, native: &ProcessIdentity, descendant: i32) {
        eprintln!(
            "shutdown fixture stage={stage}; original native={native:?}; announced descendant={descendant}"
        );
        report_fixture_process(stage, native.pid);
        report_fixture_process(stage, i64::from(descendant));
        #[cfg(target_os = "macos")]
        {
            nix::errno::Errno::clear();
            let members =
                libproc::processes::pids_by_type(libproc::processes::ProcFilter::ByProgramGroup {
                    pgrpid: u32::try_from(native.pid).expect("spawned native PID is positive"),
                });
            match members {
                Ok(members) => {
                    eprintln!(
                        "shutdown fixture stage={stage}; observed original group members={}; diagnostic record limit=16",
                        members.len()
                    );
                    for pid in members.into_iter().take(16) {
                        report_fixture_process(stage, i64::from(pid));
                    }
                }
                Err(error) => {
                    eprintln!("shutdown fixture stage={stage}; original group enumeration={error}")
                }
            }
        }
    }

    #[test]
    fn runner_real_shutdown_closes_a_slave_retaining_descendant_and_preserves_the_native_zero_exit()
    {
        let fixture = Fixture::new();
        let ready = fixture._dir.path().join("descendant-ready");
        let mut spec = fixture.spec();
        // The descendant must not fork: a new group member appearing during KILL enumeration is unpinned.
        spec.argv=vec!["/bin/sh".into(),"-c".into(),r#"/bin/sh -c 'trap "" TERM HUP; printf ready > "$1"; exec /bin/sleep 600' descendant "$1" & descendant=$!; while [ ! -f "$1" ]; do sleep 0.01; done; printf 'descendant=%s\n' "$descendant"; exit 0"#.into(),"parent".into(),ready.to_str().unwrap().into()];
        let mut owned = OwnedPty::prepare(spec.size);
        owned.session.enable_ownership_diagnostics().unwrap();
        let native = owned.session.spawn(&spec).unwrap();
        eprintln!("shutdown fixture spawned native={native:?}");
        let output = read_until(&mut owned.session, "\n");
        let pid = output
            .lines()
            .find_map(|line| line.trim().strip_prefix("descendant="))
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        report_fixture_group("descendant announced before exit polling", &native, pid);
        let wait_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let waited = owned.session.try_wait();
            if waited.is_err() {
                eprintln!("shutdown fixture native wait error={waited:?}");
                report_fixture_group("native wait error", &native, pid);
            }
            if let Some(exit) = waited.unwrap() {
                eprintln!("shutdown fixture observed native exit={exit:?}");
                report_fixture_group("native exit observed", &native, pid);
                assert_eq!(exit, ChildExit::Code(0));
                break;
            }
            assert!(Instant::now() < wait_deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let start = Instant::now();
        let outcome = owned.stop();
        let elapsed = start.elapsed();
        let ownership = owned.session.take_ownership_diagnostics();
        eprintln!("shutdown fixture bounded ownership refresh trace={ownership:?}");
        eprintln!("shutdown fixture cleanup elapsed={elapsed:?}; outcome={outcome:?}");
        report_fixture_group("cleanup returned", &native, pid);
        assert!(elapsed < Duration::from_millis(4750));
        assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
        assert_eq!(outcome.cleanup_state, CleanupState::Complete, "{outcome:?}");
        let ownership = ownership.expect("enabled ownership diagnostics are collected once");
        assert!(
            !ownership.records.is_empty(),
            "actual refresh observations were recorded"
        );
        let observation = std::process::Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        assert!(observation.status.success() || observation.status.code() == Some(1));
        let state = String::from_utf8(observation.stdout).unwrap();
        assert!(
            state.trim().is_empty() || state.trim().starts_with('Z'),
            "owned descendant remains running: {state}"
        );
    }
}

#[cfg(test)]
mod ownership_refresh_tests {
    use super::*;
    use std::collections::VecDeque;

    const GROUP: i32 = 41001;
    const A: i64 = 41001;
    const B: i64 = 41002;
    const C: i64 = 41003;

    #[derive(Debug)]
    enum Step {
        Members(i32, Result<Vec<i64>>),
        Identity(i64, Result<Option<ProcessIdentity>>),
        Stopped(i64, Result<bool>),
    }

    struct Script(VecDeque<Step>);

    impl Script {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self(steps.into_iter().collect())
        }

        fn next(&mut self) -> Step {
            self.0.pop_front().expect("every observation is scripted")
        }

        fn finished(self) {
            assert!(self.0.is_empty(), "unused observations: {:?}", self.0);
        }
    }

    impl ProcessObservations for Script {
        fn members(&mut self, group: i32) -> Result<Vec<i64>> {
            let Step::Members(expected, result) = self.next() else {
                panic!("expected the next scripted operation to enumerate group {group}");
            };
            assert_eq!(group, expected);
            result
        }

        fn identity(&mut self, pid: i64) -> Result<Option<ProcessIdentity>> {
            let Step::Identity(expected, result) = self.next() else {
                panic!("expected the next scripted operation to identify PID {pid}");
            };
            assert_eq!(pid, expected);
            result
        }

        fn stopped(&mut self, pid: i64) -> Result<bool> {
            let Step::Stopped(expected, result) = self.next() else {
                panic!("expected the next scripted operation to inspect stopped PID {pid}");
            };
            assert_eq!(pid, expected);
            result
        }
    }

    fn identity(pid: i64) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            start_identity: format!("synthetic-generation-{pid}"),
        }
    }

    fn replacement(pid: i64) -> ProcessIdentity {
        ProcessIdentity {
            pid,
            start_identity: format!("synthetic-replacement-{pid}"),
        }
    }

    fn members(pids: &[i64]) -> Step {
        Step::Members(GROUP, Ok(pids.to_vec()))
    }

    fn observed(process: ProcessIdentity) -> Step {
        Step::Identity(process.pid, Ok(Some(process)))
    }

    fn absent(pid: i64) -> Step {
        Step::Identity(pid, Ok(None))
    }

    fn active(process: ProcessIdentity) -> [Step; 2] {
        [
            observed(process.clone()),
            Step::Stopped(process.pid, Ok(false)),
        ]
    }

    fn inert(pinned: &[ProcessIdentity], traced: bool) -> PtySession {
        let origin = Instant::now();
        let mut session = PtySession {
            master: None,
            slave: None,
            io: None,
            child: None,
            native: None,
            exit: Some(ChildExit::Code(0)),
            owned: pinned.iter().cloned().map(|p| (p.pid, p)).collect(),
            group_closed: false,
            shutdown_clock: Some(
                ShutdownClock::new(origin).clipped(origin + Duration::from_millis(900)),
            ),
            ownership_diagnostics: None,
            pending_ownership_refresh: None,
        };
        if traced {
            session.enable_ownership_diagnostics().unwrap();
        }
        session.native = Some(identity(A));
        session
    }

    fn boundaries(session: &PtySession) -> [Instant; 3] {
        let clock = session
            .shutdown_clock
            .expect("fixture retains its original clock");
        [
            clock.child_deadline(),
            clock.display_deadline(),
            clock.storage_deadline(),
        ]
    }

    fn assert_state(
        session: &PtySession,
        pinned: &BTreeMap<i64, ProcessIdentity>,
        closed: bool,
        clock: [Instant; 3],
    ) {
        assert_eq!(&session.owned, pinned);
        assert_eq!(session.group_closed, closed);
        assert_eq!(boundaries(session), clock);
        assert!(session.master.is_none());
        assert!(session.slave.is_none());
        assert!(session.io.is_none());
        assert!(session.child.is_none());
        assert_eq!(session.native, Some(identity(A)));
        assert_eq!(session.exit, Some(ChildExit::Code(0)));
        assert!(session.pending_ownership_refresh.is_none());
    }

    fn assert_error(actual: Result<()>, expected: &CafleetError) {
        let actual = actual.expect_err("the exact scripted failure must propagate");
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    }

    fn baseline() {
        let mut session = inert(&[identity(A)], false);
        let clock = boundaries(&session);
        let mut script = Script::new(
            [members(&[A, B])]
                .into_iter()
                .chain(active(identity(A)))
                .chain(active(identity(B)))
                .chain([observed(identity(A))]),
        );
        session.refresh_owned_with(&mut script).unwrap();
        script.finished();
        let expected = [(A, identity(A)), (B, identity(B))].into_iter().collect();
        assert_state(&session, &expected, false, clock);
    }

    fn false_fold(recheck: Step) -> Script {
        Script::new(
            [members(&[A])]
                .into_iter()
                .chain(active(identity(A)))
                .chain([absent(A), recheck]),
        )
    }

    fn traced(session: &mut PtySession, script: &mut Script) -> Result<()> {
        session.refresh_owned_traced_with(OwnershipRefreshPhase::TryWait, script)
    }

    fn assert_false_record(record: &OwnershipRefreshRecord, outcome: OwnershipRefreshOutcome) {
        assert_eq!(record.phase, OwnershipRefreshPhase::TryWait);
        assert!(!record.child_present);
        assert_eq!(record.native_exit, Some(ChildExit::Code(0)));
        assert!(!record.seeded_anchor);
        assert!(!record.group_closed_before);
        assert_eq!(record.sampled_member_count, Some(1));
        assert_eq!(record.anchored, Some(false));
        assert_eq!(record.outcome, outcome);
        let active = record
            .active
            .as_ref()
            .expect("actual active scan is retained");
        assert_eq!(active.total, 1);
        assert_eq!(active.entries[0].pid, A);
        assert_eq!(record.pinned.total, 1);
        assert_eq!(record.pinned.entries[0].pid, A);
        assert_eq!(record.matches.attempted, 1);
        assert_eq!(record.matches.entries.len(), 1);
        assert_eq!(record.matches.entries[0].pid, A);
        assert_eq!(
            record.matches.entries[0].outcome,
            OwnershipMatchOutcome::DifferentOrAbsent
        );
    }

    #[test]
    fn step5_refresh_matching_anchor_adopts_only_original_active_generations() {
        baseline();
    }

    #[test]
    fn step5_refresh_initial_empty_closes_with_one_query_and_retains_map_and_clock() {
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new([members(&[])]);
        session.refresh_owned_with(&mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
    }

    #[test]
    fn step5_refresh_stale_active_then_fresh_empty_closes_without_adoption_or_more_reads() {
        baseline();
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new(
            [members(&[A, B])]
                .into_iter()
                .chain(active(identity(A)))
                .chain(active(identity(B)))
                .chain([absent(A), members(&[])]),
        );
        session.refresh_owned_with(&mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
    }

    #[test]
    fn step5_refresh_fresh_remaining_members_preserve_exact_refusal_without_adoption() {
        for fresh in [vec![B], vec![A], vec![B, C]] {
            baseline();
            let mut session = inert(&[identity(A)], false);
            let pinned = session.owned.clone();
            let clock = boundaries(&session);
            let mut script = false_fold(members(&fresh));
            assert_error(
                session.refresh_owned_with(&mut script),
                &error("owned process group lost its pinned generation"),
            );
            script.finished();
            assert_state(&session, &pinned, false, clock);
        }
    }

    #[test]
    fn step5_refresh_same_pid_replacement_cannot_supply_an_anchor_or_be_adopted() {
        baseline();
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new(
            [members(&[A])]
                .into_iter()
                .chain(active(identity(A)))
                .chain([observed(replacement(A)), members(&[A])]),
        );
        assert_error(
            session.refresh_owned_with(&mut script),
            &error("owned process group lost its pinned generation"),
        );
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }

    #[test]
    fn step5_refresh_original_observation_errors_stop_at_the_actual_first_failure() {
        let failure = CafleetError::DatabaseBusy("synthetic-original-observation".into());
        for prefix in [
            vec![Step::Members(GROUP, Err(failure.clone()))],
            vec![members(&[A, B]), Step::Identity(A, Err(failure.clone()))],
            vec![
                members(&[A, B]),
                observed(identity(A)),
                Step::Stopped(A, Err(failure.clone())),
            ],
            vec![
                members(&[A]),
                observed(identity(A)),
                Step::Stopped(A, Ok(false)),
                Step::Identity(A, Err(failure.clone())),
            ],
        ] {
            baseline();
            let mut session = inert(&[identity(A)], false);
            let pinned = session.owned.clone();
            let clock = boundaries(&session);
            let mut script = Script::new(prefix);
            assert_error(session.refresh_owned_with(&mut script), &failure);
            script.finished();
            assert_state(&session, &pinned, false, clock);
        }
    }

    #[test]
    fn step5_refresh_anchor_fold_still_reads_later_pins_after_a_match_and_propagates_error() {
        baseline();
        let failure = CafleetError::Value("synthetic-later-anchor-read".into());
        let mut session = inert(&[identity(A), identity(C)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new(
            [members(&[A])]
                .into_iter()
                .chain(active(identity(A)))
                .chain([
                    observed(identity(A)),
                    Step::Identity(C, Err(failure.clone())),
                ]),
        );
        assert_error(session.refresh_owned_with(&mut script), &failure);
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }

    #[test]
    fn step5_refresh_fresh_query_error_propagates_without_closure_or_followup_reads() {
        baseline();
        let failure = CafleetError::DatabaseBusy("synthetic-fresh-enumeration".into());
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = false_fold(Step::Members(GROUP, Err(failure.clone())));
        assert_error(session.refresh_owned_with(&mut script), &failure);
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }

    #[test]
    fn step5_refresh_anchored_pid_mismatch_retains_fence_without_fresh_query_fallback() {
        baseline();
        let mut session = inert(&[identity(A), identity(C)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new(
            [members(&[A, C])]
                .into_iter()
                .chain(active(replacement(A)))
                .chain(active(identity(C)))
                .chain([observed(replacement(A)), observed(identity(C))]),
        );
        assert_error(
            session.refresh_owned_with(&mut script),
            &error("owned PID was replaced by another generation"),
        );
        script.finished();
        assert_state(&session, &pinned, false, clock);
    }

    #[cfg(target_os = "linux")]
    mod reaped_stat {
        use super::*;

        fn reaped_stat_read() -> io::Error {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let mut stat = std::fs::File::open(format!("/proc/{}/stat", child.id())).unwrap();
            child.wait().unwrap();
            let mut text = String::new();
            stat.read_to_string(&mut text)
                .expect_err("a stat read of a reaped task fails")
        }

        #[test]
        fn runner_process_stat_read_of_a_task_reaped_after_open_fails_with_esrch() {
            assert_eq!(reaped_stat_read().raw_os_error(), Some(nix::libc::ESRCH));
        }

        #[test]
        fn runner_process_esrch_stat_read_is_a_vanished_process() {
            assert_eq!(
                ProcessIdentity::from_stat(B, Err(reaped_stat_read())).unwrap(),
                None
            );
            assert!(stopped_from_stat(Err(reaped_stat_read())).unwrap());
        }

        #[test]
        fn runner_process_other_stat_read_errors_still_fail() {
            let denied = || io::Error::from_raw_os_error(nix::libc::EACCES);
            assert!(ProcessIdentity::from_stat(B, Err(denied())).is_err());
            assert!(stopped_from_stat(Err(denied())).is_err());
        }

        #[test]
        fn runner_process_live_stat_text_keeps_its_identity_and_running_state() {
            let pid = i64::from(std::process::id());
            let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            assert_eq!(
                ProcessIdentity::from_stat(pid, Ok(text.clone())).unwrap(),
                Some(ProcessIdentity::current().unwrap())
            );
            assert!(!stopped_from_stat(Ok(text)).unwrap());
        }

        #[test]
        fn runner_refresh_member_reaped_between_enumeration_and_stat_read_keeps_the_runner_alive() {
            for vanished in [
                vec![Step::Identity(
                    B,
                    ProcessIdentity::from_stat(B, Err(reaped_stat_read())),
                )],
                vec![
                    observed(identity(B)),
                    Step::Stopped(B, stopped_from_stat(Err(reaped_stat_read()))),
                ],
            ] {
                let mut session = inert(&[identity(A)], false);
                let clock = boundaries(&session);
                let mut script = Script::new(
                    [members(&[A, B])]
                        .into_iter()
                        .chain(active(identity(A)))
                        .chain(vanished)
                        .chain([observed(identity(A))]),
                );
                session.refresh_owned_with(&mut script).unwrap();
                script.finished();
                let pinned = [(A, identity(A))].into_iter().collect();
                assert_state(&session, &pinned, false, clock);
            }
        }
    }

    #[test]
    fn step5_refresh_absent_and_stopped_members_close_before_any_anchor_fold() {
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = Script::new([
            members(&[A, B]),
            absent(A),
            observed(identity(B)),
            Step::Stopped(B, Ok(true)),
        ]);
        session.refresh_owned_with(&mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
    }

    #[test]
    fn step5_refresh_traced_fresh_empty_records_second_observation_and_resolved_false_fold() {
        let mut session = inert(&[identity(A)], true);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = false_fold(members(&[]));
        traced(&mut session, &mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 1);
        assert_eq!(trace.records_discarded, 0);
        let record = &trace.records[0];
        assert_eq!(record.sequence, 1);
        assert_false_record(record, OwnershipRefreshOutcome::FreshEmpty);
        assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Empty));
        let first = trace
            .first_unanchored
            .expect("completed false fold is retained");
        assert_eq!(first.sequence, 1);
        assert_false_record(&first, OwnershipRefreshOutcome::FreshEmpty);
        assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Empty));
    }

    #[test]
    fn step5_refresh_traced_nonempty_records_exact_fresh_count_and_unresolved_refusal() {
        let mut session = inert(&[identity(A)], true);
        let mut script = false_fold(members(&[B, C]));
        assert_error(
            traced(&mut session, &mut script),
            &error("owned process group lost its pinned generation"),
        );
        script.finished();
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 1);
        let record = &trace.records[0];
        assert_false_record(record, OwnershipRefreshOutcome::Unanchored);
        assert_eq!(
            record.group_recheck,
            Some(OwnershipGroupRecheck::Nonempty { members: 2 })
        );
        let first = trace
            .first_unanchored
            .expect("first refusal retains its actual observation");
        assert_false_record(&first, OwnershipRefreshOutcome::Unanchored);
        assert_eq!(first.group_recheck, record.group_recheck);
    }

    #[test]
    fn step5_refresh_traced_fresh_error_retains_false_fold_without_hiding_source_error() {
        let failure = CafleetError::Value("synthetic-fresh-query-trace".into());
        let mut session = inert(&[identity(A)], true);
        let mut script = false_fold(Step::Members(GROUP, Err(failure.clone())));
        assert_error(traced(&mut session, &mut script), &failure);
        script.finished();
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 1);
        let record = &trace.records[0];
        assert_false_record(record, OwnershipRefreshOutcome::Error);
        assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Error));
        let first = trace
            .first_unanchored
            .expect("failed recheck retains its completed false fold");
        assert_false_record(&first, OwnershipRefreshOutcome::Error);
        assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Error));
    }

    #[test]
    fn step5_refresh_trace_initial_empty_and_already_closed_remain_distinct_from_recheck() {
        let mut session = inert(&[identity(A)], true);
        let clock = boundaries(&session);
        let mut script = Script::new([members(&[])]);
        traced(&mut session, &mut script).unwrap();
        script.finished();
        let mut no_reads = Script::new([]);
        session
            .refresh_owned_traced_with(OwnershipRefreshPhase::Survivors, &mut no_reads)
            .unwrap();
        no_reads.finished();
        assert_eq!(boundaries(&session), clock);
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 2);
        assert!(trace.first_unanchored.is_none());
        let initial = &trace.records[0];
        assert_eq!(initial.outcome, OwnershipRefreshOutcome::NoMembers);
        assert_eq!(initial.sampled_member_count, Some(0));
        assert_eq!(initial.anchored, None);
        assert_eq!(initial.group_recheck, None);
        let closed = &trace.records[1];
        assert_eq!(closed.phase, OwnershipRefreshPhase::Survivors);
        assert_eq!(closed.outcome, OwnershipRefreshOutcome::AlreadyClosed);
        assert!(closed.group_closed_before);
        assert_eq!(closed.sampled_member_count, None);
        assert_eq!(closed.group_recheck, None);
    }

    #[test]
    fn step5_refresh_trace_anchor_success_and_fold_error_have_no_recheck() {
        for fails in [false, true] {
            let mut session = inert(&[identity(A)], true);
            let failure = CafleetError::Value("synthetic-anchor-trace".into());
            let last = if fails {
                Step::Identity(A, Err(failure.clone()))
            } else {
                observed(identity(A))
            };
            let mut script = Script::new(
                [members(&[A])]
                    .into_iter()
                    .chain(active(identity(A)))
                    .chain([last]),
            );
            let result = traced(&mut session, &mut script);
            if fails {
                assert_error(result, &failure);
            } else {
                result.unwrap();
            }
            script.finished();
            let trace = session.take_ownership_diagnostics().unwrap();
            assert_eq!(trace.records.len(), 1);
            assert!(trace.first_unanchored.is_none());
            let record = &trace.records[0];
            assert_eq!(record.group_recheck, None);
            assert_eq!(record.anchored, if fails { None } else { Some(true) });
            assert_eq!(
                record.outcome,
                if fails {
                    OwnershipRefreshOutcome::Error
                } else {
                    OwnershipRefreshOutcome::Anchored
                }
            );
            assert_eq!(record.matches.attempted, 1);
            assert_eq!(
                record.matches.entries[0].outcome,
                if fails {
                    OwnershipMatchOutcome::Error
                } else {
                    OwnershipMatchOutcome::Matches
                }
            );
        }
    }

    #[test]
    fn step5_refresh_trace_first_resolved_false_fold_survives_ring_eviction() {
        let mut session = inert(&[identity(A)], true);
        let clock = boundaries(&session);
        let mut script = false_fold(members(&[]));
        traced(&mut session, &mut script).unwrap();
        script.finished();
        for _ in 0..70 {
            let mut no_reads = Script::new([]);
            traced(&mut session, &mut no_reads).unwrap();
            no_reads.finished();
        }
        assert_eq!(boundaries(&session), clock);
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 64);
        assert_eq!(trace.records_discarded, 7);
        assert_eq!(trace.records.first().unwrap().sequence, 8);
        assert_eq!(trace.records.last().unwrap().sequence, 71);
        assert!(
            trace
                .records
                .iter()
                .all(|r| r.outcome == OwnershipRefreshOutcome::AlreadyClosed)
        );
        let first = trace
            .first_unanchored
            .expect("resolved first false fold survives eviction");
        assert_eq!(first.sequence, 1);
        assert_false_record(&first, OwnershipRefreshOutcome::FreshEmpty);
        assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Empty));
    }

    #[test]
    fn step5_refresh_trace_first_failed_recheck_survives_later_refusal_resolution_and_eviction() {
        let mut session = inert(&[identity(A)], true);
        let failure = CafleetError::Value("synthetic-first-recheck-error".into());
        let mut first_script = false_fold(Step::Members(GROUP, Err(failure.clone())));
        assert_error(traced(&mut session, &mut first_script), &failure);
        first_script.finished();
        let mut second = false_fold(members(&[B]));
        assert_error(
            traced(&mut session, &mut second),
            &error("owned process group lost its pinned generation"),
        );
        second.finished();
        let mut third = false_fold(members(&[]));
        traced(&mut session, &mut third).unwrap();
        third.finished();
        for _ in 0..64 {
            let mut no_reads = Script::new([]);
            traced(&mut session, &mut no_reads).unwrap();
            no_reads.finished();
        }
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 64);
        assert_eq!(trace.records_discarded, 3);
        assert_eq!(trace.records.first().unwrap().sequence, 4);
        assert_eq!(trace.records.last().unwrap().sequence, 67);
        let first = trace
            .first_unanchored
            .expect("the first failed recheck remains first");
        assert_eq!(first.sequence, 1);
        assert_false_record(&first, OwnershipRefreshOutcome::Error);
        assert_eq!(first.group_recheck, Some(OwnershipGroupRecheck::Error));
    }

    #[test]
    fn step5_refresh_trace_identity_match_and_utf8_limits_also_bound_retained_false_fold() {
        let pins: Vec<_> = (A..A + 18)
            .map(|pid| ProcessIdentity {
                pid,
                start_identity: "界".repeat(50),
            })
            .collect();
        let mut session = inert(&pins, true);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut steps = vec![members(&pins.iter().map(|p| p.pid).collect::<Vec<_>>())];
        for pin in &pins {
            steps.extend(active(pin.clone()));
        }
        steps.extend(pins.iter().map(|p| absent(p.pid)));
        steps.push(members(&[]));
        let mut script = Script::new(steps);
        traced(&mut session, &mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
        let trace = session.take_ownership_diagnostics().unwrap();
        assert_eq!(trace.records.len(), 1);
        let first = trace
            .first_unanchored
            .as_ref()
            .expect("bounded false fold is retained");
        for record in [&trace.records[0], first.as_ref()] {
            assert_eq!(record.outcome, OwnershipRefreshOutcome::FreshEmpty);
            assert_eq!(record.anchored, Some(false));
            assert_eq!(record.group_recheck, Some(OwnershipGroupRecheck::Empty));
            assert_eq!(record.sampled_member_count, Some(18));
            for snapshot in [record.active.as_ref().unwrap(), &record.pinned] {
                assert_eq!(snapshot.total, 18);
                assert_eq!(snapshot.entries.len(), 16);
                assert!(snapshot.truncated);
                for (offset, entry) in snapshot.entries.iter().enumerate() {
                    assert_eq!(entry.pid, A + i64::try_from(offset).unwrap());
                    assert_eq!(entry.original_start_bytes, 150);
                    assert_eq!(entry.start_identity.len(), 126);
                    assert_eq!(entry.start_identity, "界".repeat(42));
                    assert!(entry.start_truncated);
                }
            }
            assert_eq!(record.matches.attempted, 18);
            assert_eq!(record.matches.entries.len(), 16);
            assert!(record.matches.truncated);
            for (offset, entry) in record.matches.entries.iter().enumerate() {
                assert_eq!(entry.pid, A + i64::try_from(offset).unwrap());
                assert_eq!(entry.outcome, OwnershipMatchOutcome::DifferentOrAbsent);
            }
        }
    }

    #[test]
    fn step5_refresh_traced_wrapper_without_diagnostics_uses_only_the_scripted_core() {
        let mut session = inert(&[identity(A)], false);
        let pinned = session.owned.clone();
        let clock = boundaries(&session);
        let mut script = false_fold(members(&[]));
        traced(&mut session, &mut script).unwrap();
        script.finished();
        assert_state(&session, &pinned, true, clock);
        assert!(session.ownership_diagnostics.is_none());
    }
}
