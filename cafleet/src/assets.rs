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
    pub(crate) skills_dir: PathBuf,
    pub(crate) preset: Option<(&'static str, PathBuf)>,
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

mod driver;
mod files;
mod journal;
mod locks;
mod types;
pub(crate) use journal::read_journal;
pub(crate) use types::*;

use driver::Driver;
use files::error;
use std::fs;
use std::io::Write;

fn entry_error(preset: bool, target: &Path, cause: impl std::fmt::Display) -> CafleetError {
    if preset {
        error(format!(
            "failed to install preset into {}: {cause}",
            target.display()
        ))
    } else {
        error(format!(
            "failed to install skills into {}: {cause}",
            target.parent().unwrap_or(target).display()
        ))
    }
}

pub(crate) fn prepare_install(
    paths: &AgentPaths,
    agent: &str,
    version: &str,
) -> Result<InstallPlan, CafleetError> {
    let mut entries = Vec::new();
    for name in SKILL_NAMES {
        let prefix = format!("{name}/");
        let mut source = SKILLS
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(&prefix)
                    .map(|p| (PathBuf::from(p), *bytes))
            })
            .collect::<Vec<_>>();
        source.sort_by(|a, b| a.0.cmp(&b.0));
        if !source.iter().any(|(p, _)| p == Path::new("SKILL.md")) {
            return Err(error(format!("embedded skill {name} has no SKILL.md")));
        }
        let manifest = source
            .iter()
            .map(|(relative_path, bytes)| ManifestFile {
                relative_path: relative_path.clone(),
                size: bytes.len() as u64,
                sha256: files::digest(bytes),
            })
            .collect();
        entries.push(PlannedEntry {
            kind: EntryKind::Skill(name),
            target: paths.skills_dir.join(name),
            manifest,
            payload: source.into_iter().map(|(_, b)| b).collect(),
        });
    }
    if let Some((source, target)) = &paths.preset {
        let bytes = lookup(PRESETS, source)
            .ok_or_else(|| error(format!("missing embedded preset {source}")))?;
        entries.push(PlannedEntry {
            kind: EntryKind::Preset,
            target: target.clone(),
            manifest: vec![ManifestFile {
                relative_path: PathBuf::new(),
                size: bytes.len() as u64,
                sha256: files::digest(bytes),
            }],
            payload: vec![bytes],
        });
    }
    entries.push(PlannedEntry {
        kind: EntryKind::ObsoleteResearch,
        target: paths.skills_dir.join("cafleet-research"),
        manifest: vec![],
        payload: vec![],
    });
    Ok(InstallPlan {
        coding_agent: agent.into(),
        identity: PathBuf::from(&paths.identity),
        version: version.into(),
        entries,
    })
}

fn recover_locked(
    conn: &mut Connection,
    locations: &[PathBuf],
    hooks: &InstallHooks<'_>,
) -> Result<Vec<RecoveryOutcome>, InstallFailure> {
    let mut outcomes = Vec::new();
    // Check every discovered journal before mutating any transaction.
    let journals = locations
        .iter()
        .map(|path| read_journal(path))
        .collect::<Result<Vec<_>, _>>()?;
    let db = driver::database_path(conn)?;
    for j in &journals {
        if j.database_path != db {
            return Err(error(format!("assets journal database {} differs from current database {}; run 'cafleet setup' with the original database configuration",j.database_path.display(),db.display())).into());
        }
    }
    for journal in journals {
        let committed = journal.phase == InstallPhase::Committed;
        let mut driver = Driver {
            journal,
            hooks,
            durable_committed: committed,
            uncertain: false,
        };
        if committed {
            let cleanup_pending = match driver.cleanup(conn) {
                Ok(()) => None,
                Err(InstallFailure::Interrupted(e)) => return Err(InstallFailure::Interrupted(e)),
                Err(e) => Some(driver.incomplete(e.into_error())),
            };
            outcomes.push(RecoveryOutcome::Committed {
                installed_record: driver.journal.new_record,
                cleanup_pending,
            });
        } else {
            driver.rollback(conn)?;
            outcomes.push(RecoveryOutcome::RolledBack);
        }
    }
    Ok(outcomes)
}

