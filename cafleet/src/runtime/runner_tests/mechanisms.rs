use std::fs::File;
use std::io::ErrorKind;
use std::time::{Duration, Instant};

use super::*;

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
fn runner_real_shutdown_kills_and_reaps_a_term_ignoring_owned_child_within_the_shared_deadline() {
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
    eprintln!("shutdown fixture stage={stage}; pid={pid}; identity={identity:?}; group={group:?}");
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
fn runner_real_shutdown_closes_a_slave_retaining_descendant_and_preserves_the_native_zero_exit() {
    let fixture = Fixture::new();
    let ready = fixture._dir.path().join("descendant-ready");
    let mut spec = fixture.spec();
    spec.argv=vec!["/bin/sh".into(),"-c".into(),r#"/bin/sh -c 'trap "" TERM HUP; printf ready > "$1"; while :; do sleep 1; done' descendant "$1" & descendant=$!; while [ ! -f "$1" ]; do sleep 0.01; done; printf 'descendant=%s\n' "$descendant"; exit 0"#.into(),"parent".into(),ready.to_str().unwrap().into()];
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
