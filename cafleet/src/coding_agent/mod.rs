mod native_version;
pub(crate) use native_version::native_launch_identity;
#[cfg(test)]
pub(crate) use native_version::version_token;
mod policy;
pub use policy::LaunchPolicySelection;
mod profiles;
pub(crate) use profiles::AnswerStage;
#[cfg(test)]
pub(crate) use profiles::ScreenRegion;
pub use profiles::{AnswerCapability, NativeCapabilities, ProfileCatalog};
pub(crate) use profiles::{AnswerMatch, AnswerPlan};
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "the reviewed input coordinator consumes the canonical composer exports"
    )
)]
pub(crate) use profiles::{ComposerOperation, ComposerPlan};
pub(crate) use profiles::{
    Detector, MatchEvidence, ObservationKind, ScreenObservation, SemanticFingerprint,
};
#[cfg(test)]
pub(crate) use profiles::{inspection_fixture_definition, inspection_fixture_form};

#[cfg(test)]
mod policy_contract_tests {
    use super::*;
    use crate::broker::runtime::ModeSource;

    fn version(backend: CodingAgent) -> Option<&'static str> {
        match backend {
            CodingAgent::Claude => Some("2.1.302"),
            CodingAgent::Codex => Some("0.163.0"),
            CodingAgent::Opencode => None,
        }
    }

    fn permission_argv(backend: CodingAgent, selected: &str) -> Vec<String> {
        match backend {
            CodingAgent::Claude => vec!["--permission-mode".into(), selected.into()],
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
    fn runtime_policy_omission_preserves_the_complete_existing_native_argv() {
        for &backend in CodingAgent::all() {
            let selection = backend.resolve_policy(version(backend), None).unwrap();
            let expected_mode = match backend {
                CodingAgent::Claude => "dontAsk",
                CodingAgent::Codex => "never",
                CodingAgent::Opencode => "cafleet",
            };
            assert_eq!(selection.requested_mode, None);
            assert_eq!(selection.resolved_mode, expected_mode);
            assert_eq!(selection.mode_source, ModeSource::Default);
            assert_eq!(
                selection.permission_argv,
                permission_argv(backend, expected_mode)
            );
            assert_eq!(
                backend
                    .build_spawn_argv_with_policy(
                        "literal '$x' `text`",
                        "fixture name",
                        Some("provider/model"),
                        None,
                        &selection
                    )
                    .unwrap(),
                backend.build_spawn_argv(
                    "literal '$x' `text`",
                    "fixture name",
                    Some("provider/model"),
                    None
                )
            );
        }
    }

    #[test]
    fn runtime_policy_each_registered_explicit_choice_maps_to_one_literal_native_mode() {
        for &backend in CodingAgent::all() {
            let choices = backend.policy_choices(version(backend));
            assert!(!choices.is_empty());
            let mut unique = std::collections::BTreeSet::new();
            for mode in choices {
                assert!(unique.insert(mode.clone()), "duplicate policy {mode}");
                let selection = backend
                    .resolve_policy(version(backend), Some(&mode))
                    .unwrap();
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

    #[test]
    fn runtime_policy_claude_default_and_manual_preserve_their_requested_spelling() {
        let backend = CodingAgent::Claude;
        for mode in ["default", "manual"] {
            assert!(
                backend
                    .policy_choices(Some("2.1.302"))
                    .contains(&mode.into())
            );
            let selected = backend.resolve_policy(Some("2.1.302"), Some(mode)).unwrap();
            assert_eq!(selected.resolved_mode, mode);
            assert_eq!(selected.permission_argv, ["--permission-mode", mode]);
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
    fn runtime_policy_opencode_catalog_offers_cafleet_and_cafleet_ask_for_every_native_version() {
        for native_version in [None, Some("2.1.4"), Some("2.1.3"), Some("9.0.0-beta.1")] {
            assert_eq!(
                CodingAgent::Opencode.policy_choices(native_version),
                ["cafleet", "cafleet-ask"],
                "{native_version:?}"
            );
            for mode in ["cafleet", "cafleet-ask"] {
                let selection = CodingAgent::Opencode
                    .resolve_policy(native_version, Some(mode))
                    .unwrap();
                assert_eq!(selection.permission_argv, ["--agent", mode]);
            }
        }
    }

    #[test]
    fn runtime_policy_opencode_explicit_cafleet_ask_launches_its_agent_and_records_launch_intent() {
        let selection = CodingAgent::Opencode
            .resolve_policy(Some("2.1.4"), Some("cafleet-ask"))
            .unwrap();
        assert_eq!(selection.requested_mode.as_deref(), Some("cafleet-ask"));
        assert_eq!(selection.resolved_mode, "cafleet-ask");
        assert_eq!(selection.mode_source, ModeSource::Explicit);
        assert_eq!(
            serde_json::to_value(&selection.permission_argv).unwrap(),
            serde_json::json!(["--agent", "cafleet-ask"])
        );
        assert_eq!(
            CodingAgent::Opencode
                .build_spawn_argv_with_policy("assignment", "worker", None, None, &selection)
                .unwrap(),
            [
                "opencode",
                "mini",
                "--standalone",
                "--agent",
                "cafleet-ask",
                "--prompt",
                "assignment"
            ]
        );
        assert_eq!(
            CodingAgent::Opencode
                .build_spawn_argv_with_policy(
                    "assignment",
                    "worker",
                    Some("provider/model"),
                    None,
                    &selection
                )
                .unwrap(),
            [
                "opencode",
                "mini",
                "--standalone",
                "--agent",
                "cafleet-ask",
                "--model",
                "provider/model",
                "--prompt",
                "assignment"
            ]
        );
    }

    #[test]
    fn runtime_policy_opencode_omitted_selection_keeps_the_cafleet_agent_as_default() {
        let selection = CodingAgent::Opencode
            .resolve_policy(Some("2.1.4"), None)
            .unwrap();
        assert_eq!(selection.requested_mode, None);
        assert_eq!(selection.resolved_mode, "cafleet");
        assert_eq!(selection.mode_source, ModeSource::Default);
        assert_eq!(selection.permission_argv, ["--agent", "cafleet"]);
        assert_eq!(
            CodingAgent::Opencode
                .build_spawn_argv_with_policy("assignment", "worker", None, None, &selection)
                .unwrap(),
            [
                "opencode",
                "mini",
                "--standalone",
                "--agent",
                "cafleet",
                "--prompt",
                "assignment"
            ]
        );
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
    fn runtime_policy_help_lists_both_opencode_choices_with_the_cafleet_default() {
        assert!(
            CodingAgent::permission_mode_help()
                .contains("opencode: cafleet, cafleet-ask (default cafleet)"),
            "{}",
            CodingAgent::permission_mode_help()
        );
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

use std::path::{Path, PathBuf};

use clap::ValueEnum;

use crate::config_dir::{self, EnvLookup, ResolvedDir};
use crate::error::CafleetError;

pub trait SpawnProbe {
    fn resolve_binary(&self, name: &str) -> Option<PathBuf>;
    fn home_dir(&self) -> PathBuf;
    fn env_var(&self, name: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum CodingAgent {
    Claude,
    Codex,
    Opencode,
}

impl CodingAgent {
    pub fn all() -> &'static [Self] {
        Self::value_variants()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }

    pub fn binary_name(self) -> &'static str {
        self.name()
    }

    pub fn config_variable(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
            Self::Opencode => "OPENCODE_CONFIG_DIR",
        }
    }

    pub fn config_dir(self, env: EnvLookup<'_>, home: &Path) -> Result<ResolvedDir, CafleetError> {
        match self {
            Self::Claude => config_dir::claude_config_dir(env, home),
            Self::Codex => config_dir::codex_home(env, home),
            Self::Opencode => config_dir::opencode_preset_base(env, home),
        }
    }

    pub fn required_preset(self, config: &Path, mode: &str) -> Option<PathBuf> {
        (self == Self::Opencode).then(|| config.join(format!("agents/{mode}.md")))
    }

    pub fn validate_model(self, model: Option<&str>) -> Result<(), CafleetError> {
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

    pub fn validate_effort(self, effort: Option<&str>) -> Result<(), CafleetError> {
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

    pub fn ensure_available(
        self,
        probe: &dyn SpawnProbe,
        mode: &str,
    ) -> Result<PathBuf, CafleetError> {
        let config = self.config_dir(&|name| probe.env_var(name), &probe.home_dir())?;
        let executable = probe.resolve_binary(self.binary_name()).ok_or_else(|| {
            CafleetError::App(format!("binary {} not found on PATH", self.binary_name()))
        })?;
        if let Some(preset) = self.required_preset(&config.path, mode) {
            std::fs::read(&preset).map_err(|error| CafleetError::App(format!(
                "cannot read opencode agent preset at {}: {error}; run 'cafleet setup --coding-agent opencode' first", preset.display()
            )))?;
        }
        Ok(executable)
    }

    pub fn launch_argv(
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

    pub fn build_spawn_argv(
        self,
        prompt: &str,
        display_name: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Vec<String> {
        let policy = self
            .resolve_policy(None, None)
            .expect("registered backend has a valid default policy");
        self.build_spawn_argv_with_policy(prompt, display_name, model, effort, &policy)
            .expect("spawn inputs satisfy backend validation")
    }
}

pub fn coding_agent(name: &str) -> Option<CodingAgent> {
    CodingAgent::all()
        .iter()
        .copied()
        .find(|agent| agent.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe {
        agent: CodingAgent,
        home: PathBuf,
        config: Option<String>,
        available: bool,
    }

    impl SpawnProbe for Probe {
        fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
            assert_eq!(name, self.agent.binary_name());
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
                    "dontAsk",
                    "--prompt-suggestions",
                    "false",
                    "--allowedTools",
                    "Bash(cafleet message *)",
                    "--name",
                    "display name",
                ],
            ),
            (
                CodingAgent::Codex,
                vec![
                    "codex",
                    "--ask-for-approval",
                    "never",
                    "--sandbox",
                    "workspace-write",
                ],
            ),
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
                    agent.build_spawn_argv(
                        prompt,
                        "display name",
                        configured.then_some("provider/model with spaces"),
                        effort
                    ),
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

    mod native_version_probe {
        use super::*;
        use crate::broker::runtime::Coverage;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

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
            let only_version =
                "[ \"$#\" -eq 1 ] && [ \"$1\" = --version ] || exit 3\nread -r ignored || true";
            for (output, expected) in [
                ("2.1.302 (Claude Code)", "2.1.302"),
                ("codex-cli 0.163.0", "0.163.0"),
                ("opencode v2.1.4", "2.1.4"),
                ("1.2.3-beta.1", "1.2.3-beta.1"),
            ] {
                let path = stub(
                    dir.path(),
                    "native",
                    &format!("{only_version}\nprintf '%s\\n' '{output}'\nprintf 'ignored\\n' >&2"),
                );
                assert_eq!(probe(&path).unwrap(), expected, "{output}");
            }
        }

        #[test]
        fn native_version_probe_rejects_failed_ambiguous_invalid_or_missing_versions() {
            let dir = tempfile::tempdir().unwrap();
            for (case, body) in [
                ("nonzero exit", "printf '2.1.302\\n'; exit 1"),
                ("no version token", "printf 'Claude Code\\n'"),
                ("two version tokens", "printf '2.1.302 2.1.303\\n'"),
                (
                    "v-prefixed and plain version tokens",
                    "printf 'v2.1.4 2.1.5\\n'",
                ),
                (
                    "version outside the first line",
                    "printf 'Claude Code\\n2.1.302\\n'",
                ),
                ("empty output", "exit 0"),
                ("non-UTF-8 output", "printf '2.1.302 \\377\\n'"),
            ] {
                let path = stub(dir.path(), "native", body);
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
            assert_eq!(tested.coverage, Coverage::Tested);
            assert_eq!(tested.warning, None);
            let unprofiled =
                native_launch_identity(CodingAgent::Claude, Ok("2.1.301".into()), &catalog);
            assert_eq!(unprofiled.native_version.as_deref(), Some("2.1.301"));
            assert_eq!(unprofiled.profile_version, None);
            assert_eq!(unprofiled.coverage, Coverage::Unknown);
            assert_eq!(unprofiled.warning, None);
            let failed = native_launch_identity(
                CodingAgent::Codex,
                Err(CafleetError::App("probe exited with status 1".into())),
                &catalog,
            );
            assert_eq!(failed.native_version, None);
            assert_eq!(failed.profile_version, None);
            assert_eq!(failed.coverage, Coverage::Unknown);
            assert_eq!(
                failed.warning.as_deref(),
                Some(
                    "Warning: native version unavailable for codex: probe exited with status 1; coverage unknown."
                )
            );
        }
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
                    PathBuf::from("/fixture/bin").join(agent.binary_name())
                );
            }
        }
        assert!(coding_agent("Claude").is_none());
        assert!(coding_agent("unknown").is_none());
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

    #[test]
    fn native_launch_probes_and_launches_the_same_resolved_executable() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
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
}
