pub(crate) mod catalog;
pub(crate) mod detector;
pub(crate) mod hermes;

use crate::broker::runtime::Coverage;
use crate::broker::runtime::ModeSource;
pub(crate) use crate::coding_agent::catalog::AnswerCapability;
pub(crate) use crate::coding_agent::catalog::AnswerPlan;
pub(crate) use crate::coding_agent::catalog::AnswerStage;
pub(crate) use crate::coding_agent::catalog::ComposerOperation;
pub(crate) use crate::coding_agent::catalog::ProfileCatalog;
pub(crate) use crate::coding_agent::catalog::StageGate;
pub(crate) use crate::coding_agent::detector::AnnotationBlock;
pub(crate) use crate::coding_agent::detector::AnnotationFingerprint;
pub(crate) use crate::coding_agent::detector::AnswerMatch;
pub(crate) use crate::coding_agent::detector::Detector;
pub(crate) use crate::coding_agent::detector::MatchEvidence;
pub(crate) use crate::coding_agent::detector::ObservationKind;
pub(crate) use crate::coding_agent::detector::ScreenObservation;
pub(crate) use crate::coding_agent::detector::SemanticFingerprint;
use crate::config::EnvLookup;
use crate::error::CafleetError;
use clap::ValueEnum;
use std::cmp::Ordering;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

pub(crate) trait SpawnProbe {
    fn resolve_binary(&self, name: &str) -> Option<PathBuf>;
    fn home_dir(&self) -> Result<PathBuf, CafleetError>;
    fn env_var(&self, name: &str) -> Result<Option<String>, CafleetError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Deserialize)]
#[value(rename_all = "lower")]
#[serde(rename_all = "lowercase")]
pub(crate) enum CodingAgent {
    Claude,
    Codex,
    Opencode,
    Hermes,
    Pi,
}

const PI_THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

