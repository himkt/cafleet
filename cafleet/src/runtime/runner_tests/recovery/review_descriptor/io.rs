use super::*;

#[test]
fn runner_recovery_review_r4_historical_descriptor_pin_remove_uses_the_supplied_connection_budget()
{
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
fn runner_recovery_review_r4_descriptor_pin_drop_preserves_path_for_explicit_completion() {
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
fn runner_recovery_review_r4_descriptor_remove_revalidates_owner_and_expired_deadline_before_dispatch()
 {
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
fn runner_recovery_review_r4_actual_descriptor_unlink_closure_failure_preserves_file_and_retries() {
    let mut case = described_creator();
    let bytes = owned_bytes(&case);
    let lock = case.held.as_ref().unwrap().clone();
    let until = Instant::now() + Duration::from_secs(1);
    let mut pinned =
        RecoveryDescriptor::pin(&mut case.fixture.conn, &case.fixture.run, &lock, until).unwrap();
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

struct UnlinkHost {
    inner: Host,
    path: std::path::PathBuf,
    calls: usize,
    fail: bool,
}
impl UnlinkHost {
    fn new(case: &Case, until: Instant, fail: bool) -> Self {
        Self {
            inner: Host::new(until),
            path: case.artifacts.paths.descriptor.clone(),
            calls: 0,
            fail,
        }
    }
}
impl RecoveryHost for UnlinkHost {
    fn current_process(&mut self) -> TestResult<ProcessIdentity> {
        self.inner.current_process()
    }
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.inner.process_evidence(process, deadline)
    }
    fn startup_process_evidence(
        &mut self,
        run: &RunRecord,
        placement: Option<&Placement>,
        deadline: Instant,
    ) -> TestResult<ProcessEvidence> {
        self.inner
            .startup_process_evidence(run, placement, deadline)
    }
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> TestResult<RecoveryHealth> {
        self.inner.health(run, deadline)
    }
    fn checkpoint(
        &mut self,
        boundary: RecoveryBoundary,
        run: &RunRecord,
        deadline: Instant,
    ) -> TestResult<()> {
        self.inner.checkpoint(boundary, run, deadline)
    }
    fn cleanup_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownClock,
    ) -> TestResult<ShutdownOutcome> {
        self.inner.cleanup_owned(run, clock)
    }
    fn unlink_descriptor(&mut self, path: &Path, deadline: Instant) -> TestResult<()> {
        self.calls += 1;
        assert_eq!(path, self.path);
        self.inner.check(deadline)?;
        if self.fail {
            return Err(CafleetError::App(
                "owned coordinator unlink-dispatch failure".into(),
            ));
        }
        std::fs::remove_file(path)
            .map_err(|_| CafleetError::App("owned coordinator descriptor removal failed".into()))
    }
}
fn recover(case: &mut Case, host: &mut UnlinkHost) -> TestResult<RecoveryReport> {
    case.release();
    recover_member_with_host(
        &mut case.fixture.conn,
        &case.settings,
        case.fixture.run.member_id,
        None,
        host.inner.deadline,
        host,
    )
}

#[test]
fn runner_recovery_review_r4_coordinator_propagates_actual_unlink_failure_and_retries_historical_epoch()
 {
    let mut case = described_creator();
    let bytes = owned_bytes(&case);
    let mut failed = UnlinkHost::new(&case, Instant::now() + Duration::from_secs(2), true);
    let result = recover(&mut case, &mut failed);
    assert_eq!(
        failed.calls, 1,
        "the real coordinator reaches the approved unlink operation"
    );
    incomplete(&case, result);
    assert!(
        owned_bytes(&case) == bytes,
        "the failed dispatch preserves its selected descriptor"
    );
    let first = events(&case)
        .into_iter()
        .map(|view| view.event)
        .collect::<Vec<_>>();
    assert_eq!(first.len(), 1);
    let epoch = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
        .unwrap()
        .owner_epoch;
    let mut retry = UnlinkHost::new(&case, Instant::now() + Duration::from_secs(2), false);
    let report = recover(&mut case, &mut retry).unwrap();
    assert_eq!(retry.calls, 1);
    assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
    assert_eq!(counts(&report), (0, 0, 0));
    assert_eq!(
        events(&case)
            .into_iter()
            .map(|view| view.event)
            .collect::<Vec<_>>(),
        first
    );
    assert_eq!(
        storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
            .unwrap()
            .owner_epoch,
        epoch + 1
    );
    assert!(!case.artifacts.paths.descriptor.exists());
}

#[test]
fn runner_recovery_review_r4_expired_original_budget_prevents_actual_unlink_dispatch() {
    let mut case = described_creator();
    let bytes = owned_bytes(&case);
    let mut host = UnlinkHost::new(&case, Instant::now() + Duration::from_millis(500), false);
    host.inner.action = Some(Box::new(|boundary, _, deadline| {
        if boundary == "before_finish" {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        Ok(())
    }));
    let result = recover(&mut case, &mut host);
    assert!(host.inner.calls.iter().any(|name| name == "before_finish"));
    assert!(Instant::now() >= host.inner.deadline);
    assert_eq!(
        host.calls, 0,
        "the expired deadline is checked before forwarding the unlink seam"
    );
    incomplete(&case, result);
    assert!(
        owned_bytes(&case) == bytes,
        "expiry preserves the selected descriptor"
    );
}
