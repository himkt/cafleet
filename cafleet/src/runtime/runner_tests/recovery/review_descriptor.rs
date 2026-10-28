use super::*;
mod io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

const PRIVATE_VALUE: &str = "descriptor-private-fixture-canary";

fn described_creator() -> Case {
    let case = Case::creator();
    let mut spec = case.fixture.spec();
    spec.argv = vec!["/usr/bin/false".into(), PRIVATE_VALUE.into()];
    spec.environment
        .push(("CAFLEET_TEST_PRIVATE_VALUE".into(), PRIVATE_VALUE.into()));
    let descriptor = LaunchDescriptor::new(&case.fixture.run, spec).unwrap();
    case.artifacts.write_descriptor(&descriptor).unwrap();
    case
}
fn identity(path: &Path) -> (u64, u64, u64, i64, i64) {
    let value = std::fs::symlink_metadata(path).unwrap();
    (
        value.dev(),
        value.ino(),
        value.len(),
        value.mtime(),
        value.mtime_nsec(),
    )
}
fn owned_bytes(case: &Case) -> Vec<u8> {
    let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
    let current = ProcessIdentity::current().unwrap();
    assert_eq!(run.owner_pid, current.pid);
    assert_eq!(run.owner_start_identity, current.start_identity);
    let acquired;
    let lock = if let Some(held) = &case.held {
        held.as_ref()
    } else {
        acquired = RunLock::acquire(&case.fixture.conn, &run.owner())
            .unwrap()
            .unwrap();
        &acquired
    };
    lock.validate(&case.fixture.conn, &run.owner()).unwrap();
    std::fs::read(&case.artifacts.paths.descriptor).unwrap()
}
fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
fn secret_free(text: &str) {
    assert!(
        !text.contains(PRIVATE_VALUE),
        "descriptor diagnostics preserve private spawn data"
    );
    assert!(
        !text.contains("CAFLEET_TEST_PRIVATE_VALUE"),
        "descriptor diagnostics preserve private environment names"
    );
}
fn incomplete(case: &Case, result: TestResult<RecoveryReport>) {
    match result {
        Err(error) => secret_free(&error.message()),
        Ok(report) => {
            assert!(
                matches!(report.outcome, RecoveryOutcome::Recovering),
                "descriptor failure remains an error or incomplete outcome"
            );
            assert!(!report.cleanup_diagnostics.is_empty());
            secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
        }
    }
    let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
    assert_eq!(run.cleanup_state, CleanupState::Incomplete);
    secret_free(&serde_json::to_string(&run.cleanup_diagnostics_json).unwrap());
}

#[test]
fn runner_recovery_review_r4_verified_creator_recovery_removes_its_private_descriptor() {
    let mut case = described_creator();
    let report = case.recover(&mut new_host()).unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(report.status, RunStatus::Lost);
    assert_eq!(counts(&report), (0, 1, 0));
    secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
    assert!(
        !case.artifacts.paths.descriptor.exists(),
        "successful creator cleanup removes its launch descriptor"
    );
}

#[test]
fn runner_recovery_review_r4_historical_descriptor_survives_reconciler_retries_until_verified_removal()
 {
    let mut case = described_creator();
    let bytes = owned_bytes(&case);
    let original_epoch = case.fixture.run.owner_epoch;
    for increment in 1..=2 {
        let mut host = new_host();
        host.action = Some(Box::new(|boundary, _, _| {
            if boundary == "before_finish" {
                return Err(CafleetError::App(
                    "owned descriptor completion interruption".into(),
                ));
            }
            Ok(())
        }));
        assert!(case.recover(&mut host).is_err());
        assert!(host.calls.iter().any(|name| name == "before_finish"));
        case.refresh();
        assert_eq!(case.fixture.run.owner_epoch, original_epoch + increment);
        assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
        assert!(
            owned_bytes(&case) == bytes,
            "interrupted cleanup retains its original descriptor"
        );
        assert_eq!(events(&case).len(), 1);
    }
    let report = case.recover(&mut new_host()).unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(counts(&report), (0, 0, 0));
    assert_eq!(events(&case).len(), 1);
    assert!(
        !case.artifacts.paths.descriptor.exists(),
        "a positive historical epoch remains removable after legitimate takeovers"
    );
}