impl CodingAgent {
    pub(crate) fn all() -> &'static [Self] {
        Self::value_variants()
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Hermes => "hermes",
            Self::Pi => "pi",
        }
    }

    pub(crate) fn config_variable(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
            Self::Opencode => "OPENCODE_CONFIG_DIR",
            Self::Hermes => "HERMES_HOME",
            Self::Pi => "PI_CODING_AGENT_DIR",
        }
    }

    pub(crate) fn config_dir(
        self,
        env: EnvLookup<'_>,
        home: &Path,
    ) -> Result<PathBuf, CafleetError> {
        let default = match self {
            Self::Claude => home.join(".claude"),
            Self::Codex => home.join(".codex"),
            Self::Opencode => crate::config::opencode_skills_base(home),
            Self::Hermes => home.join(".hermes"),
            Self::Pi => home.join(".pi/agent"),
        };
        crate::config::resolve(self.config_variable(), env, default)
    }

    pub(crate) fn forwarded_config(
        self,
        env: EnvLookup<'_>,
    ) -> Result<Option<(String, String)>, CafleetError> {
        env(self.config_variable())?
            .map(|value| {
                crate::config::require_absolute(self.config_variable(), &value)?;
                Ok((self.config_variable().to_owned(), value))
            })
            .transpose()
    }

    pub(crate) fn required_preset(self, config: &Path, mode: &str) -> Option<PathBuf> {
        (self == Self::Opencode).then(|| config.join(format!("agents/{mode}.md")))
    }

    pub(crate) fn validate_model(self, model: Option<&str>) -> Result<(), CafleetError> {
        if self == Self::Opencode
            && let Some(model) = model
            && !model
                .split_once('/')
                .is_some_and(|(provider, id)| !provider.is_empty() && !id.is_empty())
        {
            return Err(CafleetError::Usage(format!(
                "--model for the opencode backend must be '<provider-id>/<model-id>' (got '{model}')."
            )));
        }
        if self == Self::Pi
            && let Some(model) = model
            && model
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| PI_THINKING_LEVELS.contains(&suffix))
        {
            return Err(CafleetError::Usage(format!(
                "--model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '{model}')."
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_prompt(self, prompt: &str) -> Result<(), CafleetError> {
        if self == Self::Pi && prompt.starts_with('@') {
            return Err(CafleetError::Usage(
                "a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_effort(self, effort: Option<&str>) -> Result<(), CafleetError> {
        let Some(effort) = effort else { return Ok(()) };
        let choices = match self {
            Self::Claude => &["low", "medium", "high", "xhigh", "max"][..],
            Self::Codex => &["minimal", "low", "medium", "high", "xhigh"][..],
            Self::Hermes => &[
                "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ][..],
            Self::Pi => &PI_THINKING_LEVELS[..],
            Self::Opencode => {
                return Err(CafleetError::Usage(
                    "opencode does not support reasoning effort.".into(),
                ));
            }
        };
        if choices.contains(&effort) {
            Ok(())
        } else {
            Err(CafleetError::Usage(format!(
                "--effort for the {} backend must be one of {} (got '{effort}').",
                self.name(),
                choices.join(", ")
            )))
        }
    }

    pub(crate) fn ensure_available(
        self,
        probe: &dyn SpawnProbe,
        mode: &str,
    ) -> Result<PathBuf, CafleetError> {
        let config = self.config_dir(&|name| probe.env_var(name), &probe.home_dir()?)?;
        let executable = probe.resolve_binary(self.name()).ok_or_else(|| {
            CafleetError::App(format!("binary {} not found on PATH", self.name()))
        })?;
        if let Some(preset) = self.required_preset(&config, mode) {
            std::fs::read(&preset).map_err(|error| CafleetError::App(format!(
                "cannot read opencode agent preset at {}: {error}; run 'cafleet setup --coding-agent opencode' first", preset.display()
            )))?;
        }
        if self == Self::Pi {
            let extension = pi_extension(&config);
            std::fs::read(&extension).map_err(|error| CafleetError::App(format!(
                "cannot read the CAFleet pi extension at {}: {error}; run 'cafleet setup --coding-agent pi' first", extension.display()
            )))?;
        }
        Ok(executable)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_argv(
        self,
        executable: &Path,
        prompt: &str,
        display_name: &str,
        cwd: &Path,
        model: Option<&str>,
        effort: Option<&str>,
        policy: &LaunchPolicySelection,
    ) -> Result<Vec<String>, CafleetError> {
        let program = executable
            .to_str()
            .filter(|_| executable.is_absolute())
            .ok_or_else(|| {
                CafleetError::App(format!(
                    "native executable must be an absolute UTF-8 path (got {})",
                    executable.display()
                ))
            })?;
        let mut argv =
            self.build_spawn_argv_with_policy(prompt, display_name, cwd, model, effort, policy)?;
        argv[0] = program.into();
        Ok(argv)
    }
}

pub(crate) fn pi_extension(base: &Path) -> PathBuf {
    base.join("extensions/cafleet.ts")
}

pub(crate) fn coding_agent(name: &str) -> Option<CodingAgent> {
    CodingAgent::all()
        .iter()
        .copied()
        .find(|agent| agent.name() == name)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LaunchPolicySelection {
    requested_mode: Option<String>,
    resolved_mode: &'static str,
    mode_source: ModeSource,
    permission_argv: Vec<String>,
}

impl LaunchPolicySelection {
    pub(crate) fn requested_mode(&self) -> Option<&str> {
        self.requested_mode.as_deref()
    }
    pub(crate) fn resolved_mode(&self) -> &'static str {
        self.resolved_mode
    }
    pub(crate) fn mode_source(&self) -> ModeSource {
        self.mode_source
    }
    pub(crate) fn permission_argv(&self) -> &[String] {
        &self.permission_argv
    }
}
impl CodingAgent {
    fn default_policy(self) -> &'static str {
        match self {
            Self::Claude => "auto",
            Self::Codex => "approve-for-me",
            Self::Opencode => "cafleet",
            Self::Hermes => "smart",
            Self::Pi => "cafleet",
        }
    }
    pub(crate) fn permission_mode_help() -> String {
        let catalog = Self::all()
            .iter()
            .map(|agent| {
                format!(
                    "{}: {} (default {})",
                    agent.name(),
                    agent.policy_choices().join(", "),
                    agent.default_policy()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("Launch-only native permission policy for the resolved backend — {catalog}")
    }
    pub(crate) fn policy_choices(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &[
                "default",
                "manual",
                "dontAsk",
                "acceptEdits",
                "auto",
                "bypassPermissions",
                "plan",
            ],
            Self::Codex => &["on-request", "never", "approve-for-me"],
            Self::Opencode => &["cafleet", "cafleet-ask"],
            Self::Hermes => &["smart", "manual"],
            Self::Pi => &["cafleet", "cafleet-ask"],
        }
    }
    fn permission_arguments(self, mode: &str) -> Vec<String> {
        match self {
            Self::Claude => vec!["--permission-mode".into(), mode.into()],
            Self::Codex if mode == "approve-for-me" => vec!["--approve-for-me".into()],
            Self::Codex => vec![
                "--ask-for-approval".into(),
                mode.into(),
                "--sandbox".into(),
                "workspace-write".into(),
            ],
            Self::Opencode => vec!["--agent".into(), mode.into()],
            Self::Hermes => vec![],
            Self::Pi => vec!["--cafleet-gate".into(), mode.into()],
        }
    }
    pub(crate) fn resolve_policy(
        self,
        requested: Option<&str>,
    ) -> Result<LaunchPolicySelection, CafleetError> {
        let choices = self.policy_choices();
        let mode = match requested {
            Some(value) => choices
                .iter()
                .copied()
                .find(|choice| *choice == value)
                .ok_or_else(|| {
                    CafleetError::Usage(format!(
                        "--permission-mode for {} must be one of {} (got '{value}')",
                        self.name(),
                        choices.join(", ")
                    ))
                })?,
            None => self.default_policy(),
        };
        Ok(LaunchPolicySelection {
            requested_mode: requested.map(str::to_owned),
            resolved_mode: mode,
            mode_source: if requested.is_some() {
                ModeSource::Explicit
            } else {
                ModeSource::Default
            },
            permission_argv: self.permission_arguments(mode),
        })
    }
    pub(crate) fn build_spawn_argv_with_policy(
        self,
        prompt: &str,
        display_name: &str,
        cwd: &Path,
        model: Option<&str>,
        effort: Option<&str>,
        policy: &LaunchPolicySelection,
    ) -> Result<Vec<String>, CafleetError> {
        self.validate_model(model)?;
        self.validate_effort(effort)?;
        assert!(
            self.validate_prompt(prompt).is_ok(),
            "member create validates the spawn prompt before argv construction"
        );
        let mut argv = vec![self.name().into()];
        match self {
            Self::Opencode => argv.extend(["mini".into(), "--standalone".into()]),
            Self::Hermes => {
                let cwd = cwd.to_str().ok_or_else(|| {
                    CafleetError::App(format!(
                        "member working directory must be UTF-8 (got {})",
                        cwd.display()
                    ))
                })?;
                argv.extend(["chat", "--cli", "--source", "tool", "--in", cwd].map(String::from));
            }
            Self::Claude | Self::Codex | Self::Pi => {}
        }
        argv.extend(policy.permission_argv.clone());
        if self == Self::Claude {
            argv.extend([
                "--prompt-suggestions".into(),
                "false".into(),
                "--allowedTools".into(),
                "Bash(cafleet message *)".into(),
                "--name".into(),
                display_name.into(),
            ]);
        }
        if self == Self::Pi {
            argv.extend([
                "--no-approve".into(),
                "--tui-mode".into(),
                "fullscreen".into(),
                "--name".into(),
                display_name.into(),
            ]);
        }
        if let Some(model) = model {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = effort {
            match self {
                Self::Claude => argv.extend(["--effort".into(), effort.into()]),
                Self::Codex => argv.push(format!("--config=model_reasoning_effort={effort}")),
                Self::Hermes => argv.extend(["--reasoning".into(), effort.into()]),
                Self::Pi => argv.extend(["--thinking".into(), effort.into()]),
                Self::Opencode => unreachable!("effort validated before argv construction"),
            }
        }
        match self {
            Self::Opencode => argv.push("--prompt".into()),
            Self::Hermes => argv.push("-q".into()),
            Self::Pi => argv.push("--".into()),
            Self::Claude | Self::Codex => {}
        }
        argv.push(prompt.into());
        Ok(argv)
    }
}

#[cfg(test)]
impl CodingAgent {
    pub(crate) fn validate_spawn_policy_argv(
        self,
        argv: &[String],
        policy: &LaunchPolicySelection,
    ) -> Result<(), CafleetError> {
        let invalid = || CafleetError::App("capture emitted permission argv mismatch".into());
        let cwd = match self {
            Self::Hermes => {
                let index = argv
                    .iter()
                    .position(|value| value == "--in")
                    .ok_or_else(invalid)?;
                Path::new(argv.get(index + 1).ok_or_else(invalid)?)
            }
            _ => Path::new("/"),
        };
        let flag_prompt = usize::from(matches!(self, Self::Opencode | Self::Hermes | Self::Pi));
        let baseline = self.build_spawn_argv_with_policy("", "", cwd, None, None, policy)?;
        let name_index = matches!(self, Self::Claude | Self::Pi).then(|| {
            baseline
                .iter()
                .position(|value| value == "--name")
                .expect("Claude and pi name option")
                + 1
        });
        let display_name = match name_index {
            Some(index) => argv.get(index).ok_or_else(invalid)?.as_str(),
            None => "",
        };
        let mut cursor = baseline.len() - 1 - flag_prompt;
        let model = if argv.len() > cursor + 1 && argv[cursor] == "--model" {
            let value = argv.get(cursor + 1).ok_or_else(invalid)?;
            cursor += 2;
            Some(value.as_str())
        } else {
            None
        };
        let effort = if argv.len() > cursor + 1 {
            match self {
                Self::Claude if argv[cursor] == "--effort" => {
                    let value = argv.get(cursor + 1).ok_or_else(invalid)?;
                    cursor += 2;
                    Some(value.as_str())
                }
                Self::Pi if argv[cursor] == "--thinking" => {
                    let value = argv.get(cursor + 1).ok_or_else(invalid)?;
                    cursor += 2;
                    Some(value.as_str())
                }
                Self::Hermes if argv[cursor] == "--reasoning" => {
                    let value = argv.get(cursor + 1).ok_or_else(invalid)?;
                    cursor += 2;
                    Some(value.as_str())
                }
                Self::Codex => {
                    let value = argv[cursor].strip_prefix("--config=model_reasoning_effort=");
                    if value.is_some() {
                        cursor += 1;
                    }
                    value
                }
                _ => None,
            }
        } else {
            None
        };
        let tail = argv.get(cursor..).ok_or_else(invalid)?;
        let prompt = match (self, tail) {
            (_, []) => None,
            (Self::Opencode, [flag, prompt]) if flag == "--prompt" => Some(prompt.as_str()),
            (Self::Hermes, [flag, prompt]) if flag == "-q" => Some(prompt.as_str()),
            (Self::Pi, [flag, prompt]) if flag == "--" => Some(prompt.as_str()),
            (Self::Claude | Self::Codex, [prompt]) => Some(prompt.as_str()),
            _ => return Err(invalid()),
        };
        let mut expected = self.build_spawn_argv_with_policy(
            prompt.unwrap_or(""),
            display_name,
            cwd,
            model,
            effort,
            policy,
        )?;
        if prompt.is_none() {
            expected.truncate(expected.len() - 1 - flag_prompt);
        }
        if argv.len() != expected.len() || argv.get(1..) != expected.get(1..) {
            return Err(invalid());
        }
        Ok(())
    }
}

const OUTPUT_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct NativeVersion(semver::Version);

impl TryFrom<String> for NativeVersion {
    type Error = &'static str;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or("canonical SemVer version required")
    }
}

impl NativeVersion {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        semver::Version::parse(text).ok().map(Self)
    }
    pub(crate) fn precedence(&self, other: &Self) -> Ordering {
        self.0.cmp_precedence(&other.0)
    }
}

pub(crate) struct NativeLaunchIdentity {
    pub(crate) native_version: Option<String>,
    pub(crate) coverage: Coverage,
    pub(crate) warning: Option<String>,
}

pub(crate) fn native_launch_identity(
    backend: CodingAgent,
    probe: Result<String, CafleetError>,
    catalog: &ProfileCatalog,
) -> Result<NativeLaunchIdentity, CafleetError> {
    match probe {
        Ok(version) => {
            if let Some(minimum) = catalog.unsupported_minimum(backend, &version) {
                return Err(CafleetError::App(format!(
                    "unsupported native version for {}: {version}; minimum supported version is {minimum}",
                    backend.name()
                )));
            }
            let coverage = catalog.detection_coverage(backend, Some(&version));
            Ok(NativeLaunchIdentity {
                native_version: Some(version),
                coverage,
                warning: None,
            })
        }
        Err(cause) => Ok(NativeLaunchIdentity {
            native_version: None,
            coverage: Coverage::Unknown,
            warning: Some(format!(
                "Warning: native version unavailable for {}: {}; coverage unknown.",
                backend.name(),
                cause.message()
            )),
        }),
    }
}

pub(crate) use crate::runtime::system::BoundedOutput;

/// Runs a native command with null stdin and bounded output, returning its
/// UTF-8 stdout or the failure detail that callers prefix with the command.
pub(crate) fn run_bounded(
    executable: &Path,
    args: &[&str],
    environment: &[(&str, &Path)],
    deadline: Instant,
) -> Result<String, String> {
    let BoundedOutput {
        status,
        stdout,
        stderr,
    } = run_bounded_output(executable, args, environment, deadline)?;
    if !status.success() {
        let first = String::from_utf8_lossy(&stderr)
            .lines()
            .next()
            .map(str::to_owned);
        return Err(match first {
            Some(line) if !line.trim().is_empty() => format!("exited with {status}: {line}"),
            _ => format!("exited with {status}"),
        });
    }
    String::from_utf8(stdout).map_err(|_| "printed non-UTF-8 output".to_owned())
}

/// Runs a native command like [`run_bounded`] and returns its exit status and
/// both output streams; only a start failure or the deadline is an error.
pub(crate) fn run_bounded_output(
    executable: &Path,
    args: &[&str],
    environment: &[(&str, &Path)],
    deadline: Instant,
) -> Result<BoundedOutput, String> {
    use crate::runtime::system::BoundedFailure;
    crate::runtime::system::run_until_exit(executable, args, environment, deadline, OUTPUT_BYTES)
        .map_err(|failure| match failure {
            BoundedFailure::NotStarted(cause) => format!("could not start: {cause}"),
            BoundedFailure::Deadline { exited: false } => {
                "did not finish within its deadline".into()
            }
            BoundedFailure::Deadline { exited: true } => {
                "output did not close within its deadline".into()
            }
            BoundedFailure::Io(cause) => format!("output could not be read: {cause}"),
        })
}

impl CodingAgent {
    pub(crate) fn probe_native_version(
        self,
        executable: &Path,
        deadline: Instant,
    ) -> Result<String, CafleetError> {
        let fail =
            |detail: String| CafleetError::App(format!("{} --version {detail}", self.name()));
        let text = run_bounded(executable, &["--version"], &[], deadline).map_err(fail)?;
        let first = text
            .lines()
            .next()
            .ok_or_else(|| fail("printed no output".into()))?;
        self.version_from_line(first)
            .map(str::to_owned)
            .map_err(|cause| fail(cause.into()))
    }

    pub(crate) fn version_from_line(self, line: &str) -> Result<&str, &'static str> {
        match self {
            Self::Hermes => hermes_version(line),
            Self::Claude | Self::Codex | Self::Opencode | Self::Pi => version_token(line),
        }
    }
}

fn hermes_version(line: &str) -> Result<&str, &'static str> {
    static RULE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^Hermes Agent v\S+ \((?P<version>[^)]+)\)$")
            .expect("the Hermes version rule is a valid pattern")
    });
    let version = RULE
        .captures(line)
        .ok_or("printed no Hermes version line")?
        .name("version")
        .expect("the Hermes version rule captures its version")
        .as_str();
    NativeVersion::parse(version)
        .map(|_| version)
        .ok_or("printed a non-SemVer Hermes version")
}

pub(crate) fn version_token(line: &str) -> Result<&str, &'static str> {
    let tokens = line
        .split_whitespace()
        .map(|token| token.strip_prefix('v').unwrap_or(token))
        .filter(|token| NativeVersion::parse(token).is_some())
        .collect::<Vec<_>>();
    match tokens.as_slice() {
        [version] => Ok(version),
        [] => Err("printed no version token"),
        _ => Err("printed several version tokens"),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::coding_agent::CodingAgent;
    use crate::coding_agent::SpawnProbe;
    use crate::error::CafleetError;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::Duration;

    pub(crate) fn executable_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    pub(crate) fn run_stub<T>(
        mut run: impl FnMut() -> Result<T, CafleetError>,
    ) -> Result<T, CafleetError> {
        // A parallel test's fork can briefly inherit a fresh stub's write descriptor.
        for _ in 0..50 {
            match run() {
                Err(error) if error.message().contains("Text file busy") => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                result => return result,
            }
        }
        panic!("an executable stub stayed busy")
    }

    /// A spawn probe that resolves each name in `binaries` and reads each variable from `env`.
    pub(crate) struct FakeProbe {
        pub(crate) home: PathBuf,
        pub(crate) binaries: BTreeMap<String, PathBuf>,
        pub(crate) env: BTreeMap<String, String>,
    }

    impl FakeProbe {
        /// Every backend binary resolves to `/fixture/bin/<name>`; no variable is set.
        pub(crate) fn installed(home: &Path) -> Self {
            Self {
                home: home.into(),
                binaries: CodingAgent::all()
                    .iter()
                    .map(|agent| {
                        (
                            agent.name().to_owned(),
                            Path::new("/fixture/bin").join(agent.name()),
                        )
                    })
                    .collect(),
                env: BTreeMap::new(),
            }
        }

        pub(crate) fn with_binary(mut self, name: &str, path: &Path) -> Self {
            self.binaries.insert(name.into(), path.into());
            self
        }

        pub(crate) fn with_env(mut self, name: &str, value: &str) -> Self {
            self.env.insert(name.into(), value.into());
            self
        }
    }

    impl SpawnProbe for FakeProbe {
        fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
            self.binaries.get(name).cloned()
        }
        fn home_dir(&self) -> Result<PathBuf, CafleetError> {
            Ok(self.home.clone())
        }
        fn env_var(&self, name: &str) -> Result<Option<String>, CafleetError> {
            Ok(self.env.get(name).cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::test_support::FakeProbe;
    use crate::coding_agent::*;
    use std::time::Duration;

    const CWD: &str = "/fixture/member work dir";

    fn cwd() -> &'static Path {
        Path::new(CWD)
    }

    fn only(var: &str, value: &str) -> impl Fn(&str) -> Result<Option<String>, CafleetError> {
        let (var, value) = (var.to_owned(), value.to_owned());
        move |name: &str| Ok((name == var).then(|| value.clone()))
    }

    #[test]
    fn literal_argv_preserves_permission_and_optional_flag_boundaries() {
        let prompt = "--model 'quoted value'\n{member_id} \"next\"";
        for (agent, base) in [
            (
                CodingAgent::Claude,
                vec![
                    "claude",
                    "--permission-mode",
                    "auto",
                    "--prompt-suggestions",
                    "false",
                    "--allowedTools",
                    "Bash(cafleet message *)",
                    "--name",
                    "display name",
                ],
            ),
            (CodingAgent::Codex, vec!["codex", "--approve-for-me"]),
            (
                CodingAgent::Opencode,
                vec!["opencode", "mini", "--standalone", "--agent", "cafleet-ask"],
            ),
            (
                CodingAgent::Hermes,
                vec!["hermes", "chat", "--cli", "--source", "tool", "--in", CWD],
            ),
            (
                CodingAgent::Pi,
                vec![
                    "pi",
                    "--cafleet-gate",
                    "cafleet-ask",
                    "--no-approve",
                    "--tui-mode",
                    "fullscreen",
                    "--name",
                    "display name",
                ],
            ),
        ] {
            for configured in [false, true] {
                let mut expected = base
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>();
                if configured {
                    expected.extend(["--model".into(), "provider/model with spaces".into()]);
                }
                let effort = if configured && agent != CodingAgent::Opencode {
                    Some("high")
                } else {
                    None
                };
                match (agent, effort) {
                    (CodingAgent::Claude, Some(value)) => {
                        expected.extend(["--effort".into(), value.into()])
                    }
                    (CodingAgent::Codex, Some(value)) => {
                        expected.push(format!("--config=model_reasoning_effort={value}"))
                    }
                    (CodingAgent::Hermes, Some(value)) => {
                        expected.extend(["--reasoning".into(), value.into()])
                    }
                    (CodingAgent::Pi, Some(value)) => {
                        expected.extend(["--thinking".into(), value.into()])
                    }
                    _ => {}
                }
                match agent {
                    CodingAgent::Opencode => expected.push("--prompt".into()),
                    CodingAgent::Hermes => expected.push("-q".into()),
                    CodingAgent::Pi => expected.push("--".into()),
                    _ => {}
                }
                expected.push(prompt.into());
                assert_eq!(
                    agent
                        .build_spawn_argv_with_policy(
                            prompt,
                            "display name",
                            cwd(),
                            configured.then_some("provider/model with spaces"),
                            effort,
                            &agent.resolve_policy(None).unwrap()
                        )
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn model_and_effort_validation_preserves_backend_rules() {
        for agent in CodingAgent::all() {
            assert!(agent.validate_model(None).is_ok());
            assert!(agent.validate_effort(None).is_ok());
            assert!(agent.validate_effort(Some("turbo")).is_err());
        }
        for model in ["provider/model", "a/b/c"] {
            assert!(CodingAgent::Opencode.validate_model(Some(model)).is_ok());
        }
        for model in ["model", "/model", "provider/", ""] {
            assert!(CodingAgent::Opencode.validate_model(Some(model)).is_err());
        }
        for (agent, valid, invalid) in [
            (
                CodingAgent::Claude,
                &["low", "medium", "high", "xhigh", "max"][..],
                "minimal",
            ),
            (
                CodingAgent::Codex,
                &["minimal", "low", "medium", "high", "xhigh"][..],
                "max",
            ),
            (
                CodingAgent::Hermes,
                &[
                    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
                ][..],
                "HIGH",
            ),
            (
                CodingAgent::Pi,
                &["off", "minimal", "low", "medium", "high", "xhigh", "max"][..],
                "none",
            ),
        ] {
            for effort in valid {
                agent.validate_effort(Some(effort)).unwrap();
            }
            assert!(agent.validate_effort(Some(invalid)).is_err());
            agent.validate_model(Some("arbitrary model")).unwrap();
        }
        let failure = CodingAgent::Hermes
            .validate_effort(Some("HIGH"))
            .expect_err("an unknown hermes effort fails");
        assert!(matches!(failure, CafleetError::Usage(_)));
        assert_eq!(
            failure.message(),
            "--effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got 'HIGH')."
        );
        assert!(
            CodingAgent::Hermes
                .build_spawn_argv_with_policy(
                    "assignment",
                    "worker",
                    cwd(),
                    None,
                    Some("HIGH"),
                    &CodingAgent::Hermes.resolve_policy(None).unwrap()
                )
                .is_err()
        );
        assert!(CodingAgent::Opencode.validate_effort(Some("high")).is_err());
    }

    #[test]
    fn selected_preflight_checks_paths_then_binary_and_relocated_required_preset() {
        let dir = tempfile::tempdir().unwrap();
        for agent in CodingAgent::all() {
            let mode = agent.resolve_policy(None).unwrap().resolved_mode();
            let config = dir.path().join(agent.name());
            let mut probe =
                FakeProbe::installed(dir.path()).with_env(agent.config_variable(), "relative");
            assert!(
                agent
                    .ensure_available(&probe, mode)
                    .unwrap_err()
                    .message()
                    .contains(agent.config_variable())
            );
            probe = probe.with_env(agent.config_variable(), config.to_str().unwrap());
            let installed = probe.binaries.remove(agent.name()).unwrap();
            assert!(
                agent
                    .ensure_available(&probe, mode)
                    .unwrap_err()
                    .message()
                    .contains("not found on PATH")
            );
            probe = probe.with_binary(agent.name(), &installed);
            if *agent == CodingAgent::Opencode {
                let preset = config.join(format!("agents/{mode}.md"));
                std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
                std::fs::write(&preset, "preset").unwrap();
            }
            if *agent == CodingAgent::Pi {
                let extension = config.join("extensions/cafleet.ts");
                std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
                std::fs::write(&extension, "extension").unwrap();
            }
            assert_eq!(agent.ensure_available(&probe, mode).unwrap(), installed);
        }
        assert!(coding_agent("Claude").is_none());
        assert!(coding_agent("unknown").is_none());
    }

    fn every_config_variable(name: &str) -> Result<Option<String>, CafleetError> {
        Ok(match name {
            "CLAUDE_CONFIG_DIR" => Some("/cfg/claude dir/".into()),
            "CODEX_HOME" => Some("/cfg/codex".into()),
            "OPENCODE_CONFIG_DIR" => Some("/cfg/opencode".into()),
            "HERMES_HOME" => Some("/cfg/hermes home".into()),
            "PI_CODING_AGENT_DIR" => Some("/cfg/pi agent".into()),
            _ => None,
        })
    }

    #[test]
    fn forwarded_config_returns_only_the_selected_variable_with_the_creator_exact_value() {
        for &agent in CodingAgent::all() {
            let expected = every_config_variable(agent.config_variable())
                .unwrap()
                .unwrap();
            assert_eq!(
                agent.forwarded_config(&every_config_variable).unwrap(),
                Some((agent.config_variable().to_owned(), expected)),
                "{agent:?} forwards only its own variable with the creator's exact value"
            );
        }
    }

    #[test]
    fn forwarded_config_is_absent_when_the_selected_variable_is_unset() {
        for &agent in CodingAgent::all() {
            let others = |name: &str| {
                if name == agent.config_variable() {
                    Ok(None)
                } else {
                    every_config_variable(name)
                }
            };
            assert_eq!(
                agent.forwarded_config(&others).unwrap(),
                None,
                "{agent:?} forwards no default and no other backend's variable"
            );
        }
    }

    #[test]
    fn forwarded_config_rejects_an_invalid_selected_value_with_the_config_dir_error() {
        for &agent in CodingAgent::all() {
            let lookup = only(agent.config_variable(), "relative/cfg");
            let expected = format!(
                "{} must be an absolute path (got 'relative/cfg')",
                agent.config_variable()
            );
            let forwarded = agent
                .forwarded_config(&lookup)
                .expect_err("an invalid config location fails before registration");
            assert!(matches!(forwarded, CafleetError::App(_)));
            assert_eq!(forwarded.message(), expected);
            assert_eq!(
                agent.config_dir(&lookup, &home()).unwrap_err().message(),
                expected
            );
        }
    }

    #[test]
    fn required_preset_is_the_selected_opencode_agent_file_and_absent_for_other_backends() {
        let config = Path::new("/fixture/preset-base");
        for mode in CodingAgent::Opencode.policy_choices() {
            assert_eq!(
                CodingAgent::Opencode.required_preset(config, mode),
                Some(config.join(format!("agents/{mode}.md")))
            );
        }
        for agent in [CodingAgent::Claude, CodingAgent::Codex, CodingAgent::Hermes] {
            for mode in agent.policy_choices() {
                assert_eq!(agent.required_preset(config, mode), None, "{mode}");
            }
        }
    }

    #[test]
    fn opencode_launch_policy_records_the_agent_for_the_omitted_and_explicit_modes() {
        let omitted = CodingAgent::Opencode.resolve_policy(None).unwrap();
        assert_eq!(omitted.requested_mode(), None);
        assert_eq!(omitted.resolved_mode(), "cafleet-ask");
        assert_eq!(omitted.mode_source(), ModeSource::Default);
        assert_eq!(omitted.permission_argv(), ["--agent", "cafleet-ask"]);
        let config = Path::new("/fixture/preset-base");
        assert_eq!(
            CodingAgent::Opencode.required_preset(config, omitted.resolved_mode()),
            Some(config.join("agents/cafleet-ask.md"))
        );
        let explicit = CodingAgent::Opencode
            .resolve_policy(Some("cafleet"))
            .unwrap();
        assert_eq!(explicit.requested_mode(), Some("cafleet"));
        assert_eq!(explicit.resolved_mode(), "cafleet");
        assert_eq!(explicit.mode_source(), ModeSource::Explicit);
        assert_eq!(explicit.permission_argv(), ["--agent", "cafleet"]);
        assert_eq!(
            CodingAgent::Opencode.policy_choices(),
            ["cafleet", "cafleet-ask"]
        );
    }

    #[test]
    fn selected_preflight_reads_only_the_selected_opencode_preset_with_the_shipped_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("preset-base");
        let probe = FakeProbe::installed(dir.path())
            .with_env("OPENCODE_CONFIG_DIR", config.to_str().unwrap());
        let agents = config.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        let deny_preset = agents.join("cafleet.md");
        let ask_preset = agents.join("cafleet-ask.md");
        let assert_missing = |mode: &str, preset: &Path| {
            let failure = CodingAgent::Opencode
                .ensure_available(&probe, mode)
                .expect_err("a missing selected preset fails the preflight");
            assert!(
                matches!(failure, CafleetError::App(_)),
                "exit 1: {}",
                failure.message()
            );
            let text = failure.message();
            let cause = text
                .strip_prefix(&format!(
                    "cannot read opencode agent preset at {}: ",
                    preset.display()
                ))
                .and_then(|rest| {
                    rest.strip_suffix("; run 'cafleet setup --coding-agent opencode' first")
                })
                .unwrap_or_else(|| panic!("shipped missing-preset error: {text}"));
            assert!(!cause.is_empty(), "the read cause is carried: {text}");
        };

        std::fs::write(&deny_preset, "deny preset").unwrap();
        CodingAgent::Opencode
            .ensure_available(&probe, "cafleet")
            .unwrap();
        assert_missing("cafleet-ask", &ask_preset);

        std::fs::remove_file(&deny_preset).unwrap();
        std::fs::write(&ask_preset, "ask preset").unwrap();
        assert_eq!(
            CodingAgent::Opencode
                .ensure_available(&probe, "cafleet-ask")
                .unwrap(),
            PathBuf::from("/fixture/bin/opencode"),
            "the unselected cafleet preset is not consulted"
        );
        assert_missing("cafleet", &deny_preset);

        std::fs::remove_file(&ask_preset).unwrap();
        std::fs::create_dir(&ask_preset).unwrap();
        assert_missing("cafleet-ask", &ask_preset);
    }

    fn home() -> PathBuf {
        PathBuf::from("/home/tester")
    }

    fn default_config_dir(agent: CodingAgent) -> PathBuf {
        home().join(match agent {
            CodingAgent::Claude => ".claude",
            CodingAgent::Codex => ".codex",
            CodingAgent::Opencode => ".config/opencode",
            CodingAgent::Hermes => ".hermes",
            CodingAgent::Pi => ".pi/agent",
        })
    }

    #[test]
    fn unset_variable_resolves_the_default() {
        for &agent in CodingAgent::all() {
            assert_eq!(
                agent.config_dir(&|_| Ok(None), &home()).unwrap(),
                default_config_dir(agent),
                "default for {agent:?}"
            );
        }
    }

    #[test]
    fn absolute_value_is_used_verbatim() {
        for &agent in CodingAgent::all() {
            assert_eq!(
                agent
                    .config_dir(&only(agent.config_variable(), "/custom/cfg"), &home())
                    .unwrap(),
                PathBuf::from("/custom/cfg")
            );
        }
    }

    #[test]
    fn each_resolver_reads_only_its_own_variable() {
        for &agent in CodingAgent::all() {
            let var = agent.config_variable();
            let lookup = move |name: &str| Ok((name != var).then(|| "/other/cfg".to_string()));
            assert_eq!(
                agent.config_dir(&lookup, &home()).unwrap(),
                default_config_dir(agent),
                "{var}'s resolver must ignore every other variable"
            );
        }
    }

    #[test]
    fn native_launch_probes_and_launches_the_same_resolved_executable() {
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let versions = dir.path().join("versions/2.1.302");
        std::fs::create_dir_all(&versions).unwrap();
        let executable = executable_stub(
            &versions,
            "claude",
            "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nread -r ignored || true\nprintf '2.1.302 (Claude Code)\\n'\nprintf 'ignored\\n' >&2",
        );
        let backend = CodingAgent::Claude;
        let probe = FakeProbe::installed(dir.path())
            .with_binary("claude", &executable)
            .with_env(
                "CLAUDE_CONFIG_DIR",
                dir.path().join("claude-config").to_str().unwrap(),
            );
        let resolved = backend.ensure_available(&probe, "dontAsk").unwrap();
        assert_eq!(resolved, executable);
        assert_eq!(
            run_stub(
                || backend.probe_native_version(&resolved, Instant::now() + Duration::from_secs(2))
            )
            .unwrap(),
            "2.1.302",
            "the probe runs the resolved executable"
        );
        let policy = backend.resolve_policy(None).unwrap();
        let bare = backend
            .build_spawn_argv_with_policy(
                "assignment",
                "worker",
                cwd(),
                None,
                Some("high"),
                &policy,
            )
            .unwrap();
        assert_eq!(bare[0], "claude");
        let launched = backend
            .launch_argv(
                &resolved,
                "assignment",
                "worker",
                cwd(),
                None,
                Some("high"),
                &policy,
            )
            .unwrap();
        assert_eq!(
            launched[0],
            executable.to_str().unwrap(),
            "argv[0] is the probed path"
        );
        assert_eq!(launched[1..], bare[1..]);
        assert!(
            backend
                .launch_argv(
                    Path::new("claude"),
                    "assignment",
                    "worker",
                    cwd(),
                    None,
                    None,
                    &policy
                )
                .is_err(),
            "a launched argv[0] is always an absolute path"
        );
    }

    #[test]
    fn version_token_records_the_documented_first_lines_without_a_leading_v() {
        for (line, expected) in [
            ("2.1.302 (Claude Code)", Ok("2.1.302")),
            ("codex-cli 0.163.0", Ok("0.163.0")),
            ("opencode v2.1.4", Ok("2.1.4")),
            ("v1.2.3-beta.1", Ok("1.2.3-beta.1")),
            ("codex 2.1 build 2.1.302", Ok("2.1.302")),
            ("codex 2.1", Err("printed no version token")),
            ("opencode vv2.1.4", Err("printed no version token")),
            ("v2.1.4 2.1.5", Err("printed several version tokens")),
        ] {
            assert_eq!(version_token(line), expected, "{line}");
        }
    }

    mod native_version_probe {
        use crate::coding_agent::test_support::executable_stub as stub;
        use crate::coding_agent::test_support::run_stub;
        use crate::coding_agent::tests::*;
        use std::time::Instant;

        fn probe(executable: &Path) -> Result<String, CafleetError> {
            run_stub(|| {
                CodingAgent::Claude
                    .probe_native_version(executable, Instant::now() + Duration::from_secs(2))
            })
        }

        #[test]
        fn bounded_runs_name_an_unstartable_program_and_each_expired_deadline() {
            let dir = tempfile::tempdir().unwrap();
            let run = |program: &Path| {
                let mut result = None;
                for _ in 0..50 {
                    let attempt = run_bounded_output(
                        program,
                        &[],
                        &[],
                        Instant::now() + Duration::from_millis(300),
                    );
                    match attempt {
                        Err(cause) if cause.contains("Text file busy") => {
                            std::thread::sleep(Duration::from_millis(20))
                        }
                        other => {
                            result = Some(other);
                            break;
                        }
                    }
                }
                result
                    .expect("an executable stub stayed busy")
                    .err()
                    .expect("the bounded run fails")
            };
            assert!(
                run(&dir.path().join("absent")).starts_with("could not start: "),
                "unstartable binary"
            );
            assert_eq!(
                run(&stub(dir.path(), "sleeping", "exec /bin/sleep 30")),
                "did not finish within its deadline"
            );
            assert_eq!(
                run(&stub(dir.path(), "holding", "/bin/sleep 30 & exit 0")),
                "output did not close within its deadline"
            );
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
            // A stub launch can take about half a second on a macOS host, so the bound leaves the
            // stub time to record its pid and reach its sleep before the probe times out. A loaded
            // host can still kill the stub before it records its pid; only an attempt whose stub
            // recorded its pid can show the reap, so such an attempt is repeated.
            let bound = Duration::from_secs(2);
            let pid = (0..3)
                .find_map(|_| {
                    let start = Instant::now();
                    let result = CodingAgent::Claude.probe_native_version(&path, start + bound);
                    assert!(result.is_err());
                    assert!(
                        start.elapsed() < bound + Duration::from_secs(1),
                        "{:?}",
                        start.elapsed()
                    );
                    std::fs::read_to_string(&pid_file)
                        .ok()
                        .filter(|pid| !pid.is_empty())
                })
                .expect("the stub records its pid within the bound in one of three attempts");
            let pid: i32 = pid.parse().unwrap();
            assert_eq!(
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH),
                "the timed-out probe is killed and reaped"
            );
        }

        use crate::coding_agent::catalog::test_support::inspection_fixture_definition;
        use crate::coding_agent::catalog::test_support::thresholds;

        fn launched(
            backend: CodingAgent,
            version: &str,
            catalog: &ProfileCatalog,
        ) -> NativeLaunchIdentity {
            native_launch_identity(backend, Ok(version.into()), catalog)
                .unwrap_or_else(|error| panic!("{backend:?} {version}: {}", error.message()))
        }

        #[test]
        fn native_launch_identity_selects_the_boundary_profile_from_its_minimum_onwards() {
            let catalog = ProfileCatalog::from_definitions(&thresholds(&[
                ("low", "1.0.0"),
                ("high", "2.0.0"),
            ]))
            .unwrap();
            for version in ["1.0.0", "1.5.0", "2.0.0", "3.0.0-beta.1"] {
                let identity = launched(CodingAgent::Codex, version, &catalog);
                assert_eq!(identity.native_version.as_deref(), Some(version));
                assert_eq!(
                    identity.coverage,
                    catalog.detection_coverage(CodingAgent::Codex, Some(version)),
                    "{version}"
                );
                assert_eq!(identity.coverage, Coverage::Tested, "{version}");
                assert_eq!(
                    identity.warning, None,
                    "{version} launches without a version-mismatch warning"
                );
            }
        }

        #[test]
        fn native_launch_identity_rejects_a_known_version_below_the_lowest_boundary() {
            let several = ProfileCatalog::from_definitions(&thresholds(&[
                ("high", "2.0.0"),
                ("low", "1.0.0"),
            ]))
            .unwrap();
            let error =
                match native_launch_identity(CodingAgent::Codex, Ok("0.9.0".into()), &several) {
                    Err(error) => error,
                    Ok(_) => panic!("0.9.0 is below every boundary"),
                };
            assert_eq!(error.exit_code(), 1);
            assert_eq!(
                error.message(),
                "unsupported native version for codex: 0.9.0; minimum supported version is 1.0.0"
            );
        }

        #[test]
        fn native_launch_identity_launches_a_failed_probe_with_unknown_coverage_and_its_warning() {
            let catalog = ProfileCatalog::bundled();
            let failed = native_launch_identity(
                CodingAgent::Codex,
                Err(CafleetError::App("probe exited with status 1".into())),
                catalog,
            )
            .unwrap_or_else(|error| panic!("a failed probe still launches: {}", error.message()));
            assert_eq!(failed.native_version, None);
            assert_eq!(failed.coverage, Coverage::Unknown);
            assert_eq!(
                failed.warning.as_deref(),
                Some(
                    "Warning: native version unavailable for codex: probe exited with status 1; coverage unknown."
                )
            );
        }

        #[test]
        fn native_launch_identity_without_any_profile_for_the_backend_records_unknown_silently() {
            let catalog =
                ProfileCatalog::from_definitions(&inspection_fixture_definition()).unwrap();
            for backend in [CodingAgent::Claude, CodingAgent::Opencode] {
                let identity = launched(backend, "2.1.302", &catalog);
                assert_eq!(identity.native_version.as_deref(), Some("2.1.302"));
                assert_eq!(identity.coverage, Coverage::Unknown);
                assert_eq!(identity.warning, None);
            }
        }
    }

    fn permission_argv(backend: CodingAgent, selected: &str) -> Vec<String> {
        match backend {
            CodingAgent::Claude => vec!["--permission-mode".into(), selected.into()],
            CodingAgent::Codex if selected == "approve-for-me" => vec!["--approve-for-me".into()],
            CodingAgent::Codex => vec![
                "--ask-for-approval".into(),
                selected.into(),
                "--sandbox".into(),
                "workspace-write".into(),
            ],
            CodingAgent::Opencode => vec!["--agent".into(), selected.into()],
            CodingAgent::Hermes => vec![],
            CodingAgent::Pi => vec!["--cafleet-gate".into(), selected.into()],
        }
    }

    #[test]
    fn runtime_policy_omission_selects_each_backends_default_argv() {
        for &backend in CodingAgent::all() {
            let selection = backend.resolve_policy(None).unwrap();
            let expected_mode = match backend {
                CodingAgent::Claude => "auto",
                CodingAgent::Codex => "approve-for-me",
                CodingAgent::Opencode => "cafleet-ask",
                CodingAgent::Hermes => "smart",
                CodingAgent::Pi => "cafleet-ask",
            };
            assert_eq!(selection.requested_mode(), None, "{backend:?}");
            assert_eq!(selection.resolved_mode(), expected_mode, "{backend:?}");
            assert_eq!(selection.mode_source(), ModeSource::Default);
            assert_eq!(
                selection.permission_argv(),
                permission_argv(backend, expected_mode)
            );
        }
    }

    #[test]
    fn runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode() {
        for &backend in CodingAgent::all() {
            let choices = backend.policy_choices();
            assert!(!choices.is_empty());
            let mut unique = std::collections::BTreeSet::new();
            for &mode in choices {
                assert!(unique.insert(mode), "duplicate policy {mode}");
                let selection = backend.resolve_policy(Some(mode)).unwrap();
                assert_eq!(selection.requested_mode(), Some(mode));
                assert_eq!(selection.resolved_mode(), mode);
                assert_eq!(selection.mode_source(), ModeSource::Explicit);
                assert_eq!(selection.permission_argv(), permission_argv(backend, mode));
                let mut expected: Vec<String> = match backend {
                    CodingAgent::Claude => vec![
                        "claude".into(),
                        "--permission-mode".into(),
                        mode.into(),
                        "--prompt-suggestions".into(),
                        "false".into(),
                        "--allowedTools".into(),
                        "Bash(cafleet message *)".into(),
                        "--name".into(),
                        "fixture name".into(),
                    ],
                    CodingAgent::Codex => {
                        let mut argv = vec!["codex".into()];
                        argv.extend(permission_argv(backend, mode));
                        argv
                    }
                    CodingAgent::Opencode => vec![
                        "opencode".into(),
                        "mini".into(),
                        "--standalone".into(),
                        "--agent".into(),
                        mode.into(),
                    ],
                    CodingAgent::Hermes => vec![
                        "hermes".into(),
                        "chat".into(),
                        "--cli".into(),
                        "--source".into(),
                        "tool".into(),
                        "--in".into(),
                        CWD.into(),
                    ],
                    CodingAgent::Pi => vec![
                        "pi".into(),
                        "--cafleet-gate".into(),
                        mode.into(),
                        "--no-approve".into(),
                        "--tui-mode".into(),
                        "fullscreen".into(),
                        "--name".into(),
                        "fixture name".into(),
                    ],
                };
                expected.extend(["--model".into(), "provider/model".into()]);
                let effort = (backend != CodingAgent::Opencode).then_some("high");
                match backend {
                    CodingAgent::Claude => expected.extend(["--effort".into(), "high".into()]),
                    CodingAgent::Codex => {
                        expected.push("--config=model_reasoning_effort=high".into())
                    }
                    CodingAgent::Opencode => expected.push("--prompt".into()),
                    CodingAgent::Hermes => {
                        expected.extend(["--reasoning".into(), "high".into(), "-q".into()])
                    }
                    CodingAgent::Pi => {
                        expected.extend(["--thinking".into(), "high".into(), "--".into()])
                    }
                }
                expected.push("literal '$x' `text` assignment".into());
                assert_eq!(
                    backend
                        .build_spawn_argv_with_policy(
                            "literal '$x' `text` assignment",
                            "fixture name",
                            cwd(),
                            Some("provider/model"),
                            effort,
                            &selection
                        )
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn runtime_policy_invalid_explicit_selection_reports_backend_and_catalog_choices() {
        for &backend in CodingAgent::all() {
            for invalid in ["", " ", "unsupported-fixture-mode"] {
                let failure = backend
                    .resolve_policy(Some(invalid))
                    .expect_err("invalid explicit policy must fail");
                assert!(matches!(failure, CafleetError::Usage(_)));
                let text = failure.message();
                assert!(text.contains(backend.name()), "{text}");
                for choice in backend.policy_choices() {
                    assert!(text.contains(choice), "{text}");
                }
            }
        }
    }

    #[test]
    fn runtime_policy_help_lists_every_catalog_with_its_default() {
        assert_eq!(
            CodingAgent::permission_mode_help(),
            "Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart, manual (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)"
        );
    }

    #[test]
    fn runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error() {
        let failure = CodingAgent::Codex
            .resolve_policy(Some("auto"))
            .expect_err("unknown Codex policy must fail");
        assert!(matches!(failure, CafleetError::Usage(_)));
        assert_eq!(failure.exit_code(), 2);
        assert_eq!(
            failure.message(),
            "--permission-mode for codex must be one of on-request, never, approve-for-me (got 'auto')"
        );
    }

    pub(super) mod hermes {
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use crate::coding_agent::tests::*;
        use std::time::Instant;

        fn smart() -> LaunchPolicySelection {
            CodingAgent::Hermes.resolve_policy(None).unwrap()
        }

        #[test]
        fn hermes_capture_argv_validation_accepts_the_built_argv_and_rejects_an_added_flag() {
            let built = CodingAgent::Hermes
                .launch_argv(
                    Path::new("/opt/hermes/bin/hermes"),
                    "assignment",
                    "worker",
                    cwd(),
                    Some("m"),
                    Some("high"),
                    &smart(),
                )
                .unwrap();
            CodingAgent::Hermes
                .validate_spawn_policy_argv(&built, &smart())
                .unwrap();
            let mut added = built.clone();
            added.insert(1, "--resume".into());
            assert!(
                CodingAgent::Hermes
                    .validate_spawn_policy_argv(&added, &smart())
                    .is_err()
            );
        }

        #[test]
        fn hermes_version_rule_records_the_parenthesized_release_date() {
            for (line, expected) in [
                ("Hermes Agent vunknown (2026.9.24)", "2026.9.24"),
                ("Hermes Agent v0.22.4 (2026.9.24)", "2026.9.24"),
                ("Hermes Agent v(1) (2026.10.1)", "2026.10.1"),
                ("Hermes Agent vunknown (2026.9.24-rc.1)", "2026.9.24-rc.1"),
            ] {
                assert_eq!(
                    CodingAgent::Hermes.version_from_line(line),
                    Ok(expected),
                    "{line}"
                );
            }
        }

        #[test]
        fn hermes_version_rule_rejects_a_line_that_does_not_match() {
            for line in [
                "",
                "Hermes Agent v0.22.4",
                "Hermes Agent vunknown (2026.9.24) ",
                " Hermes Agent vunknown (2026.9.24)",
                "Hermes Agent vunknown (2026.9.24) (2026.9.25)",
            ] {
                assert_eq!(
                    CodingAgent::Hermes.version_from_line(line),
                    Err("printed no Hermes version line"),
                    "{line:?}"
                );
            }
        }

        #[test]
        fn hermes_version_rule_requires_a_canonical_semver_release_date() {
            for line in [
                "Hermes Agent vunknown (2026.09.24)",
                "Hermes Agent vunknown (unknown)",
            ] {
                assert_eq!(
                    CodingAgent::Hermes.version_from_line(line),
                    Err("printed a non-SemVer Hermes version"),
                    "{line:?}"
                );
            }
        }

        #[test]
        fn hermes_native_version_probe_fails_when_only_the_package_semver_is_printed() {
            let dir = tempfile::tempdir().unwrap();
            let path = executable_stub(dir.path(), "hermes", "printf 'Hermes Agent v0.22.4\\n'");
            let failure = run_stub(|| {
                CodingAgent::Hermes
                    .probe_native_version(&path, Instant::now() + Duration::from_secs(2))
            })
            .expect_err("the package version is not the native version");
            assert_eq!(
                failure.message(),
                "hermes --version printed no Hermes version line"
            );
        }

        fn availability_error(probe: &FakeProbe) -> CafleetError {
            CodingAgent::Hermes
                .ensure_available(probe, "smart")
                .expect_err("the hermes preflight refuses the member")
        }

        #[test]
        fn hermes_availability_checks_the_home_then_the_binary() {
            let mut probe = FakeProbe::installed(&home());
            for config in [None, Some("/srv/hermes")] {
                if let Some(config) = config {
                    probe = probe.with_env("HERMES_HOME", config);
                }
                assert_eq!(
                    CodingAgent::Hermes
                        .ensure_available(&probe, "smart")
                        .unwrap(),
                    PathBuf::from("/fixture/bin/hermes")
                );
            }
            probe = probe.with_env("HERMES_HOME", "relative");
            assert_eq!(
                availability_error(&probe).message(),
                "HERMES_HOME must be an absolute path (got 'relative')"
            );
            probe.env.clear();
            probe.binaries.clear();
            assert_eq!(
                availability_error(&probe).message(),
                "binary hermes not found on PATH"
            );
        }
    }

    mod pi {
        use crate::coding_agent::tests::*;

        const MODES: [&str; 2] = ["cafleet", "cafleet-ask"];
        const THINKING: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
        const PROMPT_ERROR: &str = "a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).";

        fn policy(mode: Option<&str>) -> LaunchPolicySelection {
            CodingAgent::Pi.resolve_policy(mode).unwrap()
        }

        fn argv(
            prompt: &str,
            model: Option<&str>,
            effort: Option<&str>,
            mode: Option<&str>,
        ) -> Result<Vec<String>, CafleetError> {
            CodingAgent::Pi.build_spawn_argv_with_policy(
                prompt,
                "pi worker",
                cwd(),
                model,
                effort,
                &policy(mode),
            )
        }

        fn strings(values: &[&str]) -> Vec<String> {
            values.iter().map(|value| (*value).to_owned()).collect()
        }

        fn pi_probe(base: &Path) -> FakeProbe {
            FakeProbe::installed(&home()).with_env("PI_CODING_AGENT_DIR", base.to_str().unwrap())
        }

        fn install_extension(base: &Path) -> PathBuf {
            let extension = base.join("extensions/cafleet.ts");
            std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
            std::fs::write(&extension, "export default function () {}").unwrap();
            extension
        }

        fn assert_missing_extension(failure: CafleetError, extension: &Path) {
            assert!(
                matches!(failure, CafleetError::App(_)),
                "exit 1: {}",
                failure.message()
            );
            let text = failure.message();
            let cause = text
                .strip_prefix(&format!(
                    "cannot read the CAFleet pi extension at {}: ",
                    extension.display()
                ))
                .and_then(|rest| rest.strip_suffix("; run 'cafleet setup --coding-agent pi' first"))
                .unwrap_or_else(|| panic!("shipped missing-extension error: {text}"));
            assert!(!cause.is_empty(), "the read cause is carried: {text}");
        }

        #[test]
        fn pi_is_the_last_registry_token_with_its_config_variable() {
            assert_eq!(CodingAgent::all().last(), Some(&CodingAgent::Pi));
            assert_eq!(CodingAgent::Pi.name(), "pi");
            assert_eq!(coding_agent("pi"), Some(CodingAgent::Pi));
            assert_eq!(CodingAgent::Pi.config_variable(), "PI_CODING_AGENT_DIR");
            assert_eq!(
                CodingAgent::from_str("pi", false).unwrap(),
                CodingAgent::Pi,
                "--coding-agent pi parses"
            );
        }

        #[test]
        fn pi_argv_for_every_mode_carries_the_gate_trust_layout_and_name_then_the_prompt_after_a_separator()
         {
            let prompt = "assignment {member_id}\n'quoted'";
            for (requested, mode) in [
                (None, "cafleet-ask"),
                (Some("cafleet"), "cafleet"),
                (Some("cafleet-ask"), "cafleet-ask"),
            ] {
                let head = strings(&[
                    "pi",
                    "--cafleet-gate",
                    mode,
                    "--no-approve",
                    "--tui-mode",
                    "fullscreen",
                    "--name",
                    "pi worker",
                ]);
                for (model, effort) in [
                    (None, None),
                    (Some("openrouter/moonshotai/kimi-k2.6"), None),
                    (None, Some("xhigh")),
                    (Some("kimi k2"), Some("off")),
                ] {
                    let mut expected = head.clone();
                    if let Some(model) = model {
                        expected.extend(strings(&["--model", model]));
                    }
                    if let Some(effort) = effort {
                        expected.extend(strings(&["--thinking", effort]));
                    }
                    expected.extend(strings(&["--", prompt]));
                    assert_eq!(
                        argv(prompt, model, effort, requested).unwrap(),
                        expected,
                        "{requested:?} {model:?} {effort:?}"
                    );
                }
            }
        }

        #[test]
        fn pi_argv_keeps_a_prompt_beginning_with_a_dash_positional_after_the_separator() {
            for prompt in ["--model other", "-n", "--"] {
                let built = argv(prompt, None, None, None).unwrap();
                assert_eq!(
                    built[built.len() - 2..],
                    strings(&["--", prompt]),
                    "{prompt}"
                );
                assert!(
                    !built.iter().any(|value| value == "--model"),
                    "the prompt adds no option token: {built:?}"
                );
            }
        }

        #[test]
        fn pi_argv_starts_a_fresh_session_without_resume_options() {
            let built = argv(
                "assignment",
                Some("provider/id"),
                Some("high"),
                Some("cafleet-ask"),
            )
            .unwrap();
            for option in [
                "--continue",
                "-c",
                "--resume",
                "-r",
                "--session",
                "--session-id",
                "--fork",
            ] {
                assert!(
                    !built.iter().any(|value| value == option),
                    "{option}: {built:?}"
                );
            }
        }

        #[test]
        fn pi_launch_policy_records_the_gate_pair_for_the_omitted_and_explicit_modes() {
            let omitted = policy(None);
            assert_eq!(omitted.requested_mode(), None);
            assert_eq!(omitted.resolved_mode(), "cafleet-ask");
            assert_eq!(omitted.mode_source(), ModeSource::Default);
            assert_eq!(
                omitted.permission_argv(),
                strings(&["--cafleet-gate", "cafleet-ask"])
            );
            let explicit = policy(Some("cafleet"));
            assert_eq!(explicit.requested_mode(), Some("cafleet"));
            assert_eq!(explicit.resolved_mode(), "cafleet");
            assert_eq!(explicit.mode_source(), ModeSource::Explicit);
            assert_eq!(
                explicit.permission_argv(),
                strings(&["--cafleet-gate", "cafleet"])
            );
            assert_eq!(CodingAgent::Pi.policy_choices(), MODES);
        }

        #[test]
        fn pi_unknown_launch_mode_fails_with_the_exact_usage_error() {
            let failure = CodingAgent::Pi
                .resolve_policy(Some("auto"))
                .expect_err("an unknown pi mode fails");
            assert!(matches!(failure, CafleetError::Usage(_)));
            assert_eq!(failure.exit_code(), 2);
            assert_eq!(
                failure.message(),
                "--permission-mode for pi must be one of cafleet, cafleet-ask (got 'auto')"
            );
        }

        #[test]
        fn pi_model_with_a_thinking_suffix_fails_with_the_exact_usage_error() {
            for level in THINKING {
                let model = format!("openrouter/moonshotai/kimi-k2.6:{level}");
                let failure = CodingAgent::Pi
                    .validate_model(Some(&model))
                    .expect_err("a thinking suffix is rejected");
                assert!(matches!(failure, CafleetError::Usage(_)));
                assert_eq!(failure.exit_code(), 2);
                assert_eq!(
                    failure.message(),
                    format!(
                        "--model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '{model}')."
                    )
                );
                assert!(argv("assignment", Some(&model), None, None).is_err());
            }
            for model in ["kimi:high", "provider/model:free:max"] {
                assert!(
                    CodingAgent::Pi.validate_model(Some(model)).is_err(),
                    "the text after the last ':' decides: {model}"
                );
            }
        }

        #[test]
        fn pi_model_passes_through_verbatim_when_no_thinking_level_follows_its_last_colon() {
            for model in [
                "openrouter/meta-llama/llama-3.3-70b-instruct:free",
                "openrouter/moonshotai/kimi-k2.6",
                "claude-opus-5-5",
                "kimi k2",
                "provider/model:high:free",
                "model:",
            ] {
                CodingAgent::Pi
                    .validate_model(Some(model))
                    .unwrap_or_else(|error| panic!("{model}: {}", error.message()));
                let built = argv("assignment", Some(model), None, None).unwrap();
                let index = built.iter().position(|value| value == "--model").unwrap();
                assert_eq!(built[index + 1], model);
            }
        }

        #[test]
        fn pi_effort_forwards_every_thinking_level_as_the_thinking_option() {
            for level in THINKING {
                CodingAgent::Pi.validate_effort(Some(level)).unwrap();
                let built = argv("assignment", None, Some(level), None).unwrap();
                let index = built
                    .iter()
                    .position(|value| value == "--thinking")
                    .unwrap();
                assert_eq!(built[index + 1], level);
                assert!(!built.iter().any(|value| value == "--effort"), "{built:?}");
            }
        }

        #[test]
        fn pi_effort_outside_the_thinking_levels_fails_with_the_exact_usage_error() {
            for value in ["none", "ultra", "HIGH", ""] {
                let failure = CodingAgent::Pi
                    .validate_effort(Some(value))
                    .expect_err("an unknown pi effort fails");
                assert!(matches!(failure, CafleetError::Usage(_)));
                assert_eq!(failure.exit_code(), 2);
                assert_eq!(
                    failure.message(),
                    format!(
                        "--effort for the pi backend must be one of off, minimal, low, medium, high, xhigh, max (got '{value}')."
                    )
                );
                assert!(argv("assignment", None, Some(value), None).is_err());
            }
        }

        #[test]
        fn pi_prompt_beginning_with_an_at_sign_fails_with_the_exact_usage_error() {
            for prompt in ["@notes.md", "@", "@{member_id} assignment"] {
                let failure = CodingAgent::Pi
                    .validate_prompt(prompt)
                    .expect_err("a leading '@' is rejected for pi");
                assert!(matches!(failure, CafleetError::Usage(_)));
                assert_eq!(failure.exit_code(), 2);
                assert_eq!(failure.message(), PROMPT_ERROR);
            }
        }

        #[test]
        fn pi_prompt_validation_accepts_every_prompt_whose_first_character_is_not_an_at_sign() {
            for prompt in [
                "assignment",
                " @notes.md",
                "\n@notes.md",
                "mail me@example.com",
                "-x",
                "{member_id}",
                "",
            ] {
                CodingAgent::Pi
                    .validate_prompt(prompt)
                    .unwrap_or_else(|error| panic!("{prompt:?}: {}", error.message()));
            }
        }

        #[test]
        fn every_other_backend_accepts_a_prompt_beginning_with_an_at_sign() {
            for &agent in CodingAgent::all() {
                if agent == CodingAgent::Pi {
                    continue;
                }
                for prompt in ["@notes.md", "@", "assignment"] {
                    agent.validate_prompt(prompt).unwrap_or_else(|error| {
                        panic!("{agent:?} {prompt:?}: {}", error.message())
                    });
                }
            }
        }

        #[test]
        #[should_panic]
        fn pi_argv_construction_asserts_that_the_prompt_passed_validation() {
            let _ = argv("@notes.md", None, None, None);
        }

        #[test]
        fn pi_availability_checks_the_base_then_the_binary_then_the_extension() {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().join("pi agent");
            for mode in MODES {
                let mut probe =
                    FakeProbe::installed(&home()).with_env("PI_CODING_AGENT_DIR", "relative");
                probe.binaries.clear();
                assert_eq!(
                    CodingAgent::Pi
                        .ensure_available(&probe, mode)
                        .unwrap_err()
                        .message(),
                    "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')",
                    "{mode}: the base resolves first"
                );
                probe = probe.with_env("PI_CODING_AGENT_DIR", base.to_str().unwrap());
                assert_eq!(
                    CodingAgent::Pi
                        .ensure_available(&probe, mode)
                        .unwrap_err()
                        .message(),
                    "binary pi not found on PATH",
                    "{mode}: the binary precedes the extension"
                );
            }
        }

        #[test]
        fn pi_availability_reads_the_extension_in_every_launch_mode() {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().join("pi agent");
            let probe = pi_probe(&base);
            let extension = base.join("extensions/cafleet.ts");
            for mode in MODES {
                assert_missing_extension(
                    CodingAgent::Pi
                        .ensure_available(&probe, mode)
                        .expect_err("a missing extension fails the preflight"),
                    &extension,
                );
            }
            install_extension(&base);
            for mode in MODES {
                assert_eq!(
                    CodingAgent::Pi.ensure_available(&probe, mode).unwrap(),
                    PathBuf::from("/fixture/bin/pi"),
                    "{mode}"
                );
            }
            std::fs::remove_file(&extension).unwrap();
            std::fs::create_dir(&extension).unwrap();
            for mode in MODES {
                assert_missing_extension(
                    CodingAgent::Pi
                        .ensure_available(&probe, mode)
                        .expect_err("an unreadable extension fails the preflight"),
                    &extension,
                );
            }
        }

        #[test]
        fn pi_availability_reads_the_extension_under_the_default_base() {
            let dir = tempfile::tempdir().unwrap();
            let probe = FakeProbe::installed(dir.path());
            let base = dir.path().join(".pi/agent");
            assert_missing_extension(
                CodingAgent::Pi
                    .ensure_available(&probe, "cafleet")
                    .expect_err("the default base has no extension"),
                &base.join("extensions/cafleet.ts"),
            );
            install_extension(&base);
            CodingAgent::Pi.ensure_available(&probe, "cafleet").unwrap();
        }

        #[test]
        fn pi_config_dir_defaults_to_the_pi_agent_directory_and_honors_the_override() {
            assert_eq!(
                CodingAgent::Pi.config_dir(&|_| Ok(None), &home()).unwrap(),
                home().join(".pi/agent")
            );
            assert_eq!(
                CodingAgent::Pi
                    .config_dir(&only("PI_CODING_AGENT_DIR", "/srv/pi agent"), &home())
                    .unwrap(),
                PathBuf::from("/srv/pi agent")
            );
            assert_eq!(
                CodingAgent::Pi
                    .forwarded_config(&only("PI_CODING_AGENT_DIR", "/srv/pi agent"))
                    .unwrap(),
                Some(("PI_CODING_AGENT_DIR".to_owned(), "/srv/pi agent".to_owned()))
            );
            assert_eq!(
                CodingAgent::Pi.forwarded_config(&|_| Ok(None)).unwrap(),
                None
            );
        }

        #[test]
        fn pi_version_probe_records_the_single_version_token() {
            assert_eq!(CodingAgent::Pi.version_from_line("1.5.0"), Ok("1.5.0"));
            assert_eq!(CodingAgent::Pi.version_from_line("v1.5.0"), Ok("1.5.0"));
            assert_eq!(
                CodingAgent::Pi.version_from_line("pi"),
                Err("printed no version token")
            );
        }

        #[test]
        fn pi_launch_argv_replaces_only_the_program_with_the_resolved_executable() {
            let launched = CodingAgent::Pi
                .launch_argv(
                    Path::new("/opt/pi/bin/pi"),
                    "assignment",
                    "pi worker",
                    cwd(),
                    None,
                    None,
                    &policy(None),
                )
                .unwrap();
            let bare = argv("assignment", None, None, None).unwrap();
            assert_eq!(launched[0], "/opt/pi/bin/pi");
            assert_eq!(launched[1..], bare[1..]);
        }

        #[test]
        fn pi_capture_argv_validation_accepts_the_built_argv_with_or_without_the_prompt_tail() {
            for mode in MODES {
                let selection = policy(Some(mode));
                for (model, effort) in [(None, None), (Some("provider/id"), Some("max"))] {
                    let built = CodingAgent::Pi
                        .launch_argv(
                            Path::new("/opt/pi/bin/pi"),
                            "-assignment",
                            "pi worker",
                            cwd(),
                            model,
                            effort,
                            &selection,
                        )
                        .unwrap();
                    CodingAgent::Pi
                        .validate_spawn_policy_argv(&built, &selection)
                        .unwrap();
                    let without_prompt = &built[..built.len() - 2];
                    assert_eq!(built[built.len() - 2], "--");
                    CodingAgent::Pi
                        .validate_spawn_policy_argv(without_prompt, &selection)
                        .unwrap();
                }
            }
        }

        #[test]
        fn pi_capture_argv_validation_rejects_an_added_flag_a_missing_separator_or_another_mode() {
            let selection = policy(None);
            let built = CodingAgent::Pi
                .launch_argv(
                    Path::new("/opt/pi/bin/pi"),
                    "assignment",
                    "pi worker",
                    cwd(),
                    None,
                    None,
                    &selection,
                )
                .unwrap();
            let mut added = built.clone();
            added.insert(1, "--continue".into());
            let mut unseparated = built.clone();
            unseparated.remove(built.len() - 2);
            for argv in [added, unseparated] {
                assert!(
                    CodingAgent::Pi
                        .validate_spawn_policy_argv(&argv, &selection)
                        .is_err(),
                    "{argv:?}"
                );
            }
            assert!(
                CodingAgent::Pi
                    .validate_spawn_policy_argv(&built, &policy(Some("cafleet")))
                    .is_err()
            );
        }
    }
}
