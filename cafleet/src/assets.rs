//! Embedded skills/presets and the offline installer backing the assets half
//! of `cafleet setup` (SPEC §6.3, amendment A3): no network access — every
//! artifact is compiled into the binary.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use rusqlite::Connection;

use crate::broker::record_asset_install;
use crate::error::CafleetError;

pub static SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../skills");

pub static PRESETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../presets");

const SKILL_NAMES: [&str; 3] = ["cafleet", "cafleet-design-doc", "cafleet-research"];
const TARGET_AGENTS: [&str; 3] = ["claude", "codex", "opencode"];

fn skills_dir(home: &Path, agent: &str) -> PathBuf {
    match agent {
        "claude" => home.join(".claude/skills"),
        "codex" => home.join(".codex/skills"),
        _ => home.join(".config/opencode/skills"),
    }
}

fn preset(home: &Path, agent: &str) -> Option<(&'static str, PathBuf)> {
    match agent {
        "codex" => Some((
            "codex/cafleet.rules",
            home.join(".codex/rules/cafleet.rules"),
        )),
        "opencode" => Some((
            "opencode/cafleet.md",
            home.join(".opencode/agents/cafleet.md"),
        )),
        _ => None,
    }
}

/// The assets half: per non-skipped agent, delete-and-reinstall the three
/// embedded skills and the agent's bundled preset (where one exists), then
/// record the `asset_installs` row. An install failure aborts the loop; rows
/// recorded before the failure remain.
pub fn install_assets(
    conn: &mut Connection,
    skip: &BTreeSet<String>,
    version: &str,
    home: &Path,
) -> Result<(), CafleetError> {
    if !crate::broker::asset_installs_table_exists(conn) {
        return Err(CafleetError::App(
            "the database schema is missing or outdated; run 'cafleet setup' first".to_string(),
        ));
    }
    for agent in TARGET_AGENTS {
        if skip.contains(agent) {
            continue;
        }
        install_skills(home, agent, version)?;
        install_preset(home, agent, version)?;
        record_asset_install(conn, agent, version)?;
    }
    Ok(())
}

fn install_skills(home: &Path, agent: &str, version: &str) -> Result<(), CafleetError> {
    let skills_dir = skills_dir(home, agent);
    let fail = |e: std::io::Error| {
        CafleetError::App(format!(
            "failed to install skills into {}: {e}",
            skills_dir.display()
        ))
    };
    std::fs::create_dir_all(&skills_dir).map_err(fail)?;
    for skill in SKILL_NAMES {
        let target = skills_dir.join(skill);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(fail)?;
        }
        // Entry paths carry the `<skill>/` prefix, so extraction is rooted at
        // the skills dir; the skill's own directory must exist first.
        std::fs::create_dir_all(&target).map_err(fail)?;
        let embedded = SKILLS
            .get_dir(skill)
            .unwrap_or_else(|| panic!("the embedded skills tree carries '{skill}'"));
        embedded.extract(&skills_dir).map_err(fail)?;
    }
    println!(
        "{agent}: installed cafleet, cafleet-design-doc, cafleet-research (v{version}) -> {}",
        skills_dir.display()
    );
    Ok(())
}

fn install_preset(home: &Path, agent: &str, version: &str) -> Result<(), CafleetError> {
    let Some((source, target)) = preset(home, agent) else {
        return Ok(());
    };
    let fail = |e: std::io::Error| {
        CafleetError::App(format!(
            "failed to install preset into {}: {e}",
            target.display()
        ))
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    // The symlink check comes first: a directory check follows symlinks and a
    // recursive delete refuses them.
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::remove_file(&target).map_err(fail)?;
        }
        Ok(metadata) if metadata.is_dir() => {
            std::fs::remove_dir_all(&target).map_err(fail)?;
        }
        Ok(_) => {
            std::fs::remove_file(&target).map_err(fail)?;
        }
        Err(_) => {}
    }
    let embedded = PRESETS
        .get_file(source)
        .unwrap_or_else(|| panic!("the embedded presets tree carries '{source}'"));
    std::fs::write(&target, embedded.contents()).map_err(fail)?;
    println!(
        "{agent}: installed preset (v{version}) -> {}",
        target.display()
    );
    Ok(())
}
