//! codex backend (SPEC §6.7): pass-through model validation, its own
//! effort enum, the one-token reasoning-effort config, and an ignored
//! `display_name`. The colocated tests pin the contract; see
//! [`super::test_support`] for the API.

use super::{CodingAgent, SpawnProbe, missing_binary};
use crate::error::CafleetError;

const EFFORT_LEVELS: [&str; 5] = ["minimal", "low", "medium", "high", "xhigh"];

pub struct Codex;

impl CodingAgent for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn binary_name(&self) -> &'static str {
        "codex"
    }

    fn validate_model(&self, _model: Option<&str>) -> Result<(), CafleetError> {
        Ok(())
    }

    fn validate_effort(&self, effort: Option<&str>) -> Result<(), CafleetError> {
        match effort {
            None => Ok(()),
            Some(level) if EFFORT_LEVELS.contains(&level) => Ok(()),
            Some(level) => Err(CafleetError::Usage(format!(
                "--effort for the codex backend must be one of minimal, low, medium, \
                 high, xhigh (got '{level}')."
            ))),
        }
    }

    fn ensure_available(&self, probe: &dyn SpawnProbe) -> Result<(), CafleetError> {
        if probe.binary_on_path(self.binary_name()) {
            Ok(())
        } else {
            Err(missing_binary(self.binary_name()))
        }
    }

    fn build_spawn_argv(
        &self,
        prompt: &str,
        _display_name: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Vec<String> {
        let mut argv = vec![
            "codex".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ];
        if let Some(model) = model {
            argv.push("--model".to_string());
            argv.push(model.to_string());
        }
        if let Some(effort) = effort {
            argv.push(format!("--config=model_reasoning_effort={effort}"));
        }
        argv.push(prompt.to_string());
        argv
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::coding_agent::coding_agent;
    use crate::coding_agent::test_support::{FakeProbe, argv};

    fn codex() -> &'static dyn crate::coding_agent::CodingAgent {
        coding_agent("codex").unwrap()
    }

    #[test]
    fn names_are_pinned() {
        assert_eq!(codex().name(), "codex");
        assert_eq!(codex().binary_name(), "codex");
    }

    #[test]
    fn validate_model_is_pass_through() {
        assert!(codex().validate_model(None).is_ok());
        assert!(codex().validate_model(Some("o3")).is_ok());
    }

    #[test]
    fn validate_effort_accepts_the_five_levels_and_none() {
        for level in ["minimal", "low", "medium", "high", "xhigh"] {
            assert!(codex().validate_effort(Some(level)).is_ok(), "{level}");
        }
        assert!(codex().validate_effort(None).is_ok());
    }

    #[test]
    fn validate_effort_rejects_an_unknown_level_with_the_pinned_message() {
        let err = codex()
            .validate_effort(Some("max"))
            .expect_err("'max' is claude's level, not codex's");
        assert_eq!(
            err.message(),
            "--effort for the codex backend must be one of minimal, low, medium, \
             high, xhigh (got 'max')."
        );
    }

    #[test]
    fn spawn_argv_ignores_display_name_and_packs_effort_into_one_token() {
        assert_eq!(
            codex().build_spawn_argv("do it", "ignored", Some("o3"), Some("high")),
            argv(&[
                "codex",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
                "--model",
                "o3",
                "--config=model_reasoning_effort=high",
                "do it",
            ]),
            "the reasoning-effort config is ONE token"
        );
    }

    #[test]
    fn spawn_argv_omits_all_tokens_for_none_model_and_effort() {
        assert_eq!(
            codex().build_spawn_argv("do it", "ignored", None, None),
            argv(&[
                "codex",
                "--ask-for-approval",
                "never",
                "--sandbox",
                "workspace-write",
                "do it",
            ])
        );
    }

    #[test]
    fn ensure_available_is_a_path_check_only() {
        let home = TempDir::new().unwrap();
        let err = codex()
            .ensure_available(&FakeProbe::without_binaries(home.path()))
            .expect_err("a missing binary must be rejected");
        assert_eq!(err.message(), "binary codex not found on PATH");

        assert!(
            codex()
                .ensure_available(&FakeProbe::with_binary("codex", home.path()))
                .is_ok(),
            "the codex rules file is not a spawn precondition"
        );
    }
}
