mod policy;
pub use policy::LaunchPolicySelection;
mod profiles;
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "native permission response stages consume the canonical AnswerStage type"
    )
)]
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
            CodingAgent::Opencode => vec!["--agent".into(), "cafleet".into()],
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
                        "cafleet".into(),
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
        assert_eq!(backend.policy_choices(None), ["cafleet"]);
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
    fn binary_on_path(&self, name: &str) -> bool;
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

    pub fn required_preset(self, config: &Path) -> Option<PathBuf> {
        (self == Self::Opencode).then(|| config.join("agents/cafleet.md"))
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

    pub fn ensure_available(self, probe: &dyn SpawnProbe) -> Result<(), CafleetError> {
        let config = self.config_dir(&|name| probe.env_var(name), &probe.home_dir())?;
        if !probe.binary_on_path(self.binary_name()) {
            return Err(CafleetError::App(format!(
                "binary {} not found on PATH",
                self.binary_name()
            )));
        }
        if let Some(preset) = self.required_preset(&config.path) {
            std::fs::read(&preset).map_err(|error| CafleetError::App(format!(
                "cannot read opencode agent preset at {}: {error}; run 'cafleet setup --coding-agent opencode' first", preset.display()
            )))?;
        }
        Ok(())
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
        fn binary_on_path(&self, name: &str) -> bool {
            assert_eq!(name, self.agent.binary_name());
            self.available
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

    #[test]
    fn selected_preflight_checks_paths_then_binary_and_relocated_required_preset() {
        let dir = tempfile::tempdir().unwrap();
        for agent in CodingAgent::all() {
            let mut probe = Probe {
                agent: *agent,
                home: dir.path().into(),
                config: Some("relative".into()),
                available: true,
            };
            assert!(
                agent
                    .ensure_available(&probe)
                    .unwrap_err()
                    .message()
                    .contains(agent.config_variable())
            );
            probe.config = Some(dir.path().join(agent.name()).display().to_string());
            probe.available = false;
            assert!(
                agent
                    .ensure_available(&probe)
                    .unwrap_err()
                    .message()
                    .contains("not found on PATH")
            );
            probe.available = true;
            if *agent == CodingAgent::Opencode {
                assert!(agent.ensure_available(&probe).is_err());
                let preset =
                    PathBuf::from(probe.config.as_ref().unwrap()).join("agents/cafleet.md");
                std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
                std::fs::write(&preset, "preset").unwrap();
                agent.ensure_available(&probe).unwrap();
                std::fs::remove_file(&preset).unwrap();
                std::fs::create_dir(&preset).unwrap();
                assert!(agent.ensure_available(&probe).is_err());
            } else {
                agent.ensure_available(&probe).unwrap();
            }
        }
        assert!(coding_agent("Claude").is_none());
        assert!(coding_agent("unknown").is_none());
    }
}
