//! One operation driver implements install, rollback and committed cleanup.
use super::files::error;
use super::types::*;
use super::{files, journal, locks};
use crate::diagnosis::AssetInstallRecord;
use crate::error::CafleetError;
use rusqlite::{Connection, OptionalExtension, params};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) fn checkpoint(
    hooks: &InstallHooks<'_>,
    event: InstallEvent,
) -> Result<(), InstallFailure> {
    match (hooks.checkpoint)(&event) {
        Ok(()) => Ok(()),
        Err(InstallFault::Fail(message)) => Err(error(message).into()),
        Err(InstallFault::Interrupt) => Err(InstallFailure::Interrupted(event)),
    }
}
pub(super) fn database_path(conn: &Connection) -> Result<PathBuf, CafleetError> {
    let path = conn
        .path()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| error("recoverable assets installation requires a file-backed database"))?;
    Path::new(path).canonicalize().map_err(error)
}
pub(super) fn read_record(
    conn: &Connection,
    agent: &str,
    identity: &Path,
) -> Result<Option<AssetInstallRecord>, CafleetError> {
    conn.query_row("SELECT coding_agent,path,cafleet_version,installed_at FROM asset_installs WHERE coding_agent=?1 AND path=?2",params![agent,identity.to_string_lossy()],|r|Ok(AssetInstallRecord{coding_agent:r.get(0)?,path:r.get(1)?,cafleet_version:r.get(2)?,installed_at:r.get(3)?})).optional().map_err(error)
}
fn write_record(
    conn: &mut Connection,
    j: &InstallJournal,
    restore: bool,
) -> Result<(), CafleetError> {
    let tx = conn.transaction().map_err(error)?;
    let record = if restore {
        j.previous_record.as_ref()
    } else {
        Some(&j.new_record)
    };
    if let Some(r) = record {
        tx.execute("INSERT INTO asset_installs(coding_agent,path,cafleet_version,installed_at) VALUES (?1,?2,?3,?4) ON CONFLICT(coding_agent,path) DO UPDATE SET cafleet_version=excluded.cafleet_version,installed_at=excluded.installed_at",params![r.coding_agent,r.path,r.cafleet_version,r.installed_at]).map_err(error)?;
    } else {
        tx.execute(
            "DELETE FROM asset_installs WHERE coding_agent=?1 AND path=?2",
            params![j.coding_agent, j.identity.to_string_lossy()],
        )
        .map_err(error)?;
    }
    tx.commit().map_err(error)
}
pub(super) struct Driver<'a, 'h> {
    pub(super) journal: InstallJournal,
    pub(super) hooks: &'a InstallHooks<'h>,
    pub(super) durable_committed: bool,
    pub(super) uncertain: bool,
}
impl Driver<'_, '_> {
    fn location(&self) -> PathBuf {
        journal::journal_path(&self.journal.identity)
    }
    fn event(
        &self,
        operation: InstallOperation,
        edge: Edge,
        entry: Option<usize>,
        path: Option<PathBuf>,
    ) -> InstallEvent {
        InstallEvent {
            operation,
            edge,
            entry,
            path,
            journal: self.location(),
            phase: Some(self.journal.phase),
        }
    }
    fn check(
        &self,
        operation: InstallOperation,
        edge: Edge,
        entry: Option<usize>,
        path: Option<PathBuf>,
    ) -> Result<(), InstallFailure> {
        checkpoint(self.hooks, self.event(operation, edge, entry, path))
    }
    fn durable(&mut self, path: &Path, value: &serde_json::Value) -> Result<(), InstallFailure> {
        files::durable_json(path, value).map_err(|e| {
            self.uncertain = true;
            InstallFailure::from(e)
        })
    }
    pub(super) fn persist(&mut self) -> Result<(), InstallFailure> {
        let path = self.location();
        self.check(
            InstallOperation::JournalPersist,
            Edge::Before,
            None,
            Some(path.clone()),
        )?;
        self.durable(&path, &journal::value(&self.journal))?;
        if self.journal.phase == InstallPhase::Committed {
            self.durable_committed = true;
        }
        self.check(
            InstallOperation::JournalPersist,
            Edge::After,
            None,
            Some(path),
        )
    }
    fn intent(
        &mut self,
        operation: InstallOperation,
        entry: Option<usize>,
    ) -> Result<(), InstallFailure> {
        self.journal.pending = Some(JournalOperation { operation, entry });
        self.persist()
    }
    fn done(&mut self) -> Result<(), InstallFailure> {
        self.journal.pending = None;
        self.persist()
    }
    pub(super) fn intents(&mut self, state: IntentState) -> Result<(), InstallFailure> {
        for key in locks::journal_keys(&self.journal)? {
            let path = locks::intent_path(&key);
            self.check(
                InstallOperation::IntentPersist,
                Edge::Before,
                None,
                Some(path.clone()),
            )?;
            self.durable(&path, &journal::intent_value(&self.journal, state))?;
            self.check(
                InstallOperation::IntentPersist,
                Edge::After,
                None,
                Some(path),
            )?;
        }
        Ok(())
    }
    fn rename(
        &mut self,
        op: InstallOperation,
        index: usize,
        from: &Path,
        to: &Path,
    ) -> Result<(), InstallFailure> {
        self.intent(op, Some(index))?;
        let target = self.journal.entries[index].target.clone();
        self.check(op, Edge::Before, Some(index), Some(target.clone()))?;
        fs::rename(from, to).map_err(|e| InstallFailure::from(self.entry_error(index, e)))?;
        files::sync_dir(to.parent().unwrap()).map_err(|e| {
            self.uncertain = true;
            InstallFailure::from(e)
        })?;
        self.check(op, Edge::After, Some(index), Some(target))
    }
    fn remove(
        &mut self,
        op: InstallOperation,
        index: usize,
        path: &Path,
    ) -> Result<(), InstallFailure> {
        self.intent(op, Some(index))?;
        self.check(op, Edge::Before, Some(index), Some(path.to_path_buf()))?;
        files::remove(path).map_err(|e| {
            self.uncertain = true;
            InstallFailure::from(e)
        })?;
        self.check(op, Edge::After, Some(index), Some(path.to_path_buf()))?;
        self.done()
    }
    fn entry_error(&self, index: usize, cause: impl std::fmt::Display) -> CafleetError {
        let entry = &self.journal.entries[index];
        super::entry_error(
            index == 2 && self.journal.coding_agent != "claude",
            &entry.target,
            cause,
        )
    }
    pub(super) fn stage(&self, plan: &InstallPlan) -> Result<(), InstallFailure> {
        for (i, (entry, planned)) in self.journal.entries.iter().zip(&plan.entries).enumerate() {
            let Some(stage) = &entry.stage else {
                continue;
            };
            if matches!(planned.kind, EntryKind::Skill(_)) {
                fs::create_dir(stage).map_err(|e| InstallFailure::from(self.entry_error(i, e)))?;
            }
            for (manifest, bytes) in planned.manifest.iter().zip(&planned.payload) {
                let path = if manifest.relative_path.as_os_str().is_empty() {
                    stage.clone()
                } else {
                    stage.join(&manifest.relative_path)
                };
                fs::create_dir_all(path.parent().unwrap())
                    .map_err(|e| InstallFailure::from(self.entry_error(i, e)))?;
                let event = |edge| InstallEvent {
                    operation: InstallOperation::StageWrite,
                    edge,
                    entry: Some(i),
                    path: Some(path.clone()),
                    journal: self.location(),
                    phase: None,
                };
                checkpoint(self.hooks, event(Edge::Before))?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| InstallFailure::from(self.entry_error(i, e)))?;
                file.write_all(bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|e| InstallFailure::from(self.entry_error(i, e)))?;
                files::sync_dir(path.parent().unwrap()).map_err(|e| self.entry_error(i, e))?;
                checkpoint(self.hooks, event(Edge::After))?;
            }
        }
        let event = |edge| InstallEvent {
            operation: InstallOperation::StageValidate,
            edge,
            entry: None,
            path: None,
            journal: self.location(),
            phase: None,
        };
        checkpoint(self.hooks, event(Edge::Before))?;
        for (i, entry) in self.journal.entries.iter().enumerate() {
            if let Some(stage) = &entry.stage {
                files::verify(stage, &entry.manifest).map_err(|e| self.entry_error(i, e))?;
                sync_tree(stage).map_err(|e| self.entry_error(i, e))?;
                files::sync_dir(stage.parent().unwrap()).map_err(|e| self.entry_error(i, e))?;
            }
        }
        checkpoint(self.hooks, event(Edge::After))
    }
    pub(super) fn install(&mut self, conn: &mut Connection) -> Result<(), InstallFailure> {
        self.persist()?;
        self.intents(IntentState::Active)?;
        self.journal.phase = InstallPhase::Swapping;
        self.persist()?;
        for i in 0..self.journal.entries.len() {
            let e = self.journal.entries[i].clone();
            if e.previous.is_some() {
                self.rename(InstallOperation::BackupRename, i, &e.target, &e.backup)?;
                self.journal.entries[i].state = EntryState::BackedUp;
                self.done()?;
            }
            if let Some(stage) = e.stage {
                self.rename(InstallOperation::InstallRename, i, &stage, &e.target)?;
            }
            self.journal.entries[i].state = EntryState::Installed;
            self.done()?;
        }
        self.journal.phase = InstallPhase::Recording;
        self.persist()?;
        self.intent(InstallOperation::RecordCommit, None)?;
        self.check(InstallOperation::RecordCommit, Edge::Before, None, None)?;
        write_record(conn, &self.journal, false)?;
        self.check(InstallOperation::RecordCommit, Edge::After, None, None)?;
        self.done()?;
        self.journal.phase = InstallPhase::Committed;
        self.persist()?;
        self.cleanup(conn)
    }
    pub(super) fn rollback(&mut self, conn: &mut Connection) -> Result<(), InstallFailure> {
        if self.journal.phase == InstallPhase::Prepared {
            // Validate the entire untouched install before changing even a discovery sidecar.
            if !journal::prepared_original(&self.journal)
                || database_path(conn)? != self.journal.database_path
                || read_record(conn, &self.journal.coding_agent, &self.journal.identity)?
                    .as_ref()
                    .map(journal::record_value)
                    != self
                        .journal
                        .previous_record
                        .as_ref()
                        .map(journal::record_value)
            {
                return Err(error(
                    "unverified Prepared assets database or journal state; recovery stopped",
                )
                .into());
            }
            for entry in &self.journal.entries {
                if files::exists(&entry.backup)?
                    || files::fingerprint(&entry.target)? != entry.previous
                {
                    return Err(error(
                        "unverified Prepared assets entry or backup; recovery stopped",
                    )
                    .into());
                }
            }
            if !files::exists(&self.location())? {
                self.persist()?;
            }
            // Stay Prepared throughout normalization so a second interruption remains discoverable.
            self.intents(IntentState::Active)?;
        }
        self.journal.phase = InstallPhase::RollingBack;
        self.persist()?;
        for i in (0..self.journal.entries.len()).rev() {
            let e = self.journal.entries[i].clone();
            let backup = files::fingerprint(&e.backup)?;
            let target = files::fingerprint(&e.target)?;
            if let Some(previous) = &e.previous {
                if let Some(backup) = backup {
                    if &backup != previous {
                        return Err(
                            error("assets backup fingerprint mismatch; recovery stopped").into(),
                        );
                    }
                    if target.is_some() {
                        // The only disposable entry here is the newly installed payload.
                        files::verify(&e.target, &e.manifest)?;
                        self.remove(InstallOperation::RemoveNew, i, &e.target)?;
                    }
                    self.rename(InstallOperation::RestoreBackup, i, &e.backup, &e.target)?;
                } else if target.as_ref() != Some(previous) {
                    return Err(error(format!("missing assets backup or changed old entry at {}; run 'cafleet setup' to recover",e.target.display())).into());
                }
            } else {
                if backup.is_some() {
                    return Err(error("unexpected assets backup for absent old entry").into());
                }
                if target.is_some() {
                    files::verify(&e.target, &e.manifest)?;
                    self.remove(InstallOperation::RemoveNew, i, &e.target)?;
                }
            }
            self.journal.entries[i].state = EntryState::Restored;
            self.done()?;
        }
        self.intent(InstallOperation::RestoreRecord, None)?;
        self.check(InstallOperation::RestoreRecord, Edge::Before, None, None)?;
        write_record(conn, &self.journal, true)?;
        self.check(InstallOperation::RestoreRecord, Edge::After, None, None)?;
        self.done()?;
        self.clean_stages()?;
        self.finish()
    }
    fn clean_stages(&mut self) -> Result<(), InstallFailure> {
        for i in 0..self.journal.entries.len() {
            if let Some(stage) = self.journal.entries[i].stage.clone()
                && files::exists(&stage)?
            {
                self.remove(InstallOperation::CleanupStage, i, &stage)?;
            }
        }
        Ok(())
    }
    pub(super) fn cleanup(&mut self, conn: &Connection) -> Result<(), InstallFailure> {
        for e in &self.journal.entries {
            if e.stage.is_some() {
                files::verify(&e.target, &e.manifest)?;
            } else if files::exists(&e.target)? {
                return Err(
                    error("obsolete research entry remains after committed install").into(),
                );
            }
        }
        let actual = read_record(conn, &self.journal.coding_agent, &self.journal.identity)?;
        if actual.as_ref().map(journal::record_value)
            != Some(journal::record_value(&self.journal.new_record))
        {
            return Err(error(
                "committed assets database record mismatch; run 'cafleet setup' to recover",
            )
            .into());
        }
        for i in 0..self.journal.entries.len() {
            let backup = self.journal.entries[i].backup.clone();
            if files::exists(&backup)? {
                self.remove(InstallOperation::CleanupBackup, i, &backup)?;
            }
        }
        self.clean_stages()?;
        self.finish()
    }
    fn finish(&mut self) -> Result<(), InstallFailure> {
        self.intents(IntentState::Finished)?;
        self.intent(InstallOperation::JournalRemove, None)?;
        let location = self.location();
        self.check(
            InstallOperation::JournalRemove,
            Edge::Before,
            None,
            Some(location.clone()),
        )?;
        files::remove(&location).map_err(|e| {
            self.uncertain = true;
            InstallFailure::from(e)
        })?;
        self.check(
            InstallOperation::JournalRemove,
            Edge::After,
            None,
            Some(location),
        )
    }
    pub(super) fn incomplete(&self, cause: impl std::fmt::Display) -> IncompleteInstall {
        IncompleteInstall {
            identity: self.journal.identity.clone(),
            journal: self.location(),
            cause: cause.to_string(),
        }
    }
}
fn sync_tree(path: &Path) -> Result<(), CafleetError> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    if meta.is_dir() {
        for e in fs::read_dir(path).map_err(error)? {
            sync_tree(&e.map_err(error)?.path())?;
        }
        files::sync_dir(path)?;
    } else if meta.is_file() {
        fs::File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(error)?;
    } else {
        return Err(error("stage contains non-regular entry"));
    }
    Ok(())
}
