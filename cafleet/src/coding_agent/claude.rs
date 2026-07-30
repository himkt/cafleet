//! claude backend (SPEC §6.7): pass-through model validation, the five-level
//! effort enum, and the only backend that honors `display_name`. The colocated
//! tests pin the contract; see [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::coding_agent::coding_agent;
    use crate::coding_agent::test_support::{FakeProbe, argv};

    fn claude() -> &'static dyn crate::coding_agent::CodingAgent {
        coding_agent("claude").unwrap()
    }

    #[test]
    fn names_are_pinned() {
        assert_eq!(claude().name(), "claude");
        assert_eq!(claude().binary_name(), "claude");
    }

    #[test]
    fn validate_model_is_pass_through() {
        assert!(claude().validate_model(None).is_ok());
        assert!(claude().validate_model(Some("opus")).is_ok());
        assert!(claude().validate_model(Some("anything goes")).is_ok());
    }

    #[test]
    fn validate_effort_accepts_the_five_levels_and_none() {
        for level in ["low", "medium", "high", "xhigh", "max"] {
            assert!(claude().validate_effort(Some(level)).is_ok(), "{level}");
        }
        assert!(claude().validate_effort(None).is_ok());
    }

    #[test]
    fn validate_effort_rejects_an_unknown_level_with_the_pinned_message() {
        let err = claude()
            .validate_effort(Some("turbo"))
            .expect_err("an unknown level must be rejected");
        assert_eq!(
            err.message(),
            "--effort for the claude backend must be one of low, medium, high, \
             xhigh, max (got 'turbo')."
        );
    }

    #[test]
    fn spawn_argv_carries_the_display_name_and_optional_tokens() {
        assert_eq!(
            claude().build_spawn_argv("do it", "worker", Some("opus"), Some("high")),
            argv(&[
                "claude",
                "--permission-mode",
                "dontAsk",
                "--name",
                "worker",
                "--model",
                "opus",
                "--effort",
                "high",
                "do it",
            ])
        );
    }

    #[test]
    fn spawn_argv_omits_all_tokens_for_none_model_and_effort() {
        assert_eq!(
            claude().build_spawn_argv("do it", "worker", None, None),
            argv(&[
                "claude",
                "--permission-mode",
                "dontAsk",
                "--name",
                "worker",
                "do it",
            ]),
            "no --model/--effort tokens at all — never an empty value"
        );
    }

    #[test]
    fn ensure_available_is_a_path_check_only() {
        let home = TempDir::new().unwrap();
        let err = claude()
            .ensure_available(&FakeProbe::without_binaries(home.path()))
            .expect_err("a missing binary must be rejected");
        assert_eq!(err.message(), "binary claude not found on PATH");

        assert!(
            claude()
                .ensure_available(&FakeProbe::with_binary("claude", home.path()))
                .is_ok(),
            "no preset precondition for claude"
        );
    }
}
