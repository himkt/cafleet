//! Phase A: connect from assets.rs with #[cfg(test)] mod step10_contract_tests.
//! Real filesystem and file-backed SQLite; no alternate installer.
use super::*;
use crate::diagnosis::{self, AssetInstallRecord, AssetMode, AssetState};
use crate::embedded::{PRESETS, SKILLS, lookup};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};
use std::sync::mpsc;
use std::time::Duration;

const VERSION: &str = "step10-new";
const OLD_TIME: &str = "2001-02-03T04:05:06.123456+00:00";
type Row = (String, String, String, String);
type Tree = Vec<(PathBuf, String, Vec<u8>)>;

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tree(path: &Path) -> Tree {
    fn visit(root: &Path, path: &Path, result: &mut Tree) {
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("{}: {error}", path.display()),
        };
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if metadata.file_type().is_symlink() {
            use std::os::unix::ffi::OsStrExt;
            result.push((
                relative,
                "link".into(),
                fs::read_link(path).unwrap().as_os_str().as_bytes().to_vec(),
            ));
        } else if metadata.is_dir() {
            result.push((relative, "dir".into(), vec![]));
            let mut children: Vec<_> = fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                visit(root, &child, result);
            }
        } else {
            result.push((relative, "file".into(), fs::read(path).unwrap()));
        }
    }
    let mut result = Vec::new();
    visit(path, path, &mut result);
    result
}

fn row(conn: &Connection, paths: &AgentPaths, agent: &str) -> Option<Row> {
    conn.query_row("SELECT coding_agent,path,cafleet_version,installed_at FROM asset_installs WHERE coding_agent=?1 AND path=?2",
        params![agent, paths.identity], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().unwrap()
}
fn record_tuple(record: &AssetInstallRecord) -> Row {
    (
        record.coding_agent.clone(),
        record.path.clone(),
        record.cafleet_version.clone(),
        record.installed_at.clone(),
    )
}
fn okay(_: &InstallEvent) -> Result<(), InstallFault> {
    Ok(())
}
fn hooks(checkpoint: &dyn Fn(&InstallEvent) -> Result<(), InstallFault>) -> InstallHooks<'_> {
    InstallHooks {
        lock_mode: LockMode::Wait,
        checkpoint,
    }
}
fn matches_event(
    event: &InstallEvent,
    operation: &InstallOperation,
    edge: &Edge,
    entry: Option<usize>,
) -> bool {
    &event.operation == operation && &event.edge == edge && event.entry == entry
}
fn failed(result: Result<InstallOutcome, InstallFailure>, text: &str) {
    match result {
        Err(InstallFailure::Failed(error)) => assert!(error.message().contains(text), "{error:?}"),
        other => panic!("expected failure containing {text}: {other:?}"),
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    database: PathBuf,
    conn: Connection,
    paths: AgentPaths,
    agent: &'static str,
}
impl Fixture {
    fn new(agent: &'static str, old_entries: bool, old_record: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(".step10-assets-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let database = root.join("assets.sqlite3");
        let mut conn = Connection::open(&database).unwrap();
        crate::db::migrate_to_head(&mut conn).unwrap();
        let paths = agent_paths(&|_| None, &root, agent).unwrap();
        let result = Self {
            dir,
            database,
            conn,
            paths,
            agent,
        };
        if old_entries {
            for (index, entry) in result.plan().entries.iter().enumerate() {
                fs::create_dir_all(entry.target.parent().unwrap()).unwrap();
                if matches!(entry.kind, EntryKind::Preset) {
                    fs::write(&entry.target, b"old preset bytes").unwrap();
                } else {
                    fs::create_dir_all(&entry.target).unwrap();
                    fs::write(entry.target.join("old.txt"), format!("old entry {index}")).unwrap();
                }
            }
        }
        if old_record {
            result
                .conn
                .execute(
                    "INSERT INTO asset_installs VALUES (?1,?2,?3,?4)",
                    params![agent, result.paths.identity, "old-version", OLD_TIME],
                )
                .unwrap();
        }
        result
    }
    fn plan(&self) -> InstallPlan {
        prepare_install(&self.paths, self.agent, VERSION).unwrap()
    }
    fn snapshot(&self) -> Vec<Tree> {
        self.plan()
            .entries
            .iter()
            .map(|entry| tree(&entry.target))
            .collect()
    }
    fn record(&self) -> Option<Row> {
        row(&self.conn, &self.paths, self.agent)
    }
    fn install(
        &mut self,
        checkpoint: &dyn Fn(&InstallEvent) -> Result<(), InstallFault>,
    ) -> Result<InstallOutcome, InstallFailure> {
        let plan = self.plan();
        execute_install(&mut self.conn, &plan, &hooks(checkpoint))
    }
    fn fresh_recover(
        &self,
        checkpoint: &dyn Fn(&InstallEvent) -> Result<(), InstallFault>,
    ) -> Result<RecoveryOutcome, InstallFailure> {
        // The installer receives a fresh independent SQLite handle, retaining no invocation state.
        let mut conn = Connection::open(&self.database).unwrap();
        recover_install(&mut conn, &self.paths, &hooks(checkpoint))
    }
    fn assert_old(&self, entries: &[Tree], record: &Option<Row>) {
        assert_eq!(self.snapshot(), entries);
        assert_eq!(&self.record(), record);
    }
    fn assert_new(&self, version: &str) {
        for entry in self.plan().entries {
            if matches!(entry.kind, EntryKind::ObsoleteResearch) {
                assert!(fs::symlink_metadata(&entry.target).is_err());
                continue;
            }
            let mut observed = Vec::new();
            for (relative, kind, bytes) in tree(&entry.target) {
                if kind == "dir" {
                    continue;
                }
                assert_eq!(kind, "file", "new payload may not retain old symlinks");
                observed.push((relative, bytes.len() as u64, sha256_hex(&bytes)));
            }
            let expected: Vec<_> = entry
                .manifest
                .iter()
                .map(|m| (m.relative_path.clone(), m.size, m.sha256.clone()))
                .collect();
            assert_eq!(observed, expected, "{}", entry.target.display());
        }
        let record = self.record().expect("installed row");
        assert_eq!(record.0, self.agent);
        assert_eq!(record.1, self.paths.identity);
        assert_eq!(record.2, version);
        assert!(!record.3.is_empty());
        assert_ne!(record.3, OLD_TIME);
    }
    fn assert_finished(&self) {
        assert!(inspect_install(&self.paths).unwrap().is_none());
        // Lock/Finished intent files intentionally survive; referenced scratch does not.
        for (relative, kind, bytes) in tree(self.dir.path()) {
            if kind == "file"
                && relative
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".intent")
            {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(
                    value["state"].as_str().unwrap().to_ascii_lowercase(),
                    "finished"
                );
                assert!(!Path::new(value["journal"].as_str().unwrap()).exists());
            }
        }
    }
}

