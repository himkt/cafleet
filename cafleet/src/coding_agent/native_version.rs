use super::{CodingAgent, ProfileCatalog};
use crate::broker::runtime::Coverage;
use crate::error::CafleetError;
use regex::Regex;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const OUTPUT_BYTES: u64 = 4096;
const VERSION_TOKEN: &str = r"^[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$";

pub(crate) struct NativeLaunchIdentity {
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub coverage: Coverage,
    pub warning: Option<String>,
}

pub(crate) fn native_launch_identity(
    backend: CodingAgent,
    probe: Result<String, CafleetError>,
    catalog: &ProfileCatalog,
) -> NativeLaunchIdentity {
    match probe {
        Ok(version) => {
            let capabilities = catalog.capabilities(backend, Some(&version));
            NativeLaunchIdentity {
                native_version: Some(version),
                profile_version: capabilities.profile_version,
                coverage: capabilities.detection,
                warning: None,
            }
        }
        Err(cause) => NativeLaunchIdentity {
            native_version: None,
            profile_version: None,
            coverage: Coverage::Unknown,
            warning: Some(format!(
                "Warning: native version unavailable for {}: {}; coverage unknown.",
                backend.name(),
                cause.message()
            )),
        },
    }
}

fn bounded_output(source: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = source
            .take(OUTPUT_BYTES)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        // The receiver is dropped once the probe has already failed at its deadline.
        let _ = sender.send(result);
    });
    receiver
}

impl CodingAgent {
    pub(crate) fn probe_native_version(
        self,
        executable: &Path,
        deadline: Instant,
    ) -> Result<String, CafleetError> {
        let fail = |detail: String| {
            CafleetError::App(format!("{} --version {detail}", self.binary_name()))
        };
        let mut child = Command::new(executable)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|cause| fail(format!("could not start: {cause}")))?;
        let stdout = bounded_output(child.stdout.take().expect("piped probe stdout"));
        let stderr = bounded_output(child.stderr.take().expect("piped probe stderr"));
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|cause| fail(format!("wait failed: {cause}")))?
            {
                break status;
            }
            let now = Instant::now();
            if now >= deadline {
                child
                    .kill()
                    .map_err(|cause| fail(format!("timed out and could not be killed: {cause}")))?;
                child
                    .wait()
                    .map_err(|cause| fail(format!("timed out and could not be reaped: {cause}")))?;
                return Err(fail("did not finish within its deadline".into()));
            }
            std::thread::sleep(Duration::from_millis(10).min(deadline - now));
        };
        let collect = |receiver: mpsc::Receiver<std::io::Result<Vec<u8>>>| {
            receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| fail("output did not close within its deadline".into()))?
                .map_err(|cause| fail(format!("output could not be read: {cause}")))
        };
        let output = collect(stdout)?;
        let diagnostic = collect(stderr)?;
        if !status.success() {
            let first = String::from_utf8_lossy(&diagnostic)
                .lines()
                .next()
                .map(str::to_owned);
            return Err(fail(match first {
                Some(line) if !line.trim().is_empty() => format!("exited with {status}: {line}"),
                _ => format!("exited with {status}"),
            }));
        }
        let text =
            String::from_utf8(output).map_err(|_| fail("printed non-UTF-8 output".into()))?;
        let first = text
            .lines()
            .next()
            .ok_or_else(|| fail("printed no output".into()))?;
        let pattern = Regex::new(VERSION_TOKEN).expect("valid version token pattern");
        let tokens = first
            .split_whitespace()
            .filter(|token| pattern.is_match(token))
            .collect::<Vec<_>>();
        match tokens.as_slice() {
            [version] => Ok((*version).to_owned()),
            [] => Err(fail("printed no version token".into())),
            _ => Err(fail("printed several version tokens".into())),
        }
    }
}
