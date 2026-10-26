use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use super::*;
use rusqlite::params;

#[test]
fn runner_artifact_namespace_is_database_adjacent_and_separates_sibling_databases() {
    let first = Fixture::new();
    let mut second = Connection::open(first._dir.path().join("sibling.db")).unwrap();
    crate::db::migrate_to_head(&mut second).unwrap();
    let a = RuntimePaths::for_run(&storage::database_identity(&first.conn).unwrap(), 1).unwrap();
    let b = RuntimePaths::for_run(&storage::database_identity(&second).unwrap(), 1).unwrap();
    assert_ne!(a.namespace_dir, b.namespace_dir);
    assert_ne!(a.run_dir, b.run_dir);
    assert_ne!(a.control_socket, b.control_socket);
    assert_ne!(a.outbox, b.outbox);
    assert_eq!(
        a.namespace_dir.parent().unwrap(),
        std::fs::canonicalize(first._dir.path())
            .unwrap()
            .join(".cafleet-runs")
    );
    assert_eq!(
        a.namespace_dir.file_name().unwrap().to_str().unwrap().len(),
        26
    );
    assert_eq!(a.run_dir.file_name().unwrap(), "1");
}

#[test]
fn runner_artifact_identity_changes_for_copied_or_replaced_database_files() {
    let fixture = Fixture::new();
    let original = storage::database_identity(&fixture.conn).unwrap();
    let copy = fixture._dir.path().join("copy.db");
    std::fs::copy(fixture.conn.path().unwrap(), &copy).unwrap();
    let copied = crate::db::connect(&format!("sqlite:///{}", copy.display())).unwrap();
    let copied_identity = storage::database_identity(&copied).unwrap();
    assert_eq!(original["instance_id"], copied_identity["instance_id"]);
    assert_ne!(
        RuntimePaths::for_run(&original, 1).unwrap().namespace_dir,
        RuntimePaths::for_run(&copied_identity, 1)
            .unwrap()
            .namespace_dir
    );
    let selected = std::path::PathBuf::from(fixture.conn.path().unwrap());
    std::fs::rename(&selected, fixture._dir.path().join("old.db")).unwrap();
    std::fs::copy(&copy, &selected).unwrap();
    let replacement = crate::db::connect(&format!("sqlite:///{}", selected.display())).unwrap();
    let replacement_identity = storage::database_identity(&replacement).unwrap();
    assert_eq!(original["instance_id"], replacement_identity["instance_id"]);
    assert_ne!(original["inode"], replacement_identity["inode"]);
    assert_ne!(
        RuntimePaths::for_run(&original, 1).unwrap().namespace_dir,
        RuntimePaths::for_run(&replacement_identity, 1)
            .unwrap()
            .namespace_dir
    );
}

#[test]
fn runner_artifact_allocation_pins_private_directory_and_lock_inodes() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    for directory in [&artifacts.paths.namespace_dir, &artifacts.paths.run_dir] {
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let lock = std::fs::metadata(&artifacts.paths.owner_lock).unwrap();
    assert_eq!(lock.permissions().mode() & 0o777, 0o600);
    assert!(fixture.run.runtime_dir_identity_json.is_some());
    assert!(fixture.run.run_lock_identity_json.is_some());
    assert_eq!(
        fixture.run.runtime_path,
        artifacts.paths.run_dir.to_str().unwrap()
    );
}