#[test]
fn plan_is_read_only_with_exact_embedded_manifests_and_fixed_entry_order() {
    for agent in TARGET_AGENTS {
        let fixture = Fixture::new(agent, false, false);
        let before = tree(fixture.dir.path());
        let plan = fixture.plan();
        assert_eq!(tree(fixture.dir.path()), before);
        assert_eq!(plan.coding_agent, agent);
        assert_eq!(plan.identity, Path::new(&fixture.paths.identity));
        assert_eq!(plan.version, VERSION);
        assert!(matches!(plan.entries[0].kind, EntryKind::Skill("cafleet")));
        assert!(matches!(
            plan.entries[1].kind,
            EntryKind::Skill("cafleet-design-doc")
        ));
        assert!(matches!(
            plan.entries.last().unwrap().kind,
            EntryKind::ObsoleteResearch
        ));
        assert_eq!(plan.entries.len(), if agent == "claude" { 3 } else { 4 });
        for entry in &plan.entries {
            let expected: Vec<(PathBuf, u64, String)> = match entry.kind {
                EntryKind::Skill(name) => {
                    let prefix = format!("{name}/");
                    let mut files: Vec<_> = SKILLS
                        .iter()
                        .filter_map(|(path, bytes)| {
                            path.strip_prefix(&prefix).map(|relative| {
                                (
                                    PathBuf::from(relative),
                                    bytes.len() as u64,
                                    sha256_hex(bytes),
                                )
                            })
                        })
                        .collect();
                    files.sort();
                    assert!(files.iter().any(|file| file.0 == Path::new("SKILL.md")));
                    files
                }
                EntryKind::Preset => {
                    let source = fixture.paths.preset.as_ref().unwrap().0;
                    let bytes = lookup(PRESETS, source).unwrap();
                    vec![(PathBuf::new(), bytes.len() as u64, sha256_hex(bytes))]
                }
                EntryKind::ObsoleteResearch => vec![],
            };
            assert_eq!(
                entry
                    .manifest
                    .iter()
                    .map(|m| (m.relative_path.clone(), m.size, m.sha256.clone()))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn success_same_version_replaces_bytes_records_once_and_leaves_finished_intents() {
    for agent in TARGET_AGENTS {
        let mut fixture = Fixture::new(agent, true, true);
        fixture
            .conn
            .execute("UPDATE asset_installs SET cafleet_version=?1", [VERSION])
            .unwrap();
        let calls = RefCell::new(Vec::new());
        let outcome = fixture
            .install(&|e| {
                calls
                    .borrow_mut()
                    .push(format!("{:?}/{:?}", e.operation, e.edge));
                Ok(())
            })
            .unwrap();
        fixture.assert_new(VERSION);
        fixture.assert_finished();
        assert_eq!(
            fixture.record(),
            Some(record_tuple(&outcome.installed_record))
        );
        assert!(outcome.cleanup_pending.is_none());
        assert!(!outcome.recovered_only);
        assert_eq!(
            calls
                .borrow()
                .iter()
                .filter(|s| *s == "RecordCommit/After")
                .count(),
            1
        );
    }
}

#[test]
fn symlink_entries_and_research_file_directory_or_link_never_modify_referents_or_siblings() {
    for research_kind in ["directory", "file", "link", "absent"] {
        let mut fixture = Fixture::new("codex", false, true);
        let outside = fixture.dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"untouched").unwrap();
        let plan = fixture.plan();
        for entry in &plan.entries[..3] {
            fs::create_dir_all(entry.target.parent().unwrap()).unwrap();
            symlink(&outside, &entry.target).unwrap();
        }
        let research = &plan.entries[3].target;
        match research_kind {
            "directory" => {
                fs::create_dir_all(research).unwrap();
                fs::write(research.join("old"), b"old").unwrap();
            }
            "file" => fs::write(research, b"obsolete").unwrap(),
            "link" => symlink(&outside, research).unwrap(),
            _ => {}
        }
        let sibling = fixture.paths.skills_dir.join("unrelated");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("keep"), b"sibling").unwrap();
        let outside_before = tree(&outside);
        let sibling_before = tree(&sibling);
        fixture.install(&okay).unwrap();
        fixture.assert_new(VERSION);
        fixture.assert_finished();
        assert_eq!(tree(&outside), outside_before);
        assert_eq!(tree(&sibling), sibling_before);
    }
}

#[test]
fn stage_write_and_validation_failures_leave_old_entries_and_exact_record() {
    for (operation, entry) in [
        (InstallOperation::StageWrite, Some(0)),
        (InstallOperation::StageWrite, Some(1)),
        (InstallOperation::StageWrite, Some(2)),
        (InstallOperation::StageValidate, None),
    ] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, true);
            let before = fixture.snapshot();
            let record = fixture.record();
            let hit = Cell::new(false);
            let stage_files = RefCell::new(Vec::new());
            failed(
                fixture.install(&|e| {
                    if e.operation == InstallOperation::StageWrite {
                        stage_files.borrow_mut().push(e.path.clone().unwrap());
                    }
                    if matches_event(e, &operation, &edge, entry) && !hit.replace(true) {
                        return Err(InstallFault::Fail("stage failure".into()));
                    }
                    Ok(())
                }),
                "stage failure",
            );
            assert!(hit.get());
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
            for path in stage_files.into_inner() {
                assert!(fs::symlink_metadata(path).is_err());
            }
        }
    }
}

#[test]
fn real_stage_validation_rejects_changed_bytes_missing_entrypoint_and_extra_file() {
    for damage in ["bytes", "missing", "extra"] {
        let mut fixture = Fixture::new("codex", true, true);
        let before = fixture.snapshot();
        let record = fixture.record();
        let hit = Cell::new(false);
        let result = fixture.install(&|e| {
            if e.operation == InstallOperation::StageWrite
                && e.edge == Edge::After
                && e.entry == Some(0)
                && e.path
                    .as_ref()
                    .is_some_and(|p| p.file_name().is_some_and(|n| n == "SKILL.md"))
                && !hit.replace(true)
            {
                let path = e.path.as_ref().unwrap();
                if damage == "bytes" {
                    fs::write(path, b"invalid stage bytes").unwrap();
                } else if damage == "missing" {
                    fs::remove_file(path).unwrap();
                } else {
                    fs::write(
                        path.parent().unwrap().join("unexpected-stage-file"),
                        b"extra",
                    )
                    .unwrap();
                }
            }
            Ok(())
        });
        assert!(hit.get());
        assert!(matches!(result, Err(InstallFailure::Failed(_))));
        fixture.assert_old(&before, &record);
        fixture.assert_finished();
    }
}

#[test]
fn every_backup_and_install_rename_failure_restores_present_or_absent_entries_and_rows() {
    for old in [false, true] {
        let operations = if old {
            vec![
                InstallOperation::BackupRename,
                InstallOperation::InstallRename,
            ]
        } else {
            vec![InstallOperation::InstallRename]
        };
        for operation in operations {
            let count = if operation == InstallOperation::BackupRename {
                4
            } else {
                3
            };
            for index in 0..count {
                for edge in [Edge::Before, Edge::After] {
                    let mut fixture = Fixture::new("codex", old, old);
                    let before = fixture.snapshot();
                    let record = fixture.record();
                    let hit = Cell::new(false);
                    failed(
                        fixture.install(&|e| {
                            if matches_event(e, &operation, &edge, Some(index))
                                && !hit.replace(true)
                            {
                                return Err(InstallFault::Fail("rename failure".into()));
                            }
                            Ok(())
                        }),
                        "rename failure",
                    );
                    assert!(hit.get(), "{operation:?}/{edge:?}/{index}");
                    fixture.assert_old(&before, &record);
                    fixture.assert_finished();
                }
            }
        }
    }
}

#[test]
fn record_commit_before_and_after_failure_restore_all_four_old_columns_or_absence() {
    for old_record in [false, true] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, old_record);
            let before = fixture.snapshot();
            let record = fixture.record();
            let hit = Cell::new(false);
            let database = fixture.database.clone();
            let identity = fixture.paths.identity.clone();
            failed(fixture.install(&|e| {
                if matches_event(e,&InstallOperation::RecordCommit,&edge,None) && !hit.replace(true) {
                    let observer = Connection::open(&database).unwrap();
                    let observed: Option<String> = observer.query_row("SELECT cafleet_version FROM asset_installs WHERE coding_agent='codex' AND path=?1",[&identity],|r| r.get(0)).optional().unwrap();
                    if edge == Edge::After { assert_eq!(observed.as_deref(),Some(VERSION)); }
                    return Err(InstallFault::Fail("record boundary".into()));
                }
                Ok(())
            }),"record boundary");
            assert!(hit.get());
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
        }
    }
}

#[test]
fn real_sql_trigger_failure_rolls_back_entries_and_preserves_exact_old_timestamp() {
    for old_record in [false, true] {
        let mut fixture = Fixture::new("codex", true, old_record);
        let before = fixture.snapshot();
        let record = fixture.record();
        fixture.conn.execute_batch("CREATE TRIGGER deny_new_assets BEFORE INSERT ON asset_installs WHEN NEW.cafleet_version='step10-new' BEGIN SELECT RAISE(ABORT,'actual asset record trigger failure'); END;").unwrap();
        failed(
            fixture.install(&okay),
            "actual asset record trigger failure",
        );
        fixture.assert_old(&before, &record);
        fixture.assert_finished();
    }
}

#[test]
fn durable_committed_journal_after_is_success_boundary_and_before_still_rolls_back() {
    for edge in [Edge::Before, Edge::After] {
        let mut fixture = Fixture::new("codex", true, true);
        let before = fixture.snapshot();
        let record = fixture.record();
        let hit = Cell::new(false);
        let result = fixture.install(&|e| {
            if e.operation == InstallOperation::JournalPersist
                && e.edge == edge
                && e.phase == Some(InstallPhase::Committed)
                && !hit.replace(true)
            {
                return Err(InstallFault::Fail("committed boundary".into()));
            }
            Ok(())
        });
        assert!(hit.get());
        if edge == Edge::Before {
            failed(result, "committed boundary");
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
        } else {
            let outcome = result.unwrap();
            assert!(outcome.cleanup_pending.is_some());
            fixture.assert_new(VERSION);
            assert!(inspect_install(&fixture.paths).unwrap().is_some());
            assert!(matches!(
                fixture.fresh_recover(&okay).unwrap(),
                RecoveryOutcome::Committed {
                    cleanup_pending: None,
                    ..
                }
            ));
            fixture.assert_new(VERSION);
            fixture.assert_finished();
        }
    }
}

#[test]
fn rollback_failure_keeps_primary_and_secondary_causes_and_remaining_backups_for_retry() {
    for operation in [
        InstallOperation::RemoveNew,
        InstallOperation::RestoreBackup,
        InstallOperation::RestoreRecord,
    ] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, true);
            let before = fixture.snapshot();
            let record = fixture.record();
            let primary = Cell::new(false);
            let secondary = Cell::new(false);
            let result = fixture.install(&|e| {
                if matches_event(e, &InstallOperation::RecordCommit, &Edge::After, None)
                    && !primary.replace(true)
                {
                    return Err(InstallFault::Fail("primary record failure".into()));
                }
                if matches!(
                    e.operation,
                    InstallOperation::RemoveNew
                        | InstallOperation::RestoreBackup
                        | InstallOperation::RestoreRecord
                ) && e.edge == Edge::Before
                {
                    let journal = read_journal(&e.journal).unwrap();
                    let pending = journal.pending.as_ref().expect("durable rollback intent");
                    assert_eq!(pending.operation, e.operation);
                    assert_eq!(pending.entry, e.entry);
                }
                if e.operation == operation && e.edge == edge && !secondary.replace(true) {
                    return Err(InstallFault::Fail("secondary rollback failure".into()));
                }
                Ok(())
            });
            match result {
                Err(InstallFailure::Failed(e)) => {
                    assert!(e.message().contains("primary record failure"));
                    assert!(e.message().contains("secondary rollback failure"));
                }
                other => panic!("{other:?}"),
            }
            assert!(primary.get() && secondary.get());
            let incomplete = inspect_install(&fixture.paths).unwrap().unwrap();
            let journal = read_journal(&incomplete.journal).unwrap();
            assert_eq!(journal.phase, InstallPhase::RollingBack);
            if operation == InstallOperation::RemoveNew {
                assert!(journal.entries.iter().any(|entry| entry.backup.exists()));
            }
            assert!(matches!(
                fixture.fresh_recover(&okay).unwrap(),
                RecoveryOutcome::RolledBack
            ));
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
        }
    }
}

