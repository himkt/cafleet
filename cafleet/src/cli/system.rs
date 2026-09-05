//! The real process-facing seams: the subprocess [`CommandRunner`] and the
//! PATH/HOME [`SpawnProbe`].

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
        let (program, args) = argv.split_first().expect("argv carries the program");
        let spawned = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
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
        if let Some(secs) = timeout_secs {
            let deadline = Instant::now() + Duration::from_secs(secs);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(RunError::Timeout);
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(e) => {
                        return Err(RunError::Failed {
                            stderr: e.to_string(),
                        });
                    }
                }
            }
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

    fn sleep(&self, seconds: f64) {
        std::thread::sleep(Duration::from_secs_f64(seconds));
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
}
