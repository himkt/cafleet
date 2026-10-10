//! Member-permission facts (SPEC §6.3 *doctor*): the Claude Code settings
//! that would stop a claude member from running its broker commands. CLI
//! policy and display belong to callers.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

/// One invocation of every broker command a claude member is spawned allowed
/// to run; a setting blocks a member when it blocks any of these.
pub(crate) const PROBE_COMMANDS: [&str; 7] = [
    "cafleet message send --from-member-id 1 --to-member-id 2 x",
    "cafleet message broadcast --from-member-id 1 x",
    "cafleet message poll 1",
    "cafleet message ack 1",
    "cafleet message show 1",
    "cafleet monitor scan 1",
    "cafleet member ping 1",
];

const MANAGED_SETTINGS_PATH: &str = if cfg!(target_os = "macos") {
    "/Library/Application Support/ClaudeCode/managed-settings.json"
} else {
    "/etc/claude-code/managed-settings.json"
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsFile {
    pub(crate) path: PathBuf,
    pub(crate) managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FindingKind {
    /// A `permissions.deny` or `permissions.ask` rule matches `command`.
    BlockingRule {
        list: &'static str,
        rule: String,
        command: &'static str,
    },
    /// The managed file allows managed rules only, and its own
    /// `permissions.allow` leaves `command` unmatched.
    ManagedRulesOnly { command: &'static str },
    /// The file exists but does not parse as JSON.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) file: PathBuf,
    pub(crate) kind: FindingKind,
}

/// The settings files Claude Code reads for a member spawned in `cwd`: user,
/// project, project-local, then managed.
pub(crate) fn claude_settings_files(config_dir: &Path, cwd: &Path) -> Vec<SettingsFile> {
    let user_or_project = |path: PathBuf| SettingsFile {
        path,
        managed: false,
    };
    vec![
        user_or_project(config_dir.join("settings.json")),
        user_or_project(cwd.join(".claude").join("settings.json")),
        user_or_project(cwd.join(".claude").join("settings.local.json")),
        SettingsFile {
            path: PathBuf::from(MANAGED_SETTINGS_PATH),
            managed: true,
        },
    ]
}

/// Every finding across `files`, in file order. A missing file is expected
/// and yields nothing.
pub(crate) fn diagnose(files: &[SettingsFile]) -> Vec<Finding> {
    files.iter().flat_map(diagnose_file).collect()
}

fn diagnose_file(file: &SettingsFile) -> Vec<Finding> {
    let finding = |kind| Finding {
        file: file.path.clone(),
        kind,
    };
    let text = match std::fs::read_to_string(&file.path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(_) => return vec![finding(FindingKind::Unreadable)],
    };
    let Ok(settings) = serde_json::from_str::<Value>(&text) else {
        return vec![finding(FindingKind::Unreadable)];
    };

    let mut findings = Vec::new();
    for list in ["deny", "ask"] {
        for rule in rules(&settings, list) {
            if let Some(command) = PROBE_COMMANDS
                .into_iter()
                .find(|probe| rule_matches(rule, probe))
            {
                findings.push(finding(FindingKind::BlockingRule {
                    list,
                    rule: rule.to_string(),
                    command,
                }));
            }
        }
    }
    if file.managed && settings["allowManagedPermissionRulesOnly"] == Value::Bool(true) {
        let allow = rules(&settings, "allow");
        if let Some(command) = PROBE_COMMANDS
            .into_iter()
            .find(|probe| !allow.iter().any(|rule| rule_matches(rule, probe)))
        {
            findings.push(finding(FindingKind::ManagedRulesOnly { command }));
        }
    }
    findings
}

/// The string rules of `permissions.<list>`; a settings file is user-written,
/// so an absent list or a non-string entry carries no rule.
fn rules<'a>(settings: &'a Value, list: &str) -> Vec<&'a str> {
    settings["permissions"][list]
        .as_array()
        .map(|entries| entries.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// A rule matches a probe when it is the bare `Bash`, or `Bash(<pattern>)`
/// whose pattern matches the probe.
fn rule_matches(rule: &str, probe: &str) -> bool {
    if rule == "Bash" {
        return true;
    }
    rule.strip_prefix("Bash(")
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|pattern| pattern_matches(pattern, probe))
}

/// `*` matches any text; a trailing ` *` or the legacy `:*` also matches the
/// bare command.
fn pattern_matches(pattern: &str, probe: &str) -> bool {
    let pattern = match pattern.strip_suffix(":*") {
        Some(command) => format!("{command} *"),
        None => pattern.to_string(),
    };
    glob_matches(&pattern, probe)
        || pattern
            .strip_suffix(" *")
            .is_some_and(|bare| glob_matches(bare, probe))
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let literal_parts: Vec<String> = pattern.split('*').map(regex::escape).collect();
    Regex::new(&format!("(?s)^{}$", literal_parts.join(".*")))
        .expect("escaped literals joined by wildcards form a valid pattern")
        .is_match(text)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const SEND: &str = "cafleet message send --from-member-id 1 --to-member-id 2 x";

    fn settings_file(dir: &TempDir, name: &str, contents: &str, managed: bool) -> SettingsFile {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        SettingsFile { path, managed }
    }

    fn managed(dir: &TempDir, contents: &str) -> SettingsFile {
        settings_file(dir, "managed-settings.json", contents, true)
    }

    fn blocking(
        file: &SettingsFile,
        list: &'static str,
        rule: &str,
        command: &'static str,
    ) -> Finding {
        Finding {
            file: file.path.clone(),
            kind: FindingKind::BlockingRule {
                list,
                rule: rule.to_string(),
                command,
            },
        }
    }

    #[test]
    fn settings_that_leave_the_broker_commands_alone_yield_no_finding() {
        let dir = TempDir::new().unwrap();
        let missing = SettingsFile {
            path: dir.path().join("absent.json"),
            managed: false,
        };
        let user = settings_file(
            &dir,
            "settings.json",
            r#"{"permissions": {
                "allow": ["Bash(cafleet *)"],
                "deny": ["Bash(rm *)", "Read(./secrets/**)", "Bash(cafleet fleet delete *)"],
                "ask": ["Bash(git push *)", "Bash(cafleet member exec *)"]
            }}"#,
            false,
        );
        let managed = managed(&dir, r#"{"permissions": {"allow": []}}"#);

        assert_eq!(diagnose(&[missing, user, managed]), []);
    }

    #[test]
    fn deny_and_ask_rules_are_each_reported_with_the_first_probe_they_match() {
        let dir = TempDir::new().unwrap();
        let user = settings_file(
            &dir,
            "settings.json",
            r#"{"permissions": {
                "ask": ["Bash(cafleet member ping *)"],
                "deny": ["Bash(cafleet *)", "Bash(cafleet message ack:*)"]
            }}"#,
            false,
        );
        let project = settings_file(
            &dir,
            "project-settings.json",
            r#"{"permissions": {"ask": ["Bash(cafleet message poll 1 *)"]}}"#,
            false,
        );

        assert_eq!(
            diagnose(&[user.clone(), project.clone()]),
            [
                blocking(&user, "deny", "Bash(cafleet *)", SEND),
                blocking(
                    &user,
                    "deny",
                    "Bash(cafleet message ack:*)",
                    "cafleet message ack 1"
                ),
                blocking(
                    &user,
                    "ask",
                    "Bash(cafleet member ping *)",
                    "cafleet member ping 1"
                ),
                blocking(
                    &project,
                    "ask",
                    "Bash(cafleet message poll 1 *)",
                    "cafleet message poll 1"
                ),
            ]
        );
    }

    #[test]
    fn a_bare_bash_rule_matches_every_probe() {
        for list in ["deny", "ask"] {
            let dir = TempDir::new().unwrap();
            let user = settings_file(
                &dir,
                "settings.json",
                &format!(r#"{{"permissions": {{"{list}": ["Bash"]}}}}"#),
                false,
            );

            assert_eq!(
                diagnose(std::slice::from_ref(&user)),
                [blocking(&user, list, "Bash", SEND)]
            );
        }
    }

    #[test]
    fn managed_rules_only_reports_the_first_probe_its_allow_list_leaves_unmatched() {
        for (settings, unmatched) in [
            (r#"{"allowManagedPermissionRulesOnly": true}"#, SEND),
            (
                r#"{"allowManagedPermissionRulesOnly": true,
                    "permissions": {"allow": ["Bash(cafleet message *)"]}}"#,
                "cafleet monitor scan 1",
            ),
        ] {
            let dir = TempDir::new().unwrap();
            let managed = managed(&dir, settings);

            assert_eq!(
                diagnose(std::slice::from_ref(&managed)),
                [Finding {
                    file: managed.path.clone(),
                    kind: FindingKind::ManagedRulesOnly { command: unmatched },
                }],
                "{settings}"
            );
        }
    }

    #[test]
    fn managed_rules_only_with_a_covering_allow_list_yields_no_finding() {
        for allow in [
            r#"["Bash(cafleet *)"]"#,
            r#"["Bash(cafleet message *)", "Bash(cafleet monitor scan *)",
                "Bash(cafleet member ping:*)"]"#,
        ] {
            let dir = TempDir::new().unwrap();
            let managed = managed(
                &dir,
                &format!(
                    r#"{{"allowManagedPermissionRulesOnly": true,
                        "permissions": {{"allow": {allow}}}}}"#
                ),
            );

            assert_eq!(diagnose(&[managed]), [], "{allow}");
        }
    }

    #[test]
    fn a_settings_file_that_is_not_json_is_unreadable() {
        let dir = TempDir::new().unwrap();
        let user = settings_file(&dir, "settings.json", r#"{"permissions": "#, false);

        assert_eq!(
            diagnose(std::slice::from_ref(&user)),
            [Finding {
                file: user.path.clone(),
                kind: FindingKind::Unreadable,
            }]
        );
    }
}