#[test]
fn runner_recovery_review_r4_absent_descriptor_and_completed_retry_are_idempotent() {
    let mut case = Case::creator();
    let report = case.recover(&mut new_host()).unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert!(!case.artifacts.paths.descriptor.exists());
    let before = rows(&case.fixture.conn);
    assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
    assert_eq!(rows(&case.fixture.conn), before);
}

#[derive(Clone, Copy)]
enum Mutation {
    Replacement,
    Symlink,
    Content,
    Appeared,
    Disappeared,
    ForeignOwner,
}
fn changed_after_pin(mutation: Mutation) {
    let mut case = described_creator();
    let original = owned_bytes(&case);
    if matches!(mutation, Mutation::Appeared) {
        case.artifacts.remove_descriptor().unwrap();
    }
    let path = case.artifacts.paths.descriptor.clone();
    let retained = case.artifacts.paths.run_dir.join("retained-launch.json");
    let changed = Arc::new(Mutex::new(None));
    let published = changed.clone();
    let database = case.fixture.conn.path().unwrap().to_owned();
    let mut host = new_host();
    host.action = Some(Box::new(move |boundary, run, _| {
        if boundary != "before_finish" {
            return Ok(());
        }
        let conn = Connection::open(&database).unwrap();
        assert!(RunLock::acquire(&conn, &run.owner()).unwrap().is_none());
        match mutation {
            Mutation::Replacement => {
                std::fs::rename(&path, &retained).unwrap();
                write_private(&path, &original);
                assert_ne!(identity(&path).1, identity(&retained).1);
                *published.lock().unwrap() = Some(original.clone());
            }
            Mutation::Symlink => {
                std::fs::rename(&path, &retained).unwrap();
                std::os::unix::fs::symlink(&retained, &path).unwrap();
                *published.lock().unwrap() = Some(original.clone());
            }
            Mutation::Content => {
                let before = identity(&path);
                let mut value: Value = serde_json::from_slice(&original).unwrap();
                value["spawn"]["argv"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("owned-in-place-change"));
                let bytes = serde_json::to_vec(&value).unwrap();
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                assert_eq!((identity(&path).0, identity(&path).1), (before.0, before.1));
                *published.lock().unwrap() = Some(bytes);
            }
            Mutation::Appeared => {
                write_private(&path, &original);
                *published.lock().unwrap() = Some(original.clone());
            }
            Mutation::Disappeared => {
                std::fs::remove_file(&path).unwrap();
            }
            Mutation::ForeignOwner => {
                conn.execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [run.run_id],
                )
                .unwrap();
                *published.lock().unwrap() = Some(original.clone());
            }
        }
        Ok(())
    }));
    let result = case.recover(&mut host);
    assert!(host.calls.iter().any(|name| name == "before_finish"));
    if matches!(mutation, Mutation::Disappeared) {
        assert!(matches!(
            result.unwrap().outcome,
            RecoveryOutcome::Recovered
        ));
        assert!(!case.artifacts.paths.descriptor.exists());
        return;
    }
    incomplete(&case, result);
    if matches!(mutation, Mutation::Symlink) {
        assert!(
            std::fs::symlink_metadata(&case.artifacts.paths.descriptor)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
        let lock = RunLock::acquire(&case.fixture.conn, &run.owner())
            .unwrap()
            .unwrap();
        lock.validate(&case.fixture.conn, &run.owner()).unwrap();
        assert!(
            std::fs::read(case.artifacts.paths.run_dir.join("retained-launch.json")).unwrap()
                == *changed.lock().unwrap().as_ref().unwrap(),
            "the symlink target remains unchanged"
        );
    } else {
        assert!(
            owned_bytes(&case) == *changed.lock().unwrap().as_ref().unwrap(),
            "the refused descriptor remains unchanged"
        );
    }
}
#[test]
fn runner_recovery_review_r4_replaced_descriptor_after_pin_is_preserved_with_incomplete_cleanup() {
    changed_after_pin(Mutation::Replacement);
}
#[test]
fn runner_recovery_review_r4_symlink_after_pin_preserves_link_and_target_with_incomplete_cleanup() {
    changed_after_pin(Mutation::Symlink);
}
#[test]
fn runner_recovery_review_r4_in_place_descriptor_change_after_pin_is_preserved() {
    changed_after_pin(Mutation::Content);
}
#[test]
fn runner_recovery_review_r4_descriptor_appearing_after_absence_pin_is_preserved() {
    changed_after_pin(Mutation::Appeared);
}
#[test]
fn runner_recovery_review_r4_descriptor_disappearing_after_pin_is_safe_absence() {
    changed_after_pin(Mutation::Disappeared);
}
#[test]
fn runner_recovery_review_r4_foreign_epoch_before_unlink_preserves_descriptor() {
    changed_after_pin(Mutation::ForeignOwner);
}

