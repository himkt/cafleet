pub(crate) mod catalog;
pub(crate) mod detector;
pub(crate) mod hermes_settlement;

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
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

#[cfg(test)]
pub(crate) use crate::coding_agent::detector::ScreenRegion;

pub(crate) trait SpawnProbe {
    fn resolve_binary(&self, name: &str) -> Option<PathBuf>;
    fn home_dir(&self) -> PathBuf;
    fn env_var(&self, name: &str) -> Option<String>;
    fn hermes_approvals(
        &self,
        executable: &Path,
        config_base: &Path,
    ) -> Result<String, CafleetError>;
    fn hermes_client_env_passthrough(
        &self,
        executable: &Path,
        config_base: &Path,
    ) -> Result<String, CafleetError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub(crate) enum CodingAgent {
    Claude,
    Codex,
    Opencode,
    Hermes,
}

pub(crate) const HERMES_DIRECTOR_ONLY_DENIES: [&str; 6] = [
    "cafleet member prompt *",
    "cafleet member exec *",
    "cafleet member press *",
    "cafleet member recover *",
    "cafleet permission respond *",
    "cafleet input reconcile *",
];

pub(crate) const HERMES_CLIENT_ENV_PASSTHROUGH: [&str; 10] = [
    "CAFLEET_DATABASE_URL",
    "TMUX",
    "TMUX_PANE",
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_BIN_PATH",
    "HERDR_SESSION",
];

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
        }
    }

    pub(crate) fn config_variable(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
            Self::Opencode => "OPENCODE_CONFIG_DIR",
            Self::Hermes => "HERMES_HOME",
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
        };
        crate::config::resolve(self.config_variable(), env, default)
    }

    pub(crate) fn spawn_environment(
        self,
        database_url: &str,
        env: EnvLookup<'_>,
    ) -> Result<Vec<(String, String)>, CafleetError> {
        let mut environment = vec![("CAFLEET_DATABASE_URL".to_owned(), database_url.to_owned())];
        if let Some(value) = env(self.config_variable()) {
            crate::config::require_absolute(self.config_variable(), &value)?;
            environment.push((self.config_variable().to_owned(), value));
        }
        Ok(environment)
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
        let config = self.config_dir(&|name| probe.env_var(name), &probe.home_dir())?;
        let executable = probe.resolve_binary(self.name()).ok_or_else(|| {
            CafleetError::App(format!("binary {} not found on PATH", self.name()))
        })?;
        if let Some(preset) = self.required_preset(&config, mode) {
            std::fs::read(&preset).map_err(|error| CafleetError::App(format!(
                "cannot read opencode agent preset at {}: {error}; run 'cafleet setup --coding-agent opencode' first", preset.display()
            )))?;
        }
        if self == Self::Hermes {
            hermes_approvals_preflight(probe.hermes_approvals(&executable, &config))?;
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

pub(crate) fn coding_agent(name: &str) -> Option<CodingAgent> {
    CodingAgent::all()
        .iter()
        .copied()
        .find(|agent| agent.name() == name)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LaunchPolicySelection {
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv: Vec<String>,
    backend: CodingAgent,
    native_version: Option<String>,
}
impl CodingAgent {
    fn default_policy(self) -> &'static str {
        match self {
            Self::Claude => "auto",
            Self::Codex => "approve-for-me",
            Self::Opencode => "cafleet",
            Self::Hermes => "smart",
        }
    }
    pub(crate) fn permission_mode_help() -> String {
        let catalog = Self::all()
            .iter()
            .map(|agent| {
                format!(
                    "{}: {} (default {})",
                    agent.name(),
                    agent.policy_choices(None).join(", "),
                    agent.default_policy()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("Launch-only native permission policy for the resolved backend — {catalog}")
    }
    pub(crate) fn policy_choices(self, native_version: Option<&str>) -> Vec<String> {
        let choices: &[&str] = match (self, native_version) {
            (Self::Claude, None | Some("2.1.302")) => &[
                "default",
                "manual",
                "dontAsk",
                "acceptEdits",
                "auto",
                "bypassPermissions",
                "plan",
            ],
            (Self::Codex, None | Some("0.163.0")) => &["on-request", "never", "approve-for-me"],
            (Self::Opencode, _) => &["cafleet", "cafleet-ask"],
            (Self::Hermes, _) => &["smart", "manual"],
            _ => &[],
        };
        choices.iter().map(|value| (*value).into()).collect()
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
        }
    }
    pub(crate) fn resolve_policy(
        self,
        native_version: Option<&str>,
        requested: Option<&str>,
    ) -> Result<LaunchPolicySelection, CafleetError> {
        let choices = self.policy_choices(native_version);
        if let Some(value) = requested
            && !choices.iter().any(|choice| choice == value)
        {
            return Err(CafleetError::Usage(format!(
                "--permission-mode for {} must be one of {} (got '{value}')",
                self.name(),
                choices.join(", ")
            )));
        }
        let mode = requested.unwrap_or_else(|| self.default_policy());
        Ok(LaunchPolicySelection {
            requested_mode: requested.map(str::to_owned),
            resolved_mode: mode.into(),
            mode_source: if requested.is_some() {
                ModeSource::Explicit
            } else {
                ModeSource::Default
            },
            permission_argv: self.permission_arguments(mode),
            backend: self,
            native_version: native_version.map(str::to_owned),
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
        if policy.backend != self
            || *policy
                != self.resolve_policy(
                    policy.native_version.as_deref(),
                    policy.requested_mode.as_deref(),
                )?
        {
            return Err(CafleetError::Usage(
                "launch policy does not match its backend/catalog".into(),
            ));
        }
        self.validate_model(model)?;
        self.validate_effort(effort)?;
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
            Self::Claude | Self::Codex => {}
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
        if let Some(model) = model {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = effort {
            match self {
                Self::Claude => argv.extend(["--effort".into(), effort.into()]),
                Self::Codex => argv.push(format!("--config=model_reasoning_effort={effort}")),
                Self::Hermes => argv.extend(["--reasoning".into(), effort.into()]),
                Self::Opencode => unreachable!("effort validated before argv construction"),
            }
        }
        match self {
            Self::Opencode => argv.push("--prompt".into()),
            Self::Hermes => argv.push("-q".into()),
            Self::Claude | Self::Codex => {}
        }
        argv.push(prompt.into());
        Ok(argv)
    }
    #[cfg(test)]
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
        let flag_prompt = usize::from(matches!(self, Self::Opencode | Self::Hermes));
        let baseline = self.build_spawn_argv_with_policy("", "", cwd, None, None, policy)?;
        let name_index = (self == Self::Claude).then(|| {
            baseline
                .iter()
                .position(|value| value == "--name")
                .expect("Claude name option")
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

const OUTPUT_BYTES: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeVersion(semver::Version);

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
            let capabilities = catalog.capabilities(backend, Some(&version));
            Ok(NativeLaunchIdentity {
                native_version: Some(version),
                coverage: capabilities.detection,
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

/// Runs a native command with null stdin and bounded output, returning its
/// UTF-8 stdout or the failure detail that callers prefix with the command.
pub(crate) fn run_bounded(
    executable: &Path,
    args: &[&str],
    environment: &[(&str, &Path)],
    deadline: Instant,
) -> Result<String, String> {
    let mut child = Command::new(executable)
        .args(args)
        .envs(environment.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|cause| format!("could not start: {cause}"))?;
    let stdout = bounded_output(child.stdout.take().expect("piped probe stdout"));
    let stderr = bounded_output(child.stderr.take().expect("piped probe stderr"));
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|cause| format!("wait failed: {cause}"))?
        {
            break status;
        }
        let now = Instant::now();
        if now >= deadline {
            child
                .kill()
                .map_err(|cause| format!("timed out and could not be killed: {cause}"))?;
            child
                .wait()
                .map_err(|cause| format!("timed out and could not be reaped: {cause}"))?;
            return Err("did not finish within its deadline".into());
        }
        std::thread::sleep(Duration::from_millis(10).min(deadline - now));
    };
    let collect = |receiver: mpsc::Receiver<std::io::Result<Vec<u8>>>| {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "output did not close within its deadline".to_owned())?
            .map_err(|cause| format!("output could not be read: {cause}"))
    };
    let output = collect(stdout)?;
    let diagnostic = collect(stderr)?;
    if !status.success() {
        let first = String::from_utf8_lossy(&diagnostic)
            .lines()
            .next()
            .map(str::to_owned);
        return Err(match first {
            Some(line) if !line.trim().is_empty() => format!("exited with {status}: {line}"),
            _ => format!("exited with {status}"),
        });
    }
    String::from_utf8(output).map_err(|_| "printed non-UTF-8 output".to_owned())
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
            Self::Claude | Self::Codex | Self::Opencode => version_token(line),
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

pub(crate) fn hermes_approvals_preflight(
    output: Result<String, CafleetError>,
) -> Result<(), CafleetError> {
    hermes_approval_denies(&hermes_approvals(output)?)
}

pub(crate) fn hermes_approvals(
    output: Result<String, CafleetError>,
) -> Result<serde_json::Map<String, serde_json::Value>, CafleetError> {
    let unreadable = |cause: String| {
        CafleetError::App(format!(
            "cannot read hermes configuration through 'hermes config get': {cause}"
        ))
    };
    let text = output.map_err(|cause| unreadable(cause.message().into_owned()))?;
    match serde_json::from_str(&text) {
        Ok(serde_json::Value::Object(approvals)) => Ok(approvals),
        Ok(other) => Err(unreadable(format!("expected a JSON object, got {other}"))),
        Err(cause) => Err(unreadable(format!("invalid JSON: {cause}"))),
    }
}

pub(crate) fn hermes_approval_denies(
    approvals: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), CafleetError> {
    let present = match approvals.get("deny") {
        Some(serde_json::Value::Array(entries)) => entries
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let missing = HERMES_DIRECTOR_ONLY_DENIES
        .into_iter()
        .filter(|glob| !present.contains(glob))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "hermes approvals.deny lacks CAFleet's Director-only rules: {}; run 'cafleet setup --coding-agent hermes'",
            missing.join(", ")
        )))
    }
}

pub(crate) fn hermes_client_env_names(
    output: Result<String, CafleetError>,
) -> Result<(), CafleetError> {
    let text = output.map_err(|cause| {
        CafleetError::App(format!(
            "cannot read hermes configuration through 'hermes config get': {}",
            cause.message()
        ))
    })?;
    let present = serde_json::from_str::<Vec<String>>(&text).map_err(|cause| {
        CafleetError::App(format!(
            "cannot read hermes configuration through 'hermes config get': terminal.client_env_passthrough is not a JSON array of strings: {cause}"
        ))
    })?;
    let missing = HERMES_CLIENT_ENV_PASSTHROUGH
        .into_iter()
        .filter(|name| !present.iter().any(|entry| entry == name))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "hermes terminal.client_env_passthrough lacks CAFleet's member names: {}; run 'cafleet setup --coding-agent hermes'",
            missing.join(", ")
        )))
    }
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
    use crate::error::CafleetError;
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
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::*;

    struct Probe {
        agent: CodingAgent,
        home: PathBuf,
        config: Option<String>,
        available: bool,
    }

    impl SpawnProbe for Probe {
        fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
            assert_eq!(name, self.agent.name());
            self.available
                .then(|| PathBuf::from("/fixture/bin").join(name))
        }
        fn home_dir(&self) -> PathBuf {
            self.home.clone()
        }
        fn env_var(&self, name: &str) -> Option<String> {
            assert_eq!(name, self.agent.config_variable());
            self.config.clone()
        }
        fn hermes_approvals(
            &self,
            executable: &Path,
            _config_base: &Path,
        ) -> Result<String, CafleetError> {
            assert_eq!(self.agent, CodingAgent::Hermes);
            assert_eq!(executable, Path::new("/fixture/bin/hermes"));
            Ok(hermes::approvals_json(
                "smart",
                &HERMES_DIRECTOR_ONLY_DENIES,
            ))
        }
        fn hermes_client_env_passthrough(
            &self,
            _: &Path,
            _: &Path,
        ) -> Result<String, CafleetError> {
            unreachable!("availability reads no terminal.client_env_passthrough")
        }
    }

    const CWD: &str = "/fixture/member work dir";

    fn cwd() -> &'static Path {
        Path::new(CWD)
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
                vec!["opencode", "mini", "--standalone", "--agent", "cafleet"],
            ),
            (
                CodingAgent::Hermes,
                vec!["hermes", "chat", "--cli", "--source", "tool", "--in", CWD],
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
                    _ => {}
                }
                match agent {
                    CodingAgent::Opencode => expected.push("--prompt".into()),
                    CodingAgent::Hermes => expected.push("-q".into()),
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
                            &agent.resolve_policy(None, None).unwrap()
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
        ] {
            for effort in valid {
                agent.validate_effort(Some(effort)).unwrap();
            }
            assert!(agent.validate_effort(Some(invalid)).is_err());
            agent.validate_model(Some("arbitrary model")).unwrap();
        }
        assert!(CodingAgent::Opencode.validate_effort(Some("high")).is_err());
    }

    #[test]
    fn selected_preflight_checks_paths_then_binary_and_relocated_required_preset() {
        let dir = tempfile::tempdir().unwrap();
        for agent in CodingAgent::all() {
            let mode = agent.resolve_policy(None, None).unwrap().resolved_mode;
            let mut probe = Probe {
                agent: *agent,
                home: dir.path().into(),
                config: Some("relative".into()),
                available: true,
            };
            assert!(
                agent
                    .ensure_available(&probe, &mode)
                    .unwrap_err()
                    .message()
                    .contains(agent.config_variable())
            );
            probe.config = Some(dir.path().join(agent.name()).display().to_string());
            probe.available = false;
            assert!(
                agent
                    .ensure_available(&probe, &mode)
                    .unwrap_err()
                    .message()
                    .contains("not found on PATH")
            );
            probe.available = true;
            if *agent == CodingAgent::Opencode {
                assert!(agent.ensure_available(&probe, &mode).is_err());
                let preset =
                    PathBuf::from(probe.config.as_ref().unwrap()).join("agents/cafleet.md");
                std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
                std::fs::write(&preset, "preset").unwrap();
                agent.ensure_available(&probe, &mode).unwrap();
                std::fs::remove_file(&preset).unwrap();
                std::fs::create_dir(&preset).unwrap();
                assert!(agent.ensure_available(&probe, &mode).is_err());
            } else {
                assert_eq!(
                    agent.ensure_available(&probe, &mode).unwrap(),
                    PathBuf::from("/fixture/bin").join(agent.name())
                );
            }
        }
        assert!(coding_agent("Claude").is_none());
        assert!(coding_agent("unknown").is_none());
    }

    const DATABASE_URL: &str = "sqlite:///fixture/db/registry.db";

    fn every_config_variable(name: &str) -> Option<String> {
        match name {
            "CLAUDE_CONFIG_DIR" => Some("/cfg/claude dir/".into()),
            "CODEX_HOME" => Some("/cfg/codex".into()),
            "OPENCODE_CONFIG_DIR" => Some("/cfg/opencode".into()),
            "HERMES_HOME" => Some("/cfg/hermes home".into()),
            _ => None,
        }
    }

    #[test]
    fn forwarded_config_returns_only_the_selected_variable_with_the_creator_exact_value() {
        for &agent in CodingAgent::all() {
            let expected = every_config_variable(agent.config_variable()).unwrap();
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
                (name != agent.config_variable())
                    .then(|| every_config_variable(name))
                    .flatten()
            };
            assert_eq!(
                agent.forwarded_config(&others).unwrap(),
                None,
                "{agent:?} forwards no default and no other backend's variable"
            );
        }
    }

    #[test]
    fn forwarded_config_ignores_an_invalid_value_of_another_backend() {
        for &agent in CodingAgent::all() {
            let others_invalid = |name: &str| {
                if name == agent.config_variable() {
                    Some("/cfg/selected".to_string())
                } else {
                    Some("relative".to_string())
                }
            };
            assert_eq!(
                agent.forwarded_config(&others_invalid).unwrap(),
                Some((
                    agent.config_variable().to_owned(),
                    "/cfg/selected".to_owned()
                ))
            );
        }
    }

    #[test]
    fn spawn_environment_orders_database_url_member_id_then_the_forwarded_config() {
        for &agent in CodingAgent::all() {
            let forwarded = agent.forwarded_config(&every_config_variable).unwrap();
            let expected = every_config_variable(agent.config_variable()).unwrap();
            assert_eq!(
                CodingAgent::spawn_environment(DATABASE_URL, 34, forwarded),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    ("CAFLEET_MEMBER_ID".to_owned(), "34".to_owned()),
                    (agent.config_variable().to_owned(), expected),
                ],
                "{agent:?} with its config variable set"
            );
        }
    }

    #[test]
    fn spawn_environment_holds_database_url_and_member_id_when_the_config_variable_is_unset() {
        for &agent in CodingAgent::all() {
            let forwarded = agent.forwarded_config(&|_| None).unwrap();
            assert_eq!(
                CodingAgent::spawn_environment(DATABASE_URL, 34, forwarded),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    ("CAFLEET_MEMBER_ID".to_owned(), "34".to_owned()),
                ],
                "{agent:?} with its config variable unset"
            );
        }
    }

    #[test]
    fn spawn_environment_renders_the_member_id_in_decimal() {
        for member_id in [1, 9, 10, 1234567890123] {
            assert_eq!(
                CodingAgent::spawn_environment(DATABASE_URL, member_id, None),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    ("CAFLEET_MEMBER_ID".to_owned(), member_id.to_string()),
                ]
            );
        }
    }

    #[test]
    fn forwarded_config_rejects_an_invalid_selected_value_with_the_config_dir_error() {
        for &agent in CodingAgent::all() {
            for value in ["", "relative/cfg", "~/cfg"] {
                let lookup =
                    |name: &str| (name == agent.config_variable()).then(|| value.to_string());
                let failure = agent
                    .forwarded_config(&lookup)
                    .expect_err("an invalid config location fails before registration");
                assert!(matches!(failure, CafleetError::App(_)));
                assert_eq!(
                    failure.message(),
                    format!(
                        "{} must be an absolute path (got '{value}')",
                        agent.config_variable()
                    )
                );
            }
        }
    }

    #[test]
    fn selected_preflight_reads_the_native_opencode_config_directory_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let probe = Probe {
            agent: CodingAgent::Opencode,
            home: dir.path().into(),
            config: None,
            available: true,
        };
        let former = dir.path().join(".opencode/agents");
        std::fs::create_dir_all(&former).unwrap();
        for mode in ["cafleet", "cafleet-ask"] {
            std::fs::write(former.join(format!("{mode}.md")), "former base").unwrap();
            let failure = CodingAgent::Opencode
                .ensure_available(&probe, mode)
                .expect_err("a preset at the former base is not consulted");
            let preset = dir
                .path()
                .join(format!(".config/opencode/agents/{mode}.md"));
            assert!(
                failure.message().starts_with(&format!(
                    "cannot read opencode agent preset at {}: ",
                    preset.display()
                )),
                "{}",
                failure.message()
            );
            std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
            std::fs::write(&preset, "native base").unwrap();
            assert_eq!(
                CodingAgent::Opencode
                    .ensure_available(&probe, mode)
                    .unwrap(),
                PathBuf::from("/fixture/bin/opencode")
            );
        }
    }

    #[test]
    fn required_preset_is_the_selected_opencode_agent_file_and_absent_for_other_backends() {
        let config = Path::new("/fixture/preset-base");
        for mode in CodingAgent::Opencode.policy_choices(None) {
            assert_eq!(
                CodingAgent::Opencode.required_preset(config, &mode),
                Some(config.join(format!("agents/{mode}.md")))
            );
        }
        assert_eq!(
            CodingAgent::Opencode.required_preset(config, "cafleet-ask"),
            Some(PathBuf::from("/fixture/preset-base/agents/cafleet-ask.md"))
        );
        for agent in [CodingAgent::Claude, CodingAgent::Codex, CodingAgent::Hermes] {
            for mode in agent.policy_choices(None) {
                assert_eq!(agent.required_preset(config, &mode), None, "{mode}");
            }
        }
    }

    #[test]
    fn selected_preflight_reads_only_the_selected_opencode_preset_with_the_shipped_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("preset-base");
        let probe = Probe {
            agent: CodingAgent::Opencode,
            home: dir.path().into(),
            config: Some(config.display().to_string()),
            available: true,
        };
        let agents = config.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        let default_preset = agents.join("cafleet.md");
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

        std::fs::write(&default_preset, "default preset").unwrap();
        CodingAgent::Opencode
            .ensure_available(&probe, "cafleet")
            .unwrap();
        assert_missing("cafleet-ask", &ask_preset);

        std::fs::remove_file(&default_preset).unwrap();
        std::fs::write(&ask_preset, "ask preset").unwrap();
        assert_eq!(
            CodingAgent::Opencode
                .ensure_available(&probe, "cafleet-ask")
                .unwrap(),
            PathBuf::from("/fixture/bin/opencode"),
            "the unselected default preset is not consulted"
        );
        assert_missing("cafleet", &default_preset);

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
        })
    }

    #[test]
    fn unset_variable_resolves_the_default() {
        for &agent in CodingAgent::all() {
            assert_eq!(
                agent.config_dir(&|_| None, &home()).unwrap(),
                default_config_dir(agent),
                "default for {agent:?}"
            );
        }
    }

    #[test]
    fn absolute_value_is_used_verbatim() {
        for &agent in CodingAgent::all() {
            let var = agent.config_variable();
            let lookup = move |name: &str| (name == var).then(|| "/custom/cfg".to_string());
            assert_eq!(
                agent.config_dir(&lookup, &home()).unwrap(),
                PathBuf::from("/custom/cfg")
            );
        }
    }

    #[test]
    fn each_resolver_reads_only_its_own_variable() {
        for &agent in CodingAgent::all() {
            let var = agent.config_variable();
            let lookup = move |name: &str| (name != var).then(|| "/other/cfg".to_string());
            assert_eq!(
                agent.config_dir(&lookup, &home()).unwrap(),
                default_config_dir(agent),
                "{var}'s resolver must ignore every other variable"
            );
        }
    }

    #[test]
    fn empty_value_fails_with_the_pinned_error() {
        for &agent in CodingAgent::all() {
            let var = agent.config_variable();
            let lookup = move |name: &str| (name == var).then(String::new);
            let err = agent.config_dir(&lookup, &home()).unwrap_err();
            assert!(matches!(err, CafleetError::App(_)), "App error for {var}");
            assert_eq!(
                err.message(),
                format!("{var} must be an absolute path (got '')")
            );
        }
    }

    #[test]
    fn relative_or_tilde_value_fails_with_the_pinned_error() {
        for &agent in CodingAgent::all() {
            let var = agent.config_variable();
            for value in ["cfg/dir", "~/cfg"] {
                let lookup = move |name: &str| (name == var).then(|| value.to_string());
                let err = agent.config_dir(&lookup, &home()).unwrap_err();
                assert!(matches!(err, CafleetError::App(_)), "App error for {var}");
                assert_eq!(
                    err.message(),
                    format!("{var} must be an absolute path (got '{value}')")
                );
            }
        }
    }

    #[test]
    fn opencode_config_dir_defaults_to_the_skills_base_the_native_cli_reads() {
        let home = home();
        assert_eq!(
            CodingAgent::Opencode.config_dir(&|_| None, &home).unwrap(),
            crate::config::opencode_skills_base(&home)
        );
    }

    #[test]
    fn native_launch_probes_and_launches_the_same_resolved_executable() {
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use std::time::Duration;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let versions = dir.path().join("versions/2.1.302");
        std::fs::create_dir_all(&versions).unwrap();
        let executable = executable_stub(
            &versions,
            "claude",
            "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nprintf '2.1.302 (Claude Code)\\n'",
        );
        struct Resolved {
            executable: PathBuf,
            home: PathBuf,
        }
        impl SpawnProbe for Resolved {
            fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
                assert_eq!(name, "claude");
                Some(self.executable.clone())
            }
            fn home_dir(&self) -> PathBuf {
                self.home.clone()
            }
            fn env_var(&self, name: &str) -> Option<String> {
                assert_eq!(name, "CLAUDE_CONFIG_DIR");
                Some(self.home.join("claude-config").display().to_string())
            }
            fn hermes_approvals(&self, _: &Path, _: &Path) -> Result<String, CafleetError> {
                unreachable!("only the hermes preflight reads its approvals")
            }
            fn hermes_client_env_passthrough(
                &self,
                _: &Path,
                _: &Path,
            ) -> Result<String, CafleetError> {
                unreachable!("availability reads no terminal.client_env_passthrough")
            }
        }
        let backend = CodingAgent::Claude;
        let resolved = backend
            .ensure_available(
                &Resolved {
                    executable: executable.clone(),
                    home: dir.path().into(),
                },
                "dontAsk",
            )
            .unwrap();
        assert_eq!(resolved, executable);
        assert_eq!(
            run_stub(
                || backend.probe_native_version(&resolved, Instant::now() + Duration::from_secs(2))
            )
            .unwrap(),
            "2.1.302",
            "the probe runs the resolved executable"
        );
        let policy = backend.resolve_policy(None, None).unwrap();
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
    fn version_token_accepts_semver_prerelease_and_build_metadata_together() {
        for (line, expected) in [
            ("v2.1.310-beta.1+build.5", "2.1.310-beta.1+build.5"),
            ("codex-cli 1.0.0+exp.sha.5114f85", "1.0.0+exp.sha.5114f85"),
        ] {
            assert_eq!(version_token(line), Ok(expected), "{line}");
        }
    }

    #[test]
    fn version_token_requires_one_complete_canonical_semver_token() {
        for line in [
            "codex 2.1",
            "codex 2.1.302.1",
            "codex 01.2.3",
            "codex 1.2.3-01",
        ] {
            assert_eq!(
                version_token(line),
                Err("printed no version token"),
                "{line}"
            );
        }
        assert_eq!(version_token("codex 2.1 build 2.1.302"), Ok("2.1.302"));
        assert_eq!(
            version_token("2.1.302 1.0.0-beta"),
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
        use crate::coding_agent::test_support::executable_stub as stub;
        use crate::coding_agent::test_support::run_stub;
        use crate::coding_agent::tests::*;

        fn probe(executable: &Path) -> Result<String, CafleetError> {
            run_stub(|| {
                CodingAgent::Claude
                    .probe_native_version(executable, Instant::now() + Duration::from_secs(2))
            })
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
            // A stub launch can take about half a second on a macOS host, so the bound leaves the
            // stub time to record its pid and reach its sleep before the probe times out.
            let bound = Duration::from_secs(2);
            let start = Instant::now();
            let result = CodingAgent::Claude.probe_native_version(&path, start + bound);
            assert!(result.is_err());
            assert!(
                start.elapsed() < bound + Duration::from_secs(1),
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

        use crate::coding_agent::catalog::test_support::BUNDLED_BOUNDARIES;
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
            let catalog = ProfileCatalog::bundled().unwrap();
            for boundary in BUNDLED_BOUNDARIES {
                let backend = boundary.backend;
                for version in std::iter::once(boundary.min_version).chain(boundary.later) {
                    let identity = launched(backend, version, &catalog);
                    assert_eq!(identity.native_version.as_deref(), Some(version));
                    assert_eq!(
                        catalog
                            .capabilities(backend, Some(version))
                            .min_version
                            .as_deref(),
                        Some(boundary.min_version),
                        "{backend:?} {version}"
                    );
                    assert_eq!(identity.coverage, Coverage::Tested, "{backend:?} {version}");
                    assert_eq!(
                        identity.warning, None,
                        "{backend:?} {version} launches without a version-mismatch warning"
                    );
                }
            }
        }

        #[test]
        fn native_launch_identity_rejects_a_known_version_below_the_lowest_boundary() {
            let catalog = ProfileCatalog::bundled().unwrap();
            for boundary in BUNDLED_BOUNDARIES {
                let below = boundary.below;
                let error =
                    match native_launch_identity(boundary.backend, Ok(below.into()), &catalog) {
                        Err(error) => error,
                        Ok(_) => panic!("{:?} {below} must not launch", boundary.backend),
                    };
                assert_eq!(error.exit_code(), 1, "{below}");
                assert_eq!(
                    error.message(),
                    format!(
                        "unsupported native version for {}: {below}; minimum supported version is {}",
                        boundary.backend.name(),
                        boundary.min_version
                    )
                );
            }
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
            assert_eq!(
                error.message(),
                "unsupported native version for codex: 0.9.0; minimum supported version is 1.0.0"
            );
        }

        #[test]
        fn native_launch_identity_launches_a_failed_probe_with_unknown_coverage_and_its_warning() {
            let catalog = ProfileCatalog::bundled().unwrap();
            let failed = native_launch_identity(
                CodingAgent::Codex,
                Err(CafleetError::App("probe exited with status 1".into())),
                &catalog,
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

    fn version(backend: CodingAgent) -> Option<&'static str> {
        versions(backend)[0]
    }

    /// The bundled exact version, an unprobed launch and unbundled versions.
    fn versions(backend: CodingAgent) -> &'static [Option<&'static str>] {
        match backend {
            CodingAgent::Claude => &[Some("2.1.302"), None, Some("2.1.306")],
            CodingAgent::Codex => &[Some("0.163.0"), None, Some("0.164.0")],
            CodingAgent::Opencode => &[None, Some("2.1.4"), Some("2.1.3"), Some("9.0.0-beta.1")],
            CodingAgent::Hermes => &[Some("2026.9.24"), None, Some("2026.10.1"), Some("1.0.0")],
        }
    }

    /// The versions with an explicit-choice catalog: Claude and Codex offer one
    /// only at the bundled version or unprobed; OpenCode and Hermes at every version.
    fn catalog_versions(backend: CodingAgent) -> &'static [Option<&'static str>] {
        match backend {
            CodingAgent::Claude | CodingAgent::Codex => &versions(backend)[..2],
            CodingAgent::Opencode | CodingAgent::Hermes => versions(backend),
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
        }
    }

    #[test]
    fn runtime_policy_omission_selects_each_backends_automatic_default_argv() {
        for &backend in CodingAgent::all() {
            for &native_version in versions(backend) {
                let selection = backend.resolve_policy(native_version, None).unwrap();
                let expected_mode = match backend {
                    CodingAgent::Claude => "auto",
                    CodingAgent::Codex => "approve-for-me",
                    CodingAgent::Opencode => "cafleet",
                    CodingAgent::Hermes => "smart",
                };
                assert_eq!(selection.requested_mode, None, "{native_version:?}");
                assert_eq!(selection.resolved_mode, expected_mode, "{native_version:?}");
                assert_eq!(selection.mode_source, ModeSource::Default);
                assert_eq!(
                    selection.permission_argv,
                    permission_argv(backend, expected_mode)
                );
            }
        }
    }

    #[test]
    fn runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode() {
        for &backend in CodingAgent::all() {
            for &native_version in catalog_versions(backend) {
                let choices = backend.policy_choices(native_version);
                assert!(!choices.is_empty());
                assert_eq!(choices, backend.policy_choices(None), "{native_version:?}");
                let mut unique = std::collections::BTreeSet::new();
                for mode in choices {
                    assert!(unique.insert(mode.clone()), "duplicate policy {mode}");
                    let selection = backend.resolve_policy(native_version, Some(&mode)).unwrap();
                    assert_eq!(selection.requested_mode.as_deref(), Some(mode.as_str()));
                    assert_eq!(selection.resolved_mode, mode);
                    assert_eq!(selection.mode_source, ModeSource::Explicit);
                    assert_eq!(selection.permission_argv, permission_argv(backend, &mode));
                    let mut expected: Vec<String> = match backend {
                        CodingAgent::Claude => vec![
                            "claude".into(),
                            "--permission-mode".into(),
                            mode.clone(),
                            "--prompt-suggestions".into(),
                            "false".into(),
                            "--allowedTools".into(),
                            "Bash(cafleet message *)".into(),
                            "--name".into(),
                            "fixture name".into(),
                        ],
                        CodingAgent::Codex => {
                            let mut argv = vec!["codex".into()];
                            argv.extend(permission_argv(backend, &mode));
                            argv
                        }
                        CodingAgent::Opencode => vec![
                            "opencode".into(),
                            "mini".into(),
                            "--standalone".into(),
                            "--agent".into(),
                            mode.clone(),
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
    }

    #[test]
    fn runtime_policy_invalid_explicit_selection_reports_backend_and_catalog_choices() {
        for &backend in CodingAgent::all() {
            for invalid in ["", " ", "unsupported-fixture-mode"] {
                let failure = backend
                    .resolve_policy(version(backend), Some(invalid))
                    .expect_err("invalid explicit policy must fail");
                assert!(matches!(failure, CafleetError::Usage(_)));
                let text = failure.message();
                assert!(text.contains(backend.name()), "{text}");
                for choice in backend.policy_choices(version(backend)) {
                    assert!(text.contains(&choice), "{text}");
                }
            }
        }
    }

    #[test]
    fn runtime_policy_fixed_preset_and_unknown_native_version_keep_launch_intent() {
        let backend = CodingAgent::Opencode;
        assert_eq!(backend.policy_choices(None), ["cafleet", "cafleet-ask"]);
        assert_eq!(
            backend
                .resolve_policy(None, Some("cafleet"))
                .unwrap()
                .mode_source,
            ModeSource::Explicit
        );
        for &backend in CodingAgent::all() {
            let known = backend.resolve_policy(version(backend), None).unwrap();
            let unknown = backend.resolve_policy(None, None).unwrap();
            assert_eq!(unknown.resolved_mode, known.resolved_mode);
            assert_eq!(unknown.permission_argv, known.permission_argv);
        }
    }

    #[test]
    fn runtime_policy_opencode_unknown_choice_fails_with_the_exact_usage_error() {
        for native_version in [None, Some("2.1.4")] {
            let failure = CodingAgent::Opencode
                .resolve_policy(native_version, Some("ask"))
                .expect_err("unknown OpenCode policy must fail");
            assert!(matches!(failure, CafleetError::Usage(_)));
            assert_eq!(
                failure.message(),
                "--permission-mode for opencode must be one of cafleet, cafleet-ask (got 'ask')"
            );
        }
    }

    #[test]
    fn runtime_policy_help_lists_every_catalog_with_the_automatic_defaults() {
        assert_eq!(
            CodingAgent::permission_mode_help(),
            "Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet); hermes: smart, manual (default smart)"
        );
    }

    #[test]
    fn runtime_policy_codex_unknown_choice_fails_with_the_exact_usage_error() {
        for native_version in [None, Some("0.163.0")] {
            for value in ["auto", "untrusted", "approve_for_me"] {
                let failure = CodingAgent::Codex
                    .resolve_policy(native_version, Some(value))
                    .expect_err("unknown Codex policy must fail");
                assert!(matches!(failure, CafleetError::Usage(_)));
                assert_eq!(failure.exit_code(), 2);
                assert_eq!(
                    failure.message(),
                    format!(
                        "--permission-mode for codex must be one of on-request, never, approve-for-me (got '{value}')"
                    )
                );
            }
        }
    }

    #[test]
    fn runtime_policy_foreign_selection_is_rejected_before_native_argv_construction() {
        let selected = CodingAgent::Claude
            .resolve_policy(Some("2.1.302"), Some("default"))
            .unwrap();
        assert!(
            CodingAgent::Codex
                .build_spawn_argv_with_policy("assignment", "worker", cwd(), None, None, &selected)
                .is_err()
        );
        let smart = CodingAgent::Hermes.resolve_policy(None, None).unwrap();
        for backend in [
            CodingAgent::Claude,
            CodingAgent::Codex,
            CodingAgent::Opencode,
        ] {
            assert!(
                backend
                    .build_spawn_argv_with_policy("assignment", "worker", cwd(), None, None, &smart)
                    .is_err(),
                "{backend:?} rejects a hermes selection"
            );
        }
    }

    pub(super) mod hermes {
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use crate::coding_agent::tests::*;
        use serde_json::json;
        use std::cell::RefCell;

        const HERMES_EFFORTS: [&str; 8] = [
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ];

        pub(crate) fn approvals_json(mode: &str, deny: &[&str]) -> String {
            json!({"mode": mode, "timeout": 300, "deny": deny}).to_string()
        }

        fn smart() -> LaunchPolicySelection {
            CodingAgent::Hermes.resolve_policy(None, None).unwrap()
        }

        fn argv(model: Option<&str>, effort: Option<&str>) -> Vec<String> {
            CodingAgent::Hermes
                .build_spawn_argv_with_policy(
                    "assignment",
                    "worker",
                    cwd(),
                    model,
                    effort,
                    &smart(),
                )
                .unwrap()
        }

        fn strings(values: &[&str]) -> Vec<String> {
            values.iter().map(|value| (*value).to_owned()).collect()
        }

        #[test]
        fn hermes_registry_entry_names_its_binary_config_variable_and_default_home() {
            assert_eq!(coding_agent("hermes"), Some(CodingAgent::Hermes));
            assert_eq!(CodingAgent::Hermes.name(), "hermes");
            assert_eq!(CodingAgent::Hermes.config_variable(), "HERMES_HOME");
            assert!(CodingAgent::all().contains(&CodingAgent::Hermes));
            assert_eq!(
                CodingAgent::Hermes.config_dir(&|_| None, &home()).unwrap(),
                PathBuf::from("/home/tester/.hermes")
            );
            assert_eq!(
                CodingAgent::Hermes
                    .config_dir(
                        &|name| (name == "HERMES_HOME").then(|| "/srv/hermes home".into()),
                        &home()
                    )
                    .unwrap(),
                PathBuf::from("/srv/hermes home")
            );
            for value in ["", "relative/hermes", "~/.hermes"] {
                let failure = CodingAgent::Hermes
                    .config_dir(
                        &|name| (name == "HERMES_HOME").then(|| value.to_string()),
                        &home(),
                    )
                    .unwrap_err();
                assert_eq!(failure.exit_code(), 1);
                assert_eq!(
                    failure.message(),
                    format!("HERMES_HOME must be an absolute path (got '{value}')")
                );
            }
            assert_eq!(
                CodingAgent::Hermes.required_preset(Path::new("/srv/hermes"), "smart"),
                None
            );
        }

        #[test]
        fn hermes_argv_is_chat_cli_with_tool_source_and_cwd_then_model_reasoning_and_flag_prompt() {
            let base = ["hermes", "chat", "--cli", "--source", "tool", "--in", CWD];
            for (model, effort, tail) in [
                (None, None, vec!["-q", "assignment"]),
                (
                    Some("openai/gpt-6-astra"),
                    None,
                    vec!["--model", "openai/gpt-6-astra", "-q", "assignment"],
                ),
                (
                    None,
                    Some("ultra"),
                    vec!["--reasoning", "ultra", "-q", "assignment"],
                ),
                (
                    Some("plain-model"),
                    Some("none"),
                    vec![
                        "--model",
                        "plain-model",
                        "--reasoning",
                        "none",
                        "-q",
                        "assignment",
                    ],
                ),
            ] {
                let mut expected = strings(&base);
                expected.extend(strings(&tail));
                assert_eq!(argv(model, effort), expected, "{model:?} {effort:?}");
            }
        }

        #[test]
        fn hermes_argv_keeps_a_flag_like_prompt_and_spaced_cwd_as_single_values() {
            let prompt = "--resume 'x' \n{member_id}";
            let argv = CodingAgent::Hermes
                .build_spawn_argv_with_policy(prompt, "worker", cwd(), None, None, &smart())
                .unwrap();
            assert_eq!(argv[argv.len() - 2..], strings(&["-q", prompt]));
            assert_eq!(argv[6], CWD);
            assert_eq!(argv.len(), 9);
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
        fn hermes_argv_ignores_the_display_name() {
            let named = |name: &str| {
                CodingAgent::Hermes
                    .build_spawn_argv_with_policy("assignment", name, cwd(), None, None, &smart())
                    .unwrap()
            };
            assert_eq!(named("worker"), named("--name other display"));
            assert!(!named("unique-display-name").contains(&"unique-display-name".to_owned()));
        }

        #[test]
        fn only_hermes_argv_carries_the_member_cwd() {
            for &backend in CodingAgent::all() {
                let policy = backend.resolve_policy(None, None).unwrap();
                let build = |cwd: &str| {
                    backend
                        .build_spawn_argv_with_policy(
                            "assignment",
                            "worker",
                            Path::new(cwd),
                            None,
                            None,
                            &policy,
                        )
                        .unwrap()
                };
                let first = build("/first/cwd");
                let second = build("/second/cwd");
                if backend == CodingAgent::Hermes {
                    assert!(first.contains(&"/first/cwd".to_owned()), "{first:?}");
                    assert!(second.contains(&"/second/cwd".to_owned()), "{second:?}");
                } else {
                    assert_eq!(first, second, "{backend:?} ignores cwd");
                }
            }
        }

        #[test]
        fn hermes_launch_argv_replaces_only_the_binary_with_the_resolved_executable() {
            let executable = Path::new("/opt/hermes/bin/hermes");
            let bare = argv(Some("m"), Some("high"));
            let launched = CodingAgent::Hermes
                .launch_argv(
                    executable,
                    "assignment",
                    "worker",
                    cwd(),
                    Some("m"),
                    Some("high"),
                    &smart(),
                )
                .unwrap();
            assert_eq!(launched[0], "/opt/hermes/bin/hermes");
            assert_eq!(launched[1..], bare[1..]);
        }

        #[test]
        fn hermes_effort_accepts_its_reasoning_levels_and_rejects_others_with_the_usage_error() {
            for effort in HERMES_EFFORTS {
                CodingAgent::Hermes.validate_effort(Some(effort)).unwrap();
            }
            for invalid in ["turbo", "", "HIGH", "auto", "minimal "] {
                let failure = CodingAgent::Hermes
                    .validate_effort(Some(invalid))
                    .expect_err("an unknown hermes effort fails");
                assert!(matches!(failure, CafleetError::Usage(_)));
                assert_eq!(failure.exit_code(), 2);
                assert_eq!(
                    failure.message(),
                    format!(
                        "--effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got '{invalid}')."
                    )
                );
                assert!(
                    CodingAgent::Hermes
                        .build_spawn_argv_with_policy(
                            "assignment",
                            "worker",
                            cwd(),
                            None,
                            Some(invalid),
                            &smart()
                        )
                        .is_err()
                );
            }
        }

        #[test]
        fn hermes_model_passes_through_verbatim() {
            for model in ["openai/gpt-6-astra", "plain", "a/b/c", "model with spaces"] {
                CodingAgent::Hermes.validate_model(Some(model)).unwrap();
                let argv = argv(Some(model), None);
                let index = argv.iter().position(|token| token == "--model").unwrap();
                assert_eq!(argv[index + 1], model);
                assert_eq!(argv[index + 2], "-q");
            }
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
                "hermes 2026.9.24",
                "2026.9.24",
                "Hermes Agent v (2026.9.24)",
                "Hermes Agent vunknown(2026.9.24)",
                "Hermes Agent vunknown (2026.9.24) ",
                " Hermes Agent vunknown (2026.9.24)",
                "Hermes Agent  vunknown (2026.9.24)",
                "hermes agent vunknown (2026.9.24)",
                "Hermes Agent vunknown ()",
                "Hermes Agent vunknown (2026.9.24) (2026.9.25)",
                "Hermes Agent vunknown ((2026.9.24))",
                "Hermes Agent vunknown (2026.9.24",
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
                "Hermes Agent vunknown (2026.9)",
                "Hermes Agent vunknown (2026-09-24)",
                "Hermes Agent vunknown (unknown)",
                "Hermes Agent vunknown (2026.9.24.1)",
                "Hermes Agent vunknown (v2026.9.24)",
                "Hermes Agent vunknown (2026.9.24 extra)",
            ] {
                assert_eq!(
                    CodingAgent::Hermes.version_from_line(line),
                    Err("printed a non-SemVer Hermes version"),
                    "{line:?}"
                );
            }
        }

        #[test]
        fn other_backends_keep_the_single_token_version_rule() {
            for &backend in CodingAgent::all() {
                if backend == CodingAgent::Hermes {
                    continue;
                }
                for line in [
                    "2.1.302 (Claude Code)",
                    "codex-cli 0.163.0",
                    "opencode v2.1.4",
                    "v2.1.4 2.1.5",
                    "codex 2.1",
                    "Hermes Agent vunknown (2026.9.24)",
                ] {
                    assert_eq!(
                        backend.version_from_line(line),
                        version_token(line),
                        "{backend:?} {line}"
                    );
                }
            }
        }

        fn stub(dir: &Path, body: &str) -> PathBuf {
            executable_stub(dir, "hermes", body)
        }

        fn probe(executable: &Path) -> Result<String, CafleetError> {
            run_stub(|| {
                CodingAgent::Hermes
                    .probe_native_version(executable, Instant::now() + Duration::from_secs(2))
            })
        }

        #[test]
        fn hermes_native_version_probe_applies_the_hermes_rule_to_the_first_line() {
            let dir = tempfile::tempdir().unwrap();
            let path = stub(
                dir.path(),
                "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nprintf 'Hermes Agent vunknown (2026.9.24)\\nUpdate available: 2026.10.1\\n'",
            );
            assert_eq!(probe(&path).unwrap(), "2026.9.24");
        }

        #[test]
        fn hermes_native_version_probe_fails_when_only_the_package_semver_is_printed() {
            let dir = tempfile::tempdir().unwrap();
            let path = stub(dir.path(), "printf 'Hermes Agent v0.22.4\\n'");
            let failure = probe(&path).expect_err("the package version is not the native version");
            assert_eq!(
                failure.message(),
                "hermes --version printed no Hermes version line"
            );
            let catalog = ProfileCatalog::bundled().unwrap();
            let identity = native_launch_identity(CodingAgent::Hermes, Err(failure), &catalog)
                .unwrap_or_else(|error| {
                    panic!("a failed probe still launches: {}", error.message())
                });
            assert_eq!(identity.native_version, None);
            assert_eq!(identity.coverage, Coverage::Unknown);
            assert_eq!(
                identity.warning.as_deref(),
                Some(
                    "Warning: native version unavailable for hermes: hermes --version printed no Hermes version line; coverage unknown."
                )
            );
        }

        #[test]
        fn hermes_native_version_probe_rejects_a_non_semver_release_date() {
            let dir = tempfile::tempdir().unwrap();
            let path = stub(dir.path(), "printf 'Hermes Agent vunknown (2026.09.24)\\n'");
            assert_eq!(
                probe(&path).unwrap_err().message(),
                "hermes --version printed a non-SemVer Hermes version"
            );
        }

        #[test]
        fn hermes_launch_policy_is_smart_or_manual_at_every_version_with_no_permission_argv() {
            for native_version in [None, Some("2026.9.24"), Some("2026.10.1"), Some("1.0.0")] {
                assert_eq!(
                    CodingAgent::Hermes.policy_choices(native_version),
                    ["smart", "manual"],
                    "{native_version:?}"
                );
                let omitted = CodingAgent::Hermes
                    .resolve_policy(native_version, None)
                    .unwrap();
                assert_eq!(omitted.requested_mode, None);
                assert_eq!(omitted.resolved_mode, "smart");
                assert_eq!(omitted.mode_source, ModeSource::Default);
                assert!(omitted.permission_argv.is_empty());
                assert_eq!(
                    serde_json::to_value(&omitted.permission_argv).unwrap(),
                    json!([])
                );
                for mode in ["smart", "manual"] {
                    let explicit = CodingAgent::Hermes
                        .resolve_policy(native_version, Some(mode))
                        .unwrap();
                    assert_eq!(explicit.requested_mode.as_deref(), Some(mode));
                    assert_eq!(explicit.resolved_mode, mode);
                    assert_eq!(explicit.mode_source, ModeSource::Explicit);
                    assert!(explicit.permission_argv.is_empty());
                }
            }
        }

        #[test]
        fn hermes_launch_policy_rejects_every_other_mode_with_the_exact_usage_error() {
            for native_version in [None, Some("2026.9.24")] {
                for value in ["off", "", "Smart", "Manual", "auto"] {
                    let failure = CodingAgent::Hermes
                        .resolve_policy(native_version, Some(value))
                        .expect_err("only smart and manual are hermes launch policies");
                    assert!(matches!(failure, CafleetError::Usage(_)));
                    assert_eq!(failure.exit_code(), 2);
                    assert_eq!(
                        failure.message(),
                        format!(
                            "--permission-mode for hermes must be one of smart, manual (got '{value}')"
                        )
                    );
                }
            }
        }

        #[test]
        fn hermes_forwarded_config_returns_hermes_home() {
            let lookup = |name: &str| (name == "HERMES_HOME").then(|| "/srv/hermes".to_string());
            assert_eq!(
                CodingAgent::Hermes.forwarded_config(&lookup).unwrap(),
                Some(("HERMES_HOME".to_owned(), "/srv/hermes".to_owned()))
            );
        }

        #[test]
        fn hermes_spawn_environment_places_hermes_home_after_the_member_id() {
            let lookup = |name: &str| (name == "HERMES_HOME").then(|| "/srv/hermes".to_string());
            let forwarded = CodingAgent::Hermes.forwarded_config(&lookup).unwrap();
            assert_eq!(
                CodingAgent::spawn_environment(DATABASE_URL, 34, forwarded),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    ("CAFLEET_MEMBER_ID".to_owned(), "34".to_owned()),
                    ("HERMES_HOME".to_owned(), "/srv/hermes".to_owned()),
                ]
            );
        }

        #[test]
        fn hermes_director_only_denies_equal_the_opencode_preset_director_owned_denies() {
            let text = std::str::from_utf8(
                crate::assets::lookup(crate::assets::PRESETS, "opencode/cafleet.md")
                    .expect("embedded opencode cafleet preset"),
            )
            .unwrap();
            let frontmatter = text
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n"))
                .map(|(frontmatter, _)| frontmatter)
                .expect("preset frontmatter");
            let preset: serde_json::Value = serde_json::from_str(frontmatter).unwrap();
            let denied = preset["permission"]["shell"]
                .as_object()
                .expect("shell permission map")
                .iter()
                .filter(|(pattern, verdict)| *pattern != "*" && *verdict == "deny")
                .map(|(pattern, _)| pattern.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                denied,
                HERMES_DIRECTOR_ONLY_DENIES
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
            );
            assert_eq!(
                HERMES_DIRECTOR_ONLY_DENIES,
                [
                    "cafleet member prompt *",
                    "cafleet member exec *",
                    "cafleet member press *",
                    "cafleet member recover *",
                    "cafleet permission respond *",
                    "cafleet input reconcile *",
                ]
            );
        }

        #[test]
        fn hermes_client_env_passthrough_names_the_member_database_id_and_multiplexer_variables() {
            assert_eq!(
                HERMES_CLIENT_ENV_PASSTHROUGH,
                [
                    "CAFLEET_DATABASE_URL",
                    "CAFLEET_MEMBER_ID",
                    "TMUX",
                    "TMUX_PANE",
                    "HERDR_ENV",
                    "HERDR_PANE_ID",
                    "HERDR_TAB_ID",
                    "HERDR_WORKSPACE_ID",
                    "HERDR_SOCKET_PATH",
                    "HERDR_BIN_PATH",
                    "HERDR_SESSION",
                ]
            );
        }

        fn preflight(output: Result<String, CafleetError>) -> Result<(), CafleetError> {
            hermes_approvals_preflight(output)
        }

        fn assert_app_error(result: Result<(), CafleetError>, expected: &str) {
            let failure = result.expect_err(expected);
            assert!(
                matches!(failure, CafleetError::App(_)),
                "{}",
                failure.message()
            );
            assert_eq!(failure.exit_code(), 1);
            assert_eq!(failure.message(), expected);
        }

        #[test]
        fn hermes_preflight_accepts_every_director_only_deny_among_others_in_any_mode() {
            preflight(Ok(approvals_json("smart", &HERMES_DIRECTOR_ONLY_DENIES))).unwrap();
            preflight(Ok(approvals_json("manual", &HERMES_DIRECTOR_ONLY_DENIES))).unwrap();
            let mut deny = vec!["rm -rf /*", "git push *"];
            deny.extend(HERMES_DIRECTOR_ONLY_DENIES.iter().rev());
            preflight(Ok(approvals_json("smart", &deny))).unwrap();
            preflight(Ok(format!(
                "{}\n",
                approvals_json("smart", &HERMES_DIRECTOR_ONLY_DENIES)
            )))
            .unwrap();
        }

        #[test]
        fn hermes_preflight_reports_an_unreadable_configuration_with_its_cause() {
            assert_app_error(
                preflight(Err(CafleetError::App("hermes config get timed out".into()))),
                "cannot read hermes configuration through 'hermes config get': hermes config get timed out",
            );
            for output in [
                "",
                "not json",
                "[\"smart\"]",
                "\"smart\"",
                "null",
                "{\"mode\":",
            ] {
                let failure = preflight(Ok(output.into()))
                    .expect_err("output that is not a JSON object fails");
                assert!(matches!(failure, CafleetError::App(_)), "{output:?}");
                let cause = failure
                    .message()
                    .strip_prefix("cannot read hermes configuration through 'hermes config get': ")
                    .unwrap_or_else(|| panic!("{output:?}: {}", failure.message()))
                    .to_owned();
                assert!(!cause.is_empty(), "{output:?}");
            }
        }

        #[test]
        fn hermes_preflight_names_every_missing_director_only_glob_in_constant_order() {
            assert_app_error(
                preflight(Ok(approvals_json(
                    "smart",
                    &["cafleet input reconcile *", "cafleet member exec *"],
                ))),
                "hermes approvals.deny lacks CAFleet's Director-only rules: cafleet member prompt *, cafleet member press *, cafleet member recover *, cafleet permission respond *; run 'cafleet setup --coding-agent hermes'",
            );
            let every = HERMES_DIRECTOR_ONLY_DENIES.join(", ");
            for output in [
                approvals_json("smart", &[]),
                json!({"mode": "smart"}).to_string(),
                json!({"mode": "smart", "deny": null}).to_string(),
                json!({"mode": "smart", "deny": "cafleet member prompt *"}).to_string(),
                json!({"mode": "smart", "deny": [1, 2]}).to_string(),
                approvals_json("smart", &["cafleet member prompt", "cafleet member *"]),
                approvals_json(
                    "smart",
                    &["Cafleet member prompt *", " cafleet member exec *"],
                ),
            ] {
                assert_app_error(
                    preflight(Ok(output)),
                    &format!(
                        "hermes approvals.deny lacks CAFleet's Director-only rules: {every}; run 'cafleet setup --coding-agent hermes'"
                    ),
                );
            }
        }

        struct HermesProbe {
            home: PathBuf,
            config: Option<String>,
            available: bool,
            approvals: Result<String, String>,
            calls: RefCell<Vec<(PathBuf, PathBuf)>>,
        }

        impl SpawnProbe for HermesProbe {
            fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
                assert_eq!(name, "hermes");
                self.available.then(|| PathBuf::from("/fixture/bin/hermes"))
            }
            fn home_dir(&self) -> PathBuf {
                self.home.clone()
            }
            fn env_var(&self, name: &str) -> Option<String> {
                assert_eq!(name, "HERMES_HOME");
                self.config.clone()
            }
            fn hermes_approvals(
                &self,
                executable: &Path,
                config_base: &Path,
            ) -> Result<String, CafleetError> {
                self.calls
                    .borrow_mut()
                    .push((executable.into(), config_base.into()));
                self.approvals.clone().map_err(CafleetError::App)
            }
            fn hermes_client_env_passthrough(
                &self,
                _: &Path,
                _: &Path,
            ) -> Result<String, CafleetError> {
                unreachable!("availability reads no terminal.client_env_passthrough")
            }
        }

        fn hermes_probe(approvals: Result<String, String>) -> HermesProbe {
            HermesProbe {
                home: PathBuf::from("/home/tester"),
                config: None,
                available: true,
                approvals,
                calls: RefCell::new(Vec::new()),
            }
        }

        #[test]
        fn hermes_availability_reads_the_effective_approvals_of_the_resolved_home_once() {
            let mut probe = hermes_probe(Ok(approvals_json("smart", &HERMES_DIRECTOR_ONLY_DENIES)));
            assert_eq!(
                CodingAgent::Hermes
                    .ensure_available(&probe, "smart")
                    .unwrap(),
                PathBuf::from("/fixture/bin/hermes")
            );
            assert_eq!(
                *probe.calls.borrow(),
                [(
                    PathBuf::from("/fixture/bin/hermes"),
                    PathBuf::from("/home/tester/.hermes")
                )]
            );
            probe.config = Some("/srv/hermes".into());
            probe.calls.borrow_mut().clear();
            CodingAgent::Hermes
                .ensure_available(&probe, "manual")
                .unwrap();
            assert_eq!(
                *probe.calls.borrow(),
                [(
                    PathBuf::from("/fixture/bin/hermes"),
                    PathBuf::from("/srv/hermes")
                )]
            );
        }

        #[test]
        fn hermes_availability_checks_the_home_then_the_binary_before_reading_approvals() {
            let mut probe = hermes_probe(Ok(approvals_json("smart", &HERMES_DIRECTOR_ONLY_DENIES)));
            probe.config = Some("relative".into());
            assert_eq!(
                CodingAgent::Hermes
                    .ensure_available(&probe, "smart")
                    .unwrap_err()
                    .message(),
                "HERMES_HOME must be an absolute path (got 'relative')"
            );
            probe.config = None;
            probe.available = false;
            assert_eq!(
                CodingAgent::Hermes
                    .ensure_available(&probe, "smart")
                    .unwrap_err()
                    .message(),
                "binary hermes not found on PATH"
            );
            assert!(probe.calls.borrow().is_empty());
        }

        #[test]
        fn hermes_availability_fails_with_each_preflight_error() {
            for (approvals, expected) in [
                (
                    Err("exited with exit status: 1".to_owned()),
                    "cannot read hermes configuration through 'hermes config get': exited with exit status: 1".to_owned(),
                ),
                (
                    Ok(approvals_json("smart", &HERMES_DIRECTOR_ONLY_DENIES[1..])),
                    "hermes approvals.deny lacks CAFleet's Director-only rules: cafleet member prompt *; run 'cafleet setup --coding-agent hermes'".to_owned(),
                ),
            ] {
                let probe = hermes_probe(approvals);
                let failure = CodingAgent::Hermes
                    .ensure_available(&probe, "smart")
                    .expect_err(&expected);
                assert_eq!(failure.exit_code(), 1);
                assert_eq!(failure.message(), expected);
            }
        }
    }
}
