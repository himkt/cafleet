use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::coding_agent::CodingAgent;
use crate::config_dir::{EnvLookup, opencode_skills_base};
use crate::embedded::{PRESETS, SKILLS, lookup};
use crate::error::CafleetError;

pub struct AgentPaths {
    pub identity: String,
    skills_dir: PathBuf,
    presets: Vec<(String, PathBuf)>,
}

pub fn agent_paths(
    env: EnvLookup<'_>,
    home: &Path,
    agent: CodingAgent,
) -> Result<AgentPaths, CafleetError> {
    let base = agent.config_dir(env, home)?.path;
    let skills_dir = match agent {
        CodingAgent::Opencode => opencode_skills_base(home).join("skills"),
        _ => base.join("skills"),
    };
    let presets = match agent {
        CodingAgent::Claude => vec![],
        CodingAgent::Codex => vec![(
            "codex/cafleet.rules".into(),
            base.join("rules/cafleet.rules"),
        )],
        CodingAgent::Opencode => agent
            .policy_choices(None)
            .into_iter()
            .map(|mode| {
                let target = agent
                    .required_preset(&base, &mode)
                    .expect("OpenCode requires a preset for every launch policy");
                (format!("opencode/{mode}.md"), target)
            })
            .collect(),
    };
    Ok(AgentPaths {
        identity: base.display().to_string(),
        skills_dir,
        presets,
    })
}

pub fn install_agent(
    conn: &mut Connection,
    agent: CodingAgent,
    paths: &AgentPaths,
    version: &str,
    out: &mut dyn Write,
) -> Result<(), CafleetError> {
    let mut report = |line: String| {
        writeln!(out, "{line}")
            .map_err(|error| CafleetError::App(format!("failed to write setup output: {error}")))
    };
    let fail = |error: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {error}",
            paths.skills_dir.display()
        ))
    };
    let roots = SKILLS
        .iter()
        .filter_map(|(path, _)| {
            path.strip_suffix("/SKILL.md")
                .filter(|root| !root.contains('/'))
        })
        .collect::<Vec<_>>();
    assert!(
        !roots.is_empty(),
        "embedded skills have top-level manifests"
    );
    for skill in &roots {
        let prefix = format!("{skill}/");
        remove_existing_target(&paths.skills_dir.join(skill)).map_err(fail)?;
        for (path, bytes) in SKILLS.iter().filter(|(path, _)| path.starts_with(&prefix)) {
            let target = paths.skills_dir.join(path);
            std::fs::create_dir_all(target.parent().expect("skill file has a parent"))
                .map_err(fail)?;
            std::fs::write(target, bytes).map_err(fail)?;
        }
    }
    report(format!(
        "{}: installed {} (v{version}) -> {}",
        agent.name(),
        roots.join(", "),
        paths.skills_dir.display()
    ))?;
    for (source, target) in &paths.presets {
        let fail = |error: std::io::Error| {
            CafleetError::App(format!(
                "failed to install preset into {}: {error}",
                target.display()
            ))
        };
        let bytes = lookup(PRESETS, source).expect("embedded preset exists");
        std::fs::create_dir_all(target.parent().expect("preset has a parent")).map_err(fail)?;
        remove_existing_target(target).map_err(fail)?;
        std::fs::write(target, bytes).map_err(fail)?;
        report(format!(
            "{}: installed preset (v{version}) -> {}",
            agent.name(),
            target.display()
        ))?;
    }
    crate::broker::record_asset_install(conn, agent.name(), &paths.identity, version)?;
    Ok(())
}

