//! The real process-facing seams: the subprocess [`CommandRunner`] and the
//! PATH/HOME [`SpawnProbe`].

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, poll};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::coding_agent::SpawnProbe;
use crate::multiplexer::{CommandRunner, RunError};

/// Resolve `name` against the `PATH` environment variable to an executable
/// regular file.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        let Ok(metadata) = candidate.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        return Some(candidate);
    }
    None
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn binary_exists(&self, name: &str) -> bool {
        find_on_path(name).is_some()
    }

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError> {
        run_process(argv, timeout_secs.map(Duration::from_secs))
    }

    fn sleep(&self, seconds: f64) {
        std::thread::sleep(Duration::from_secs_f64(seconds));
    }
}

impl crate::multiplexer::spawn::TimedCommandRunner for SystemRunner {
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        if timeout.is_zero() {
            return Err(RunError::Timeout);
        }
        run_process(argv, Some(timeout))
    }
}

fn run_process(argv: &[String], timeout: Option<Duration>) -> Result<String, RunError> {
    let started = Instant::now();
    let (program, args) = argv.split_first().expect("argv carries the program");
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let child = match spawned {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(RunError::BinaryNotFound(e.to_string()));
        }
        Err(e) => {
            return Err(RunError::Failed {
                stderr: e.to_string(),
            });
        }
    };
    if let Some(timeout) = timeout {
        return run_timed(
            child,
            timeout.saturating_sub(started.elapsed()),
            &mut SystemProcessHooks,
        );
    }
    let output = child.wait_with_output().map_err(|e| RunError::Failed {
        stderr: e.to_string(),
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(RunError::Failed {
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// Per-invocation observation/failure seam. Cleanup checks run *after* the real
/// operation, so injected kill/wait errors never prevent actual child recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessOperation {
    ConfigureStdout,
    ConfigureStderr,
    ReadStdout,
    ReadStderr,
    Poll,
    TryWait,
    Kill,
    Wait,
}

trait ProcessHooks {
    fn check(&mut self, _operation: ProcessOperation) -> io::Result<()> {
        Ok(())
    }

    fn cleanup_diagnostic(&mut self, diagnostic: &str) {
        // Timeout has no diagnostic payload; preserve that public category and
        // report secondary failures on stderr without panicking on a closed FD.
        let _ = writeln!(io::stderr().lock(), "{diagnostic}");
    }
}

struct SystemProcessHooks;
impl ProcessHooks for SystemProcessHooks {}

struct OwnedChild {
    child: Child,
    reaped: bool,
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = retry_interrupted(|| self.child.wait());
        }
    }
}

fn retry_interrupted<T>(mut action: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match action() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

fn run_timed(
    child: Child,
    timeout: Duration,
    hooks: &mut impl ProcessHooks,
) -> Result<String, RunError> {
    let started = Instant::now();
    let mut owned = OwnedChild {
        child,
        reaped: false,
    };
    let result = collect_timed(&mut owned, started, timeout, hooks);
    match result {
        Ok((status, stdout, stderr)) => {
            if status.success() {
                Ok(String::from_utf8_lossy(&stdout).into_owned())
            } else {
                Err(RunError::Failed {
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                })
            }
        }
        Err(mut primary) => {
            // collect_timed owns the read ends, so they have already closed.
            let mut diagnostics = Vec::new();
            if !owned.reaped {
                let killed = retry_interrupted(|| owned.child.kill());
                if let Err(error) = killed.and_then(|()| hooks.check(ProcessOperation::Kill)) {
                    diagnostics.push(format!(
                        "cleanup failed for child {} kill: {error}",
                        owned.child.id()
                    ));
                }
            }
            let waited = retry_interrupted(|| owned.child.wait());
            owned.reaped = waited.is_ok();
            if let Err(error) = waited.and_then(|_| hooks.check(ProcessOperation::Wait)) {
                diagnostics.push(format!(
                    "cleanup failed for child {} wait: {error}",
                    owned.child.id()
                ));
            }
            for diagnostic in diagnostics {
                match &mut primary {
                    RunError::Failed { stderr } => {
                        stderr.push('\n');
                        stderr.push_str(&diagnostic);
                    }
                    _ => hooks.cleanup_diagnostic(&diagnostic),
                }
            }
            Err(primary)
        }
    }
}

fn io_failure(error: io::Error) -> RunError {
    RunError::Failed {
        stderr: error.to_string(),
    }
}

fn remaining(started: Instant, timeout: Duration) -> Result<Duration, RunError> {
    timeout
        .checked_sub(started.elapsed())
        .filter(|left| !left.is_zero())
        .ok_or(RunError::Timeout)
}

fn configure_pipe(
    fd: BorrowedFd<'_>,
    operation: ProcessOperation,
    started: Instant,
    timeout: Duration,
    hooks: &mut impl ProcessHooks,
) -> Result<(), RunError> {
    loop {
        remaining(started, timeout)?;
        let result: io::Result<()> = (|| {
            hooks.check(operation)?;
            let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
            fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
            Ok(())
        })();
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result.map_err(io_failure),
        }
    }
}

/// One stream gets at most 64 KiB per iteration, including short reads. An
/// interruption yields to the outer deadline/status check instead of spinning.
fn drain_pipe(
    pipe: &mut Option<impl Read>,
    output: &mut Vec<u8>,
    operation: ProcessOperation,
    hooks: &mut impl ProcessHooks,
) -> io::Result<()> {
    let Some(reader) = pipe.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0; 8192];
    let mut budget = 64 * 1024;
    while budget > 0 {
        let capacity = buffer.len().min(budget);
        let result = hooks
            .check(operation)
            .and_then(|()| reader.read(&mut buffer[..capacity]));
        match result {
            Ok(0) => {
                *pipe = None;
                break;
            }
            Ok(count) => {
                output.extend_from_slice(&buffer[..count]);
                budget -= count;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn collect_timed(
    owned: &mut OwnedChild,
    started: Instant,
    timeout: Duration,
    hooks: &mut impl ProcessHooks,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), RunError> {
    let mut stdout = owned.child.stdout.take();
    let mut stderr = owned.child.stderr.take();
    for (fd, operation) in [
        (
            stdout.as_ref().expect("stdout is piped").as_fd(),
            ProcessOperation::ConfigureStdout,
        ),
        (
            stderr.as_ref().expect("stderr is piped").as_fd(),
            ProcessOperation::ConfigureStderr,
        ),
    ] {
        configure_pipe(fd, operation, started, timeout, hooks)?;
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut status = None;
    loop {
        remaining(started, timeout)?;
        if status.is_none() {
            match hooks
                .check(ProcessOperation::TryWait)
                .and_then(|()| owned.child.try_wait())
            {
                Ok(observed) => {
                    status = observed;
                    owned.reaped = status.is_some();
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(io_failure(error)),
            }
        }
        drain_pipe(&mut stdout, &mut out, ProcessOperation::ReadStdout, hooks)
            .map_err(io_failure)?;
        drain_pipe(&mut stderr, &mut err, ProcessOperation::ReadStderr, hooks)
            .map_err(io_failure)?;
        if stdout.is_none()
            && stderr.is_none()
            && let Some(status) = status
        {
            return Ok((status, out, err));
        }
        let wait = remaining(started, timeout)?.min(Duration::from_millis(20));
        let mut fds = Vec::with_capacity(2);
        if let Some(pipe) = &stdout {
            fds.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        if let Some(pipe) = &stderr {
            fds.push(PollFd::new(pipe.as_fd(), PollFlags::POLLIN));
        }
        // Floor the sub-millisecond remainder so poll never exceeds the budget.
        let result: io::Result<()> = hooks.check(ProcessOperation::Poll).and_then(|()| {
            poll(&mut fds, wait.as_millis() as u16)?;
            if fds.iter().any(|fd| {
                fd.revents()
                    .is_some_and(|events| events.contains(PollFlags::POLLNVAL))
            }) {
                return Err(io::Error::other("invalid subprocess pipe descriptor"));
            }
            Ok(())
        });
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_failure(error)),
            Ok(()) => {}
        }
    }
}

pub struct SystemProbe;

impl SpawnProbe for SystemProbe {
    fn binary_on_path(&self, name: &str) -> bool {
        find_on_path(name).is_some()
    }

    fn home_dir(&self) -> PathBuf {
        PathBuf::from(std::env::var("HOME").expect("HOME is set"))
    }

    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Read stdin to EOF as raw bytes.
pub fn read_stdin() -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::stdin().read_to_end(&mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    mod duration_tests {
        use super::{ProcessFixture, SystemRunner};
        use crate::multiplexer::{RunError, spawn::TimedCommandRunner};
        use std::time::Duration;
        fn run(
            fixture: &ProcessFixture,
            script: &str,
            timeout: Duration,
        ) -> Result<String, RunError> {
            SystemRunner.run_for(
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("printf '%s' \"$$\" > \"$1/pid\"\n{script}"),
                    "cafleet-duration-test".into(),
                    fixture.dir.path().to_str().unwrap().into(),
                ],
                timeout,
            )
        }
        #[test]
        fn duration_entry_drains_large_dual_streams_and_keeps_exit_classification() {
            let fixture = ProcessFixture::new();
            let result = run(
                &fixture,
                "exec /usr/bin/python3 -c 'import os; os.write(1,b\"x\"*1048576); os.write(2,b\"y\"*1048576)'",
                Duration::from_millis(10_125),
            );
            fixture.assert_direct_child_reaped(&result);
            assert_eq!(result.unwrap(), "x".repeat(1_048_576));
            let result = run(
                &fixture,
                "exec /usr/bin/python3 -c 'import os; os.write(1,b\"x\"*1048576); os.write(2,b\"y\"*1048576); raise SystemExit(7)'",
                Duration::from_millis(10_125),
            );
            fixture.assert_direct_child_reaped(&result);
            assert_eq!(
                result,
                Err(RunError::Failed {
                    stderr: "y".repeat(1_048_576)
                })
            );
        }
        #[test]
        fn duration_timeout_uses_fractional_budget_and_reaps_direct_child() {
            let fixture = ProcessFixture::new();
            let result = run(&fixture, "exec /bin/sleep 5", Duration::from_millis(1_125));
            assert_eq!(result, Err(RunError::Timeout));
            fixture.assert_direct_child_reaped(&result);
        }
    }

    use super::*;
    use nix::errno::Errno;
    use nix::sys::wait::{WaitPidFlag, waitpid};
    use nix::unistd::Pid;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    // Concurrent process launches can exhaust a one-second deadline before sh starts.
    static PROCESS_FIXTURE_LOCK: Mutex<()> = Mutex::new(());

    struct ProcessFixture {
        dir: TempDir,
        _serial: MutexGuard<'static, ()>,
    }

    impl ProcessFixture {
        fn new() -> Self {
            let serial = PROCESS_FIXTURE_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Self {
                dir: tempfile::Builder::new()
                    .prefix(".process-test-")
                    .tempdir_in(env!("CARGO_MANIFEST_DIR"))
                    .unwrap(),
                _serial: serial,
            }
        }

        fn run(&self, script: &str, timeout: Option<u64>) -> Result<String, RunError> {
            let argv = vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                format!("printf '%s' \"$$\" > \"$1/pid\"\n{script}"),
                "cafleet-process-test".to_owned(),
                self.dir.path().to_str().unwrap().to_owned(),
            ];
            SystemRunner.run(&argv, timeout)
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            std::fs::write(self.dir.path().join(name), bytes).unwrap();
        }

        fn assert_direct_child_reaped(&self, result: &Result<String, RunError>) {
            let pid = std::fs::read_to_string(self.dir.path().join("pid"))
                .unwrap_or_else(|error| {
                    panic!("child PID was not recorded: {error}; runner result: {result:?}")
                })
                .parse::<i32>()
                .unwrap();
            assert_eq!(
                waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)),
                Err(Errno::ECHILD),
                "the runner must reap its direct child before returning"
            );
        }
    }

    fn megabyte() -> Vec<u8> {
        (0..1024 * 1024).map(|i| b'a' + (i % 26) as u8).collect()
    }

    #[test]
    fn timed_runner_preserves_one_megabyte_of_stdout() {
        let fixture = ProcessFixture::new();
        let payload = megabyte();
        fixture.write("payload", &payload);
        let result = fixture.run("cat \"$1/payload\"", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap().as_bytes(), payload);
    }

    #[test]
    fn timed_runner_drains_one_megabyte_of_stderr_on_success() {
        let fixture = ProcessFixture::new();
        fixture.write("payload", &megabyte());
        let result = fixture.run("cat \"$1/payload\" >&2", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn timed_runner_preserves_one_megabyte_of_stderr_on_failure() {
        let fixture = ProcessFixture::new();
        let payload = megabyte();
        fixture.write("payload", &payload);
        let result = fixture.run("cat \"$1/payload\" >&2; exit 7", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Failed { stderr }) if stderr.as_bytes() == payload));
    }

    #[test]
    fn timed_runner_drains_simultaneous_megabyte_streams_on_success() {
        let fixture = ProcessFixture::new();
        let payload = megabyte();
        fixture.write("payload", &payload);
        let result = fixture.run(
            "cat \"$1/payload\" & writer=$!; cat \"$1/payload\" >&2; wait \"$writer\"",
            Some(10),
        );
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap().as_bytes(), payload);
    }

    #[test]
    fn timed_runner_drains_simultaneous_megabyte_streams_on_failure() {
        let fixture = ProcessFixture::new();
        let payload = megabyte();
        fixture.write("payload", &payload);
        let result = fixture.run(
            "cat \"$1/payload\" & writer=$!; cat \"$1/payload\" >&2; wait \"$writer\"; exit 9",
            Some(10),
        );
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Failed { stderr }) if stderr.as_bytes() == payload));
    }

    #[test]
    fn timed_runner_kills_and_reaps_a_sleeping_direct_child_within_five_seconds() {
        let fixture = ProcessFixture::new();
        let start = Instant::now();
        let result = fixture.run("exec sleep 30", Some(1));
        let elapsed = start.elapsed();
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert!(elapsed < Duration::from_secs(5), "elapsed: {elapsed:?}");
    }

    #[test]
    fn timed_runner_checks_deadline_while_both_streams_keep_producing() {
        let fixture = ProcessFixture::new();
        let start = Instant::now();
        let result = fixture.run(
            "while :; do printf 'stdout flood\\n'; printf 'stderr flood\\n' >&2; done",
            Some(1),
        );
        let elapsed = start.elapsed();
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert!(elapsed < Duration::from_secs(5), "elapsed: {elapsed:?}");
    }

    #[test]
    fn timed_runner_applies_deadline_after_child_exit_with_inherited_pipes_open() {
        let fixture = ProcessFixture::new();
        let start = Instant::now();
        // The descendant is deliberately bounded even against the old blocking runner.
        let result = fixture.run("sleep 6 & exit 0", Some(1));
        let elapsed = start.elapsed();
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert!(elapsed < Duration::from_secs(5), "elapsed: {elapsed:?}");
    }

    #[test]
    fn timed_runner_waits_for_child_exit_even_after_both_pipes_close() {
        let fixture = ProcessFixture::new();
        let start = Instant::now();
        let result = fixture.run("exec 1>&- 2>&-; exec sleep 30", Some(1));
        let elapsed = start.elapsed();
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert!(elapsed < Duration::from_secs(5), "elapsed: {elapsed:?}");
    }

    #[test]
    fn timed_runner_returns_empty_stdout_for_empty_success() {
        let fixture = ProcessFixture::new();
        let result = fixture.run("exit 0", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn timed_runner_reports_stderr_and_discards_stdout_for_nonzero_exit() {
        let fixture = ProcessFixture::new();
        let result = fixture.run(
            "printf 'discard me'; printf 'failure\\n' >&2; exit 3",
            Some(10),
        );
        fixture.assert_direct_child_reaped(&result);
        assert!(matches!(result, Err(RunError::Failed { stderr }) if stderr == "failure\n"));
    }

    #[test]
    fn timed_runner_decodes_successful_stdout_as_lossy_utf8() {
        let fixture = ProcessFixture::new();
        fixture.write("payload", b"a\xffb\xc3");
        let result = fixture.run("cat \"$1/payload\"", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap(), "a\u{fffd}b\u{fffd}");
    }

    #[test]
    fn timed_runner_decodes_failed_stderr_as_lossy_utf8() {
        let fixture = ProcessFixture::new();
        fixture.write("payload", b"a\xffb\xc3");
        let result = fixture.run("cat \"$1/payload\" >&2; exit 1", Some(10));
        fixture.assert_direct_child_reaped(&result);
        assert!(
            matches!(result, Err(RunError::Failed { stderr }) if stderr == "a\u{fffd}b\u{fffd}")
        );
    }

    #[test]
    fn untimed_runner_preserves_large_output_and_reaps_child() {
        let fixture = ProcessFixture::new();
        let payload = megabyte();
        fixture.write("payload", &payload);
        let result = fixture.run(
            "cat \"$1/payload\" & writer=$!; cat \"$1/payload\" >&2; wait \"$writer\"",
            None,
        );
        fixture.assert_direct_child_reaped(&result);
        assert_eq!(result.unwrap().as_bytes(), payload);
    }

    #[test]
    fn runner_preserves_binary_not_found_classification() {
        let fixture = ProcessFixture::new();
        let missing = fixture.dir.path().join("missing-executable");
        for timeout in [None, Some(1)] {
            let result = SystemRunner.run(&[missing.to_str().unwrap().to_owned()], timeout);
            assert!(
                matches!(result, Err(RunError::BinaryNotFound(_))),
                "{result:?}"
            );
        }
    }

    #[derive(Default)]
    struct FaultHooks {
        faults: Vec<(ProcessOperation, io::ErrorKind, usize)>,
        events: Vec<ProcessOperation>,
        diagnostics: Vec<String>,
    }

    impl FaultHooks {
        fn fail(&mut self, operation: ProcessOperation, kind: io::ErrorKind, times: usize) {
            self.faults.push((operation, kind, times));
        }

        fn count(&self, operation: ProcessOperation) -> usize {
            self.events
                .iter()
                .filter(|&&event| event == operation)
                .count()
        }
    }

    impl ProcessHooks for FaultHooks {
        fn check(&mut self, operation: ProcessOperation) -> io::Result<()> {
            self.events.push(operation);
            for (target, kind, remaining) in &mut self.faults {
                if *target == operation && *remaining > 0 {
                    *remaining -= 1;
                    return Err(io::Error::new(*kind, format!("injected {operation:?}")));
                }
            }
            Ok(())
        }

        fn cleanup_diagnostic(&mut self, diagnostic: &str) {
            self.diagnostics.push(diagnostic.to_owned());
        }
    }

    impl ProcessFixture {
        fn run_with_hooks(
            &self,
            script: &str,
            timeout: Duration,
            hooks: &mut impl ProcessHooks,
        ) -> Result<String, RunError> {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::MetadataExt;

            let child = Command::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let pid = Pid::from_raw(child.id() as i32);
            let pipes = [
                child.stdout.as_ref().unwrap().as_raw_fd(),
                child.stderr.as_ref().unwrap().as_raw_fd(),
            ]
            .map(|fd| {
                let path = format!("/dev/fd/{fd}");
                let metadata = std::fs::metadata(&path).unwrap();
                (path, metadata.dev(), metadata.ino())
            });
            let result = run_timed(child, timeout, hooks);
            assert_eq!(
                waitpid(pid, Some(WaitPidFlag::WNOHANG)),
                Err(Errno::ECHILD),
                "direct child was not reaped: {result:?}"
            );
            for (path, device, inode) in pipes {
                // Other unit tests may reuse the descriptor number after it closes.
                if let Ok(metadata) = std::fs::metadata(&path) {
                    assert_ne!(
                        (metadata.dev(), metadata.ino()),
                        (device, inode),
                        "original pipe {path} remained open: {result:?}"
                    );
                }
            }
            result
        }
    }

    fn assert_operation_failure(operation: ProcessOperation) {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        hooks.fail(operation, io::ErrorKind::Other, 1);
        let result = fixture.run_with_hooks("exec sleep 30", Duration::from_secs(5), &mut hooks);
        match result {
            Err(RunError::Failed { stderr }) => {
                assert_eq!(stderr, format!("injected {operation:?}"));
            }
            result => panic!("expected injected primary failure, got {result:?}"),
        }
        assert_eq!(hooks.count(operation), 1);
        assert_eq!(hooks.count(ProcessOperation::Kill), 1);
        assert_eq!(hooks.count(ProcessOperation::Wait), 1);
        assert_eq!(
            &hooks.events[hooks.events.len() - 2..],
            &[ProcessOperation::Kill, ProcessOperation::Wait]
        );
        assert!(hooks.diagnostics.is_empty());
    }

    fn assert_interrupted_operation_retries(operation: ProcessOperation) {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        hooks.fail(operation, io::ErrorKind::Interrupted, 2);
        let result =
            fixture.run_with_hooks("exec sleep 30", Duration::from_millis(100), &mut hooks);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert!(hooks.count(operation) >= 3, "events: {:?}", hooks.events);
        assert!(hooks.diagnostics.is_empty());
    }

    macro_rules! operation_contract_tests {
        ($failure:ident, $interrupted:ident, $operation:ident) => {
            #[test]
            fn $failure() {
                assert_operation_failure(ProcessOperation::$operation);
            }

            #[test]
            fn $interrupted() {
                assert_interrupted_operation_retries(ProcessOperation::$operation);
            }
        };
    }

    operation_contract_tests!(
        stdout_configuration_failure_recovers_child_and_pipes,
        stdout_configuration_interruption_retries,
        ConfigureStdout
    );
    operation_contract_tests!(
        stderr_configuration_failure_recovers_child_and_pipes,
        stderr_configuration_interruption_retries,
        ConfigureStderr
    );
    operation_contract_tests!(
        stdout_read_failure_recovers_child_and_pipes,
        stdout_read_interruption_retries,
        ReadStdout
    );
    operation_contract_tests!(
        stderr_read_failure_recovers_child_and_pipes,
        stderr_read_interruption_retries,
        ReadStderr
    );
    operation_contract_tests!(
        poll_failure_recovers_child_and_pipes,
        poll_interruption_retries,
        Poll
    );
    operation_contract_tests!(
        try_wait_failure_recovers_child_and_pipes,
        try_wait_interruption_retries,
        TryWait
    );

    #[test]
    fn persistent_interruption_at_each_operation_preserves_original_deadline() {
        for operation in [
            ProcessOperation::ConfigureStdout,
            ProcessOperation::ConfigureStderr,
            ProcessOperation::ReadStdout,
            ProcessOperation::ReadStderr,
            ProcessOperation::Poll,
            ProcessOperation::TryWait,
        ] {
            let fixture = ProcessFixture::new();
            let mut hooks = FaultHooks::default();
            hooks.fail(operation, io::ErrorKind::Interrupted, usize::MAX);
            let started = Instant::now();
            let result =
                fixture.run_with_hooks("exec sleep 30", Duration::from_millis(30), &mut hooks);
            assert!(
                matches!(result, Err(RunError::Timeout)),
                "{operation:?}: {result:?}"
            );
            assert!(started.elapsed() < Duration::from_secs(5), "{operation:?}");
            assert!(hooks.count(operation) > 1, "{operation:?}");
            assert!(hooks.diagnostics.is_empty());
        }
    }

    #[test]
    fn cleanup_failures_follow_primary_error_and_do_not_skip_reaping() {
        for cleanup in [
            vec![ProcessOperation::Kill],
            vec![ProcessOperation::Wait],
            vec![ProcessOperation::Kill, ProcessOperation::Wait],
        ] {
            let fixture = ProcessFixture::new();
            let mut hooks = FaultHooks::default();
            hooks.fail(ProcessOperation::ConfigureStderr, io::ErrorKind::Other, 1);
            for operation in &cleanup {
                hooks.fail(*operation, io::ErrorKind::Other, 1);
            }
            let result =
                fixture.run_with_hooks("exec sleep 30", Duration::from_secs(5), &mut hooks);
            let Err(RunError::Failed { stderr }) = result else {
                panic!("primary failure was replaced: {result:?}");
            };
            let lines: Vec<_> = stderr.lines().collect();
            assert_eq!(lines[0], "injected ConfigureStderr");
            assert_eq!(lines.len(), 1 + cleanup.len());
            for (line, operation) in lines[1..].iter().zip(&cleanup) {
                assert!(line.starts_with("cleanup failed for child "), "{line}");
                let detail = match operation {
                    ProcessOperation::Kill => " kill: injected Kill",
                    ProcessOperation::Wait => " wait: injected Wait",
                    _ => unreachable!(),
                };
                assert!(line.ends_with(detail), "{line}");
            }
            assert_eq!(hooks.count(ProcessOperation::Kill), 1);
            assert_eq!(hooks.count(ProcessOperation::Wait), 1);
            assert!(hooks.diagnostics.is_empty());
        }
    }

    #[test]
    fn timeout_retains_category_and_reports_both_secondary_cleanup_errors() {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        hooks.fail(ProcessOperation::Kill, io::ErrorKind::Other, 1);
        hooks.fail(ProcessOperation::Wait, io::ErrorKind::Other, 1);
        let result = fixture.run_with_hooks("exec sleep 30", Duration::from_millis(30), &mut hooks);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        assert_eq!(hooks.diagnostics.len(), 2);
        assert!(hooks.diagnostics[0].ends_with(" kill: injected Kill"));
        assert!(hooks.diagnostics[1].ends_with(" wait: injected Wait"));
        assert_eq!(hooks.count(ProcessOperation::Kill), 1);
        assert_eq!(hooks.count(ProcessOperation::Wait), 1);
    }

    #[test]
    fn successful_collection_does_not_run_cleanup_hooks() {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        let result = fixture.run_with_hooks("printf 'done'", Duration::from_secs(5), &mut hooks);
        assert_eq!(result.unwrap(), "done");
        assert_eq!(hooks.count(ProcessOperation::Kill), 0);
        assert_eq!(hooks.count(ProcessOperation::Wait), 0);
        assert!(hooks.diagnostics.is_empty());
    }

    #[test]
    fn nonzero_exit_preserves_stderr_without_spurious_cleanup() {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        let result = fixture.run_with_hooks(
            "printf 'primary' >&2; exit 2",
            Duration::from_secs(5),
            &mut hooks,
        );
        assert!(matches!(result, Err(RunError::Failed { stderr }) if stderr == "primary"));
        assert_eq!(hooks.count(ProcessOperation::Kill), 0);
        assert_eq!(hooks.count(ProcessOperation::Wait), 0);
    }

    struct EndlessReader {
        max_read: usize,
    }

    impl Read for EndlessReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.max_read);
            buffer[..count].fill(b'x');
            Ok(count)
        }
    }

    #[test]
    fn per_stream_budget_is_64_kib_even_with_short_reads() {
        for max_read in [3, 8192] {
            let mut pipe = Some(EndlessReader { max_read });
            let mut output = Vec::new();
            let mut hooks = FaultHooks::default();
            drain_pipe(
                &mut pipe,
                &mut output,
                ProcessOperation::ReadStdout,
                &mut hooks,
            )
            .unwrap();
            assert_eq!(output, vec![b'x'; 64 * 1024]);
            assert!(pipe.is_some());
        }
    }

    #[test]
    fn interrupted_or_would_block_read_yields_without_losing_prior_bytes() {
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock] {
            let mut pipe = Some(io::Cursor::new(b"tail".to_vec()));
            let mut output = b"head".to_vec();
            let mut hooks = FaultHooks::default();
            hooks.fail(ProcessOperation::ReadStdout, kind, 1);
            drain_pipe(
                &mut pipe,
                &mut output,
                ProcessOperation::ReadStdout,
                &mut hooks,
            )
            .unwrap();
            assert_eq!(output, b"head");
            assert!(pipe.is_some());
            drain_pipe(
                &mut pipe,
                &mut output,
                ProcessOperation::ReadStdout,
                &mut hooks,
            )
            .unwrap();
            assert_eq!(output, b"headtail");
            assert!(pipe.is_none());
        }
    }

    #[test]
    fn both_streams_are_serviced_between_deadline_checks() {
        let fixture = ProcessFixture::new();
        let mut hooks = FaultHooks::default();
        let result =
            fixture.run_with_hooks("exec sleep 30", Duration::from_millis(100), &mut hooks);
        assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
        let mut completed_iterations = 0;
        for iteration in hooks
            .events
            .split(|event| *event == ProcessOperation::TryWait)
            .skip(1)
        {
            if iteration.contains(&ProcessOperation::Poll) {
                assert!(iteration.contains(&ProcessOperation::ReadStdout));
                assert!(iteration.contains(&ProcessOperation::ReadStderr));
                completed_iterations += 1;
            }
        }
        assert!(completed_iterations > 0);
    }

    #[test]
    fn cleanup_retry_retries_interrupted_and_preserves_other_errors() {
        let mut attempts = 0;
        let result = retry_interrupted(|| {
            attempts += 1;
            match attempts {
                1 | 2 => Err(io::Error::from(io::ErrorKind::Interrupted)),
                _ => Ok("reaped"),
            }
        });
        assert_eq!(result.unwrap(), "reaped");
        assert_eq!(attempts, 3);
        let mut attempts = 0;
        let error = retry_interrupted::<()>(|| {
            attempts += 1;
            Err(io::Error::other("original wait failure"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "original wait failure");
        assert_eq!(attempts, 1);
    }
}
