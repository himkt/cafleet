use super::{CodingAgent, ProfileCatalog};
use crate::broker::runtime::{Coverage, ProfileMatch};
use crate::error::CafleetError;
use regex::Regex;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const OUTPUT_BYTES: u64 = 4096;
const VERSION_TOKEN: &str = r"^v?[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$";

pub(crate) struct NativeLaunchIdentity {
    pub(crate) native_version: Option<String>,
    pub(crate) profile_version: Option<String>,
    pub(crate) profile_match: Option<ProfileMatch>,
    pub(crate) coverage: Coverage,
    pub(crate) warning: Option<String>,
}

pub(crate) fn native_launch_identity(
    backend: CodingAgent,
    probe: Result<String, CafleetError>,
    catalog: &ProfileCatalog,
) -> NativeLaunchIdentity {
    match probe {
        Ok(version) => {
            let capabilities = catalog.capabilities(backend, Some(&version));
            let warning = match (capabilities.profile_match, &capabilities.profile_version) {
                (Some(ProfileMatch::Fallback), Some(profile)) => Some(format!(
                    "Warning: no bundled profile for {} {version}; using {profile} (profile_match=fallback).",
                    backend.name()
                )),
                _ => None,
            };
            NativeLaunchIdentity {
                native_version: Some(version),
                profile_version: capabilities.profile_version,
                profile_match: capabilities.profile_match,
                coverage: capabilities.detection,
                warning,
            }
        }
        Err(cause) => NativeLaunchIdentity {
            native_version: None,
            profile_version: None,
            profile_match: None,
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
        let fail =
            |detail: String| CafleetError::App(format!("{} --version {detail}", self.name()));
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
        version_token(first)
            .map(str::to_owned)
            .map_err(|cause| fail(cause.into()))
    }
}

pub(crate) fn version_token(line: &str) -> Result<&str, &'static str> {
    let pattern = Regex::new(VERSION_TOKEN).expect("valid version token pattern");
    let tokens = line
        .split_whitespace()
        .filter(|token| pattern.is_match(token))
        .collect::<Vec<_>>();
    match tokens.as_slice() {
        [version] => Ok(version.strip_prefix('v').unwrap_or(version)),
        [] => Err("printed no version token"),
        _ => Err("printed several version tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token_records_the_documented_first_lines_without_a_leading_v() {
        for (line, expected) in [
            ("2.1.302 (Claude Code)", "2.1.302"),
            ("codex-cli 0.163.0", "0.163.0"),
            ("opencode v2.1.4", "2.1.4"),
            ("v1.2.3-beta.1", "1.2.3-beta.1"),
        ] {
            assert_eq!(version_token(line), Ok(expected), "{line}");
        }
    }

    #[test]
    fn version_token_counts_a_v_prefixed_token_toward_the_exactly_one_token_rule() {
        assert_eq!(
            version_token("v2.1.4 2.1.5"),
            Err("printed several version tokens")
        );
        assert_eq!(
            version_token("opencode v2.1.4 v2.1.5"),
            Err("printed several version tokens")
        );
    }

    #[test]
    fn version_token_accepts_only_a_single_optional_leading_v() {
        for line in [
            "opencode vv2.1.4",
            "opencode V2.1.4",
            "opencode v",
            "opencode v2",
        ] {
            assert_eq!(
                version_token(line),
                Err("printed no version token"),
                "{line}"
            );
        }
    }

    mod native_version_probe {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        fn stub(dir: &Path, name: &str, body: &str) -> PathBuf {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn probe(executable: &Path) -> Result<String, CafleetError> {
            CodingAgent::Claude
                .probe_native_version(executable, Instant::now() + Duration::from_secs(2))
        }

        #[test]
        fn native_version_probe_records_the_single_version_token_of_a_version_only_invocation() {
            let dir = tempfile::tempdir().unwrap();
            let path = stub(
                dir.path(),
                "native",
                "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nread -r ignored || true\nprintf '%s\\n' '2.1.302 (Claude Code)'\nprintf 'ignored\\n' >&2",
            );
            assert_eq!(probe(&path).unwrap(), "2.1.302");
        }

        #[test]
        fn native_version_probe_rejects_failed_ambiguous_invalid_or_missing_versions() {
            let dir = tempfile::tempdir().unwrap();
            for (index, (case, body)) in [
                ("nonzero exit", "printf '2.1.302\\n'; exit 1"),
                (
                    "version outside the first line",
                    "printf 'Claude Code\\n2.1.302\\n'",
                ),
                ("empty output", "exit 0"),
                ("non-UTF-8 output", "printf '2.1.302 \\377\\n'"),
            ]
            .into_iter()
            .enumerate()
            {
                let path = stub(dir.path(), &format!("native-{index}"), body);
                assert!(probe(&path).is_err(), "{case} must not yield a version");
            }
            assert!(
                probe(&dir.path().join("absent")).is_err(),
                "unstartable binary"
            );
        }

        #[test]
        fn native_version_probe_kills_and_reaps_a_probe_running_at_its_bound() {
            let dir = tempfile::tempdir().unwrap();
            let pid_file = dir.path().join("probe.pid");
            let path = stub(
                dir.path(),
                "native",
                &format!(
                    "printf '%s' \"$$\" > '{}'\nexec /bin/sleep 30",
                    pid_file.display()
                ),
            );
            let start = Instant::now();
            let result =
                CodingAgent::Claude.probe_native_version(&path, start + Duration::from_millis(300));
            assert!(result.is_err());
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "{:?}",
                start.elapsed()
            );
            let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
            assert_eq!(
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH),
                "the timed-out probe is killed and reaped"
            );
        }

        #[test]
        fn native_launch_identity_selects_the_bundled_profile_or_records_unknown_with_a_warning() {
            let catalog = ProfileCatalog::bundled().unwrap();
            let tested =
                native_launch_identity(CodingAgent::Claude, Ok("2.1.302".into()), &catalog);
            assert_eq!(tested.native_version.as_deref(), Some("2.1.302"));
            assert_eq!(
                tested.profile_version,
                catalog
                    .capabilities(CodingAgent::Claude, Some("2.1.302"))
                    .profile_version
            );
            assert!(tested.profile_version.is_some());
            assert_eq!(tested.profile_match, Some(ProfileMatch::Exact));
            assert_eq!(tested.coverage, Coverage::Tested);
            assert_eq!(tested.warning, None);
            let failed = native_launch_identity(
                CodingAgent::Codex,
                Err(CafleetError::App("probe exited with status 1".into())),
                &catalog,
            );
            assert_eq!(failed.native_version, None);
            assert_eq!(failed.profile_version, None);
            assert_eq!(failed.profile_match, None);
            assert_eq!(failed.coverage, Coverage::Unknown);
            assert_eq!(
                failed.warning.as_deref(),
                Some(
                    "Warning: native version unavailable for codex: probe exited with status 1; coverage unknown."
                )
            );
        }

        #[test]
        fn native_launch_identity_runs_an_unprofiled_version_under_the_newest_bundled_profile() {
            let catalog = ProfileCatalog::bundled().unwrap();
            for (backend, pinned, probes) in
                crate::coding_agent::profiles::catalog::test_support::BUNDLED_PROBES
            {
                let profile = catalog
                    .capabilities(backend, Some(pinned))
                    .profile_version
                    .expect("each backend bundles a profile for its pinned version");
                for probe in probes {
                    let identity = native_launch_identity(backend, Ok(probe.into()), &catalog);
                    assert_eq!(identity.native_version.as_deref(), Some(probe));
                    assert_eq!(identity.coverage, Coverage::Tested, "{backend:?} {probe}");
                    assert_eq!(
                        identity.warning,
                        Some(format!(
                            "Warning: no bundled profile for {} {probe}; using {profile} (profile_match=fallback).",
                            backend.name()
                        ))
                    );
                }
                let exact_identity = native_launch_identity(backend, Ok(pinned.into()), &catalog);
                assert_eq!(exact_identity.profile_match, Some(ProfileMatch::Exact));
                assert_eq!(exact_identity.warning, None);
            }
        }

        #[test]
        fn native_launch_identity_without_any_profile_for_the_backend_records_unknown_silently() {
            let catalog = ProfileCatalog::from_definitions(
                &crate::coding_agent::profiles::catalog::test_support::inspection_fixture_definition(),
            )
            .unwrap();
            for backend in [CodingAgent::Claude, CodingAgent::Opencode] {
                let identity = native_launch_identity(backend, Ok("2.1.302".into()), &catalog);
                assert_eq!(identity.native_version.as_deref(), Some("2.1.302"));
                assert_eq!(identity.profile_version, None);
                assert_eq!(identity.profile_match, None);
                assert_eq!(identity.coverage, Coverage::Unknown);
                assert_eq!(identity.warning, None);
            }
        }
    }
}
