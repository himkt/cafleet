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
}

/// Read stdin to EOF as raw bytes.
pub fn read_stdin() -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::stdin().read_to_end(&mut buffer)?;
    Ok(buffer)
}