#[test]
fn runner_artifact_existing_directory_and_conflicting_namespace_manifest_are_preserved() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    let sentinel = artifacts.paths.run_dir.join("retained");
    std::fs::write(&sentinel, b"existing-owner-data").unwrap();
    assert!(RunArtifacts::create(&mut fixture.conn, fixture.run.run_id).is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"existing-owner-data");
    let mut other = Fixture::new();
    let paths = RuntimePaths::for_run(&other.run.database_identity_json, other.run.run_id).unwrap();
    std::fs::create_dir_all(&paths.namespace_dir).unwrap();
    std::fs::set_permissions(
        paths.namespace_dir.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::set_permissions(&paths.namespace_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut foreign = other.run.database_identity_json.clone();
    foreign["instance_id"] = json!("0".repeat(32));
    std::fs::write(
        &paths.namespace_manifest,
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(
        &paths.namespace_manifest,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let before = std::fs::read(&paths.namespace_manifest).unwrap();
    assert!(RunArtifacts::create(&mut other.conn, other.run.run_id).is_err());
    assert_eq!(std::fs::read(&paths.namespace_manifest).unwrap(), before);
    assert!(!paths.run_dir.exists());
}

#[test]
fn runner_artifact_allocation_rejects_a_symlinked_runtime_namespace() {
    let mut fixture = Fixture::new();
    let paths =
        RuntimePaths::for_run(&fixture.run.database_identity_json, fixture.run.run_id).unwrap();
    let target = fixture._dir.path().join("foreign");
    std::fs::create_dir(&target).unwrap();
    std::fs::create_dir(paths.namespace_dir.parent().unwrap()).unwrap();
    symlink(&target, &paths.namespace_dir).unwrap();
    assert!(RunArtifacts::create(&mut fixture.conn, fixture.run.run_id).is_err());
    assert!(std::fs::read_dir(&target).unwrap().next().is_none());
}

#[test]
fn runner_descriptor_round_trips_literal_argv_cwd_environment_and_private_permissions() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    let spec = fixture.spec();
    let descriptor = LaunchDescriptor::new(&fixture.run, spec.clone()).unwrap();
    artifacts.write_descriptor(&descriptor).unwrap();
    assert_eq!(
        std::fs::metadata(&artifacts.paths.descriptor)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let read = load_descriptor(&fixture.conn, &artifacts.paths.descriptor).unwrap();
    assert_eq!(read.spawn.argv, spec.argv);
    assert_eq!(read.spawn.cwd, spec.cwd);
    assert_eq!(read.spawn.environment, spec.environment);
    assert_eq!(read.spawn.size, spec.size);
    assert_eq!(read.run_id, fixture.run.run_id);
    assert_eq!(read.launch_nonce, fixture.run.launch_nonce);
    assert_eq!(read.owner_epoch, fixture.run.owner_epoch);
    artifacts.remove_descriptor().unwrap();
    artifacts.remove_descriptor().unwrap();
    assert!(!artifacts.paths.descriptor.exists());
}

#[test]
fn runner_descriptor_load_rejects_malformed_or_identity_mismatched_input() {
    for damage in ["json", "nonce", "epoch", "run", "database"] {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let descriptor = LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap();
        artifacts.write_descriptor(&descriptor).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&artifacts.paths.descriptor).unwrap()).unwrap();
        match damage {
            "json" => {
                std::fs::write(&artifacts.paths.descriptor, b"{").unwrap();
            }
            "nonce" => {
                value["launch_nonce"] = json!("b".repeat(64));
            }
            "epoch" => {
                value["owner_epoch"] = json!(fixture.run.owner_epoch + 1);
            }
            "run" => {
                value["run_id"] = json!(fixture.run.run_id + 1);
            }
            "database" => {
                value["database_identity_json"]["instance_id"] = json!("0".repeat(32));
            }
            _ => unreachable!(),
        }
        if damage != "json" {
            std::fs::write(
                &artifacts.paths.descriptor,
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
        assert!(
            load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err(),
            "{damage}"
        );
    }
}

#[test]
fn runner_descriptor_load_rejects_symlinks_and_world_readable_launch_input() {
    for symlinked in [false, true] {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        artifacts
            .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
            .unwrap();
        if symlinked {
            let moved = artifacts.paths.run_dir.join("moved-descriptor");
            std::fs::rename(&artifacts.paths.descriptor, &moved).unwrap();
            symlink(&moved, &artifacts.paths.descriptor).unwrap();
        } else {
            std::fs::set_permissions(
                &artifacts.paths.descriptor,
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        assert!(load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err());
    }
}

#[test]
fn runner_private_metadata_validation_rejects_a_different_owner_without_privileged_chown() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    let metadata = std::fs::metadata(&artifacts.paths.owner_lock).unwrap();
    validate_private_file(&artifacts.paths.owner_lock, metadata.uid(), 0o600).unwrap();
    assert!(
        validate_private_file(
            &artifacts.paths.owner_lock,
            metadata.uid().checked_add(1).unwrap(),
            0o600
        )
        .is_err()
    );
}

#[test]
fn runner_lock_keeps_the_original_inode_exclusive_even_if_its_path_is_unlinked() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    let owner = fixture.run.owner();
    let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
    assert!(RunLock::acquire(&fixture.conn, &owner).unwrap().is_none());
    let original = std::fs::metadata(&artifacts.paths.owner_lock)
        .unwrap()
        .ino();
    std::fs::remove_file(&artifacts.paths.owner_lock).unwrap();
    std::fs::write(&artifacts.paths.owner_lock, b"").unwrap();
    std::fs::set_permissions(
        &artifacts.paths.owner_lock,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_ne!(
        std::fs::metadata(&artifacts.paths.owner_lock)
            .unwrap()
            .ino(),
        original
    );
    assert!(RunLock::acquire(&fixture.conn, &owner).is_err());
    assert!(held.validate(&fixture.conn, &owner).is_err());
    assert_eq!(
        storage::get_run(&fixture.conn, owner.run_id)
            .unwrap()
            .owner_epoch,
        owner.owner_epoch
    );
}

#[test]
fn runner_lock_transfer_advances_epoch_and_fences_the_old_owner() {
    let mut fixture = Fixture::new();
    fixture.artifacts();
    let owner = fixture.run.owner();
    let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
    let next = held
        .transfer_to_runner(
            &mut fixture.conn,
            &owner,
            ProcessIdentity::current().unwrap(),
        )
        .unwrap();
    assert_eq!(next.owner_epoch, owner.owner_epoch + 1);
    assert_eq!(next.owner_kind, OwnerKind::Runner);
    assert!(held.validate(&fixture.conn, &owner).is_err());
    held.validate(&fixture.conn, &next).unwrap();
    assert!(RunLock::acquire(&fixture.conn, &next).unwrap().is_none());
    drop(held);
    assert!(RunLock::acquire(&fixture.conn, &next).unwrap().is_some());
}

#[test]
fn runner_artifact_paths_validate_complete_socket_length_before_creation() {
    let fixture = Fixture::new();
    let mut identity = fixture.run.database_identity_json.clone();
    identity["canonical_path"] = json!(format!("/private/tmp/{}/registry.db", "x".repeat(240)));
    assert!(RuntimePaths::for_run(&identity, 1).is_err());
    for run in [0, -1] {
        assert!(RuntimePaths::for_run(&fixture.run.database_identity_json, run).is_err());
    }
}

#[test]
fn runner_database_replacement_fences_descriptors_and_owned_lock_checks() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    artifacts
        .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
        .unwrap();
    let owner = fixture.run.owner();
    let held = RunLock::acquire(&fixture.conn, &owner).unwrap().unwrap();
    let selected = std::path::PathBuf::from(fixture.conn.path().unwrap());
    let copy = fixture._dir.path().join("replacement.db");
    std::fs::copy(&selected, &copy).unwrap();
    std::fs::rename(&selected, fixture._dir.path().join("old.db")).unwrap();
    std::fs::rename(&copy, &selected).unwrap();
    assert!(held.validate(&fixture.conn, &owner).is_err());
    assert!(load_descriptor(&fixture.conn, &artifacts.paths.descriptor).is_err());
    assert!(artifacts.paths.run_dir.exists());
    assert_eq!(
        fixture
            .conn
            .query_row(
                "SELECT owner_epoch FROM member_runs WHERE run_id=?1",
                params![owner.run_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        owner.owner_epoch
    );
}

#[test]
fn runner_artifact_canonical_aliases_share_one_database_namespace() {
    let fixture = Fixture::new();
    let alias = fixture._dir.path().join("database-alias.db");
    symlink(fixture.conn.path().unwrap(), &alias).unwrap();
    let conn = crate::db::connect(&format!("sqlite:///{}", alias.display())).unwrap();
    let identity = storage::database_identity(&conn).unwrap();
    assert_eq!(identity, fixture.run.database_identity_json);
    assert_eq!(
        RuntimePaths::for_run(&identity, 1).unwrap().run_dir,
        RuntimePaths::for_run(&fixture.run.database_identity_json, 1)
            .unwrap()
            .run_dir
    );
}

#[test]
fn runner_descriptor_removal_reports_an_unlink_failure_without_claiming_cleanup() {
    let mut fixture = Fixture::new();
    let artifacts = fixture.artifacts();
    artifacts
        .write_descriptor(&LaunchDescriptor::new(&fixture.run, fixture.spec()).unwrap())
        .unwrap();
    let retained = artifacts.paths.run_dir.join("retained-descriptor");
    std::fs::rename(&artifacts.paths.descriptor, &retained).unwrap();
    std::fs::create_dir(&artifacts.paths.descriptor).unwrap();
    assert!(artifacts.remove_descriptor().is_err());
    assert!(retained.is_file());
    assert!(artifacts.paths.descriptor.is_dir());
}

#[test]
fn runner_descriptor_construction_validates_required_spawn_inputs_before_writing() {
    for missing in ["argv", "cwd", "rows", "cols"] {
        let mut fixture = Fixture::new();
        let artifacts = fixture.artifacts();
        let mut spec = fixture.spec();
        match missing {
            "argv" => spec.argv.clear(),
            "cwd" => spec.cwd = "relative".into(),
            "rows" => spec.size.rows = 0,
            "cols" => spec.size.cols = 0,
            _ => unreachable!(),
        }
        assert!(
            LaunchDescriptor::new(&fixture.run, spec).is_err(),
            "{missing}"
        );
        assert!(!artifacts.paths.descriptor.exists());
    }
}