#[test]
fn runner_recovery_review_r4_invalid_descriptor_identity_epoch_shape_or_permissions_fails_privately()
 {
    for damage in [
        "schema",
        "database",
        "fleet",
        "member",
        "director",
        "run",
        "nonce",
        "zero_epoch",
        "future_epoch",
        "unknown_field",
        "json",
        "size",
        "permissions",
        "symlink",
    ] {
        let mut case = described_creator();
        let mut value: Value = serde_json::from_slice(&owned_bytes(&case)).unwrap();
        match damage {
            "schema" => value["schema_version"] = json!(2),
            "database" => value["database_identity_json"]["instance_id"] = json!("0".repeat(32)),
            "fleet" => value["fleet_id"] = json!(case.fixture.run.fleet_id + 1),
            "member" => value["member_id"] = json!(case.fixture.run.member_id + 1),
            "director" => {
                value["director_member_id"] = json!(case.fixture.run.director_member_id + 1)
            }
            "run" => value["run_id"] = json!(case.fixture.run.run_id + 1),
            "nonce" => value["launch_nonce"] = json!("b".repeat(64)),
            "zero_epoch" => value["owner_epoch"] = json!(0),
            "future_epoch" => value["owner_epoch"] = json!(case.fixture.run.owner_epoch + 10),
            "unknown_field" => value["unexpected"] = json!(true),
            "json" | "size" | "permissions" | "symlink" => {}
            _ => unreachable!(),
        }
        let bytes = match damage {
            "json" => b"{".to_vec(),
            "size" => vec![b' '; 1024 * 1024 + 1],
            _ => serde_json::to_vec(&value).unwrap(),
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&case.artifacts.paths.descriptor)
            .unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
        if damage == "permissions" {
            std::fs::set_permissions(
                &case.artifacts.paths.descriptor,
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        if damage == "symlink" {
            let retained = case.artifacts.paths.run_dir.join("invalid-target.json");
            std::fs::rename(&case.artifacts.paths.descriptor, &retained).unwrap();
            std::os::unix::fs::symlink(&retained, &case.artifacts.paths.descriptor).unwrap();
        }
        let before = identity(&case.artifacts.paths.descriptor);
        let result = case.recover(&mut new_host());
        incomplete(&case, result);
        assert_eq!(identity(&case.artifacts.paths.descriptor), before);
    }
}

#[test]
fn runner_recovery_review_r4_original_deadline_after_before_finish_keeps_descriptor_retryable() {
    let mut case = described_creator();
    let bytes = owned_bytes(&case);
    let mut host = Host::new(Instant::now() + Duration::from_millis(500));
    let reached = Arc::new(AtomicBool::new(false));
    let observed = reached.clone();
    host.action = Some(Box::new(move |boundary, _, deadline| {
        if boundary == "before_finish" {
            observed.store(true, Ordering::Release);
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        Ok(())
    }));
    let result = case.recover(&mut host);
    assert!(reached.load(Ordering::Acquire));
    assert!(Instant::now() >= host.deadline);
    incomplete(&case, result);
    assert!(
        owned_bytes(&case) == bytes,
        "expiry leaves the descriptor retryable"
    );
}

#[test]
fn runner_recovery_review_r4_exceptional_database_only_reconciliation_preserves_suspect_descriptor()
{
    let mut case = described_creator();
    let before = identity(&case.artifacts.paths.descriptor);
    case.release();
    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    let report = case.recover(&mut new_host()).unwrap();
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(identity(&case.artifacts.paths.descriptor), before);
    secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
}
