use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use super::*;

const SPOOL_CANARY: &[u8] = b"private-recovery-diagnosis-canary:not-a-complete-valid-frame";

#[derive(Debug, PartialEq)]
struct TableSnapshot {
    name: String,
    rows: Vec<Vec<rusqlite::types::Value>>,
}

fn registry_snapshot(conn: &Connection) -> Vec<TableSnapshot> {
    let names = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let mut statement = conn
                .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY rowid"))
                .unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width)
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            TableSnapshot { name, rows }
        })
        .collect()
}

#[derive(Debug, PartialEq)]
struct FileSnapshot {
    path: PathBuf,
    device: u64,
    inode: u64,
    mode: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
struct OwnedArtifacts {
    directories: Vec<(PathBuf, u64, u64, u32)>,
    files: Vec<FileSnapshot>,
}

fn owned_snapshot(fixture: &Fixture, artifacts: &RunArtifacts, held: &RunLock) -> OwnedArtifacts {
    held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
    let directories = [&artifacts.paths.namespace_dir, &artifacts.paths.run_dir]
        .into_iter()
        .map(|path| {
            let metadata = std::fs::symlink_metadata(path).unwrap();
            assert!(metadata.is_dir());
            (
                path.clone(),
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
            )
        })
        .collect();
    let files = [
        &artifacts.paths.namespace_manifest,
        &artifacts.paths.owner_lock,
        &artifacts.paths.outbox,
        &artifacts.paths.descriptor,
    ]
    .into_iter()
    .map(|path| {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(metadata.is_file());
        FileSnapshot {
            path: path.clone(),
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            bytes: std::fs::read(path).unwrap(),
        }
    })
    .collect();
    OwnedArtifacts { directories, files }
}

fn seed_owned_spool(fixture: &Fixture, artifacts: &RunArtifacts, held: &RunLock) {
    held.validate(&fixture.conn, &fixture.run.owner()).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&artifacts.paths.outbox)
        .unwrap();
    file.write_all(SPOOL_CANARY).unwrap();
    file.sync_all().unwrap();
    artifacts
        .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
        .unwrap();
}

fn diagnose_without_content_effects(fixture: &Fixture, expected_lock: &str) {
    let started = Instant::now();
    let diagnosis = diagnose_runtime(
        &fixture.conn,
        fixture.run.run_id,
        started + Duration::from_millis(80),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_millis(750));
    let text = diagnosis.diagnostics.join("\n");
    assert!(text.contains(expected_lock), "{text}");
    assert!(text.contains("recover"), "{text}");
    assert!(text.contains("descriptor"), "{text}");
    assert!(!text.contains(std::str::from_utf8(SPOOL_CANARY).unwrap()));
}

#[test]
fn runner_recovery_entry_doctor_preserves_held_owner_corrupt_spool_rows_and_identities() {
    let mut fixture = Fixture::new();
    let (artifacts, held) = activate(&mut fixture);
    seed_owned_spool(&fixture, &artifacts, &held);
    let before_rows = registry_snapshot(&fixture.conn);
    let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
    diagnose_without_content_effects(&fixture, "matching owner lock held: live_or_unreachable");
    assert_eq!(registry_snapshot(&fixture.conn), before_rows);
    assert_eq!(
        owned_snapshot(&fixture, &artifacts, &held),
        before_artifacts
    );
}

#[test]
fn runner_recovery_entry_doctor_preserves_available_owner_corrupt_spool_without_takeover() {
    let mut fixture = Fixture::new();
    let (artifacts, held) = activate(&mut fixture);
    seed_owned_spool(&fixture, &artifacts, &held);
    let before_rows = registry_snapshot(&fixture.conn);
    let before_artifacts = owned_snapshot(&fixture, &artifacts, &held);
    drop(held);
    diagnose_without_content_effects(&fixture, "matching owner lock is currently available");
    let held = RunLock::acquire(&fixture.conn, &fixture.run.owner())
        .unwrap()
        .expect("diagnosis releases its probe and preserves the original owner identity");
    assert_eq!(registry_snapshot(&fixture.conn), before_rows);
    assert_eq!(
        owned_snapshot(&fixture, &artifacts, &held),
        before_artifacts
    );
}
