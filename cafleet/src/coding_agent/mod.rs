mod profiles;
pub use profiles::AnswerCapability;

#[cfg(test)]
mod policy_contract_tests;

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
        let base: &[&str] = match self {
            // Claude's variadic --allowedTools ends at --name, keeping the prompt positional.
            Self::Claude => &[
                "claude",
                "--permission-mode",
                "dontAsk",
                "--prompt-suggestions",
                "false",
                "--allowedTools",
                "Bash(cafleet message *)",
                "--name",
                display_name,
            ],
            Self::Codex => &[
                "codex",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
            ],
            Self::Opencode => &["opencode", "mini", "--standalone", "--agent", "cafleet"],
        };
        let mut argv = base
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        if let Some(model) = model {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = effort {
            match self {
                Self::Claude => argv.extend(["--effort".into(), effort.into()]),
                Self::Codex => argv.push(format!("--config=model_reasoning_effort={effort}")),
                Self::Opencode => panic!("OpenCode spawn has no reasoning effort"),
            }
        }
        if self == Self::Opencode {
            argv.push("--prompt".into());
        }
        argv.push(prompt.into());
        argv
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
