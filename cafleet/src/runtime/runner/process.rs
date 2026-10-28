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
#[cfg(test)]
mod ownership_refresh_tests;

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
        let text = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(text) => text,
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => return Err(io_error(cause)),
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
#[cfg(not(target_os = "macos"))]
fn stopped(pid: i64) -> Result<bool> {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(text) => Ok(text
            .rsplit_once(')')
            .ok_or_else(|| error("malformed process status"))?
            .1
            .trim_start()
            .starts_with('Z')),
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(cause) => Err(io_error(cause)),
    }
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