#[test]
fn committed_cleanup_failures_preserve_new_install_and_recover_only_original_version() {
    for operation in [
        InstallOperation::CleanupBackup,
        InstallOperation::JournalRemove,
    ] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, true);
            let hit = Cell::new(false);
            let outcome = fixture
                .install(&|e| {
                    if e.operation == operation && e.edge == edge && !hit.replace(true) {
                        return Err(InstallFault::Fail("cleanup failure".into()));
                    }
                    Ok(())
                })
                .unwrap();
            assert!(hit.get());
            assert!(outcome.cleanup_pending.is_some());
            fixture.assert_new(VERSION);
            let installed = fixture.record();
            if operation == InstallOperation::JournalRemove && edge == Edge::After {
                // Physical removal and sync already completed; no journal remains to recover.
                fixture.assert_finished();
                continue;
            }
            let recovery_plan =
                prepare_install(&fixture.paths, fixture.agent, "newer-binary").unwrap();
            let recovered =
                execute_install(&mut fixture.conn, &recovery_plan, &hooks(&okay)).unwrap();
            assert!(recovered.recovered_only);
            assert!(recovered.cleanup_pending.is_none());
            assert_eq!(recovered.installed_record.cafleet_version, VERSION);
            assert_eq!(fixture.record(), installed);
            fixture.assert_new(VERSION);
            fixture.assert_finished();
            let installed_next =
                execute_install(&mut fixture.conn, &recovery_plan, &hooks(&okay)).unwrap();
            assert!(!installed_next.recovered_only);
            fixture.assert_new("newer-binary");
        }
    }
}

