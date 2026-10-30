use super::*;
use std::io::{Seek, SeekFrom};

struct PinnedFile {
    file: File,
    identity: Value,
    digest: [u8; 32],
}

pub(crate) struct RecoveryDescriptor {
    path: PathBuf,
    pinned: Option<PinnedFile>,
}

fn validated_paths(
    conn: &mut Connection,
    run: &RunRecord,
    lock: &RunLock,
    until: Instant,
) -> Result<RuntimePaths> {
    let paths = with_database_deadline(conn, until, |conn| {
        let current = storage::get_run(conn, run.run_id)?;
        ensure(
            super::super::recovery::stable_run_identity(&current)
                == super::super::recovery::stable_run_identity(run),
            "descriptor runtime ownership changed",
        )?;
        lock.validate(conn, &run.owner())?;
        validate_run_paths(conn, run)
    })?;
    let process = ProcessIdentity::current()?;
    ensure(
        run.owner_pid == process.pid && run.owner_start_identity == process.start_identity,
        "descriptor access requires the current runtime owner",
    )?;
    check_deadline(Instant::now(), until)?;
    Ok(paths)
}

fn read_bounded(file: &mut File, until: Instant) -> Result<Vec<u8>> {
    check_deadline(Instant::now(), until)?;
    ensure(
        file.metadata().map_err(io_error)?.len() <= DESCRIPTOR_LIMIT,
        "launch descriptor exceeds private size limit",
    )?;
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut bytes = Vec::new();
    Read::by_ref(file)
        .take(DESCRIPTOR_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    ensure(
        bytes.len() as u64 <= DESCRIPTOR_LIMIT,
        "launch descriptor exceeds private size limit",
    )?;
    check_deadline(Instant::now(), until)?;
    Ok(bytes)
}

fn validate_named(path: &Path, pinned: &PinnedFile) -> Result<()> {
    let named = private_metadata(path, uid(), 0o600, false)?;
    ensure(
        identity(&named) == pinned.identity
            && identity(&pinned.file.metadata().map_err(io_error)?) == pinned.identity,
        "descriptor identity changed before unlink",
    )
}

impl RecoveryDescriptor {
    pub(crate) fn pin(
        conn: &mut Connection,
        run: &RunRecord,
        lock: &RunLock,
        until: Instant,
    ) -> Result<Self> {
        let path = validated_paths(conn, run, lock, until)?.descriptor;
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { path, pinned: None });
            }
            Err(cause) => return Err(io_error(cause)),
        }
        let mut file = open_private(&path, false)?;
        let file_identity = identity(&file.metadata().map_err(io_error)?);
        let bytes = read_bounded(&mut file, until)?;
        let descriptor = LaunchDescriptor::parse(&bytes)?;
        ensure(
            descriptor.database_identity_json == run.database_identity_json
                && descriptor.fleet_id == run.fleet_id
                && descriptor.member_id == run.member_id
                && descriptor.director_member_id == run.director_member_id
                && descriptor.run_id == run.run_id
                && descriptor.launch_nonce == run.launch_nonce
                && descriptor.owner_epoch > 0
                && descriptor.owner_epoch <= run.owner_epoch,
            "launch descriptor identity mismatch",
        )?;
        let pinned = PinnedFile {
            file,
            identity: file_identity,
            digest: Sha256::digest(&bytes).into(),
        };
        validate_named(&path, &pinned)?;
        check_deadline(Instant::now(), until)?;
        Ok(Self {
            path,
            pinned: Some(pinned),
        })
    }

    pub(crate) fn remove(
        &mut self,
        conn: &mut Connection,
        run: &RunRecord,
        lock: &RunLock,
        until: Instant,
        unlink: impl FnOnce(&Path, Instant) -> Result<()>,
    ) -> Result<()> {
        let paths = validated_paths(conn, run, lock, until)?;
        ensure(paths.descriptor == self.path, "descriptor pathname changed")?;
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                self.pinned = None;
                return Ok(());
            }
            Err(cause) => return Err(io_error(cause)),
        }
        let pinned = self
            .pinned
            .as_mut()
            .ok_or_else(|| error("descriptor appeared after absence was pinned"))?;
        validate_named(&self.path, pinned)?;
        let bytes = read_bounded(&mut pinned.file, until)?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        ensure(
            digest == pinned.digest,
            "descriptor content changed before unlink",
        )?;
        validate_named(&self.path, pinned)?;
        check_deadline(Instant::now(), until)?;
        unlink(&self.path, until)?;
        match fs::symlink_metadata(&self.path) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            Err(cause) => return Err(io_error(cause)),
            Ok(_) => return Err(error("descriptor remains after unlink")),
        }
        check_deadline(Instant::now(), until)?;
        self.pinned = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::runtime as storage;
    use crate::error::CafleetError;
    use crate::runtime::runner::recovery::test_support::descriptor::*;
    use crate::runtime::runner::recovery::test_support::*;
    use crate::runtime::runner::*;
    use std::time::{Duration, Instant};
    #[test]
    fn runner_recovery_historical_descriptor_pin_remove_uses_the_supplied_connection_budget() {
        for takeovers in [1, 2] {
            let mut case = described_creator();
            let lock = case.held.as_ref().unwrap().clone();
            for _ in 0..takeovers {
                lock.transfer_to_reconciler_with_deadline(
                    &mut case.fixture.conn,
                    &case.fixture.run,
                    ProcessIdentity::current().unwrap(),
                    Instant::now() + Duration::from_secs(1),
                )
                .unwrap();
                case.refresh();
            }
            assert!(case.fixture.run.owner_epoch > 1);
            let before = case.fixture.run.clone();
            let path = case.artifacts.paths.descriptor.clone();
            let until = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            with_database_deadline(&mut case.fixture.conn, until, |conn| {
                let mut pinned = RecoveryDescriptor::pin(conn, &before, &lock, until)?;
                assert!(
                    load_descriptor(conn, &path).is_err(),
                    "launch-time reading still requires exact creator ownership"
                );
                pinned.remove(conn, &before, &lock, until, |selected, deadline| {
                    attempts += 1;
                    assert_eq!(selected, path.as_path());
                    assert_eq!(deadline, until);
                    assert!(Instant::now() < deadline);
                    std::fs::remove_file(selected)
                        .map_err(|_| CafleetError::App("owned descriptor removal failed".into()))
                })
            })
            .unwrap();
            assert_eq!(attempts, 1);
            assert!(!path.exists());
            assert_eq!(
                storage::get_run(&case.fixture.conn, before.run_id).unwrap(),
                before
            );
        }
    }

    #[test]
    fn runner_recovery_descriptor_pin_drop_preserves_path_for_explicit_completion() {
        let mut case = described_creator();
        let bytes = owned_bytes(&case);
        let lock = case.held.as_ref().unwrap().clone();
        let pinned = RecoveryDescriptor::pin(
            &mut case.fixture.conn,
            &case.fixture.run,
            &lock,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        drop(pinned);
        assert!(
            owned_bytes(&case) == bytes,
            "pin ownership alone schedules no pathname removal"
        );
    }

    #[test]
    fn runner_recovery_descriptor_remove_revalidates_owner_and_expired_deadline_before_dispatch() {
        for changed in ["epoch", "deadline"] {
            let mut case = described_creator();
            let before = identity(&case.artifacts.paths.descriptor);
            let lock = case.held.as_ref().unwrap().clone();
            let until = Instant::now() + Duration::from_secs(1);
            let mut pinned =
                RecoveryDescriptor::pin(&mut case.fixture.conn, &case.fixture.run, &lock, until)
                    .unwrap();
            if changed == "epoch" {
                case.fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                        [case.fixture.run.run_id],
                    )
                    .unwrap();
            }
            let deadline = if changed == "deadline" {
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .unwrap()
            } else {
                until
            };
            let mut dispatched = false;
            let result: TestResult<()> = pinned.remove(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                deadline,
                |_, _| {
                    dispatched = true;
                    Ok(())
                },
            );
            let error: CafleetError = match result {
                Err(error) => error,
                Ok(()) => panic!("changed descriptor removal precondition fails before dispatch"),
            };
            secret_free(&error.message());
            assert!(!dispatched);
            assert_eq!(identity(&case.artifacts.paths.descriptor), before);
        }
    }

    #[test]
    fn runner_recovery_actual_descriptor_unlink_closure_failure_preserves_file_and_retries() {
        let mut case = described_creator();
        let bytes = owned_bytes(&case);
        let lock = case.held.as_ref().unwrap().clone();
        let until = Instant::now() + Duration::from_secs(1);
        let mut pinned =
            RecoveryDescriptor::pin(&mut case.fixture.conn, &case.fixture.run, &lock, until)
                .unwrap();
        let mut attempts = 0;
        let result: TestResult<()> = pinned.remove(
            &mut case.fixture.conn,
            &case.fixture.run,
            &lock,
            until,
            |_, deadline| {
                attempts += 1;
                assert_eq!(deadline, until);
                Err(CafleetError::App(
                    "owned descriptor unlink-dispatch failure".into(),
                ))
            },
        );
        let error: CafleetError = match result {
            Err(error) => error,
            Ok(()) => panic!("unlink failure remains explicit"),
        };
        assert!(
            error
                .message()
                .contains("owned descriptor unlink-dispatch failure")
        );
        secret_free(&error.message());
        assert_eq!(attempts, 1);
        assert!(
            owned_bytes(&case) == bytes,
            "a failed unlink leaves its selected descriptor"
        );
        pinned
            .remove(
                &mut case.fixture.conn,
                &case.fixture.run,
                &lock,
                until,
                |path, deadline| {
                    attempts += 1;
                    assert_eq!(deadline, until);
                    std::fs::remove_file(path)
                        .map_err(|_| CafleetError::App("owned descriptor retry failed".into()))
                },
            )
            .unwrap();
        assert_eq!(attempts, 2);
        assert!(!case.artifacts.paths.descriptor.exists());
    }
}
