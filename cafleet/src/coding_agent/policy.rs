use super::CodingAgent;
use crate::broker::runtime::ModeSource;
use crate::error::CafleetError;

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPolicySelection {
    pub requested_mode: Option<String>,
    pub resolved_mode: String,
    pub mode_source: ModeSource,
    pub permission_argv: Vec<String>,
    backend: CodingAgent,
    native_version: Option<String>,
}
impl CodingAgent {
    fn default_policy(self) -> &'static str {
        match self {
            Self::Claude => "dontAsk",
            Self::Codex => "never",
            Self::Opencode => "cafleet",
        }
    }
    pub fn permission_mode_help() -> String {
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
    pub fn policy_choices(self, native_version: Option<&str>) -> Vec<String> {
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
            (Self::Codex, None | Some("0.163.0")) => &["on-request", "never"],
            (Self::Opencode, _) => &["cafleet", "cafleet-ask"],
            _ => &[],
        };
        choices.iter().map(|value| (*value).into()).collect()
    }
    fn permission_arguments(self, mode: &str) -> Vec<String> {
        match self {
            Self::Claude => vec!["--permission-mode".into(), mode.into()],
            Self::Codex => vec![
                "--ask-for-approval".into(),
                mode.into(),
                "--sandbox".into(),
                "workspace-write".into(),
            ],
            Self::Opencode => vec!["--agent".into(), mode.into()],
        }
    }
    pub fn resolve_policy(
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
    pub fn build_spawn_argv_with_policy(
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
        let mut argv = vec![self.binary_name().into()];
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
