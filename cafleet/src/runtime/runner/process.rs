use super::*;
use crate::broker::runtime::CleanupState;
use filedescriptor::FileDescriptor;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::time::{Duration, Instant};

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
        use libproc::libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};
        let pid32 = i32::try_from(pid).map_err(io_error)?;
        match pidinfo::<BSDInfo>(pid32, 0) {
            Ok(info) => Ok(Some(Self {
                pid,
                start_identity: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
            })),
            Err(cause) => {
                if std::io::Error::last_os_error().raw_os_error() == Some(nix::libc::ESRCH) {
                    return Ok(None);
                }
                match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid32), None) {
                    Err(nix::errno::Errno::ESRCH) => Ok(None),
                    _ => Err(error(format!(
                        "cannot establish process start identity: {cause}"
                    ))),
                }
            }
        }
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
    use libproc::libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};
    match pidinfo::<BSDInfo>(i32::try_from(pid).map_err(io_error)?, 0) {
        Ok(info) => Ok(info.pbi_status == 5),
        Err(_) => Ok(ProcessIdentity::read(pid)?.is_none()),
    }
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
        let Some(native) = &self.native else {
            return Ok(());
        };
        if self.group_closed {
            return Ok(());
        }
        let group = i32::try_from(native.pid).map_err(io_error)?;
        let members = group_members(group)?;
        if members.is_empty() {
            self.group_closed = true;
            return Ok(());
        }
        let mut active = Vec::new();
        for pid in members {
            if let Some(process) = ProcessIdentity::read(pid)?
                && !stopped(pid)?
            {
                active.push(process);
            }
        }
        if active.is_empty() {
            self.group_closed = true;
            return Ok(());
        }
        let anchored = self.owned.values().try_fold(
            self.child.is_some() && self.exit.is_none(),
            |found, identity| identity.matches().map(|matching| found || matching),
        )?;
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
