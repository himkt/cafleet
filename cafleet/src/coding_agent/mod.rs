mod native_version;
#[cfg(test)]
pub(crate) use native_version::version_token;
pub(crate) use native_version::{NativeVersion, native_launch_identity};
mod policy;
pub(crate) use policy::LaunchPolicySelection;
pub(crate) mod profiles;
pub(crate) use profiles::ComposerOperation;
#[cfg(test)]
pub(crate) use profiles::ScreenRegion;
pub(crate) use profiles::{
    AnnotationBlock, AnnotationFingerprint, Detector, MatchEvidence, ObservationKind,
    ScreenObservation, SemanticFingerprint,
};
pub(crate) use profiles::{AnswerCapability, ProfileCatalog};
pub(crate) use profiles::{AnswerMatch, AnswerPlan};
pub(crate) use profiles::{AnswerStage, StageGate};

use std::path::{Path, PathBuf};

use clap::ValueEnum;

use crate::config_dir::{self, EnvLookup};
use crate::error::CafleetError;

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
            Self::Opencode => config_dir::opencode_skills_base(home),
        };
        config_dir::resolve(self.config_variable(), env, default)
    }

    pub(crate) fn spawn_environment(
        self,
        database_url: &str,
        env: EnvLookup<'_>,
    ) -> Result<Vec<(String, String)>, CafleetError> {
        let mut environment = vec![("CAFLEET_DATABASE_URL".to_owned(), database_url.to_owned())];
        if let Some(value) = env(self.config_variable()) {
            config_dir::require_absolute(self.config_variable(), &value)?;
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
            config_dir::opencode_skills_base(&home)
        );
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