#[test]
fn durable_journal_and_all_active_intents_precede_real_swaps_and_finish_before_removal() {
    let mut fixture = Fixture::new("codex", true, true);
    let previous = fixture.record().unwrap();
    let root = fixture.dir.path().to_path_buf();
    let paths = RefCell::new(Vec::<PathBuf>::new());
    let validated = Cell::new(false);
    let prepared = Cell::new(false);
    let swaps = Cell::new(0);
    fixture
        .install(&|e| {
            if matches_event(e, &InstallOperation::StageValidate, &Edge::After, None) {
                validated.set(true);
            }
            if e.operation == InstallOperation::JournalPersist && e.edge == Edge::After {
                let journal = read_journal(&e.journal).unwrap();
                assert_eq!(journal.format_version, 1);
                assert_eq!(e.phase.as_ref(), Some(&journal.phase));
                assert_eq!(
                    record_tuple(journal.previous_record.as_ref().unwrap()),
                    previous
                );
                if journal.phase == InstallPhase::Prepared {
                    assert!(validated.get());
                    prepared.set(true);
                }
                for entry in &journal.entries {
                    assert_eq!(entry.backup.parent(), entry.target.parent());
                    assert!(
                        entry
                            .backup
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .contains(&journal.transaction_id)
                    );
                    paths.borrow_mut().push(entry.backup.clone());
                    if let Some(stage) = &entry.stage {
                        assert_eq!(stage.parent(), entry.target.parent());
                        assert!(
                            stage
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .contains(&journal.transaction_id)
                        );
                        paths.borrow_mut().push(stage.clone());
                    }
                }
            }
            if matches!(
                e.operation,
                InstallOperation::BackupRename | InstallOperation::InstallRename
            ) && e.edge == Edge::Before
            {
                assert!(validated.get() && prepared.get());
                let journal = read_journal(&e.journal).unwrap();
                let pending = journal
                    .pending
                    .as_ref()
                    .expect("durable write-ahead operation");
                assert_eq!(pending.operation, e.operation);
                assert_eq!(pending.entry, e.entry);
                let intents: Vec<_> = tree(&root)
                    .into_iter()
                    .filter(|(path, kind, _)| {
                        kind == "file"
                            && path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .ends_with(".intent")
                    })
                    .collect();
                assert!(
                    intents.len() >= journal.entries.len(),
                    "each target has a durable discovery intent"
                );
                for (_, _, bytes) in intents {
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(value["transaction_id"], journal.transaction_id);
                    assert_eq!(value["journal"], e.journal.display().to_string());
                    assert_eq!(
                        value["state"].as_str().unwrap().to_ascii_lowercase(),
                        "active"
                    );
                }
                swaps.set(swaps.get() + 1);
            }
            if matches_event(e, &InstallOperation::JournalRemove, &Edge::Before, None) {
                for (relative, kind, bytes) in tree(&root) {
                    if kind == "file"
                        && relative
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .ends_with(".intent")
                    {
                        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                        assert_eq!(
                            value["state"].as_str().unwrap().to_ascii_lowercase(),
                            "finished"
                        );
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(swaps.get(), 7);
    fixture.assert_new(VERSION);
    fixture.assert_finished();
    for path in paths.into_inner() {
        assert!(
            fs::symlink_metadata(&path).is_err(),
            "leftover {}",
            path.display()
        );
    }
}

struct Cut {
    ordinal: usize,
    label: String,
    committed: bool,
}
fn interruption_cuts(rollback: bool) -> Vec<Cut> {
    let mut fixture = Fixture::new("codex", true, true);
    let ordinal = Cell::new(0);
    let committed = Cell::new(false);
    let rolling_back = Cell::new(false);
    let seen = RefCell::new(BTreeSet::new());
    let cuts = RefCell::new(Vec::new());
    let result = fixture.install(&|e| {
        let n = ordinal.get();
        ordinal.set(n + 1);
        if e.operation == InstallOperation::JournalPersist
            && e.edge == Edge::After
            && e.phase == Some(InstallPhase::Committed)
        {
            committed.set(true);
        }
        if e.phase == Some(InstallPhase::RollingBack) {
            rolling_back.set(true);
        }
        let journal = read_journal(&e.journal).ok();
        let durable = journal.as_ref().map(|j| {
            format!(
                "{:?}/{:?}/{:?}",
                j.phase,
                j.pending,
                j.entries
                    .iter()
                    .map(|entry| &entry.state)
                    .collect::<Vec<_>>()
            )
        });
        // Collapse per-file stage checkpoints. Preserve every distinct phase,
        // pending operation, entry state and before/after operation boundary.
        let label = format!(
            "{:?}/{:?}/{:?}/{:?}/{durable:?}",
            e.operation, e.edge, e.entry, e.phase
        );
        let eligible = if rollback {
            rolling_back.get()
        } else {
            e.phase.is_some() || journal.is_some()
        };
        if eligible && seen.borrow_mut().insert(label.clone()) {
            cuts.borrow_mut().push(Cut {
                ordinal: n,
                label,
                committed: committed.get(),
            });
        }
        if rollback && matches_event(e, &InstallOperation::RecordCommit, &Edge::After, None) {
            return Err(InstallFault::Fail("start rollback".into()));
        }
        Ok(())
    });
    if rollback {
        failed(result, "start rollback");
    } else {
        result.unwrap();
    }
    let cuts = cuts.into_inner();
    assert!(!cuts.is_empty());
    for phase in if rollback {
        vec!["RollingBack"]
    } else {
        vec!["Prepared", "Swapping", "Recording", "Committed"]
    } {
        assert!(
            cuts.iter().any(|cut| cut.label.contains(phase)),
            "no {phase} checkpoint"
        );
    }
    for operation in if rollback {
        vec!["RemoveNew", "RestoreBackup", "RestoreRecord"]
    } else {
        vec![
            "BackupRename",
            "InstallRename",
            "RecordCommit",
            "IntentPersist",
            "JournalRemove",
        ]
    } {
        assert!(
            cuts.iter().any(|cut| cut.label.contains(operation)),
            "no {operation} checkpoint"
        );
    }
    cuts
}

#[test]
fn interrupt_every_distinct_phase_pending_and_operation_boundary_then_recover_from_disk() {
    let cuts = interruption_cuts(false);
    let completed_cuts = cuts.len();
    for cut in cuts {
        let mut fixture = Fixture::new("codex", true, true);
        let before = fixture.snapshot();
        let record = fixture.record();
        let ordinal = Cell::new(0);
        let hit = Cell::new(false);
        let result = fixture.install(&|_| {
            let n = ordinal.get();
            ordinal.set(n + 1);
            if n == cut.ordinal {
                hit.set(true);
                return Err(InstallFault::Interrupt);
            }
            Ok(())
        });
        assert!(hit.get(), "unreached {}", cut.label);
        assert!(
            matches!(result, Err(InstallFailure::Interrupted(_))),
            "{}: {result:?}",
            cut.label
        );
        let recovery = fixture
            .fresh_recover(&okay)
            .unwrap_or_else(|e| panic!("{}: {e:?}", cut.label));
        if cut.committed {
            assert!(
                matches!(
                    recovery,
                    RecoveryOutcome::Committed {
                        cleanup_pending: None,
                        ..
                    } | RecoveryOutcome::None
                ),
                "{}: {recovery:?}",
                cut.label
            );
            fixture.assert_new(VERSION);
        } else {
            assert!(
                matches!(
                    recovery,
                    RecoveryOutcome::RolledBack | RecoveryOutcome::None
                ),
                "{}: {recovery:?}",
                cut.label
            );
            fixture.assert_old(&before, &record);
        }
        fixture.assert_finished();
        assert!(matches!(
            fixture.fresh_recover(&okay).unwrap(),
            RecoveryOutcome::None
        ));
    }
    eprintln!("Step 10 install matrix completed cuts: {completed_cuts}");
}

#[test]
fn interrupt_rollback_before_and_after_every_distinct_restore_boundary_and_retry_idempotently() {
    let cuts = interruption_cuts(true);
    let completed_cuts = cuts.len();
    for cut in cuts {
        let mut fixture = Fixture::new("codex", true, true);
        let before = fixture.snapshot();
        let record = fixture.record();
        let ordinal = Cell::new(0);
        let hit = Cell::new(false);
        let result = fixture.install(&|e| {
            let n = ordinal.get();
            ordinal.set(n + 1);
            if n == cut.ordinal {
                hit.set(true);
                return Err(InstallFault::Interrupt);
            }
            if matches_event(e, &InstallOperation::RecordCommit, &Edge::After, None) {
                return Err(InstallFault::Fail("start rollback".into()));
            }
            Ok(())
        });
        assert!(hit.get(), "{}", cut.label);
        assert!(matches!(result, Err(InstallFailure::Interrupted(_))));
        fixture
            .fresh_recover(&okay)
            .unwrap_or_else(|e| panic!("{}: {e:?}", cut.label));
        fixture.assert_old(&before, &record);
        fixture.assert_finished();
        assert!(matches!(
            fixture.fresh_recover(&okay).unwrap(),
            RecoveryOutcome::None
        ));
    }
    eprintln!("Step 10 rollback matrix completed cuts: {completed_cuts}");
}

#[test]
fn fresh_recovery_itself_can_be_interrupted_after_restore_or_record_and_retried() {
    for operation in [
        InstallOperation::RemoveNew,
        InstallOperation::RestoreBackup,
        InstallOperation::RestoreRecord,
    ] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, true);
            let before = fixture.snapshot();
            let record = fixture.record();
            assert!(matches!(
                fixture.install(&|e| if matches_event(
                    e,
                    &InstallOperation::RecordCommit,
                    &Edge::After,
                    None
                ) {
                    Err(InstallFault::Interrupt)
                } else {
                    Ok(())
                }),
                Err(InstallFailure::Interrupted(_))
            ));
            let hit = Cell::new(false);
            let retry = fixture.fresh_recover(&|e| {
                if e.operation == operation && e.edge == edge && !hit.replace(true) {
                    return Err(InstallFault::Interrupt);
                }
                Ok(())
            });
            assert!(hit.get());
            assert!(matches!(retry, Err(InstallFailure::Interrupted(_))));
            fixture.fresh_recover(&okay).unwrap();
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
        }
    }
}

fn interrupt_after_swap(fixture: &mut Fixture) -> PathBuf {
    let result = fixture.install(&|e| {
        if matches_event(e, &InstallOperation::InstallRename, &Edge::After, Some(0)) {
            Err(InstallFault::Interrupt)
        } else {
            Ok(())
        }
    });
    let Err(InstallFailure::Interrupted(event)) = result else {
        panic!("{result:?}")
    };
    event.journal
}

#[test]
fn journal_corruption_and_untrusted_backup_paths_stop_without_mutating_evidence_or_other_files() {
    for damage in ["json", "format", "backup", "duplicate", "transaction"] {
        let mut fixture = Fixture::new("codex", true, true);
        let journal_path = interrupt_after_swap(&mut fixture);
        let outside = fixture.dir.path().join("do-not-touch");
        fs::write(&outside, b"sentinel").unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
        match damage {
            "json" => fs::write(&journal_path, b"{broken journal").unwrap(),
            "format" => {
                value["format_version"] = 999.into();
                fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "backup" => {
                value["entries"][0]["backup"] = outside.display().to_string().into();
                fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "duplicate" => {
                value["entries"][1]["target"] = value["entries"][0]["target"].clone();
                fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            _ => {
                value["transaction_id"] = "different-transaction".into();
                fs::write(&journal_path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        let before = tree(fixture.dir.path());
        let record = fixture.record();
        assert!(read_journal(&journal_path).is_err() || damage == "transaction");
        assert!(inspect_install(&fixture.paths).unwrap().is_some());
        assert_eq!(tree(fixture.dir.path()), before, "inspection is read-only");
        assert!(fixture.fresh_recover(&okay).is_err());
        assert_eq!(fixture.record(), record);
        assert_eq!(tree(fixture.dir.path()), before);
        assert_eq!(fs::read(&outside).unwrap(), b"sentinel");
    }
}

#[test]
fn active_intent_with_missing_journal_is_incomplete_and_finished_without_journal_is_healthy() {
    let mut fixture = Fixture::new("codex", true, true);
    let journal = interrupt_after_swap(&mut fixture);
    let original = fs::read(&journal).unwrap();
    fs::remove_file(&journal).unwrap();
    let before = tree(fixture.dir.path());
    assert!(inspect_install(&fixture.paths).unwrap().is_some());
    assert!(fixture.fresh_recover(&okay).is_err());
    assert_eq!(tree(fixture.dir.path()), before);
    fs::write(&journal, original).unwrap();
    fixture.fresh_recover(&okay).unwrap();
    fixture.assert_finished();
}

#[test]
fn finished_intents_with_remaining_committed_journal_still_drive_cleanup() {
    let mut fixture = Fixture::new("codex", true, true);
    assert!(matches!(
        fixture.install(&|e| if matches_event(
            e,
            &InstallOperation::JournalRemove,
            &Edge::Before,
            None
        ) {
            Err(InstallFault::Interrupt)
        } else {
            Ok(())
        }),
        Err(InstallFailure::Interrupted(_))
    ));
    let installed = fixture.record();
    assert!(inspect_install(&fixture.paths).unwrap().is_some());
    assert!(matches!(
        fixture.fresh_recover(&okay).unwrap(),
        RecoveryOutcome::Committed {
            cleanup_pending: None,
            ..
        }
    ));
    assert_eq!(fixture.record(), installed);
    fixture.assert_finished();
}

#[test]
fn missing_backup_and_changed_old_fingerprint_preserve_incomplete_evidence() {
    let mut fixture = Fixture::new("codex", true, true);
    let journal_path = interrupt_after_swap(&mut fixture);
    let journal = read_journal(&journal_path).unwrap();
    fs::remove_dir_all(&journal.entries[0].backup).unwrap();
    fs::write(
        journal.entries[0].target.join("SKILL.md"),
        b"neither old nor new",
    )
    .unwrap();
    let before = fixture.snapshot();
    let record = fixture.record();
    assert!(fixture.fresh_recover(&okay).is_err());
    assert!(journal_path.exists());
    assert!(inspect_install(&fixture.paths).unwrap().is_some());
    // Recovery may restore other entries, but it must not bless the damaged entry or discard its evidence.
    assert_eq!(tree(&journal.entries[0].target), before[0]);
    assert_eq!(fixture.record(), record);
}

#[test]
fn database_identity_mismatch_does_not_open_or_restore_the_recorded_database() {
    let mut fixture = Fixture::new("codex", true, true);
    let journal_path = interrupt_after_swap(&mut fixture);
    let journal = read_journal(&journal_path).unwrap();
    assert_eq!(
        journal.database_path,
        fixture.database.canonicalize().unwrap()
    );
    let different_db = fixture.dir.path().join("different.sqlite3");
    let mut conn = Connection::open(&different_db).unwrap();
    crate::db::migrate_to_head(&mut conn).unwrap();
    let before = fixture.snapshot();
    let record = fixture.record();
    let evidence = fs::read(&journal_path).unwrap();
    let error = recover_install(&mut conn, &fixture.paths, &hooks(&okay)).unwrap_err();
    match error {
        InstallFailure::Failed(e) => {
            let message = e.message();
            assert!(
                message.contains("database") || message.contains("DB"),
                "{message}"
            );
            assert!(message.contains("setup"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    fixture.assert_old(&before, &record);
    assert_eq!(fs::read(&journal_path).unwrap(), evidence);
    assert!(
        crate::broker::list_asset_installs(&conn)
            .unwrap()
            .is_empty()
    );
    fixture.fresh_recover(&okay).unwrap();
    fixture.assert_finished();
}

#[test]
fn fresh_recovery_restores_absent_rows_and_symlink_entry_identity_exactly() {
    for old_record in [false, true] {
        let mut fixture = Fixture::new("codex", false, old_record);
        let outside = fixture.dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"keep").unwrap();
        for entry in fixture.plan().entries {
            fs::create_dir_all(entry.target.parent().unwrap()).unwrap();
            symlink(&outside, &entry.target).unwrap();
        }
        let before = fixture.snapshot();
        let record = fixture.record();
        assert!(matches!(
            fixture.install(&|e| if matches_event(
                e,
                &InstallOperation::RecordCommit,
                &Edge::After,
                None
            ) {
                Err(InstallFault::Interrupt)
            } else {
                Ok(())
            }),
            Err(InstallFailure::Interrupted(_))
        ));
        fixture.fresh_recover(&okay).unwrap();
        fixture.assert_old(&before, &record);
        fixture.assert_finished();
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    }
}

#[test]
fn try_lock_contends_for_canonical_alias_targets_and_preserves_lock_inodes() {
    let fixture = Fixture::new("codex", true, true);
    let root = fixture.dir.path().canonicalize().unwrap();
    let alias = root.join("alias");
    symlink(root.join(".codex"), &alias).unwrap();
    let alias_paths = agent_paths(&|_| Some(alias.display().to_string()), &root, "codex").unwrap();
    let alias_plan = prepare_install(&alias_paths, "codex", VERSION).unwrap();
    let owner_plan = fixture.plan();
    let database = fixture.database.clone();
    let before = fixture.snapshot();
    let record = fixture.record();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner = scope.spawn(move || {
            let mut conn = Connection::open(database).unwrap();
            let once = Cell::new(false);
            execute_install(
                &mut conn,
                &owner_plan,
                &hooks(&|e| {
                    if matches_event(e, &InstallOperation::LockAcquire, &Edge::After, None)
                        && !once.replace(true)
                    {
                        held_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                    Ok(())
                }),
            )
        });
        held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let locks: Vec<_> = tree(&root)
            .into_iter()
            .filter_map(|(p, kind, _)| {
                let name = p.file_name()?.to_string_lossy();
                (kind == "file"
                    && name.starts_with(".cafleet-install-lock-")
                    && !name.ends_with(".intent"))
                .then(|| {
                    let path = root.join(p);
                    let metadata = fs::metadata(&path).unwrap();
                    (path, metadata.ino())
                })
            })
            .collect();
        assert!(!locks.is_empty());
        let mut conn = Connection::open(&fixture.database).unwrap();
        let result = execute_install(
            &mut conn,
            &alias_plan,
            &InstallHooks {
                lock_mode: LockMode::Try,
                checkpoint: &okay,
            },
        );
        assert!(matches!(result, Err(InstallFailure::Busy(_))), "{result:?}");
        fixture.assert_old(&before, &record);
        release_tx.send(()).unwrap();
        owner.join().unwrap().unwrap();
        for (path, inode) in locks {
            assert_eq!(fs::metadata(path).unwrap().ino(), inode);
        }
    });
    fixture.assert_new(VERSION);
    fixture.assert_finished();
}

#[test]
fn wait_lock_blocks_until_owner_releases_then_completes_with_an_independent_connection() {
    let fixture = Fixture::new("codex", true, true);
    let owner_plan = fixture.plan();
    let waiter_plan = fixture.plan();
    let owner_db = fixture.database.clone();
    let waiter_db = fixture.database.clone();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner = scope.spawn(move || {
            let mut conn = Connection::open(owner_db).unwrap();
            let once = Cell::new(false);
            execute_install(
                &mut conn,
                &owner_plan,
                &hooks(&|e| {
                    if matches_event(e, &InstallOperation::LockAcquire, &Edge::After, None)
                        && !once.replace(true)
                    {
                        held_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                    Ok(())
                }),
            )
            .unwrap();
        });
        held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let waiter = scope.spawn(move || {
            let mut conn = Connection::open(waiter_db).unwrap();
            let once = Cell::new(false);
            let result = execute_install(
                &mut conn,
                &waiter_plan,
                &hooks(&|e| {
                    if matches_event(e, &InstallOperation::LockAcquire, &Edge::Before, None)
                        && !once.replace(true)
                    {
                        attempt_tx.send(()).unwrap();
                    }
                    Ok(())
                }),
            );
            done_tx.send(result.is_ok()).unwrap();
            result.unwrap();
        });
        attempt_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_tx.send(()).unwrap();
        assert!(done_rx.recv_timeout(Duration::from_secs(10)).unwrap());
        owner.join().unwrap();
        waiter.join().unwrap();
    });
    fixture.assert_new(VERSION);
    fixture.assert_finished();
}

#[test]
fn distinct_identities_sharing_opencode_skills_discover_old_journal_and_do_not_relabel_old_row() {
    let mut fixture = Fixture::new("opencode", true, true);
    let previous = fixture.record();
    let root = fixture.dir.path().canonicalize().unwrap();
    let alt_base = root.join("other-opencode");
    let alt = agent_paths(&|_| Some(alt_base.display().to_string()), &root, "opencode").unwrap();
    assert_eq!(alt.skills_dir, fixture.paths.skills_dir);
    assert_ne!(alt.identity, fixture.paths.identity);
    let journal = interrupt_after_swap(&mut fixture);
    assert_eq!(inspect_install(&alt).unwrap().unwrap().journal, journal);
    let alt_plan = prepare_install(&alt, "opencode", "alternate-version").unwrap();
    let outcome = execute_install(&mut fixture.conn, &alt_plan, &hooks(&okay)).unwrap();
    assert!(!outcome.recovered_only);
    assert_eq!(outcome.installed_record.path, alt.identity);
    assert_eq!(fixture.record(), previous);
    assert_eq!(
        row(&fixture.conn, &alt, "opencode").unwrap().2,
        "alternate-version"
    );
    assert_eq!(
        fs::read(fixture.paths.preset.as_ref().unwrap().1.as_path()).unwrap(),
        b"old preset bytes"
    );
    assert_eq!(
        fs::read(&alt.preset.as_ref().unwrap().1).unwrap(),
        lookup(PRESETS, "opencode/cafleet.md").unwrap()
    );
    fixture.assert_finished();
    assert!(inspect_install(&alt).unwrap().is_none());
}

#[test]
fn distinct_identity_committed_cleanup_is_completed_before_installing_current_identity() {
    let mut fixture = Fixture::new("opencode", true, true);
    let root = fixture.dir.path().canonicalize().unwrap();
    let alt_base = root.join("alternate");
    let alt = agent_paths(&|_| Some(alt_base.display().to_string()), &root, "opencode").unwrap();
    assert!(matches!(
        fixture.install(&|e| if matches_event(
            e,
            &InstallOperation::CleanupBackup,
            &Edge::Before,
            Some(0)
        ) {
            Err(InstallFault::Interrupt)
        } else {
            Ok(())
        }),
        Err(InstallFailure::Interrupted(_))
    ));
    let first_record = fixture.record();
    let alt_plan = prepare_install(&alt, "opencode", "alternate-version").unwrap();
    let hit = Cell::new(false);
    let blocked = execute_install(
        &mut fixture.conn,
        &alt_plan,
        &hooks(&|e| {
            if e.operation == InstallOperation::CleanupBackup
                && e.edge == Edge::Before
                && !hit.replace(true)
            {
                return Err(InstallFault::Fail("foreign cleanup blocked".into()));
            }
            Ok(())
        }),
    );
    assert!(hit.get());
    assert!(matches!(blocked, Err(InstallFailure::Failed(_))));
    assert!(row(&fixture.conn, &alt, "opencode").is_none());
    assert_eq!(fixture.record(), first_record);
    let outcome = execute_install(&mut fixture.conn, &alt_plan, &hooks(&okay)).unwrap();
    assert!(!outcome.recovered_only);
    assert_eq!(outcome.installed_record.path, alt.identity);
    assert_eq!(fixture.record(), first_record);
    fixture.assert_finished();
}

#[test]
fn incomplete_facts_precede_version_and_absent_record_and_inspection_never_writes() {
    for old_record in [false, true] {
        let mut fixture = Fixture::new("codex", true, old_record);
        if old_record {
            fixture
                .conn
                .execute("UPDATE asset_installs SET cafleet_version=?1", [VERSION])
                .unwrap();
        }
        interrupt_after_swap(&mut fixture);
        let root = fixture.dir.path().canonicalize().unwrap();
        let before = tree(&root);
        for conn in [Some(&fixture.conn), None] {
            let report =
                diagnosis::diagnose_assets(conn, &|_| None, &root, VERSION, AssetMode::Report)
                    .unwrap();
            let AssetState::Incomplete {
                install, recovery, ..
            } = &report.agents[1].state
            else {
                panic!("{:?}", report.agents[1].state)
            };
            assert_eq!(install.is_some(), conn.is_some() && old_record);
            assert!(recovery.journal.exists());
            let message = format!(
                "incomplete assets install at {}; run 'cafleet setup' to recover",
                fixture.paths.identity
            );
            assert_eq!(
                crate::cli::helpers::stale_assets_guard(&report, VERSION)
                    .unwrap_err()
                    .message(),
                message
            );
            let wire = crate::presentation::doctor_assets(&report, VERSION);
            assert_eq!(wire["ok"], false);
            assert_eq!(wire["agents"][1]["state"], "incomplete");
            assert_eq!(wire["agents"][1]["error"], message);
            assert_eq!(
                wire["agents"][1]["recorded_version"],
                if conn.is_some() && old_record {
                    serde_json::json!(VERSION)
                } else {
                    serde_json::Value::Null
                }
            );
            assert_eq!(
                wire["agents"][1]["installed_at"],
                if conn.is_some() && old_record {
                    serde_json::json!(OLD_TIME)
                } else {
                    serde_json::Value::Null
                }
            );
            assert_eq!(
                wire["agents"][1]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                vec![
                    "coding_agent",
                    "path",
                    "source",
                    "recorded_version",
                    "installed_at",
                    "state",
                    "error"
                ]
            );
            assert_eq!(report.agents.len(), 3);
        }
        assert_eq!(tree(&root), before);
    }
}

#[test]
fn invalid_path_has_guard_priority_over_another_agents_incomplete_evidence() {
    let mut fixture = Fixture::new("claude", true, true);
    interrupt_after_swap(&mut fixture);
    let root = fixture.dir.path().canonicalize().unwrap();
    let before = tree(&root);
    let env = |variable: &str| (variable == "CODEX_HOME").then(|| "relative/path".to_string());
    let error =
        diagnosis::diagnose_assets(Some(&fixture.conn), &env, &root, VERSION, AssetMode::Guard)
            .unwrap_err();
    assert_eq!(
        error.message(),
        "CODEX_HOME must be an absolute path (got 'relative/path')"
    );
    let report = diagnosis::diagnose_assets(None, &env, &root, VERSION, AssetMode::Report).unwrap();
    assert!(matches!(
        report.agents[0].state,
        AssetState::Incomplete { .. }
    ));
    assert!(matches!(
        report.agents[1].state,
        AssetState::PathError { .. }
    ));
    assert_eq!(tree(&root), before);
}

#[test]
fn missing_asset_table_and_bad_schema_do_not_hide_read_only_filesystem_evidence() {
    let mut fixture = Fixture::new("codex", true, true);
    interrupt_after_swap(&mut fixture);
    fixture.conn.execute_batch("DROP TABLE asset_installs; DROP TABLE refinery_schema_history; CREATE TABLE refinery_schema_history(wrong_column);").unwrap();
    let root = fixture.dir.path().canonicalize().unwrap();
    let before = tree(&root);
    let schema = diagnosis::classify_schema(&fixture.conn, crate::db::head_version());
    assert!(matches!(schema, diagnosis::SchemaState::Unreachable { .. }));
    for conn in [Some(&fixture.conn), None] {
        let report =
            diagnosis::diagnose_assets(conn, &|_| None, &root, VERSION, AssetMode::Report).unwrap();
        assert!(matches!(
            report.agents[1].state,
            AssetState::Incomplete { install: None, .. }
        ));
        assert_eq!(
            crate::presentation::doctor_assets(&report, VERSION)["agents"][1]["state"],
            "incomplete"
        );
    }
    assert_eq!(tree(&root), before);
}

#[test]
fn preset_stage_and_backup_share_its_target_parent_without_claiming_a_second_device() {
    let mut fixture = Fixture::new("opencode", true, true);
    let journal_path = interrupt_after_swap(&mut fixture);
    let journal = read_journal(&journal_path).unwrap();
    let preset = &journal.entries[2];
    let skill = &journal.entries[0];
    assert_ne!(preset.target.parent(), skill.target.parent());
    let parent = preset.target.parent().unwrap();
    let device = fs::metadata(parent).unwrap().dev();
    assert_eq!(preset.stage.as_ref().unwrap().parent(), Some(parent));
    assert_eq!(preset.backup.parent(), Some(parent));
    assert_eq!(
        fs::metadata(preset.stage.as_ref().unwrap()).unwrap().dev(),
        device
    );
    // Both directories normally live on one fixture device: this verifies placement,
    // not a cross-device exchange, OS process death, power loss or failed fsync.
    fixture.fresh_recover(&okay).unwrap();
    fixture.assert_finished();
}

#[test]
fn cleanup_recovery_can_be_interrupted_again_without_changing_new_record_or_reinstalling() {
    let mut fixture = Fixture::new("codex", true, true);
    assert!(matches!(
        fixture.install(&|e| {
            if e.operation == InstallOperation::JournalPersist
                && e.edge == Edge::After
                && e.phase == Some(InstallPhase::Committed)
            {
                Err(InstallFault::Interrupt)
            } else {
                Ok(())
            }
        }),
        Err(InstallFailure::Interrupted(_))
    ));
    let record = fixture.record();
    let hit = Cell::new(false);
    let interrupted = fixture.fresh_recover(&|e| {
        assert_ne!(
            e.operation,
            InstallOperation::RecordCommit,
            "cleanup does not reinstall"
        );
        if e.operation == InstallOperation::CleanupBackup
            && e.edge == Edge::After
            && !hit.replace(true)
        {
            return Err(InstallFault::Interrupt);
        }
        Ok(())
    });
    assert!(hit.get());
    assert!(matches!(interrupted, Err(InstallFailure::Interrupted(_))));
    fixture.assert_new(VERSION);
    assert_eq!(fixture.record(), record);
    assert!(matches!(
        fixture.fresh_recover(&okay).unwrap(),
        RecoveryOutcome::Committed {
            cleanup_pending: None,
            ..
        }
    ));
    fixture.assert_new(VERSION);
    assert_eq!(fixture.record(), record);
    fixture.assert_finished();
}

#[test]
fn try_lock_conflicts_across_shared_skills_identities_but_unrelated_target_install_proceeds() {
    let fixture = Fixture::new("opencode", true, true);
    let root = fixture.dir.path().canonicalize().unwrap();
    let alternate = root.join("alternate-opencode");
    let paths = agent_paths(
        &|_| Some(alternate.display().to_string()),
        &root,
        "opencode",
    )
    .unwrap();
    let competing_plan = prepare_install(&paths, "opencode", VERSION).unwrap();
    let separate_root = root.join("separate-home");
    let separate_paths = agent_paths(&|_| None, &separate_root, "claude").unwrap();
    let separate_plan = prepare_install(&separate_paths, "claude", VERSION).unwrap();
    let owner_plan = fixture.plan();
    let database = fixture.database.clone();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner = scope.spawn(move || {
            let mut conn = Connection::open(database).unwrap();
            let once = Cell::new(false);
            execute_install(
                &mut conn,
                &owner_plan,
                &hooks(&|e| {
                    if matches_event(e, &InstallOperation::LockAcquire, &Edge::After, None)
                        && !once.replace(true)
                    {
                        held_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    }
                    Ok(())
                }),
            )
            .unwrap();
        });
        held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut conn = Connection::open(&fixture.database).unwrap();
        let try_hooks = InstallHooks {
            lock_mode: LockMode::Try,
            checkpoint: &okay,
        };
        assert!(matches!(
            execute_install(&mut conn, &competing_plan, &try_hooks),
            Err(InstallFailure::Busy(_))
        ));
        assert!(row(&conn, &paths, "opencode").is_none());
        let separate = execute_install(&mut conn, &separate_plan, &try_hooks).unwrap();
        assert_eq!(separate.installed_record.coding_agent, "claude");
        release_tx.send(()).unwrap();
        owner.join().unwrap();
    });
    fixture.assert_new(VERSION);
    fixture.assert_finished();
}

#[test]
fn partial_swap_rollback_stage_cleanup_failure_or_interrupt_keeps_evidence_until_retry() {
    for interrupt in [false, true] {
        for edge in [Edge::Before, Edge::After] {
            let mut fixture = Fixture::new("codex", true, true);
            let before = fixture.snapshot();
            let record = fixture.record();
            let primary = Cell::new(false);
            let secondary = Cell::new(false);
            let result = fixture.install(&|e| {
                if matches_event(e, &InstallOperation::InstallRename, &Edge::After, Some(0))
                    && !primary.replace(true)
                {
                    return Err(InstallFault::Fail("partial swap failure".into()));
                }
                if e.operation == InstallOperation::CleanupStage
                    && e.edge == edge
                    && !secondary.replace(true)
                {
                    return Err(if interrupt {
                        InstallFault::Interrupt
                    } else {
                        InstallFault::Fail("stage cleanup failure".into())
                    });
                }
                Ok(())
            });
            assert!(primary.get() && secondary.get());
            if interrupt {
                assert!(matches!(result, Err(InstallFailure::Interrupted(_))));
            } else {
                failed(result, "partial swap failure");
            }
            assert!(inspect_install(&fixture.paths).unwrap().is_some());
            fixture.fresh_recover(&okay).unwrap();
            fixture.assert_old(&before, &record);
            fixture.assert_finished();
        }
    }
}

// Reinstall coverage starts after a real successful install, unlike the initial-install matrices.
fn reinstall_intents(root: &Path) -> Vec<(PathBuf, serde_json::Value)> {
    tree(root)
        .into_iter()
        .filter(|(relative, kind, _)| {
            kind == "file"
                && relative
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".intent")
        })
        .map(|(relative, _, bytes)| (root.join(relative), serde_json::from_slice(&bytes).unwrap()))
        .collect()
}

struct ReinstallFixture {
    fixture: Fixture,
    entries: Vec<Tree>,
    record: Option<Row>,
    intent_paths: Vec<PathBuf>,
    old_transaction: String,
}
impl ReinstallFixture {
    fn new(agent: &'static str) -> Self {
        let mut fixture = Fixture::new(agent, true, true);
        fixture.install(&okay).unwrap();
        fixture.assert_new(VERSION);
        fixture.assert_finished();
        let intents = reinstall_intents(fixture.dir.path());
        assert!(intents.len() >= 4);
        let old_transaction = intents[0].1["transaction_id"].as_str().unwrap().to_string();
        for (_, value) in &intents {
            assert_eq!(value["state"], "Finished");
            assert_eq!(value["transaction_id"], old_transaction);
        }
        // Keep the installed version/row while making local bytes observably different.
        for entry in fixture.plan().entries {
            match entry.kind {
                EntryKind::Skill(name) => {
                    fs::write(
                        entry.target.join("SKILL.md"),
                        format!("local {name} changes"),
                    )
                    .unwrap();
                    fs::write(entry.target.join("local-only.txt"), b"retain on rollback").unwrap();
                }
                EntryKind::Preset => fs::write(&entry.target, b"local preset changes").unwrap(),
                EntryKind::ObsoleteResearch => {
                    fs::create_dir_all(&entry.target).unwrap();
                    fs::write(entry.target.join("local.txt"), b"local research").unwrap();
                }
            }
        }
        let entries = fixture.snapshot();
        let record = fixture.record();
        assert_eq!(record.as_ref().unwrap().2, VERSION);
        Self {
            fixture,
            entries,
            record,
            intent_paths: intents.into_iter().map(|(path, _)| path).collect(),
            old_transaction,
        }
    }
    fn assert_restored(&self) {
        self.fixture.assert_old(&self.entries, &self.record);
        self.fixture.assert_finished();
        assert!(matches!(
            self.fixture.fresh_recover(&okay).unwrap(),
            RecoveryOutcome::None
        ));
    }
    fn interrupt(&mut self, boundary: &ReinstallBoundary) -> PathBuf {
        let seen = Cell::new(0);
        let hit = Cell::new(false);
        let result = self.fixture.install(&|event| {
            if boundary.matches(event, &seen) && !hit.replace(true) {
                return Err(InstallFault::Interrupt);
            }
            Ok(())
        });
        assert!(hit.get(), "{boundary:?}");
        let Err(InstallFailure::Interrupted(event)) = result else {
            panic!("{result:?}")
        };
        let journal = read_journal(&event.journal).unwrap();
        assert_eq!(journal.phase, InstallPhase::Prepared);
        assert!(journal.pending.is_none());
        assert!(
            journal
                .entries
                .iter()
                .all(|entry| entry.state == EntryState::Original)
        );
        assert_ne!(journal.transaction_id, self.old_transaction);
        self.fixture.assert_old(&self.entries, &self.record);
        event.journal
    }
    fn assert_read_only_incomplete(&self) {
        let before = tree(self.fixture.dir.path());
        assert!(inspect_install(&self.fixture.paths).unwrap().is_some());
        let root = self.fixture.dir.path().canonicalize().unwrap();
        let report = diagnosis::diagnose_assets(
            Some(&self.fixture.conn),
            &|_| None,
            &root,
            VERSION,
            AssetMode::Report,
        )
        .unwrap();
        let agent = report
            .agents
            .iter()
            .find(|entry| entry.coding_agent == self.fixture.agent)
            .unwrap();
        assert!(matches!(&agent.state, AssetState::Incomplete { .. }));
        assert!(
            crate::cli::helpers::stale_assets_guard(&report, VERSION)
                .unwrap_err()
                .message()
                .contains("incomplete assets install")
        );
        assert_eq!(tree(self.fixture.dir.path()), before);
    }
}

#[derive(Debug)]
enum ReinstallBoundary {
    Prepared,
    Intent { edge: Edge, ordinal: usize },
}
impl ReinstallBoundary {
    fn matches(&self, event: &InstallEvent, seen: &Cell<usize>) -> bool {
        match self {
            Self::Prepared => {
                event.operation == InstallOperation::JournalPersist
                    && event.edge == Edge::After
                    && event.phase == Some(InstallPhase::Prepared)
            }
            Self::Intent { edge, ordinal } => {
                if event.operation != InstallOperation::IntentPersist
                    || &event.edge != edge
                    || event.phase != Some(InstallPhase::Prepared)
                {
                    return false;
                }
                let current = seen.get();
                seen.set(current + 1);
                current == *ordinal
            }
        }
    }
}
fn reinstall_boundaries() -> Vec<ReinstallBoundary> {
    vec![
        ReinstallBoundary::Prepared,
        ReinstallBoundary::Intent {
            edge: Edge::Before,
            ordinal: 0,
        },
        ReinstallBoundary::Intent {
            edge: Edge::After,
            ordinal: 0,
        },
        ReinstallBoundary::Intent {
            edge: Edge::Before,
            ordinal: 2,
        },
        ReinstallBoundary::Intent {
            edge: Edge::After,
            ordinal: 2,
        },
    ]
}
fn reinstall_assert_active(paths: &[PathBuf], journal: &InstallJournal) {
    for path in paths {
        let intent: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            intent["state"],
            "Active",
            "{} must normalize before rollback",
            path.display()
        );
        assert_eq!(intent["transaction_id"], journal.transaction_id);
    }
}
fn reinstall_assert_before_rollback(event: &InstallEvent, paths: &[PathBuf]) {
    if event.operation == InstallOperation::JournalPersist
        && event.edge == Edge::Before
        && event.phase == Some(InstallPhase::RollingBack)
    {
        let persisted = read_journal(&event.journal).unwrap();
        // Later rollback progress also persists RollingBack; the transition from
        // Prepared is the point at which normalization must already be durable.
        if persisted.phase == InstallPhase::Prepared {
            assert!(persisted.pending.is_none());
            assert!(
                persisted
                    .entries
                    .iter()
                    .all(|entry| entry.state == EntryState::Original)
            );
            reinstall_assert_active(paths, &persisted);
        }
    }
}

#[test]
fn reinstall_prepared_interrupt_with_old_finished_intents_restores_changed_same_version_assets() {
    let mut fixture = ReinstallFixture::new("codex");
    fixture.interrupt(&ReinstallBoundary::Prepared);
    let intents = reinstall_intents(fixture.fixture.dir.path());
    assert!(intents.iter().all(|(_, value)| value["state"] == "Finished"
        && value["transaction_id"] == fixture.old_transaction));
    fixture.assert_read_only_incomplete();
    assert!(matches!(
        fixture
            .fixture
            .fresh_recover(&|event| {
                reinstall_assert_before_rollback(event, &fixture.intent_paths);
                Ok(())
            })
            .unwrap(),
        RecoveryOutcome::RolledBack
    ));
    fixture.assert_restored();
}

#[test]
fn reinstall_active_intent_first_and_middle_interruptions_recover_mixed_finished_and_active() {
    let mut completed = 0;
    for boundary in reinstall_boundaries().into_iter().skip(1) {
        let mut fixture = ReinstallFixture::new("codex");
        fixture.interrupt(&boundary);
        fixture.assert_read_only_incomplete();
        assert!(matches!(
            fixture
                .fixture
                .fresh_recover(&|event| {
                    reinstall_assert_before_rollback(event, &fixture.intent_paths);
                    Ok(())
                })
                .unwrap(),
            RecoveryOutcome::RolledBack
        ));
        fixture.assert_restored();
        completed += 1;
    }
    assert_eq!(completed, 4);
    eprintln!("Step 10 reinstall active-boundary completed cases: {completed}");
}

#[test]
fn reinstall_normal_fail_at_prepared_or_active_boundary_normalizes_before_same_call_rollback() {
    let mut completed = 0;
    for boundary in reinstall_boundaries() {
        let mut fixture = ReinstallFixture::new("codex");
        let seen = Cell::new(0);
        let hit = Cell::new(false);
        let paths = &fixture.intent_paths;
        let result = fixture.fixture.install(&|event| {
            if !hit.get() && boundary.matches(event, &seen) {
                hit.set(true);
                return Err(InstallFault::Fail("reinstall boundary failure".into()));
            }
            if hit.get() {
                reinstall_assert_before_rollback(event, paths);
            }
            Ok(())
        });
        assert!(hit.get(), "{boundary:?}");
        failed(result, "reinstall boundary failure");
        fixture.assert_restored();
        completed += 1;
    }
    assert_eq!(completed, 5);
    eprintln!("Step 10 reinstall same-call rollback completed cases: {completed}");
}

#[test]
fn reinstall_fresh_recovery_normalization_can_fail_or_interrupt_before_and_after_intent_updates() {
    let mut completed = 0;
    for interrupt in [false, true] {
        for boundary in reinstall_boundaries().into_iter().skip(1) {
            let mut fixture = ReinstallFixture::new("codex");
            let location = fixture.interrupt(&ReinstallBoundary::Prepared);
            let seen = Cell::new(0);
            let hit = Cell::new(false);
            let result = fixture.fixture.fresh_recover(&|event| {
                if boundary.matches(event, &seen) && !hit.replace(true) {
                    return Err(if interrupt {
                        InstallFault::Interrupt
                    } else {
                        InstallFault::Fail("normalization failed".into())
                    });
                }
                Ok(())
            });
            assert!(
                hit.get(),
                "recovery must reach normalization {boundary:?}: {result:?}"
            );
            if interrupt {
                assert!(matches!(result, Err(InstallFailure::Interrupted(_))));
            } else {
                assert!(
                    matches!(result, Err(InstallFailure::Failed(ref e)) if e.message().contains("normalization failed"))
                );
            }
            let journal = read_journal(&location).unwrap();
            assert_eq!(journal.phase, InstallPhase::Prepared);
            assert!(journal.pending.is_none());
            assert!(
                journal
                    .entries
                    .iter()
                    .all(|entry| entry.state == EntryState::Original)
            );
            fixture.assert_read_only_incomplete();
            fixture
                .fixture
                .fresh_recover(&|event| {
                    reinstall_assert_before_rollback(event, &fixture.intent_paths);
                    Ok(())
                })
                .unwrap();
            fixture.assert_restored();
            completed += 1;
        }
    }
    assert_eq!(completed, 8);
    eprintln!("Step 10 reinstall normalization retry completed cases: {completed}");
}

#[test]
fn reinstall_fresh_recovery_rolling_back_persist_interrupt_keeps_only_current_active_intents() {
    for initial in [
        ReinstallBoundary::Prepared,
        ReinstallBoundary::Intent {
            edge: Edge::After,
            ordinal: 2,
        },
    ] {
        let mut fixture = ReinstallFixture::new("codex");
        let location = fixture.interrupt(&initial);
        let hit = Cell::new(false);
        let result = fixture.fixture.fresh_recover(&|event| {
            reinstall_assert_before_rollback(event, &fixture.intent_paths);
            if event.operation == InstallOperation::JournalPersist
                && event.edge == Edge::After
                && event.phase == Some(InstallPhase::RollingBack)
                && !hit.replace(true)
            {
                reinstall_assert_active(
                    &fixture.intent_paths,
                    &read_journal(&event.journal).unwrap(),
                );
                return Err(InstallFault::Interrupt);
            }
            Ok(())
        });
        assert!(hit.get(), "{result:?}");
        assert!(matches!(result, Err(InstallFailure::Interrupted(_))));
        assert_eq!(
            read_journal(&location).unwrap().phase,
            InstallPhase::RollingBack
        );
        fixture.assert_read_only_incomplete();
        fixture.fixture.fresh_recover(&okay).unwrap();
        fixture.assert_restored();
    }
}

#[test]
fn reinstall_same_call_failure_then_normalization_or_rolling_back_interrupt_recovers_fresh() {
    let mut completed = 0;
    for initial in reinstall_boundaries() {
        for during_normalization in [false, true] {
            let mut fixture = ReinstallFixture::new("codex");
            let seen = Cell::new(0);
            let primary = Cell::new(false);
            let interrupted = Cell::new(false);
            let normalization_seen = Cell::new(0);
            let paths = &fixture.intent_paths;
            let result = fixture.fixture.install(&|event| {
                if !primary.get() && initial.matches(event, &seen) {
                    primary.set(true);
                    return Err(InstallFault::Fail("reinstall primary failure".into()));
                }
                if primary.get() {
                    reinstall_assert_before_rollback(event, paths);
                    let cut = if during_normalization {
                        ReinstallBoundary::Intent {
                            edge: Edge::After,
                            ordinal: 2,
                        }
                        .matches(event, &normalization_seen)
                    } else {
                        event.operation == InstallOperation::JournalPersist
                            && event.edge == Edge::After
                            && event.phase == Some(InstallPhase::RollingBack)
                    };
                    if cut && !interrupted.replace(true) {
                        return Err(InstallFault::Interrupt);
                    }
                }
                Ok(())
            });
            assert!(
                primary.get() && interrupted.get(),
                "{initial:?}: {result:?}"
            );
            assert!(matches!(result, Err(InstallFailure::Interrupted(_))));
            fixture.assert_read_only_incomplete();
            fixture.fixture.fresh_recover(&okay).unwrap();
            fixture.assert_restored();
            completed += 1;
        }
    }
    assert_eq!(completed, 10);
    eprintln!("Step 10 reinstall failure/reinterrupt completed cases: {completed}");
}

#[test]
fn reinstall_shared_skills_identity_discovers_and_recovers_old_finished_new_prepared_state() {
    let mut fixture = ReinstallFixture::new("opencode");
    let location = fixture.interrupt(&ReinstallBoundary::Prepared);
    let root = fixture.fixture.dir.path().canonicalize().unwrap();
    let alternate = root.join("alternate-opencode");
    let paths = agent_paths(
        &|_| Some(alternate.display().to_string()),
        &root,
        "opencode",
    )
    .unwrap();
    assert_eq!(paths.skills_dir, fixture.fixture.paths.skills_dir);
    let before = tree(&root);
    assert_eq!(inspect_install(&paths).unwrap().unwrap().journal, location);
    assert_eq!(tree(&root), before);
    let mut conn = Connection::open(&fixture.fixture.database).unwrap();
    assert!(matches!(
        recover_install(
            &mut conn,
            &paths,
            &hooks(&|event| {
                reinstall_assert_before_rollback(event, &fixture.intent_paths);
                Ok(())
            })
        )
        .unwrap(),
        RecoveryOutcome::RolledBack
    ));
    assert!(row(&conn, &paths, "opencode").is_none());
    fixture.assert_restored();
    assert!(inspect_install(&paths).unwrap().is_none());
}

#[test]
fn reinstall_finished_exception_rejects_active_phase_pending_state_and_unverified_old_evidence() {
    for damage in [
        "active",
        "swapping",
        "recording",
        "rolling-back",
        "committed",
        "pending",
        "pending-record",
        "entry-state",
        "installed-state",
        "restored-state",
        "path",
        "corrupt",
        "fingerprint",
        "backup",
        "row",
        "database",
    ] {
        let mut fixture = ReinstallFixture::new("codex");
        let location = fixture.interrupt(&ReinstallBoundary::Prepared);
        let journal = read_journal(&location).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&location).unwrap()).unwrap();
        match damage {
            "active" => {
                let path = &fixture.intent_paths[0];
                let mut intent: serde_json::Value =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                intent["state"] = "Active".into();
                fs::write(path, serde_json::to_vec(&intent).unwrap()).unwrap();
            }
            "swapping" => value["phase"] = "Swapping".into(),
            "recording" => value["phase"] = "Recording".into(),
            "rolling-back" => value["phase"] = "RollingBack".into(),
            "committed" => value["phase"] = "Committed".into(),
            "pending" => {
                value["pending"] = serde_json::json!({"operation":"BackupRename","entry":0})
            }
            "pending-record" => {
                value["pending"] = serde_json::json!({"operation":"RestoreRecord","entry":null})
            }
            "entry-state" => value["entries"][0]["state"] = "BackedUp".into(),
            "installed-state" => value["entries"][2]["state"] = "Installed".into(),
            "restored-state" => value["entries"][3]["state"] = "Restored".into(),
            "path" => {
                value["entries"][0]["backup"] = fixture
                    .fixture
                    .dir
                    .path()
                    .join("untrusted-backup")
                    .display()
                    .to_string()
                    .into()
            }
            "corrupt" => {}
            "fingerprint" => fs::write(
                journal.entries[0].target.join("SKILL.md"),
                b"changed after Prepared",
            )
            .unwrap(),
            "backup" => {
                fs::create_dir_all(&journal.entries[0].backup).unwrap();
                fs::write(journal.entries[0].backup.join("keep"), b"unverified backup").unwrap();
            }
            "row" => {
                fixture
                    .fixture
                    .conn
                    .execute(
                        "UPDATE asset_installs SET installed_at='changed after Prepared'",
                        [],
                    )
                    .unwrap();
            }
            "database" => {
                let other = fixture.fixture.dir.path().join("other.sqlite3");
                let mut conn = Connection::open(&other).unwrap();
                crate::db::migrate_to_head(&mut conn).unwrap();
                value["database_path"] = other.canonicalize().unwrap().display().to_string().into();
            }
            _ => unreachable!(),
        }
        fs::write(
            &location,
            if damage == "corrupt" {
                b"{broken".to_vec()
            } else {
                serde_json::to_vec(&value).unwrap()
            },
        )
        .unwrap();
        let before = tree(fixture.fixture.dir.path());
        let record = fixture.fixture.record();
        fixture.assert_read_only_incomplete();
        let result = fixture.fixture.fresh_recover(&okay);
        assert!(
            matches!(result, Err(InstallFailure::Failed(_))),
            "must reject {damage}: {result:?}"
        );
        assert_eq!(
            tree(fixture.fixture.dir.path()),
            before,
            "preserve {damage} evidence before normalization"
        );
        assert_eq!(fixture.fixture.record(), record);
    }
}

#[test]
fn reinstall_finished_pointer_to_valid_journal_without_its_lock_key_is_rejected() {
    let mut fixture = ReinstallFixture::new("codex");
    fixture.interrupt(&ReinstallBoundary::Prepared);
    let mut foreign = ReinstallFixture::new("codex");
    let foreign_location = foreign.interrupt(&ReinstallBoundary::Prepared);
    assert!(read_journal(&foreign_location).is_ok());
    let path = &fixture.intent_paths[0];
    let mut intent: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    intent["journal"] = foreign_location.display().to_string().into();
    fs::write(path, serde_json::to_vec(&intent).unwrap()).unwrap();
    let before = tree(fixture.fixture.dir.path());
    let foreign_before = tree(foreign.fixture.dir.path());
    fixture.assert_read_only_incomplete();
    assert!(matches!(
        fixture.fixture.fresh_recover(&okay),
        Err(InstallFailure::Failed(_))
    ));
    assert_eq!(tree(fixture.fixture.dir.path()), before);
    assert_eq!(tree(foreign.fixture.dir.path()), foreign_before);
    fixture
        .fixture
        .assert_old(&fixture.entries, &fixture.record);
    foreign
        .fixture
        .assert_old(&foreign.entries, &foreign.record);
}
