pub(crate) mod catalog;
pub(crate) mod detector;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub(crate) enum CodingAgent {
    Claude,
    Codex,
    Opencode,
}

impl CodingAgent {
    pub(crate) fn all() -> &'static [Self] {
        Self::value_variants()
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }

    pub(crate) fn config_variable(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
            Self::Opencode => "OPENCODE_CONFIG_DIR",
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
        Ok(executable)
    }

    pub(crate) fn launch_argv(
        self,
        executable: &Path,
        prompt: &str,
        display_name: &str,
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
            self.build_spawn_argv_with_policy(prompt, display_name, model, effort, policy)?;
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
        if self == Self::Opencode {
            argv.extend(["mini".into(), "--standalone".into()]);
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
                Self::Opencode => unreachable!("effort validated before argv construction"),
            }
        }
        if self == Self::Opencode {
            argv.push("--prompt".into());
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
        let baseline = self.build_spawn_argv_with_policy("", "", None, None, policy)?;
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
        let mut cursor = baseline.len() - 1 - usize::from(self == Self::Opencode);
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
            (Self::Claude | Self::Codex, [prompt]) => Some(prompt.as_str()),
            _ => return Err(invalid()),
        };
        let mut expected = self.build_spawn_argv_with_policy(
            prompt.unwrap_or(""),
            display_name,
            model,
            effort,
            policy,
        )?;
        if prompt.is_none() {
            expected.truncate(expected.len() - 1 - usize::from(self == Self::Opencode));
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
                    _ => {}
                }
                if agent == CodingAgent::Opencode {
                    expected.push("--prompt".into());
                }
                expected.push(prompt.into());
                assert_eq!(
                    agent
                        .build_spawn_argv_with_policy(
                            prompt,
                            "display name",
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
            _ => None,
        }
    }

    #[test]
    fn spawn_environment_forwards_the_selected_config_variable_after_the_database_url() {
        for &agent in CodingAgent::all() {
            let expected = every_config_variable(agent.config_variable()).unwrap();
            assert_eq!(
                agent
                    .spawn_environment(DATABASE_URL, &every_config_variable)
                    .unwrap(),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    (agent.config_variable().to_owned(), expected),
                ],
                "{agent:?} forwards only its own variable with the creator's exact value"
            );
        }
    }

    #[test]
    fn spawn_environment_adds_nothing_when_the_config_variable_is_unset() {
        for &agent in CodingAgent::all() {
            let others = |name: &str| {
                (name != agent.config_variable())
                    .then(|| every_config_variable(name))
                    .flatten()
            };
            assert_eq!(
                agent.spawn_environment(DATABASE_URL, &others).unwrap(),
                vec![("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned())],
                "{agent:?} forwards no default and no other backend's variable"
            );
        }
    }

    #[test]
    fn spawn_environment_ignores_an_invalid_value_of_another_backend() {
        for &agent in CodingAgent::all() {
            let others_invalid = |name: &str| {
                if name == agent.config_variable() {
                    Some("/cfg/selected".to_string())
                } else {
                    Some("relative".to_string())
                }
            };
            assert_eq!(
                agent
                    .spawn_environment(DATABASE_URL, &others_invalid)
                    .unwrap(),
                vec![
                    ("CAFLEET_DATABASE_URL".to_owned(), DATABASE_URL.to_owned()),
                    (
                        agent.config_variable().to_owned(),
                        "/cfg/selected".to_owned()
                    ),
                ]
            );
        }
    }

    #[test]
    fn spawn_environment_rejects_an_invalid_selected_value_with_the_config_dir_error() {
        for &agent in CodingAgent::all() {
            for value in ["", "relative/cfg", "~/cfg"] {
                let lookup =
                    |name: &str| (name == agent.config_variable()).then(|| value.to_string());
                let failure = agent
                    .spawn_environment(DATABASE_URL, &lookup)
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
        for agent in [CodingAgent::Claude, CodingAgent::Codex] {
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
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("versions/2.1.302/claude");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(
            &executable,
            "#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nprintf '2.1.302 (Claude Code)\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
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
            backend
                .probe_native_version(&resolved, Instant::now() + Duration::from_secs(2))
                .unwrap(),
            "2.1.302",
            "the probe runs the resolved executable"
        );
        let policy = backend.resolve_policy(None, None).unwrap();
        let bare = backend
            .build_spawn_argv_with_policy("assignment", "worker", None, Some("high"), &policy)
            .unwrap();
        assert_eq!(bare[0], "claude");
        let launched = backend
            .launch_argv(
                &resolved,
                "assignment",
                "worker",
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
        use crate::coding_agent::tests::*;
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
        }
    }

    /// The versions with an explicit-choice catalog: Claude and Codex offer one
    /// only at the bundled version or unprobed; OpenCode at every version.
    fn catalog_versions(backend: CodingAgent) -> &'static [Option<&'static str>] {
        match backend {
            CodingAgent::Claude | CodingAgent::Codex => &versions(backend)[..2],
            CodingAgent::Opencode => versions(backend),
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
                    };
                    expected.extend(["--model".into(), "provider/model".into()]);
                    let effort = (backend != CodingAgent::Opencode).then_some("high");
                    match backend {
                        CodingAgent::Claude => expected.extend(["--effort".into(), "high".into()]),
                        CodingAgent::Codex => {
                            expected.push("--config=model_reasoning_effort=high".into())
                        }
                        CodingAgent::Opencode => expected.push("--prompt".into()),
                    }
                    expected.push("literal '$x' `text` assignment".into());
                    assert_eq!(
                        backend
                            .build_spawn_argv_with_policy(
                                "literal '$x' `text` assignment",
                                "fixture name",
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
            "Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet)"
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
                .build_spawn_argv_with_policy("assignment", "worker", None, None, &selected)
                .is_err()
        );
    }
}