fn clean_orphan_stages(keys: &std::collections::BTreeSet<PathBuf>) -> Result<(), CafleetError> {
    use std::os::unix::ffi::OsStrExt;
    for key in keys {
        let parent = key.parent().unwrap();
        if !files::exists(parent)? {
            continue;
        }
        let prefix = format!(
            ".cafleet-install-{}-",
            files::digest(key.as_os_str().as_bytes())
        );
        for entry in fs::read_dir(parent).map_err(error)? {
            let entry = entry.map_err(error)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(tx) = name
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(".stage"))
                && !tx.is_empty()
                && tx.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                files::remove(&entry.path())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn execute_install(
    conn: &mut Connection,
    plan: &InstallPlan,
    hooks: &InstallHooks<'_>,
) -> Result<InstallOutcome, InstallFailure> {
    let location = journal::journal_path(&plan.identity);
    let mut keys = std::collections::BTreeSet::new();
    for entry in &plan.entries {
        keys.insert(
            files::normalize(&entry.target, true).map_err(|e| {
                entry_error(matches!(entry.kind, EntryKind::Preset), &entry.target, e)
            })?,
        );
    }
    keys.insert(files::normalize(&location, true)?);
    let locked = locks::acquire(keys.clone(), &location, hooks)?;
    let mut own_recovered = None;
    for outcome in recover_locked(conn, &locked.journals, hooks)? {
        if let RecoveryOutcome::Committed {
            installed_record,
            cleanup_pending,
        } = outcome
        {
            if installed_record.coding_agent == plan.coding_agent
                && Path::new(&installed_record.path) == plan.identity
            {
                own_recovered = Some(InstallOutcome {
                    installed_record,
                    cleanup_pending,
                    recovered_only: true,
                });
            } else if let Some(pending) = cleanup_pending {
                return Err(error(format!("{}: {}", pending.diagnostic(), pending.cause)).into());
            }
        }
    }
    if let Some(outcome) = own_recovered {
        return Ok(outcome);
    }
    clean_orphan_stages(&keys)?;
    let tx = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_nanos()
    );
    let mut entries = Vec::new();
    for e in &plan.entries {
        let target = files::normalize(&e.target, false)?;
        let backup = files::scratch(&target, &tx, "backup");
        let stage = (!matches!(e.kind, EntryKind::ObsoleteResearch))
            .then(|| files::scratch(&target, &tx, "stage"));
        if files::exists(&backup)?
            || stage
                .as_ref()
                .map(|p| files::exists(p))
                .transpose()?
                .unwrap_or(false)
        {
            return Err(error("assets transaction scratch already exists").into());
        }
        entries.push(JournalEntry {
            previous: files::fingerprint(&target)?,
            target,
            backup,
            stage,
            manifest: e.manifest.clone(),
            state: EntryState::Original,
        });
    }
    let journal = InstallJournal {
        format_version: 1,
        transaction_id: tx,
        phase: InstallPhase::Prepared,
        coding_agent: plan.coding_agent.clone(),
        identity: plan.identity.clone(),
        database_path: driver::database_path(conn)?,
        previous_record: driver::read_record(conn, &plan.coding_agent, &plan.identity)?,
        new_record: crate::diagnosis::AssetInstallRecord {
            coding_agent: plan.coding_agent.clone(),
            path: plan.identity.to_string_lossy().into_owned(),
            cafleet_version: plan.version.clone(),
            installed_at: crate::time::format_utc(crate::time::now_utc()),
        },
        entries,
        pending: None,
    };
    let mut driver = Driver {
        journal,
        hooks,
        durable_committed: false,
        uncertain: false,
    };
    if let Err(failure) = driver.stage(plan) {
        if matches!(failure, InstallFailure::Interrupted(_)) {
            return Err(failure);
        }
        let mut original = failure.into_error();
        for entry in &driver.journal.entries {
            if let Some(stage) = &entry.stage
                && let Err(e) = files::remove(stage)
            {
                original = original.with_cleanup(e);
            }
        }
        return Err(InstallFailure::Failed(original));
    }
    let cleanup_pending = match driver.install(conn) {
        Ok(()) => None,
        Err(InstallFailure::Interrupted(event)) => return Err(InstallFailure::Interrupted(event)),
        Err(failure) if driver.durable_committed => Some(driver.incomplete(failure.into_error())),
        Err(failure) => {
            let primary = failure.into_error();
            if driver.uncertain {
                return Err(primary
                    .with_cleanup(format!(
                        "{}; persistence outcome uncertain; retained backups",
                        driver.incomplete("").diagnostic()
                    ))
                    .into());
            }
            match driver.rollback(conn) {
                Ok(()) => return Err(primary.into()),
                Err(InstallFailure::Interrupted(event)) => {
                    return Err(InstallFailure::Interrupted(event));
                }
                Err(secondary) => return Err(primary.with_cleanup(secondary.into_error()).into()),
            }
        }
    };
    Ok(InstallOutcome {
        installed_record: driver.journal.new_record,
        cleanup_pending,
        recovered_only: false,
    })
}

/// Explicit recovery entry uses the same locking and operation driver as installation.
#[cfg_attr(not(test), allow(dead_code))] // Standalone per-call recovery API shares the production driver.
pub(crate) fn recover_install(
    conn: &mut Connection,
    paths: &AgentPaths,
    hooks: &InstallHooks<'_>,
) -> Result<RecoveryOutcome, InstallFailure> {
    let location = journal::journal_path(Path::new(&paths.identity));
    let keys = locks::keys(locks::targets(paths), true)?;
    let locked = locks::acquire(keys.clone(), &location, hooks)?;
    let outcomes = recover_locked(conn, &locked.journals, hooks)?;
    if outcomes.is_empty() {
        clean_orphan_stages(&keys)?;
    }
    Ok(outcomes.into_iter().last().unwrap_or(RecoveryOutcome::None))
}

pub(crate) fn inspect_install(
    paths: &AgentPaths,
) -> Result<Option<IncompleteInstall>, CafleetError> {
    let own = journal::journal_path(Path::new(&paths.identity));
    let inspect = || -> Result<Option<PathBuf>, CafleetError> {
        let keys = locks::keys(locks::targets(paths), false)?;
        Ok(locks::discover(&keys, &own)?.into_iter().next())
    };
    match inspect() {
        Ok(None) => Ok(None),
        Ok(Some(journal)) => Ok(Some(IncompleteInstall {
            identity: PathBuf::from(&paths.identity),
            journal,
            cause: "unfinished install journal".into(),
        })),
        Err(cause) => Ok(Some(IncompleteInstall {
            identity: PathBuf::from(&paths.identity),
            journal: own,
            cause: cause.to_string(),
        })),
    }
}

pub(crate) fn install_agent_with_hooks(
    conn: &mut Connection,
    agent: &str,
    paths: &AgentPaths,
    version: &str,
    hooks: &InstallHooks<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), CafleetError> {
    let plan = prepare_install(paths, agent, version)?;
    let outcome = execute_install(conn, &plan, hooks).map_err(InstallFailure::into_error)?;
    let version = &outcome.installed_record.cafleet_version;
    writeln!(
        out,
        "{agent}: installed cafleet, cafleet-design-doc (v{version}) -> {}",
        paths.skills_dir.display()
    )
    .map_err(error)?;
    if let Some((_, target)) = &paths.preset {
        writeln!(
            out,
            "{agent}: installed preset (v{version}) -> {}",
            target.display()
        )
        .map_err(error)?;
    }
    if let Some(pending) = outcome.cleanup_pending {
        writeln!(
            err,
            "warning: assets installed at {}; cleanup pending: {}; run 'cafleet setup' to recover",
            paths.identity, pending.cause
        )
        .map_err(error)?;
    }
    Ok(())
}
pub fn install_agent(
    conn: &mut Connection,
    agent: &str,
    paths: &AgentPaths,
    version: &str,
) -> Result<(), CafleetError> {
    install_agent_with_hooks(
        conn,
        agent,
        paths,
        version,
        &InstallHooks {
            lock_mode: LockMode::Wait,
            checkpoint: &|_| Ok(()),
        },
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}

#[cfg(test)]
#[path = "assets/step10_contract_tests.rs"]
mod step10_contract_tests;
