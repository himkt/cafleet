//! Embedded skills/presets and the offline installer backing the assets half
//! of `cafleet setup` (SPEC §6.3): no network access — every artifact is
//! compiled into the binary. Install directories resolve per SPEC §6.3
//! *Config-dir resolution*.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::config_dir::{
    EnvLookup, claude_config_dir, codex_home, opencode_preset_base, opencode_skills_base,
};
use crate::embedded::{PRESETS, SKILLS, lookup};
use crate::error::CafleetError;

const SKILL_NAMES: [&str; 2] = ["cafleet", "cafleet-design-doc"];
pub const TARGET_AGENTS: [&str; 3] = ["claude", "codex", "opencode"];

/// An agent's resolved install locations plus its recorded-path identity
/// (SPEC §6.3 *Config-dir resolution*).
pub struct AgentPaths {
    pub identity: String,
    skills_dir: PathBuf,
    preset: Option<(&'static str, PathBuf)>,
}

pub fn agent_paths(env: EnvLookup, home: &Path, agent: &str) -> Result<AgentPaths, CafleetError> {
    match agent {
        "claude" => {
            let base = claude_config_dir(env, home)?.path;
            Ok(AgentPaths {
                identity: base.display().to_string(),
                skills_dir: base.join("skills"),
                preset: None,
            })
        }
        "codex" => {
            let base = codex_home(env, home)?.path;
            Ok(AgentPaths {
                identity: base.display().to_string(),
                skills_dir: base.join("skills"),
                preset: Some(("codex/cafleet.rules", base.join("rules/cafleet.rules"))),
            })
        }
        "opencode" => {
            let preset_base = opencode_preset_base(env, home)?.path;
            Ok(AgentPaths {
                identity: preset_base.display().to_string(),
                skills_dir: opencode_skills_base(home).join("skills"),
                preset: Some(("opencode/cafleet.md", preset_base.join("agents/cafleet.md"))),
            })
        }
        other => unreachable!("'{other}' is outside the clap-validated agent choice set"),
    }
}

pub fn install_agent(
    conn: &mut Connection,
    agent: &str,
    paths: &AgentPaths,
    version: &str,
) -> Result<(), CafleetError> {
    let fail = |error: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {error}",
            paths.skills_dir.display()
        ))
    };
    for skill in SKILL_NAMES {
        let prefix = format!("{skill}/");
        assert!(
            SKILLS
                .iter()
                .any(|(path, _)| *path == format!("{prefix}SKILL.md"))
        );
        remove_existing_target(&paths.skills_dir.join(skill)).map_err(fail)?;
        for (path, bytes) in SKILLS.iter().filter(|(path, _)| path.starts_with(&prefix)) {
            let target = paths.skills_dir.join(path);
            std::fs::create_dir_all(target.parent().expect("skill file has a parent"))
                .map_err(fail)?;
            std::fs::write(target, bytes).map_err(fail)?;
        }
    }
    remove_existing_target(&paths.skills_dir.join("cafleet-research")).map_err(fail)?;
    println!(
        "{agent}: installed cafleet, cafleet-design-doc (v{version}) -> {}",
        paths.skills_dir.display()
    );
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
        println!(
            "{agent}: installed preset (v{version}) -> {}",
            target.display()
        );
    }
    crate::broker::record_asset_install(conn, agent, &paths.identity, version)
}

fn remove_existing_target(target: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
