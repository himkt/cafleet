//! Stable lock inodes serialize physical entries; durable sidecars discover crashed owners.
use super::files::error;
use super::types::*;
use super::{AgentPaths, files, journal};
use crate::error::CafleetError;
use nix::fcntl::{Flock, FlockArg};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(super) fn targets(paths: &AgentPaths) -> Vec<PathBuf> {
    let mut result = vec![
        paths.skills_dir.join("cafleet"),
        paths.skills_dir.join("cafleet-design-doc"),
        paths.skills_dir.join("cafleet-research"),
        journal::journal_path(Path::new(&paths.identity)),
    ];
    if let Some((_, target)) = &paths.preset {
        result.push(target.clone());
    }
    result
}
pub(super) fn keys(
    targets: impl IntoIterator<Item = PathBuf>,
    create: bool,
) -> Result<BTreeSet<PathBuf>, CafleetError> {
    targets
        .into_iter()
        .map(|p| files::normalize(&p, create))
        .collect()
}
pub(super) fn journal_keys(j: &InstallJournal) -> Result<BTreeSet<PathBuf>, CafleetError> {
    keys(
        j.entries
            .iter()
            .map(|e| e.target.clone())
            .chain(std::iter::once(journal::journal_path(&j.identity))),
        false,
    )
}
fn lock_path(key: &Path) -> PathBuf {
    key.with_file_name(format!(
        ".cafleet-install-lock-{}",
        files::digest(key.as_os_str().as_bytes())
    ))
}
pub(super) fn intent_path(key: &Path) -> PathBuf {
    let p = lock_path(key);
    p.with_file_name(format!(
        "{}.intent",
        p.file_name().unwrap().to_string_lossy()
    ))
}
pub(super) struct Locked {
    pub(super) journals: Vec<PathBuf>,
    _guards: Vec<Flock<File>>,
}
pub(super) fn discover(keys: &BTreeSet<PathBuf>, own: &Path) -> Result<Vec<PathBuf>, CafleetError> {
    let mut journals = BTreeSet::new();
    if files::exists(own)? {
        let owner = journal::read_journal(own)?;
        journals.insert(journal::journal_path(&owner.identity));
    }
    for key in keys {
        let path = intent_path(key);
        if !files::exists(&path)? {
            continue;
        }
        let intent = journal::read_intent(&path)?;
        if !files::exists(&intent.journal)? {
            if intent.state == IntentState::Active {
                return Err(error(format!(
                    "active assets intent at {} has missing journal {}; run 'cafleet setup' with the original configuration",
                    path.display(),
                    intent.journal.display()
                )));
            }
            continue;
        }
        let j = journal::read_journal(&intent.journal)?;
        // A reinstall publishes Prepared before replacing the preceding Finished sidecars.
        // Accept only this discovery window; locked recovery still verifies all old evidence.
        let preceding_finished =
            intent.state == IntentState::Finished && journal::prepared_original(&j);
        if (j.transaction_id != intent.transaction_id && !preceding_finished)
            || !journal_keys(&j)?.contains(key)
        {
            return Err(error("assets intent transaction/target mismatch"));
        }
        journals.insert(intent.journal);
    }
    Ok(journals.into_iter().collect())
}
pub(super) fn acquire(
    mut keys: BTreeSet<PathBuf>,
    own: &Path,
    hooks: &InstallHooks<'_>,
) -> Result<Locked, InstallFailure> {
    loop {
        super::driver::checkpoint(
            hooks,
            InstallEvent {
                operation: InstallOperation::LockAcquire,
                edge: Edge::Before,
                entry: None,
                path: None,
                journal: own.to_path_buf(),
                phase: None,
            },
        )?;
        let mut guards = Vec::new();
        for key in &keys {
            let path = lock_path(key);
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| InstallFailure::from(error(e)))?;
            let arg = match hooks.lock_mode {
                LockMode::Wait => FlockArg::LockExclusive,
                LockMode::Try => FlockArg::LockExclusiveNonblock,
            };
            match Flock::lock(file, arg) {
                Ok(guard) => guards.push(guard),
                Err((_, errno)) if errno == nix::errno::Errno::EWOULDBLOCK => {
                    return Err(InstallFailure::Busy(path));
                }
                Err((_, errno)) => return Err(error(errno).into()),
            }
        }
        let journals = discover(&keys, own)?;
        let mut expanded = keys.clone();
        for path in &journals {
            expanded.extend(journal_keys(&journal::read_journal(path)?)?);
        }
        if expanded != keys {
            drop(guards);
            keys = expanded;
            continue;
        }
        super::driver::checkpoint(
            hooks,
            InstallEvent {
                operation: InstallOperation::LockAcquire,
                edge: Edge::After,
                entry: None,
                path: None,
                journal: own.to_path_buf(),
                phase: None,
            },
        )?;
        return Ok(Locked {
            journals,
            _guards: guards,
        });
    }
}
