use super::CodingAgent;
use crate::broker::runtime::ModeSource;
use crate::error::CafleetError;

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
        let invalid = || CafleetError::Value("capture emitted permission argv mismatch".into());
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

#[cfg(test)]
mod tests {
    use super::*;
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
