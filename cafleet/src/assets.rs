//! Embedded skills/presets and the offline installer backing the assets half
//! of `cafleet setup` (SPEC §6.3): no network access — every artifact is
//! compiled into the binary. Install directories resolve per SPEC §6.3
//! *Config-dir resolution*.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::broker::record_asset_install;
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

/// Install one agent's skills and preset (where one exists) at its resolved
/// paths, then upsert its `(coding_agent, path)` row — the row attests
/// skills + preset. A failure aborts the caller's loop; rows recorded before
/// the failure remain.
pub fn install_agent(
    conn: &mut Connection,
    agent: &str,
    paths: &AgentPaths,
    version: &str,
) -> Result<(), CafleetError> {
    install_skills(&paths.skills_dir, agent, version)?;
    if let Some((source, target)) = &paths.preset {
        install_preset(source, target, agent, version)?;
    }
    record_asset_install(conn, agent, &paths.identity, version)
}

fn install_skills(skills_dir: &Path, agent: &str, version: &str) -> Result<(), CafleetError> {
    let fail = |e: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {e}",
            skills_dir.display()
        ))
    };
    std::fs::create_dir_all(skills_dir).map_err(fail)?;
    for skill in SKILL_NAMES {
        let target = skills_dir.join(skill);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(fail)?;
        }
        let prefix = format!("{skill}/");
        let files: Vec<_> = SKILLS
            .iter()
            .filter(|(path, _)| path.starts_with(&prefix))
            .collect();
        assert!(
            !files.is_empty(),
            "the embedded skills tree carries '{skill}'"
        );
        for (path, bytes) in files {
            let dest = skills_dir.join(path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(fail)?;
            }
            std::fs::write(&dest, bytes).map_err(fail)?;
        }
    }
    remove_existing_target(&skills_dir.join("cafleet-research")).map_err(fail)?;
    println!(
        "{agent}: installed cafleet, cafleet-design-doc (v{version}) -> {}",
        skills_dir.display()
    );
    Ok(())
}

fn remove_existing_target(target: &Path) -> std::io::Result<()> {
    // The symlink check comes first: a directory check follows symlinks and a
    // recursive delete refuses them.
    match std::fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::remove_file(target),
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(_) => Ok(()),
    }
}

fn install_preset(
    source: &'static str,
    target: &Path,
    agent: &str,
    version: &str,
) -> Result<(), CafleetError> {
    let fail = |e: std::io::Error| {
        CafleetError::App(format!(
            "failed to install preset into {}: {e}",
            target.display()
        ))
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    remove_existing_target(target).map_err(fail)?;
    let bytes = lookup(PRESETS, source)
        .unwrap_or_else(|| panic!("the embedded presets tree carries '{source}'"));
    std::fs::write(target, bytes).map_err(fail)?;
    println!(
        "{agent}: installed preset (v{version}) -> {}",
        target.display()
    );
    Ok(())
}
