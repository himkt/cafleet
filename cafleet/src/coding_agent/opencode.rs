//! opencode backend (SPEC §6.7): the `<provider-id>/<model-id>` model rule, no
//! reasoning-effort support, the preset-existence spawn precondition, and the
//! `--prompt` flag-pair prompt. The colocated tests pin the contract; see
//! [`super::test_support`] for the API.

use super::{CodingAgent, SpawnProbe, missing_binary};
use crate::config_dir::opencode_preset_base;
use crate::error::CafleetError;

pub struct Opencode;

impl CodingAgent for Opencode {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn binary_name(&self) -> &'static str {
        "opencode"
    }

    fn validate_model(&self, model: Option<&str>) -> Result<(), CafleetError> {
        let Some(model) = model else {
            return Ok(());
        };
        let valid = model
            .split_once('/')
            .is_some_and(|(provider, id)| !provider.is_empty() && !id.is_empty());
        if valid {
            Ok(())
        } else {
            Err(CafleetError::Usage(format!(
                "--model for the opencode backend must be \
                 '<provider-id>/<model-id>' (got '{model}')."
            )))
        }
    }

    fn validate_effort(&self, effort: Option<&str>) -> Result<(), CafleetError> {
        match effort {
            None => Ok(()),
            Some(_) => Err(CafleetError::Usage(
                "opencode does not support reasoning effort.".to_string(),
            )),
        }
    }

    fn ensure_available(&self, probe: &dyn SpawnProbe) -> Result<(), CafleetError> {
        if !probe.binary_on_path(self.binary_name()) {
            return Err(missing_binary(self.binary_name()));
        }
        let env = |name: &str| probe.env_var(name);
        let preset = opencode_preset_base(&env, &probe.home_dir())?
            .path
            .join("agents/cafleet.md");
        if preset.is_file() {
            Ok(())
        } else {
            Err(CafleetError::App(format!(
                "opencode agent preset not found at {}; \
                 run 'cafleet setup --coding-agent opencode' first",
                preset.display()
            )))
        }
    }

    fn build_spawn_argv(
        &self,
        prompt: &str,
        _display_name: &str,
        model: Option<&str>,
        _effort: Option<&str>,
    ) -> Vec<String> {
        let mut argv = vec![
            "opencode".to_string(),
            "--agent".to_string(),
            "cafleet".to_string(),
        ];
        if let Some(model) = model {
            argv.push("--model".to_string());
            argv.push(model.to_string());
        }
        argv.push("--prompt".to_string());
        argv.push(prompt.to_string());
        argv
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::coding_agent::coding_agent;
    use crate::coding_agent::test_support::{FakeProbe, argv};

    fn opencode() -> &'static dyn crate::coding_agent::CodingAgent {
        coding_agent("opencode").unwrap()
    }

    #[test]
    fn names_are_pinned() {
        assert_eq!(opencode().name(), "opencode");
        assert_eq!(opencode().binary_name(), "opencode");
    }

    #[test]
    fn validate_model_splits_on_the_first_slash() {
        assert!(opencode().validate_model(None).is_ok());
        assert!(opencode().validate_model(Some("openai/gpt-4")).is_ok());
        assert!(
            opencode().validate_model(Some("a/b/c")).is_ok(),
            "provider 'a', model 'b/c'"
        );
    }

    #[test]
    fn validate_model_rejects_empty_halves_with_the_pinned_message() {
        for bad in ["abc", "a/", "/b"] {
            let err = opencode()
                .validate_model(Some(bad))
                .expect_err("an invalid model must be rejected");
            assert_eq!(
                err.message(),
                format!(
                    "--model for the opencode backend must be \
                     '<provider-id>/<model-id>' (got '{bad}')."
                )
            );
        }
    }

    #[test]
    fn validate_effort_rejects_every_non_none_value() {
        assert!(opencode().validate_effort(None).is_ok());
        for level in ["low", "high", "max"] {
            let err = opencode()
                .validate_effort(Some(level))
                .expect_err("opencode has no reasoning-effort control");
            assert_eq!(err.message(), "opencode does not support reasoning effort.");
        }
    }

    #[test]
    fn spawn_argv_uses_the_prompt_flag_pair_and_ignores_display_name() {
        assert_eq!(
            opencode().build_spawn_argv("do it", "ignored", Some("openai/gpt-4"), None),
            argv(&[
                "opencode",
                "--agent",
                "cafleet",
                "--model",
                "openai/gpt-4",
                "--prompt",
                "do it",
            ]),
            "the prompt travels as a --prompt flag pair — TWO tokens"
        );
        assert_eq!(
            opencode().build_spawn_argv("do it", "ignored", None, None),
            argv(&["opencode", "--agent", "cafleet", "--prompt", "do it"])
        );
    }

    #[test]
    fn ensure_available_checks_the_path_then_the_preset() {
        let home = TempDir::new().unwrap();
        let err = opencode()
            .ensure_available(&FakeProbe::without_binaries(home.path()))
            .expect_err("a missing binary must be rejected first");
        assert_eq!(err.message(), "binary opencode not found on PATH");

        let probe = FakeProbe::with_binary("opencode", home.path());
        let preset = home.path().join(".opencode/agents/cafleet.md");
        let err = opencode()
            .ensure_available(&probe)
            .expect_err("a missing preset must be rejected");
        assert_eq!(
            err.message(),
            format!(
                "opencode agent preset not found at {}; \
                 run 'cafleet setup --coding-agent opencode' first",
                preset.display()
            )
        );

        std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
        std::fs::write(&preset, "---\n{}\n---\n\n# CAFleet member agent\n").unwrap();
        assert!(opencode().ensure_available(&probe).is_ok());
    }

    #[test]
    fn ensure_available_resolves_the_preset_through_the_config_dir_variable() {
        let home = TempDir::new().unwrap();
        let custom = home.path().join("oc-custom");
        let mut probe = FakeProbe::with_binary("opencode", home.path());
        probe.env.insert(
            "OPENCODE_CONFIG_DIR".to_string(),
            custom.display().to_string(),
        );

        let default_preset = home.path().join(".opencode/agents/cafleet.md");
        std::fs::create_dir_all(default_preset.parent().unwrap()).unwrap();
        std::fs::write(&default_preset, "x").unwrap();

        let custom_preset = custom.join("agents/cafleet.md");
        let err = opencode()
            .ensure_available(&probe)
            .expect_err("the default-path preset does not satisfy a relocated check");
        assert_eq!(
            err.message(),
            format!(
                "opencode agent preset not found at {}; \
                 run 'cafleet setup --coding-agent opencode' first",
                custom_preset.display()
            )
        );

        std::fs::create_dir_all(custom_preset.parent().unwrap()).unwrap();
        std::fs::write(&custom_preset, "x").unwrap();
        assert!(opencode().ensure_available(&probe).is_ok());
    }

    #[test]
    fn ensure_available_surfaces_an_invalid_config_dir_variable() {
        let home = TempDir::new().unwrap();
        let preset = home.path().join(".opencode/agents/cafleet.md");
        std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
        std::fs::write(&preset, "x").unwrap();

        let mut probe = FakeProbe::with_binary("opencode", home.path());
        probe
            .env
            .insert("OPENCODE_CONFIG_DIR".to_string(), "rel/path".to_string());
        let err = opencode()
            .ensure_available(&probe)
            .expect_err("validation precedes the existence check");
        assert_eq!(
            err.message(),
            "OPENCODE_CONFIG_DIR must be an absolute path (got 'rel/path')"
        );
    }
}