fn remove_existing_target(target: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn expected_presets(agent: CodingAgent, base: &Path) -> Vec<(&'static str, PathBuf)> {
        match agent {
            CodingAgent::Claude => vec![],
            CodingAgent::Codex => vec![("codex/cafleet.rules", base.join("rules/cafleet.rules"))],
            CodingAgent::Opencode => vec![
                ("opencode/cafleet.md", base.join("agents/cafleet.md")),
                (
                    "opencode/cafleet-ask.md",
                    base.join("agents/cafleet-ask.md"),
                ),
            ],
        }
    }

    fn expected_skills_dir(agent: CodingAgent, base: &Path, home: &Path) -> PathBuf {
        match agent {
            CodingAgent::Opencode => home.join(".config/opencode/skills"),
            _ => base.join("skills"),
        }
    }

    fn output_lines(output: Vec<u8>) -> Vec<String> {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn asset_install_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM asset_installs", [], |row| row.get(0))
            .unwrap()
    }

    fn frontmatter_and_body(preset: &str) -> (serde_json::Value, String) {
        let text = std::str::from_utf8(lookup(PRESETS, preset).expect("embedded preset"))
            .unwrap()
            .to_owned();
        let rest = text
            .strip_prefix("---\n")
            .expect("frontmatter opens the preset");
        let (frontmatter, body) = rest
            .split_once("\n---\n")
            .expect("frontmatter closes before the body");
        (serde_json::from_str(frontmatter).unwrap(), body.to_owned())
    }

    fn paragraphs(body: &str) -> Vec<String> {
        body.split("\n\n")
            .map(str::trim)
            .filter(|paragraph| !paragraph.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn installation_replaces_owned_symlinks_and_embeds_bytes_at_selected_paths() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let external = home.path().join("operator");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("keep"), "operator bytes").unwrap();
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| (name == agent.config_variable()).then(|| custom.display().to_string()),
                home.path(),
                *agent,
            )
            .unwrap();
            let skills_dir = expected_skills_dir(*agent, &custom, home.path());
            let presets = expected_presets(*agent, &custom);
            std::fs::create_dir_all(&skills_dir).unwrap();
            symlink(&external, skills_dir.join("cafleet")).unwrap();
            for (_, target) in &presets {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                symlink(external.join("keep"), target).unwrap();
            }
            for _ in 0..2 {
                install_agent(
                    &mut conn,
                    *agent,
                    &paths,
                    "test-version",
                    &mut Vec::<u8>::new(),
                )
                .unwrap();
                for (relative, bytes) in SKILLS {
                    assert_eq!(std::fs::read(skills_dir.join(relative)).unwrap(), *bytes);
                }
                for (source, target) in &presets {
                    assert_eq!(
                        std::fs::read(target).unwrap(),
                        lookup(PRESETS, source).unwrap(),
                        "{}",
                        target.display()
                    );
                }
                assert_eq!(
                    std::fs::read(external.join("keep")).unwrap(),
                    b"operator bytes"
                );
            }
            if *agent == CodingAgent::Opencode {
                assert!(!custom.join("skills").exists());
            }
            let record: String = conn
                .query_row(
                    "SELECT cafleet_version FROM asset_installs WHERE coding_agent=?1 AND path=?2",
                    rusqlite::params![agent.name(), paths.identity],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(record, "test-version");
        }
    }

    #[test]
    fn installation_failure_records_only_completed_installations() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let paths = agent_paths(&|_| None, home.path(), CodingAgent::Codex).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(home.path().join(".codex/rules"), "blocked parent").unwrap();
        assert!(
            install_agent(
                &mut conn,
                CodingAgent::Codex,
                &paths,
                "version",
                &mut Vec::<u8>::new()
            )
            .is_err()
        );
        assert_eq!(asset_install_count(&conn), 0);
    }

    #[test]
    fn installation_prints_the_skills_line_then_one_preset_line_per_target_in_install_order() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        for agent in CodingAgent::all() {
            let custom = home.path().join(format!("custom-{}", agent.name()));
            let paths = agent_paths(
                &|name| (name == agent.config_variable()).then(|| custom.display().to_string()),
                home.path(),
                *agent,
            )
            .unwrap();
            let mut output = Vec::new();
            install_agent(&mut conn, *agent, &paths, "9.8.7", &mut output).unwrap();
            let lines = output_lines(output);
            let presets = expected_presets(*agent, &custom);
            assert_eq!(lines.len(), 1 + presets.len(), "{lines:?}");
            let skills_suffix = format!(
                " (v9.8.7) -> {}",
                expected_skills_dir(*agent, &custom, home.path()).display()
            );
            assert!(
                lines[0].starts_with(&format!("{}: installed ", agent.name()))
                    && !lines[0].contains("installed preset")
                    && lines[0].ends_with(&skills_suffix),
                "skills line first: {lines:?}"
            );
            for ((_, target), line) in presets.iter().zip(&lines[1..]) {
                assert_eq!(
                    *line,
                    format!(
                        "{}: installed preset (v9.8.7) -> {}",
                        agent.name(),
                        target.display()
                    )
                );
            }
        }
    }

    #[test]
    fn opencode_installation_prints_both_preset_targets_after_the_skills_line() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let base = home.path().join("preset-base");
        let paths = agent_paths(
            &|name| (name == "OPENCODE_CONFIG_DIR").then(|| base.display().to_string()),
            home.path(),
            CodingAgent::Opencode,
        )
        .unwrap();
        let mut output = Vec::new();
        install_agent(
            &mut conn,
            CodingAgent::Opencode,
            &paths,
            "1.0.0",
            &mut output,
        )
        .unwrap();
        let lines = output_lines(output);
        assert_eq!(
            lines[1..],
            [
                format!(
                    "opencode: installed preset (v1.0.0) -> {}",
                    base.join("agents/cafleet.md").display()
                ),
                format!(
                    "opencode: installed preset (v1.0.0) -> {}",
                    base.join("agents/cafleet-ask.md").display()
                ),
            ]
        );
        assert_eq!(asset_install_count(&conn), 1);
    }

    #[test]
    fn opencode_preset_failure_fails_installation_and_records_nothing() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let base = home.path().join("preset-base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("agents"), "blocked parent").unwrap();
        let paths = agent_paths(
            &|name| (name == "OPENCODE_CONFIG_DIR").then(|| base.display().to_string()),
            home.path(),
            CodingAgent::Opencode,
        )
        .unwrap();
        let mut output = Vec::new();
        let failure = install_agent(
            &mut conn,
            CodingAgent::Opencode,
            &paths,
            "1.0.0",
            &mut output,
        )
        .expect_err("a blocked preset target fails installation");
        assert!(
            failure.message().starts_with(&format!(
                "failed to install preset into {}: ",
                base.join("agents/cafleet.md").display()
            )),
            "{}",
            failure.message()
        );
        assert_eq!(asset_install_count(&conn), 0);
        let lines = output_lines(output);
        assert_eq!(lines.len(), 1, "only the completed skills line: {lines:?}");
        assert!(
            lines[0].starts_with("opencode: installed ")
                && lines[0].ends_with(&format!(
                    "-> {}",
                    home.path().join(".config/opencode/skills").display()
                )),
            "{lines:?}"
        );
        assert!(!base.join("agents/cafleet-ask.md").exists());
    }

    #[test]
    fn embedded_presets_include_both_opencode_agents() {
        assert!(lookup(PRESETS, "opencode/cafleet.md").is_some());
        assert!(lookup(PRESETS, "opencode/cafleet-ask.md").is_some());
    }

    #[test]
    fn cafleet_ask_preset_frontmatter_differs_only_in_description_and_shell_base() {
        let (default, _) = frontmatter_and_body("opencode/cafleet.md");
        let (ask, _) = frontmatter_and_body("opencode/cafleet-ask.md");
        assert_eq!(
            ask["description"],
            "CAFleet-spawned member whose unmatched shell commands open a native permission form for the Director to answer."
        );
        assert_eq!(ask["permission"]["shell"]["*"], "ask");
        assert_eq!(default["permission"]["shell"]["*"], "deny");
        let key_order = |value: &serde_json::Value| {
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        let shell_keys = key_order(&ask["permission"]["shell"]);
        assert_eq!(shell_keys, key_order(&default["permission"]["shell"]));
        assert_eq!(shell_keys.first().map(String::as_str), Some("*"));
        let director_only = [
            "cafleet member prompt *",
            "cafleet member exec *",
            "cafleet member press *",
            "cafleet member recover *",
            "cafleet permission respond *",
            "cafleet input reconcile *",
        ];
        assert_eq!(
            shell_keys[shell_keys.len() - director_only.len()..],
            director_only
        );
        for key in director_only {
            assert_eq!(ask["permission"]["shell"][key], "deny", "{key}");
        }
        assert_eq!(key_order(&ask), key_order(&default));
        assert_eq!(
            key_order(&ask["permission"]),
            key_order(&default["permission"])
        );
        let mut normalized = ask.clone();
        normalized["description"] = default["description"].clone();
        normalized["permission"]["shell"]["*"] = "deny".into();
        assert_eq!(normalized, default);
    }

    #[test]
    fn cafleet_ask_preset_body_replaces_only_the_permission_paragraph() {
        let (_, default) = frontmatter_and_body("opencode/cafleet.md");
        let (_, ask) = frontmatter_and_body("opencode/cafleet-ask.md");
        let default = paragraphs(&default);
        let ask = paragraphs(&ask);
        assert_eq!(ask.len(), default.len());
        let permission = default
            .iter()
            .position(|paragraph| paragraph.starts_with("You are a CAFleet member spawned"))
            .expect("the cafleet preset has a permission paragraph");
        assert_eq!(
            ask[permission],
            "You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks, denies the Director-owned CAFleet commands, and asks for every other command. An asked command opens a native permission form; CAFleet notifies the Director, who answers it, and you continue from the result OpenCode returns. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials."
        );
        for (index, (actual, expected)) in ask.iter().zip(&default).enumerate() {
            if index != permission {
                assert_eq!(actual, expected, "paragraph {index}");
            }
        }
    }
}
