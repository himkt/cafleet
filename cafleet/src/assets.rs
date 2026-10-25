use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::coding_agent::CodingAgent;
use crate::config_dir::{EnvLookup, opencode_skills_base};
use crate::embedded::{PRESETS, SKILLS, lookup};
use crate::error::CafleetError;

pub struct AgentPaths {
    pub identity: String,
    skills_dir: PathBuf,
    preset: Option<(&'static str, PathBuf)>,
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
    let preset = match agent {
        CodingAgent::Claude => None,
        CodingAgent::Codex => Some(("codex/cafleet.rules", base.join("rules/cafleet.rules"))),
        CodingAgent::Opencode => Some(("opencode/cafleet.md", base.join("agents/cafleet.md"))),
    };
    Ok(AgentPaths {
        identity: base.display().to_string(),
        skills_dir,
        preset,
    })
}

pub fn install_agent(
    conn: &mut Connection,
    agent: CodingAgent,
    paths: &AgentPaths,
    version: &str,
) -> Result<(), CafleetError> {
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
    if let Some((source, target)) = &paths.preset {
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
    }
    crate::broker::record_asset_install(conn, agent.name(), &paths.identity, version)?;
    println!(
        "{}: installed {} (v{version}) -> {}",
        agent.name(),
        roots.join(", "),
        paths.skills_dir.display()
    );
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

    #[test]
    fn installed_opencode_preset_preserves_routine_permissions_and_dispatch_denials() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let paths = agent_paths(&|_| None, home.path(), CodingAgent::Opencode).unwrap();
        install_agent(&mut conn, CodingAgent::Opencode, &paths, "test-version").unwrap();
        let (_, target) = paths.preset.unwrap();
        let text = std::fs::read_to_string(target).unwrap();
        let (frontmatter, _) = text
            .strip_prefix("---\n")
            .unwrap()
            .split_once("\n---\n")
            .unwrap();
        let trailing_commas = regex::Regex::new(r",(\s*[}\]])").unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&trailing_commas.replace_all(frontmatter, "$1")).unwrap();
        assert_eq!(
            parsed["permission"],
            serde_json::json!({
                "shell": {
                    "*": "deny",
                    "git add *": "allow",
                    "git commit *": "allow",
                    "git diff *": "allow",
                    "git grep *": "allow",
                    "git log *": "allow",
                    "git ls-tree *": "allow",
                    "git ls-files *": "allow",
                    "git branch *": "allow",
                    "git status": "allow",
                    "grep *": "allow",
                    "ls": "allow",
                    "ls *": "allow",
                    "stat *": "allow",
                    "tree": "allow",
                    "tree *": "allow",
                    "mise //cafleet:test": "allow",
                    "mise //cafleet:test *": "allow",
                    "mise //cafleet:lint": "allow",
                    "mise //cafleet:format": "allow",
                    "mise //cafleet:typecheck": "allow",
                    "mise //cafleet:build": "allow",
                    "wc *": "allow",
                    "cafleet *": "allow",
                    "cafleet member prompt *": "deny",
                    "cafleet member exec *": "deny"
                },
                "read": {
                    "*": "allow",
                    "**/.env": "deny",
                    "**/.env.*": "deny"
                },
                "edit": {
                    "*": "allow",
                    "**/.env": "deny",
                    "**/.env.*": "deny"
                },
                "external_directory": "deny",
                "webfetch": "deny",
                "websearch": "deny",
                "repo_clone": "deny",
                "question": "deny",
                "plan_enter": "deny",
                "plan_exit": "deny"
            })
        );
        let broad_allow = frontmatter.find("\"cafleet *\"").unwrap();
        for dispatch in ["cafleet member prompt *", "cafleet member exec *"] {
            let dispatch_deny = frontmatter.find(&format!("\"{dispatch}\"")).unwrap();
            assert!(
                broad_allow < dispatch_deny,
                "specific dispatch denial follows the broader CAFleet allow: {dispatch}"
            );
        }
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
            std::fs::create_dir_all(&paths.skills_dir).unwrap();
            symlink(&external, paths.skills_dir.join("cafleet")).unwrap();
            if let Some((_, target)) = &paths.preset {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                symlink(external.join("keep"), target).unwrap();
            }
            for _ in 0..2 {
                install_agent(&mut conn, *agent, &paths, "test-version").unwrap();
                for (relative, bytes) in SKILLS {
                    assert_eq!(
                        std::fs::read(paths.skills_dir.join(relative)).unwrap(),
                        *bytes
                    );
                }
                if let Some((source, target)) = &paths.preset {
                    assert_eq!(
                        std::fs::read(target).unwrap(),
                        lookup(PRESETS, source).unwrap()
                    );
                }
                assert_eq!(
                    std::fs::read(external.join("keep")).unwrap(),
                    b"operator bytes"
                );
            }
            if *agent == CodingAgent::Opencode {
                assert_eq!(
                    paths.skills_dir,
                    home.path().join(".config/opencode/skills")
                );
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
        assert!(install_agent(&mut conn, CodingAgent::Codex, &paths, "version").is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM asset_installs", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
