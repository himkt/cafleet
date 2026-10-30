use super::*;
use crate::broker::records::Placement;
use crate::broker::runtime::{self as storage, *};
use crate::config::Settings;
use crate::input::PaneLock;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod exceptional;
mod replay;

#[derive(Debug)]
pub(crate) enum ProcessEvidence {
    Alive,
    Terminated,
    Unknown(String),
}
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "the synchronous recovery interface transfers one bounded health snapshot by value"
)]
pub(crate) enum RecoveryHealth {
    Matching(HealthSnapshot),
    Unavailable(String),
    Mismatched(String),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryBoundary {
    AfterSnapshot,
    BeforeTakeover,
    AfterTakeover,
    BeforeReplay,
    AfterCommit,
    BeforeCompact,
    BeforeFinish,
}
pub(crate) trait RecoveryHost {
    fn current_process(&mut self) -> Result<ProcessIdentity>;
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        deadline: Instant,
    ) -> Result<ProcessEvidence>;
    fn startup_process_evidence(
        &mut self,
        run: &RunRecord,
        placement: Option<&Placement>,
        deadline: Instant,
    ) -> Result<ProcessEvidence>;
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> Result<RecoveryHealth>;
    fn cleanup_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome>;
    fn unlink_descriptor(&mut self, path: &std::path::Path, deadline: Instant) -> Result<()> {
        unlink_descriptor_file(path, deadline)
    }
    fn checkpoint(
        &mut self,
        boundary: RecoveryBoundary,
        run: &RunRecord,
        deadline: Instant,
    ) -> Result<()>;
}

pub(crate) fn unlink_descriptor_file(path: &std::path::Path, deadline: Instant) -> Result<()> {
    check_deadline(Instant::now(), deadline)?;
    std::fs::remove_file(path).map_err(|cause| error(format!("descriptor unlink failed: {cause}")))
}
#[derive(Debug)]
pub(crate) struct OwnerTerminationAttestation {
    pub(crate) expected_run_id: i64,
    pub(crate) expected_owner_epoch: i64,
    pub(crate) evidence_ref: String,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryOutcome {
    Recovered,
    Recovering,
    LiveOrUnreachable,
    OperatorReconciliationRequired,
}
impl RecoveryOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recovered => "recovered",
            Self::Recovering => "recovering",
            Self::LiveOrUnreachable => "live_or_unreachable",
            Self::OperatorReconciliationRequired => "operator_reconciliation_required",
        }
    }
}
#[derive(Debug)]
pub(crate) struct RecoveryReport {
    pub(crate) member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) outcome: RecoveryOutcome,
    pub(crate) replayed_count: u64,
    pub(crate) inbox_count: u64,
    pub(crate) archived_count: u64,
    pub(crate) status: RunStatus,
    pub(crate) cleanup_diagnostics: Vec<Value>,
}
#[derive(Debug)]
pub(crate) struct RecoveryCompletion {
    pub(crate) report: RecoveryReport,
    pub(crate) run: RunRecord,
}

pub(crate) fn validate_recovery_completion(
    expected: &RunRecord,
    completion: &RecoveryCompletion,
) -> Result<()> {
    let mut proven = expected.clone();
    proven.owner_epoch = completion.run.owner_epoch;
    proven.owner_kind = completion.run.owner_kind;
    proven.owner_pid = completion.run.owner_pid;
    proven.owner_start_identity = completion.run.owner_start_identity.clone();
    ensure(
        stable_run_identity(&proven) == stable_run_identity(&completion.run)
            && completion.report.member_id == expected.member_id
            && completion.report.run_id == expected.run_id
            && completion.report.status == completion.run.status
            && (completion.run.owner() == expected.owner()
                || (expected.owner_epoch.checked_add(1) == Some(completion.run.owner_epoch)
                    && completion.run.owner_kind == OwnerKind::Reconciler)),
        "recovery completion does not match the pinned deletion target",
    )
}

pub(crate) fn stable_run_identity(run: &RunRecord) -> Value {
    json!({"database":run.database_identity_json,"fleet_id":run.fleet_id,"member_id":run.member_id,
        "director_member_id":run.director_member_id,"backend":run.backend,"run_id":run.run_id,
        "nonce":run.launch_nonce,"epoch":run.owner_epoch,"owner_kind":run.owner_kind,
        "owner_pid":run.owner_pid,"owner_start":run.owner_start_identity,"runtime_path":run.runtime_path,
        "directory":run.runtime_dir_identity_json,"lock":run.run_lock_identity_json,
        "runner_pid":run.runner_pid,"runner_start":run.runner_start_identity,
        "child_pid":run.child_pid,"child_start":run.child_start_identity})
}
fn diagnostics(run: &RunRecord) -> Result<Vec<Value>> {
    match &run.cleanup_diagnostics_json {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(_) => Err(error("runtime cleanup diagnostics must be an array")),
    }
}
fn completion(run: RunRecord, outcome: RecoveryOutcome) -> Result<RecoveryCompletion> {
    Ok(RecoveryCompletion {
        report: RecoveryReport {
            member_id: run.member_id,
            run_id: run.run_id,
            outcome,
            replayed_count: 0,
            inbox_count: 0,
            archived_count: 0,
            status: run.status,
            cleanup_diagnostics: diagnostics(&run)?,
        },
        run,
    })
}
fn placement(conn: &Connection, member_id: i64) -> Result<Option<Placement>> {
    conn.query_row("SELECT backend,mux_session,mux_window_id,mux_pane_id,coding_agent,created_at FROM member_placements WHERE member_id=?1",
        [member_id], |row| Placement::from_row(row, 0)).optional().map_err(crate::error::CafleetError::from)
}
fn snapshot(
    conn: &mut Connection,
    member_id: i64,
    until: Instant,
) -> Result<(RunRecord, Option<Placement>)> {
    with_database_deadline(conn, until, |conn| {
        let run = storage::member_runtime(conn, member_id)?
            .ok_or_else(|| error(format!("member {member_id} has no recorded run")))?
            .run;
        validate_database(conn, &run.database_identity_json)?;
        Ok((run, placement(conn, member_id)?))
    })
}
fn check_snapshot(
    conn: &Connection,
    expected: &RunRecord,
    expected_placement: Option<&Placement>,
) -> Result<RunRecord> {
    let current = storage::member_runtime(conn, expected.member_id)?
        .ok_or_else(|| error("recorded runtime disappeared during recovery"))?
        .run;
    validate_database(conn, &expected.database_identity_json)?;
    ensure(
        stable_run_identity(&current) == stable_run_identity(expected)
            && placement(conn, expected.member_id)?.as_ref() == expected_placement,
        "runtime ownership or placement changed during recovery",
    )?;
    Ok(current)
}
fn validate_attestation(
    run: &RunRecord,
    attestation: Option<&OwnerTerminationAttestation>,
) -> Result<()> {
    if let Some(claim) = attestation {
        ensure(
            claim.expected_run_id > 0
                && claim.expected_owner_epoch > 0
                && !claim.evidence_ref.trim().is_empty(),
            "termination attestation requires positive run/epoch and nonblank evidence",
        )?;
        ensure(
            claim.expected_run_id == run.run_id && claim.expected_owner_epoch == run.owner_epoch,
            "stale termination attestation run or owner epoch",
        )?;
    }
    Ok(())
}
pub(super) fn advance_recovery_owner(
    conn: &Connection,
    expected: &RunRecord,
    process: &ProcessIdentity,
) -> Result<RunRecord> {
    let current = storage::validate_owner(conn, &expected.owner())?;
    validate_database(conn, &expected.database_identity_json)?;
    ensure(
        stable_run_identity(&current) == stable_run_identity(expected),
        "recovery ownership changed before CAS",
    )?;
    let epoch = current
        .owner_epoch
        .checked_add(1)
        .ok_or_else(|| error("owner epoch exhausted"))?;
    let changed = conn.execute("UPDATE member_runs SET owner_epoch=?1,owner_kind='reconciler',owner_pid=?2,owner_start_identity=?3,recovery_origin_status=COALESCE(recovery_origin_status,status),status='recovering' WHERE run_id=?4 AND owner_epoch=?5",
        params![epoch,process.pid,process.start_identity,current.run_id,current.owner_epoch])?;
    ensure(changed == 1, "recovery takeover lost CAS")?;
    storage::get_run(conn, expected.run_id)
}

#[cfg(test)]
pub(crate) fn recover_member_with_host(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    attestation: Option<&OwnerTerminationAttestation>,
    deadline: Instant,
    host: &mut impl RecoveryHost,
) -> Result<RecoveryReport> {
    recover(conn, settings, member_id, attestation, deadline, None, host).map(|value| value.report)
}
struct BorrowedPane<'a> {
    run: &'a RunRecord,
    placement: &'a Placement,
    guard: &'a PaneLock,
}
#[expect(
    clippy::too_many_arguments,
    reason = "borrowed recovery binds the caller's fixed run, placement and held guard"
)]
pub(crate) fn recover_member_with_held_pane_lock(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    attestation: Option<&OwnerTerminationAttestation>,
    deadline: Instant,
    held: &PaneLock,
    host: &mut impl RecoveryHost,
) -> Result<RecoveryCompletion> {
    ensure(
        member_id == expected_run.member_id,
        "borrowed recovery member identity mismatch",
    )?;
    recover(
        conn,
        settings,
        member_id,
        attestation,
        deadline,
        Some(BorrowedPane {
            run: expected_run,
            placement: expected_placement,
            guard: held,
        }),
        host,
    )
}
pub(crate) fn recover_member_under_pane_lock(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    expected_run: &RunRecord,
    expected_placement: &Placement,
    deadline: Instant,
    held: &PaneLock,
) -> Result<RecoveryCompletion> {
    recover_member_with_held_pane_lock(
        conn,
        settings,
        member_id,
        expected_run,
        expected_placement,
        None,
        deadline,
        held,
        &mut NativeRecoveryHost,
    )
}
pub(crate) fn recover_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    attestation: Option<&OwnerTerminationAttestation>,
    deadline: Instant,
) -> Result<RecoveryReport> {
    recover(
        conn,
        settings,
        member_id,
        attestation,
        deadline,
        None,
        &mut NativeRecoveryHost,
    )
    .map(|value| value.report)
}
fn recover(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    attestation: Option<&OwnerTerminationAttestation>,
    until: Instant,
    borrowed: Option<BorrowedPane<'_>>,
    host: &mut impl RecoveryHost,
) -> Result<RecoveryCompletion> {
    let clock = ShutdownClock::new(Instant::now()).clipped(until);
    check_deadline(Instant::now(), until)?;
    ensure(member_id > 0, "positive recovery member ID required")?;
    let (run, pinned) = snapshot(conn, member_id, until)?;
    if let Some(expected) = &borrowed {
        ensure(
            stable_run_identity(&run) == stable_run_identity(expected.run)
                && pinned.as_ref() == Some(expected.placement),
            "borrowed recovery starting identity mismatch",
        )?;
        expected
            .guard
            .validate_binding(&settings.database_url, expected.placement)?;
    }
    validate_attestation(&run, attestation)?;
    host.checkpoint(RecoveryBoundary::AfterSnapshot, &run, until)?;
    let acquired_pane = if borrowed.is_none() {
        if let Some(target) = pinned.as_ref().filter(|item| item.mux_pane_id.is_some()) {
            loop {
                check_deadline(Instant::now(), until)?;
                if let Some(guard) = PaneLock::try_acquire(&settings.database_url, target)? {
                    break Some(guard);
                }
                if matches!(
                    with_database_deadline(conn, until, |conn| RunLock::acquire(
                        conn,
                        &run.owner()
                    )),
                    Ok(None)
                ) {
                    return completion(run, RecoveryOutcome::LiveOrUnreachable);
                }
                std::thread::sleep(
                    Duration::from_millis(5).min(until.saturating_duration_since(Instant::now())),
                );
            }
        } else {
            None
        }
    } else {
        None
    };
    let pane_proof = borrowed
        .as_ref()
        .map(|value| value.guard)
        .or(acquired_pane.as_ref());
    let check_pane = || -> Result<()> {
        if let Some(guard) = pane_proof {
            guard.validate_binding(
                &settings.database_url,
                pinned.as_ref().expect("held concrete placement"),
            )?;
        }
        Ok(())
    };
    check_pane()?;
    let run = with_database_deadline(conn, until, |conn| {
        check_snapshot(conn, &run, pinned.as_ref())
    })?;
    validate_attestation(&run, attestation)?;
    if run.status.terminal() && run.cleanup_state == CleanupState::Complete {
        return completion(run, RecoveryOutcome::Recovered);
    }
    let owned = with_database_deadline(conn, until, |conn| RunLock::acquire(conn, &run.owner()));
    let (lock, path_failure) = match owned {
        Ok(Some(lock)) => (Some(Arc::new(lock)), None),
        Ok(None) => return completion(run, RecoveryOutcome::LiveOrUnreachable),
        Err(cause) => (None, Some(cause.message().into_owned())),
    };
    let process = ProcessIdentity {
        pid: run.owner_pid,
        start_identity: run.owner_start_identity.clone(),
    };
    let owner = host.process_evidence(&process, until)?;
    let startup = if run.owner_kind == OwnerKind::Creator {
        host.startup_process_evidence(&run, pinned.as_ref(), until)?
    } else {
        ProcessEvidence::Terminated
    };
    let health = host.health(&run, until)?;
    let health_live = match &health {
        RecoveryHealth::Matching(health) => {
            health.validate(&run)?;
            true
        }
        RecoveryHealth::Unavailable(reason) | RecoveryHealth::Mismatched(reason) => {
            ensure(
                !reason.is_empty(),
                "health uncertainty requires a diagnostic",
            )?;
            false
        }
    };
    if matches!(owner, ProcessEvidence::Alive)
        || matches!(startup, ProcessEvidence::Alive)
        || health_live
    {
        return completion(run, RecoveryOutcome::LiveOrUnreachable);
    }
    let unknown = matches!(owner, ProcessEvidence::Unknown(_))
        || matches!(startup, ProcessEvidence::Unknown(_));
    if unknown && attestation.is_none() {
        let mut result = completion(run, RecoveryOutcome::OperatorReconciliationRequired)?;
        for evidence in [owner, startup] {
            if let ProcessEvidence::Unknown(reason) = evidence {
                result.report.cleanup_diagnostics.push(json!(reason));
            }
        }
        return Ok(result);
    }
    host.checkpoint(RecoveryBoundary::BeforeTakeover, &run, until)?;
    let process = host.current_process()?;
    if path_failure.is_some() || unknown {
        return exceptional::reconcile(
            conn,
            &run,
            pinned.as_ref(),
            &process,
            attestation.filter(|_| unknown),
            until,
            |conn| {
                check_pane()?;
                let current = check_snapshot(conn, &run, pinned.as_ref())?;
                validate_attestation(&current, attestation)
            },
        );
    }
    let lock = lock.expect("verified ordinary recovery lock");
    let current = lock.transfer_to_reconciler_checked(conn, &run, process, until, |conn| {
        check_pane()?;
        let current = check_snapshot(conn, &run, pinned.as_ref())?;
        validate_attestation(&current, attestation)
    })?;
    if current.status.terminal() && current.cleanup_state == CleanupState::Complete {
        return completion(current, RecoveryOutcome::Recovered);
    }
    replay::reconcile(conn, current, pinned, lock, host, clock, until)
}

pub(super) struct NativeRecoveryHost;
impl RecoveryHost for NativeRecoveryHost {
    fn current_process(&mut self) -> Result<ProcessIdentity> {
        ProcessIdentity::current()
    }
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        until: Instant,
    ) -> Result<ProcessEvidence> {
        check_deadline(Instant::now(), until)?;
        match ProcessIdentity::read(process.pid) {
            Ok(Some(current)) if current == *process => Ok(ProcessEvidence::Alive),
            Ok(_) => Ok(ProcessEvidence::Terminated),
            Err(cause) => Ok(ProcessEvidence::Unknown(cause.message().into_owned())),
        }
    }
    fn startup_process_evidence(
        &mut self,
        _: &RunRecord,
        _: Option<&Placement>,
        until: Instant,
    ) -> Result<ProcessEvidence> {
        check_deadline(Instant::now(), until)?;
        Ok(ProcessEvidence::Unknown(
            "complete generation-bound startup process set unavailable".into(),
        ))
    }
    fn health(&mut self, run: &RunRecord, until: Instant) -> Result<RecoveryHealth> {
        match super::control::request(run, "health", until) {
            Ok(value) => match HealthSnapshot::decode(value, run) {
                Ok(health) => Ok(RecoveryHealth::Matching(health)),
                Err(cause) => Ok(RecoveryHealth::Mismatched(cause.message().into_owned())),
            },
            Err(cause) => Ok(RecoveryHealth::Unavailable(cause.message().into_owned())),
        }
    }
    fn cleanup_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome> {
        super::process::recover_descendants(run, clock)
    }
    fn checkpoint(&mut self, _: RecoveryBoundary, _: &RunRecord, until: Instant) -> Result<()> {
        check_deadline(Instant::now(), until)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;
    use serde_json::json;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::records::Placement;
    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::db::test_support::TableSnapshot;
    use crate::error::CafleetError;
    use crate::runtime::runner::test_support::*;
    use crate::runtime::runner::*;
    use serde_json::Value;
    use std::io::Write;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    pub(crate) type TestResult<T> = std::result::Result<T, CafleetError>;
    pub(crate) const NOW: &str = "2026-10-26T20:00:00.000000+00:00";

    pub(crate) struct Case {
        pub(crate) fixture: Fixture,
        pub(crate) artifacts: RunArtifacts,
        pub(crate) held: Option<Arc<RunLock>>,
        pub(crate) settings: Settings,
        pub(crate) placement: Placement,
    }

    impl Case {
        pub(crate) fn new() -> Self {
            let mut case = Self::creator();
            case.held
                .as_ref()
                .unwrap()
                .transfer_to_runner(
                    &mut case.fixture.conn,
                    &case.fixture.run.owner(),
                    ProcessIdentity::current().unwrap(),
                )
                .unwrap();
            case.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
                    rusqlite::params![NOW, case.fixture.run.run_id],
                )
                .unwrap();
            case.refresh();
            storage::reserve_event_sequences(&mut case.fixture.conn, &case.fixture.run.owner())
                .unwrap();
            case.refresh();
            case
        }

        pub(crate) fn creator() -> Self {
            let mut fixture = Fixture::new();
            let artifacts = fixture.artifacts();
            let held = Arc::new(
                RunLock::acquire(&fixture.conn, &fixture.run.owner())
                    .unwrap()
                    .unwrap(),
            );
            let settings =
                Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()), None);
            let placement = crate::broker::get_member(
                &fixture.conn,
                fixture.run.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement
            .unwrap();
            Self {
                fixture,
                artifacts,
                held: Some(held),
                settings,
                placement,
            }
        }

        pub(crate) fn seed(&self, transitions: &[ObservedTransition]) {
            let held = self
                .held
                .as_ref()
                .expect("the original owner holds its runtime lock while seeding");
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut spool = Outbox::open_owned(
                &self.fixture.conn,
                &self.fixture.run,
                held.clone(),
                deadline,
            )
            .unwrap();
            for transition in transitions {
                spool.append_synced(transition, deadline).unwrap();
            }
        }

        pub(crate) fn replace_owned_bytes(&self, bytes: &[u8]) {
            self.held
                .as_ref()
                .expect("owned spool mutation requires its original lock")
                .validate(&self.fixture.conn, &self.fixture.run.owner())
                .unwrap();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.artifacts.paths.outbox)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }

        pub(crate) fn release(&mut self) {
            self.held.take();
        }

        pub(crate) fn refresh(&mut self) {
            self.fixture.run =
                storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
        }

        /// Commits a live `RunnerExited` transition and leaves the run's cleanup incomplete, the
        /// state an owner leaves when it dies after recording its native exit.
        pub(crate) fn exited_pending_cleanup(
            &mut self,
            code: Option<i64>,
            signal: Option<i64>,
        ) -> (ObservedTransition, RuntimeEventRecord) {
            let transition = exited(&self.fixture.run, 1, code, signal);
            let event = storage::persist_event(
                &mut self.fixture.conn,
                &self.fixture.run.owner(),
                &transition,
                PersistenceMode::Live,
            )
            .unwrap();
            assert_eq!(
                self.fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET cleanup_state='incomplete' WHERE run_id=?1",
                        [self.fixture.run.run_id],
                    )
                    .unwrap(),
                1
            );
            self.refresh();
            (transition, event)
        }

        pub(crate) fn recover(&mut self, host: &mut Host) -> TestResult<RecoveryReport> {
            self.recover_with(host, None)
        }

        pub(crate) fn recover_held(
            &mut self,
            expected: &RunRecord,
            held: &crate::input::PaneLock,
            host: &mut Host,
        ) -> TestResult<RecoveryCompletion> {
            recover_member_with_held_pane_lock(
                &mut self.fixture.conn,
                &self.settings,
                self.fixture.run.member_id,
                expected,
                &self.placement,
                None,
                host.deadline,
                held,
                host,
            )
        }

        pub(crate) fn recover_with(
            &mut self,
            host: &mut Host,
            claim: Option<&OwnerTerminationAttestation>,
        ) -> TestResult<RecoveryReport> {
            self.release();
            recover_member_with_host(
                &mut self.fixture.conn,
                &self.settings,
                self.fixture.run.member_id,
                claim,
                host.deadline,
                host,
            )
        }

        pub(crate) fn spool_bytes(&self) -> Option<Vec<u8>> {
            let run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
            let current = ProcessIdentity::current().unwrap();
            assert_eq!(
                run.owner_pid, current.pid,
                "the current process owns the spool"
            );
            assert_eq!(run.owner_start_identity, current.start_identity);
            let acquired;
            let held = match &self.held {
                Some(held) => held.as_ref(),
                None => {
                    acquired = RunLock::acquire_released(&self.fixture.conn, &run.owner())
                        .unwrap()
                        .expect("the actual current owner reacquires its original matching lock");
                    &acquired
                }
            };
            held.validate(&self.fixture.conn, &run.owner()).unwrap();
            match std::fs::read(&self.artifacts.paths.outbox) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("owned spool snapshot failed: {error}"),
            }
        }
    }

    #[derive(Clone, Copy)]
    pub(crate) enum Evidence {
        Alive,
        Terminated,
        Unknown,
    }
    impl Evidence {
        pub(crate) fn value(self) -> ProcessEvidence {
            match self {
                Self::Alive => ProcessEvidence::Alive,
                Self::Terminated => ProcessEvidence::Terminated,
                Self::Unknown => {
                    ProcessEvidence::Unknown("bounded injected identity uncertainty".into())
                }
            }
        }
    }

    pub(crate) type CheckpointAction =
        Box<dyn FnMut(&str, &RunRecord, Instant) -> TestResult<()> + Send>;
    /// Brackets the coordinator's entry clock: `after_entry` is taken at the
    /// first snapshot checkpoint, which then pauses for `delay`.
    pub(crate) struct ClockProbe {
        pub(crate) before_entry: Instant,
        pub(crate) after_entry: Option<Instant>,
        pub(crate) delay: Duration,
        pub(crate) observed: Option<ShutdownClock>,
    }
    pub(crate) struct UnlinkProbe {
        pub(crate) path: std::path::PathBuf,
        pub(crate) calls: usize,
        pub(crate) fail: bool,
    }
    pub(crate) struct Host {
        pub(crate) deadline: Instant,
        pub(crate) process: Evidence,
        pub(crate) startup: Evidence,
        pub(crate) matching_health: bool,
        pub(crate) cleanup: CleanupState,
        pub(crate) calls: Vec<String>,
        pub(crate) action: Option<CheckpointAction>,
        pub(crate) clock: Option<ClockProbe>,
        pub(crate) unlink: Option<UnlinkProbe>,
    }
    impl Host {
        pub(crate) fn new(deadline: Instant) -> Self {
            Self {
                deadline,
                process: Evidence::Terminated,
                startup: Evidence::Terminated,
                matching_health: false,
                cleanup: CleanupState::Complete,
                calls: Vec::new(),
                action: None,
                clock: None,
                unlink: None,
            }
        }
        pub(crate) fn with_clock(deadline: Instant, delay: Duration) -> Self {
            Self {
                clock: Some(ClockProbe {
                    before_entry: Instant::now(),
                    after_entry: None,
                    delay,
                    observed: None,
                }),
                ..Self::new(deadline)
            }
        }
        pub(crate) fn with_unlink(case: &Case, deadline: Instant, fail: bool) -> Self {
            Self {
                unlink: Some(UnlinkProbe {
                    path: case.artifacts.paths.descriptor.clone(),
                    calls: 0,
                    fail,
                }),
                ..Self::new(deadline)
            }
        }
        fn assert_entry_clock(&mut self, clock: ShutdownClock) {
            let until = self.deadline;
            let Some(probe) = &mut self.clock else {
                return;
            };
            assert!(probe.observed.replace(clock).is_none());
            let after = probe
                .after_entry
                .expect("actual coordinator snapshot brackets its entry");
            let observed = [
                clock.child_deadline(),
                clock.display_deadline(),
                clock.storage_deadline(),
            ];
            let bounds = [2, 3, 4].map(|seconds| {
                (
                    (probe.before_entry + Duration::from_secs(seconds)).min(until),
                    (after + Duration::from_secs(seconds)).min(until),
                )
            });
            assert!(
                observed
                    .iter()
                    .zip(bounds)
                    .all(|(actual, (lower, upper))| *actual >= lower && *actual <= upper),
                "cleanup carries the one clock captured at common coordinator entry: observed={observed:?}, entry_bounds={bounds:?}, original_cap={until:?}"
            );
        }
        pub(crate) fn check(&self, deadline: Instant) -> TestResult<()> {
            assert_eq!(
                deadline, self.deadline,
                "all evidence/checkpoints share one original deadline"
            );
            if Instant::now() >= deadline {
                Err(CafleetError::DeadlineExceeded)
            } else {
                Ok(())
            }
        }
    }
    impl RecoveryHost for Host {
        fn current_process(&mut self) -> TestResult<ProcessIdentity> {
            self.calls.push("current_process".into());
            ProcessIdentity::current()
        }
        fn process_evidence(
            &mut self,
            process: &ProcessIdentity,
            deadline: Instant,
        ) -> TestResult<ProcessEvidence> {
            self.check(deadline)?;
            self.calls.push(format!(
                "process:{}:{}",
                process.pid, process.start_identity
            ));
            Ok(self.process.value())
        }
        fn startup_process_evidence(
            &mut self,
            run: &RunRecord,
            placement: Option<&Placement>,
            deadline: Instant,
        ) -> TestResult<ProcessEvidence> {
            self.check(deadline)?;
            assert_eq!(run.owner_kind, OwnerKind::Creator);
            assert!(placement.is_some());
            self.calls.push("startup".into());
            Ok(self.startup.value())
        }
        fn health(&mut self, run: &RunRecord, deadline: Instant) -> TestResult<RecoveryHealth> {
            self.check(deadline)?;
            self.calls.push("health".into());
            if self.matching_health {
                Ok(RecoveryHealth::Matching(HealthSnapshot {
                    identity: ControlIdentity::from_run(run),
                    runner: ProcessIdentity {
                        pid: run.runner_pid.unwrap(),
                        start_identity: run.runner_start_identity.clone().unwrap(),
                    },
                    child: None,
                    output_seq: 1,
                    observation: None,
                }))
            } else {
                Ok(RecoveryHealth::Unavailable(
                    "bounded injected endpoint unavailable".into(),
                ))
            }
        }
        fn cleanup_owned(
            &mut self,
            _: &RunRecord,
            clock: ShutdownClock,
        ) -> TestResult<ShutdownOutcome> {
            self.assert_entry_clock(clock);
            assert!(clock.child_deadline() <= self.deadline);
            assert!(clock.display_deadline() <= self.deadline);
            assert!(clock.storage_deadline() <= self.deadline);
            self.calls.push("cleanup".into());
            Ok(ShutdownOutcome {
                cleanup_state: self.cleanup,
                ..ShutdownOutcome::complete(None)
            })
        }
        fn checkpoint(
            &mut self,
            boundary: RecoveryBoundary,
            run: &RunRecord,
            deadline: Instant,
        ) -> TestResult<()> {
            let snapshot = matches!(boundary, RecoveryBoundary::AfterSnapshot);
            if snapshot && let Some(probe) = &mut self.clock {
                assert!(probe.after_entry.replace(Instant::now()).is_none());
            }
            self.check(deadline)?;
            let name = match boundary {
                RecoveryBoundary::AfterSnapshot => "after_snapshot",
                RecoveryBoundary::BeforeTakeover => "before_takeover",
                RecoveryBoundary::AfterTakeover => "after_takeover",
                RecoveryBoundary::BeforeReplay => "before_replay",
                RecoveryBoundary::AfterCommit => "after_commit",
                RecoveryBoundary::BeforeCompact => "before_compact",
                RecoveryBoundary::BeforeFinish => "before_finish",
            };
            self.calls.push(name.into());
            assert!(
                self.calls.len() < 1024,
                "a bounded recovery pass makes finite progress"
            );
            if let Some(action) = &mut self.action {
                action(name, run, deadline)?;
            }
            if snapshot && let Some(probe) = &self.clock {
                let resume = Instant::now() + probe.delay;
                assert!(resume < deadline);
                std::thread::sleep(resume.saturating_duration_since(Instant::now()));
            }
            Ok(())
        }
        fn unlink_descriptor(
            &mut self,
            path: &std::path::Path,
            deadline: Instant,
        ) -> TestResult<()> {
            let Some(probe) = &mut self.unlink else {
                return unlink_descriptor_file(path, deadline);
            };
            probe.calls += 1;
            assert_eq!(path, probe.path);
            let fail = probe.fail;
            self.check(deadline)?;
            if fail {
                return Err(CafleetError::App(
                    "owned coordinator unlink-dispatch failure".into(),
                ));
            }
            std::fs::remove_file(path).map_err(|_| {
                CafleetError::App("owned coordinator descriptor removal failed".into())
            })
        }
    }

    pub(crate) fn evidence(run: &RunRecord, sequence: i64) -> Value {
        storage::evidence::run_metadata(run, "recovery-fixture", u64::try_from(sequence).unwrap())
            .unwrap()
    }
    pub(crate) fn opening(run: &RunRecord, sequence: i64, episode: i64) -> ObservedTransition {
        ObservedTransition {
            run_id: run.run_id,
            event_seq: sequence,
            episode_seq: Some(episode),
            kind: RuntimeEventKind::InteractionOpened,
            classification: Some(InteractionClass::PermissionRequest),
            observed_at: NOW.into(),
            evidence_json: evidence(run, sequence),
            close_reason: None,
            cause_json: None,
        }
    }
    pub(crate) fn obscured(run: &RunRecord, sequence: i64, episode: i64) -> ObservedTransition {
        ObservedTransition {
            kind: RuntimeEventKind::InteractionObscured,
            ..opening(run, sequence, episode)
        }
    }
    pub(crate) fn exited(
        run: &RunRecord,
        sequence: i64,
        code: Option<i64>,
        signal: Option<i64>,
    ) -> ObservedTransition {
        ObservedTransition {
            run_id: run.run_id,
            event_seq: sequence,
            episode_seq: None,
            kind: RuntimeEventKind::RunnerExited,
            classification: None,
            observed_at: NOW.into(),
            evidence_json: evidence(run, sequence),
            close_reason: None,
            cause_json: Some(json!({"kind":"native_exit","exit_code":code,"exit_signal":signal})),
        }
    }
    pub(crate) use crate::runtime::runner::writer::test_support::frame;
    pub(crate) fn events(case: &Case) -> Vec<RuntimeEventView> {
        let ids = case
            .fixture
            .conn
            .prepare(
                "SELECT event_id FROM member_runtime_events WHERE run_id=?1 ORDER BY event_seq",
            )
            .unwrap()
            .query_map([case.fixture.run.run_id], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids.into_iter()
            .map(|id| storage::get_event(&case.fixture.conn, id).unwrap())
            .collect()
    }
    pub(crate) fn counts(report: &RecoveryReport) -> (u64, u64, u64) {
        (
            report.replayed_count,
            report.inbox_count,
            report.archived_count,
        )
    }
    pub(crate) fn new_host() -> Host {
        Host::new(Instant::now() + Duration::from_secs(2))
    }

    pub(crate) fn rows(conn: &Connection) -> Vec<TableSnapshot> {
        crate::db::test_support::table_snapshot(conn, |_| true)
    }

    pub(crate) mod missing_paths {
        use super::*;
        pub(crate) fn persist_opening(case: &mut Case) -> RuntimeEventRecord {
            let transition = opening(&case.fixture.run, 1, 1);
            storage::persist_event(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &transition,
                PersistenceMode::Live,
            )
            .unwrap()
        }
        pub(crate) fn missing_lock(case: &mut Case) {
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
        }
        pub(crate) fn claim(run: &RunRecord) -> OwnerTerminationAttestation {
            OwnerTerminationAttestation {
                expected_run_id: run.run_id,
                expected_owner_epoch: run.owner_epoch,
                evidence_ref: "audit:owned-fixture-termination".into(),
            }
        }
        pub(crate) fn assert_loss(case: &Case, report: &RecoveryReport, original: &RunRecord) {
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            assert_eq!(counts(report), (0, 2, 0));
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            assert_eq!(current.owner_epoch, original.owner_epoch + 1);
            assert_eq!(current.owner_kind, OwnerKind::Reconciler);
            assert_eq!(
                current.observation_gap_count,
                original.observation_gap_count + 1
            );
            let events = events(case);
            assert_eq!(
                events.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
                vec![
                    RuntimeEventKind::InteractionOpened,
                    RuntimeEventKind::InteractionClosed,
                    RuntimeEventKind::RunnerLost
                ]
            );
            assert_eq!(
                events[1].interaction.as_ref().unwrap().close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            assert!(
                events[1..]
                    .iter()
                    .all(|row| row.event.event_seq > original.event_seq_reserved_through)
            );
            assert!(
                current
                    .cleanup_diagnostics_json
                    .as_ref()
                    .unwrap()
                    .to_string()
                    .contains("runtime_path_missing")
            );
        }
    }

    pub(crate) mod descriptor {
        use super::*;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        use std::path::Path;

        pub(crate) const PRIVATE_VALUE: &str = "descriptor-private-fixture-canary";

        pub(crate) fn described_creator() -> Case {
            let case = Case::creator();
            let mut spec = case.fixture.spec();
            spec.argv = vec!["/usr/bin/false".into(), PRIVATE_VALUE.into()];
            spec.environment
                .push(("CAFLEET_TEST_PRIVATE_VALUE".into(), PRIVATE_VALUE.into()));
            let descriptor = LaunchDescriptor::new(&case.fixture.run, spec).unwrap();
            case.artifacts.write_descriptor(&descriptor).unwrap();
            case
        }
        pub(crate) fn identity(path: &Path) -> (u64, u64, u64, i64, i64) {
            let value = std::fs::symlink_metadata(path).unwrap();
            (
                value.dev(),
                value.ino(),
                value.len(),
                value.mtime(),
                value.mtime_nsec(),
            )
        }
        pub(crate) fn owned_bytes(case: &Case) -> Vec<u8> {
            let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
            let current = ProcessIdentity::current().unwrap();
            assert_eq!(run.owner_pid, current.pid);
            assert_eq!(run.owner_start_identity, current.start_identity);
            let acquired;
            let lock = if let Some(held) = &case.held {
                held.as_ref()
            } else {
                acquired = RunLock::acquire_released(&case.fixture.conn, &run.owner())
                    .unwrap()
                    .unwrap();
                &acquired
            };
            lock.validate(&case.fixture.conn, &run.owner()).unwrap();
            std::fs::read(&case.artifacts.paths.descriptor).unwrap()
        }
        pub(crate) fn write_private(path: &Path, bytes: &[u8]) {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
        }
        pub(crate) fn secret_free(text: &str) {
            assert!(
                !text.contains(PRIVATE_VALUE),
                "descriptor diagnostics preserve private spawn data"
            );
            assert!(
                !text.contains("CAFLEET_TEST_PRIVATE_VALUE"),
                "descriptor diagnostics preserve private environment names"
            );
        }
        pub(crate) fn incomplete(case: &Case, result: TestResult<RecoveryReport>) {
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
    }

    pub(crate) mod relationships {
        use super::*;
        use std::sync::Mutex;

        #[derive(Clone, Copy, Debug)]
        pub(crate) enum Relationship {
            MemberFleet,
            DirectorFleet,
            FleetDirector,
        }

        #[derive(Clone, Copy, Debug)]
        pub(crate) enum Stage {
            OrdinaryTakeover,
            ExceptionalTakeover,
            Finalization,
        }
        impl Stage {
            pub(crate) fn boundary(self) -> &'static str {
                match self {
                    Self::OrdinaryTakeover | Self::ExceptionalTakeover => "before_takeover",
                    Self::Finalization => "before_finish",
                }
            }
        }

        pub(crate) fn established_terminal() -> Case {
            let mut case = Case::new();
            case.exited_pending_cleanup(Some(7), None);
            assert_eq!(case.fixture.run.status, RunStatus::Exited);
            assert_eq!(case.fixture.run.exit_code, Some(7));
            assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
            case
        }

        pub(crate) fn prepare(stage: Stage) -> Case {
            match stage {
                Stage::OrdinaryTakeover => established_terminal(),
                Stage::ExceptionalTakeover => {
                    let mut case = established_terminal();
                    case.release();
                    std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
                    case
                }
                Stage::Finalization => {
                    let case = Case::new();
                    case.seed(&[exited(&case.fixture.run, 1, Some(7), None)]);
                    assert!(events(&case).is_empty());
                    case
                }
            }
        }

        pub(crate) fn invoke(case: &mut Case, host: &mut Host) -> TestResult<RecoveryReport> {
            case.release();
            let deadline = host.deadline;
            with_database_deadline(&mut case.fixture.conn, deadline, |conn| {
                recover_member_with_host(
                    conn,
                    &case.settings,
                    case.fixture.run.member_id,
                    None,
                    deadline,
                    host,
                )
            })
        }

        pub(crate) fn target(case: &mut Case, relationship: Relationship) -> i64 {
            match relationship {
                Relationship::MemberFleet | Relationship::DirectorFleet => {
                    create_fleet_fixture(&mut case.fixture.conn, "other-relationship-fleet").0
                }
                Relationship::FleetDirector => {
                    crate::broker::register_member(
                        &mut case.fixture.conn,
                        case.fixture.run.fleet_id,
                        "alternate-director",
                        "owned historical relationship fixture",
                        None,
                    )
                    .unwrap()
                    .member_id
                }
            }
        }

        pub(crate) fn change(
            conn: &Connection,
            run: &RunRecord,
            relationship: Relationship,
            target: i64,
        ) {
            let changed = match relationship {
                Relationship::MemberFleet => conn.execute(
                    "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                    rusqlite::params![target, run.member_id],
                ),
                Relationship::DirectorFleet => conn.execute(
                    "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                    rusqlite::params![target, run.director_member_id],
                ),
                Relationship::FleetDirector => conn.execute(
                    "UPDATE fleets SET director_member_id=?1 WHERE fleet_id=?2",
                    rusqlite::params![target, run.fleet_id],
                ),
            }
            .unwrap();
            assert_eq!(changed, 1);
            let selected: i64 = match relationship {
                Relationship::MemberFleet | Relationship::DirectorFleet => conn.query_row(
                    "SELECT fleet_id FROM members WHERE member_id=?1",
                    [if matches!(relationship, Relationship::MemberFleet) {
                        run.member_id
                    } else {
                        run.director_member_id
                    }],
                    |row| row.get(0),
                ),
                Relationship::FleetDirector => conn.query_row(
                    "SELECT director_member_id FROM fleets WHERE fleet_id=?1",
                    [run.fleet_id],
                    |row| row.get(0),
                ),
            }
            .unwrap();
            assert_eq!(selected, target);
        }

        pub(crate) fn refusal(stage: Stage, relationship: Relationship) {
            let mut case = prepare(stage);
            let original = case.fixture.run.clone();
            let target = target(&mut case, relationship);
            let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
            let expected = Arc::new(Mutex::new(None));
            let published = expected.clone();
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, run, deadline| {
                if boundary != stage.boundary() {
                    return Ok(());
                }
                assert!(Instant::now() < deadline);
                assert!(published.lock().unwrap().is_none());
                assert_eq!(run.status, RunStatus::Exited);
                assert_eq!(run.cleanup_state, CleanupState::Incomplete);
                assert_eq!(run.exit_code, Some(7));
                assert_eq!(
                    run.owner_epoch,
                    original.owner_epoch + i64::from(matches!(stage, Stage::Finalization))
                );
                let count: i64 = mutation.query_row(
            "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1 AND kind='runner_exited'",
            [run.run_id], |row| row.get(0),
        ).unwrap();
                assert_eq!(
                    count, 1,
                    "terminal truth exists before the injected relationship change"
                );
                change(&mutation, run, relationship, target);
                *published.lock().unwrap() = Some(rows(&mutation));
                Ok(())
            }));
            let result = invoke(&mut case, &mut host);
            let expected = expected.lock().unwrap();
            assert!(
                expected.is_some(),
                "the selected actual coordinator boundary executes"
            );
            if matches!(stage, Stage::Finalization) {
                for boundary in [
                    "before_replay",
                    "after_commit",
                    "before_compact",
                    "before_finish",
                ] {
                    assert!(host.calls.iter().any(|call| call == boundary), "{boundary}");
                }
                assert!(case.spool_bytes().unwrap().is_empty());
            }
            let actual = rows(&case.fixture.conn);
            let changed = actual
                .iter()
                .zip(expected.as_ref().unwrap())
                .filter_map(|(actual, expected)| {
                    (actual != expected).then_some(actual.name.as_str())
                })
                .collect::<Vec<_>>();
            assert!(
                actual.as_slice() == expected.as_ref().unwrap().as_slice(),
                "relationship refusal preserves the exact database after the injected change: {stage:?}/{relationship:?} changed {changed:?}"
            );
            assert!(
                result.is_err(),
                "a broker relationship mismatch rejects its actual mutation transaction"
            );
            if !matches!(stage, Stage::Finalization) {
                assert!(!host.calls.iter().any(|call| matches!(
                    call.as_str(),
                    "after_takeover" | "before_replay" | "cleanup"
                )));
            }
        }
        #[derive(Clone, Copy, Debug)]
        pub(crate) enum Lifecycle {
            MemberInactive,
            DirectorInactive,
            FleetDeleted,
        }
        impl Lifecycle {
            pub(crate) fn apply(self, conn: &Connection, run: &RunRecord) {
                let changed = match self {
            Self::MemberInactive | Self::DirectorInactive => conn.execute(
                "UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",
                rusqlite::params![
                    NOW,
                    if matches!(self, Self::MemberInactive) {
                        run.member_id
                    } else {
                        run.director_member_id
                    }
                ],
            ),
            Self::FleetDeleted => conn.execute(
                "UPDATE fleets SET deleted_at=?1 WHERE fleet_id=?2",
                rusqlite::params![NOW, run.fleet_id],
            ),
        }
        .unwrap();
                assert_eq!(changed, 1);
            }
            pub(crate) fn archive_reason(self) -> ArchiveReason {
                match self {
                    Self::MemberInactive => ArchiveReason::MemberInactive,
                    Self::DirectorInactive => ArchiveReason::DirectorInactive,
                    Self::FleetDeleted => ArchiveReason::FleetDeleted,
                }
            }
        }

        pub(crate) const LIFECYCLES: [Lifecycle; 3] = [
            Lifecycle::MemberInactive,
            Lifecycle::DirectorInactive,
            Lifecycle::FleetDeleted,
        ];

        /// Ends `lifecycle` before recovery; the member goes through real deregistration, which
        /// also deletes its placement.
        pub(crate) fn end_lifecycle(case: &mut Case, lifecycle: Lifecycle) {
            if matches!(lifecycle, Lifecycle::MemberInactive) {
                assert!(
                    crate::broker::deregister_member(
                        &mut case.fixture.conn,
                        case.fixture.run.member_id
                    )
                    .unwrap()
                );
            } else {
                lifecycle.apply(&case.fixture.conn, &case.fixture.run);
            }
        }

        pub(crate) fn retained_history(conn: &Connection) -> Vec<TableSnapshot> {
            rows(conn)
                .into_iter()
                .filter(|table| {
                    matches!(
                        table.name.as_str(),
                        "messages"
                            | "member_runtime_events"
                            | "member_interactions"
                            | "member_input_operations"
                    ) || table.name.starts_with("member_permission_")
                })
                .collect()
        }

        pub(crate) fn historical_control(stage: Stage) {
            for lifecycle in LIFECYCLES {
                let mut case = prepare(stage);
                let original = case.fixture.run.clone();
                let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
                let prior = Arc::new(Mutex::new(None));
                let published = prior.clone();
                let mut host = new_host();
                host.action = Some(Box::new(move |boundary, run, _| {
                    if boundary == stage.boundary() {
                        assert!(published.lock().unwrap().is_none());
                        lifecycle.apply(&mutation, run);
                        *published.lock().unwrap() = Some(retained_history(&mutation));
                    }
                    Ok(())
                }));
                let report = invoke(&mut case, &mut host).unwrap();
                assert!(prior.lock().unwrap().is_some());
                assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
                assert_eq!(report.status, RunStatus::Exited);
                assert_eq!(
                    counts(&report),
                    if matches!(stage, Stage::Finalization) {
                        (1, 1, 0)
                    } else {
                        (0, 0, 0)
                    }
                );
                let run = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
                assert_eq!(run.owner_epoch, original.owner_epoch + 1);
                assert_eq!(run.owner_kind, OwnerKind::Reconciler);
                assert_eq!(run.status, RunStatus::Exited);
                assert_eq!(run.exit_code, Some(7));
                assert_eq!(run.exit_signal, None);
                assert_eq!(run.cleanup_state, CleanupState::Complete);
                assert_eq!(
                    run.event_seq_reserved_through,
                    original.event_seq_reserved_through
                );
                assert_eq!(
                    run.observation_gap_count,
                    original.observation_gap_count
                        + i64::from(matches!(stage, Stage::ExceptionalTakeover))
                );
                assert_eq!(
                    &retained_history(&case.fixture.conn),
                    prior.lock().unwrap().as_ref().unwrap(),
                    "{stage:?}/{lifecycle:?}"
                );
                assert_eq!(events(&case).len(), 1);
                assert_eq!(
                    events(&case)[0].event.notification_state,
                    NotificationState::Inbox
                );
                if matches!(stage, Stage::ExceptionalTakeover) {
                    assert!(
                        !host
                            .calls
                            .iter()
                            .any(|call| matches!(call.as_str(), "before_replay" | "cleanup"))
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use std::io::Write;

    use super::test_support::*;
    use crate::broker::runtime::{self as storage, *};
    use crate::config::Settings;
    use crate::db::test_support::TableSnapshot;
    use crate::digest::sha256_hex;
    use crate::error::CafleetError;
    use crate::runtime::runner::*;
    use crate::runtime::system::test_support::TestChild;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    mod crash {
        use super::*;
        use std::os::unix::fs::OpenOptionsExt;
        use std::path::{Path, PathBuf};
        use std::process::{Command, Stdio};
        use std::sync::atomic::{AtomicBool, Ordering};

        fn write_json(path: &Path, value: &Value) {
            let temporary = path.with_extension("provisional");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .unwrap();
            file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
            file.sync_all().unwrap();
            std::fs::rename(temporary, path).unwrap();
        }

        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum ChildMode {
            /// Owns the run and stops its writer after the persisted frame's fsync.
            Writer,
            /// Owns the run and stops its writer before the frame is appended.
            Unsynced,
            /// Recovers with an operator claim and holds its exceptional transaction open.
            Transaction,
            /// The first of two competing exceptional recoveries, held before its commit.
            ExceptionalFirst,
            /// The competing exceptional recovery whose original epoch becomes stale.
            ExceptionalStale,
            /// Recovers and holds the takeover until released.
            Hold,
            /// Recovers without pausing.
            Recover,
        }
        impl ChildMode {
            const ALL: [Self; 7] = [
                Self::Writer,
                Self::Unsynced,
                Self::Transaction,
                Self::ExceptionalFirst,
                Self::ExceptionalStale,
                Self::Hold,
                Self::Recover,
            ];
            fn name(self) -> &'static str {
                match self {
                    Self::Writer => "writer",
                    Self::Unsynced => "unsynced",
                    Self::Transaction => "transaction",
                    Self::ExceptionalFirst => "exceptional_first",
                    Self::ExceptionalStale => "exceptional_stale",
                    Self::Hold => "hold",
                    Self::Recover => "recover",
                }
            }
            fn parse(name: &str) -> Self {
                Self::ALL
                    .into_iter()
                    .find(|mode| mode.name() == name)
                    .unwrap_or_else(|| panic!("unknown owned recovery helper mode {name:?}"))
            }
        }

        struct Task {
            child: TestChild,
            log: PathBuf,
            deadline: Instant,
        }
        impl Task {
            fn spawn(
                case: &Case,
                mode: ChildMode,
                marker: &Path,
                release: &Path,
                result: &Path,
            ) -> Self {
                let mode = mode.name();
                let config = case.fixture._dir.path().join(format!("{mode}-config.json"));
                write_json(
                    &config,
                    &json!({"mode":mode,"database_url":case.settings.database_url,"run_id":case.fixture.run.run_id,"member_id":case.fixture.run.member_id,"expected_epoch":case.fixture.run.owner_epoch,"marker":marker,"release":release,"result":result}),
                );
                let log = case.fixture._dir.path().join(format!("{mode}-child.log"));
                let output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&log)
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(6);
                let child = TestChild::new(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--ignored",
                            "--exact",
                            runner_recovery_owned_process_child_path(),
                            "--nocapture",
                        ])
                        .env("CAFLEET_TEST_RECOVERY_CONFIG", config)
                        .stdin(Stdio::null())
                        .stdout(Stdio::from(output.try_clone().unwrap()))
                        .stderr(Stdio::from(output))
                        .spawn()
                        .unwrap(),
                );
                Self {
                    child,
                    log,
                    deadline,
                }
            }
            fn await_json(&mut self, path: &Path) -> Value {
                loop {
                    match std::fs::read(path) {
                        Ok(bytes) => return serde_json::from_slice(&bytes).unwrap(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => panic!("owned marker read failed: {error}"),
                    }
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "owned helper exited before {}: {}",
                        path.display(),
                        std::fs::read_to_string(&self.log).unwrap()
                    );
                    assert!(
                        Instant::now() < self.deadline - Duration::from_millis(500),
                        "owned marker timeout: {}",
                        std::fs::read_to_string(&self.log).unwrap()
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            fn finish(&mut self) {
                let status = self
                    .child
                    .poll_until(self.deadline - Duration::from_millis(500))
                    .unwrap();
                if status.is_none() {
                    self.child.terminate_until(self.deadline);
                }
                assert!(
                    status.is_some_and(|status| status.success()),
                    "owned helper: {status:?}: {}",
                    std::fs::read_to_string(&self.log).unwrap()
                );
            }
            fn kill_and_reap(&mut self) {
                let process = ProcessIdentity::read(i64::from(self.child.id()))
                    .unwrap()
                    .expect("the selected owned crash helper remains alive");
                assert!(process.matches().unwrap());
                self.child.terminate_until(self.deadline);
                assert!(self.child.try_wait().unwrap().is_some());
                assert!(!process.matches().unwrap());
            }
        }
        impl Drop for Task {
            fn drop(&mut self) {
                self.child.terminate_until(self.deadline);
            }
        }

        struct Release(PathBuf);
        impl Release {
            fn release(&self) {
                std::fs::write(&self.0, b"release").unwrap();
            }
        }
        impl Drop for Release {
            fn drop(&mut self) {
                drop(std::fs::write(&self.0, b"release"));
            }
        }

        #[test]
        fn runner_recovery_hard_kill_after_actual_writer_fsync_replays_the_original_once() {
            let mut case = Case::creator();
            case.release();
            let marker = case.fixture._dir.path().join("fsynced.json");
            let release = case.fixture._dir.path().join("writer-release");
            let result = case.fixture._dir.path().join("unused-result.json");
            let mut task = Task::spawn(&case, ChildMode::Writer, &marker, &release, &result);
            let snapshot = task.await_json(&marker);
            case.refresh();
            assert_eq!(snapshot["run_id"], case.fixture.run.run_id);
            assert_eq!(snapshot["owner_epoch"], case.fixture.run.owner_epoch);
            assert_eq!(snapshot["owner_pid"], i64::from(task.child.id()));
            assert_eq!(
                snapshot["owner_start_identity"],
                case.fixture.run.owner_start_identity
            );
            assert_eq!(snapshot["event_seq"], 1);
            assert!(snapshot["spool_bytes"].as_u64().unwrap() > 36);
            assert!(events(&case).is_empty());
            task.kill_and_reap();
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (1, 3, 0));
            assert_eq!(report.status, RunStatus::Lost);
            let committed = events(&case);
            assert_eq!(committed.len(), 3);
            assert_eq!(committed[0].event.kind, RuntimeEventKind::InteractionOpened);
            assert_eq!(committed[0].event.event_seq, 1);
            assert_eq!(
                sha256_hex(&frame(
                    &storage::original_transition(&committed[0]).unwrap()
                )),
                snapshot["spool_digest"].as_str().unwrap()
            );
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
            assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
            assert_eq!(events(&case), committed);
        }

        #[test]
        fn runner_recovery_hard_kill_before_writer_append_records_loss_without_inventing_an_observation()
         {
            let mut case = Case::creator();
            case.release();
            let marker = case.fixture._dir.path().join("before-append.json");
            let release = case.fixture._dir.path().join("before-append-release");
            let result = case.fixture._dir.path().join("before-append-unused.json");
            let mut task = Task::spawn(&case, ChildMode::Unsynced, &marker, &release, &result);
            let snapshot = task.await_json(&marker);
            assert_eq!(snapshot["spool_bytes"], 0);
            assert!(events(&case).is_empty());
            case.refresh();
            task.kill_and_reap();
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (0, 1, 0));
            let committed = events(&case);
            assert_eq!(committed.len(), 1);
            assert_eq!(committed[0].event.kind, RuntimeEventKind::RunnerLost);
            assert!(committed[0].interaction.is_none());
        }

        #[test]
        fn runner_recovery_concurrent_owned_processes_contend_on_the_actual_original_lock_and_epoch()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            case.release();
            let marker = case.fixture._dir.path().join("takeover.json");
            let release = case.fixture._dir.path().join("takeover-release");
            let first_result = case.fixture._dir.path().join("first-result.json");
            let second_result = case.fixture._dir.path().join("second-result.json");
            let mut first = Task::spawn(&case, ChildMode::Hold, &marker, &release, &first_result);
            let gate = Release(release.clone());
            first.await_json(&marker);
            let mut second = Task::spawn(
                &case,
                ChildMode::Recover,
                &case.fixture._dir.path().join("second-marker.json"),
                &case.fixture._dir.path().join("second-release"),
                &second_result,
            );
            let deferred = second.await_json(&second_result);
            second.finish();
            assert_eq!(deferred["report"]["outcome"], "live_or_unreachable");
            assert_eq!(deferred["report"]["replayed_count"], 0);
            assert_eq!(deferred["spool_state"], "not_owner");
            gate.release();
            let recovered = first.await_json(&first_result);
            first.finish();
            assert_eq!(recovered["report"]["outcome"], "recovered");
            assert_eq!(recovered["report"]["replayed_count"], 1);
            assert_eq!(recovered["report"]["inbox_count"], 3);
            assert_eq!(recovered["spool_state"], "cleared");
            let events = events(&case);
            assert_eq!(events.len(), 3);
            assert_eq!(
                events
                    .iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                    .count(),
                1
            );
            let mut message_ids = events
                .iter()
                .map(|row| row.event.message_id.unwrap())
                .collect::<Vec<_>>();
            message_ids.sort();
            message_ids.dedup();
            assert_eq!(message_ids.len(), 3);
        }

        #[test]
        fn runner_recovery_owned_hard_kill_inside_exceptional_transaction_rolls_back_every_record()
        {
            let mut case = exceptional_case_with_concurrent_reads();
            let value = opening(&case.fixture.run, 1, 1);
            storage::persist_event(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &value,
                PersistenceMode::Live,
            )
            .unwrap();
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            let before = rows(&case.fixture.conn);
            let marker = case.fixture._dir.path().join("transaction-open.json");
            let release = case.fixture._dir.path().join("transaction-release");
            let result = case.fixture._dir.path().join("transaction-result.json");
            let mut task = Task::spawn(&case, ChildMode::Transaction, &marker, &release, &result);
            let pending = task.await_json(&marker);
            assert_eq!(pending["journal_mode"], "wal", "{pending}");
            assert_eq!(pending["event_rows"], 2, "{pending}");
            assert_eq!(pending["run_rows"], 1, "{pending}");
            assert_eq!(pending["interaction_rows"], 1, "{pending}");
            let read = with_database_deadline(
                &mut case.fixture.conn,
                task.deadline - Duration::from_millis(500),
                |conn| {
                    conn.query_row("SELECT COUNT(*) FROM member_runtime_events", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(crate::error::CafleetError::from)
                },
            );
            let read = read.unwrap_or_else(|cause| {
                panic!(
                    "{cause}; child log: {}",
                    std::fs::read_to_string(&task.log).unwrap()
                )
            });
            assert_eq!(read, 1);
            assert_eq!(
                rows(&case.fixture.conn),
                before,
                "the open exceptional transaction has no visible partial commit"
            );
            task.kill_and_reap();
            assert_eq!(
                rows(&case.fixture.conn),
                before,
                "SQLite rolls back the killed owner's actual transaction"
            );
            let attestation = OwnerTerminationAttestation {
                expected_run_id: case.fixture.run.run_id,
                expected_owner_epoch: case.fixture.run.owner_epoch,
                evidence_ref: "audit:owned-transaction-fixture".into(),
            };
            let mut host = new_host();
            host.process = Evidence::Unknown;
            let report = recover_member_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                Some(&attestation),
                host.deadline,
                &mut host,
            )
            .unwrap();
            assert_eq!(counts(&report), (0, 2, 0));
            assert_eq!(events(&case).len(), 3);
        }

        #[test]
        fn runner_recovery_concurrent_exceptional_processes_commit_one_gap_and_reject_the_original_epoch()
         {
            let mut case = exceptional_case_with_concurrent_reads();
            let value = opening(&case.fixture.run, 1, 1);
            storage::persist_event(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &value,
                PersistenceMode::Live,
            )
            .unwrap();
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            let old_epoch = case.fixture.run.owner_epoch;
            let old_gap = case.fixture.run.observation_gap_count;
            let marker = case
                .fixture
                ._dir
                .path()
                .join("exceptional-before-commit.json");
            let release = case.fixture._dir.path().join("exceptional-release");
            let first_result = case.fixture._dir.path().join("exceptional-first.json");
            let mut first = Task::spawn(
                &case,
                ChildMode::ExceptionalFirst,
                &marker,
                &release,
                &first_result,
            );
            let gate = Release(release.clone());
            let pending = first.await_json(&marker);
            assert_eq!(pending["journal_mode"], "wal", "{pending}");
            assert_eq!(pending["event_rows"], 2, "{pending}");
            let second_marker = case
                .fixture
                ._dir
                .path()
                .join("exceptional-second-snapshot.json");
            let second_result = case.fixture._dir.path().join("exceptional-second.json");
            let mut second = Task::spawn(
                &case,
                ChildMode::ExceptionalStale,
                &second_marker,
                &case.fixture._dir.path().join("unused-exceptional-release"),
                &second_result,
            );
            let captured = second.await_json(&second_marker);
            assert_eq!(captured["epoch"], old_epoch, "{captured}");
            assert_eq!(captured["journal_mode"], "wal", "{captured}");
            gate.release();
            let accepted = first.await_json(&first_result);
            first.finish();
            let rejected = second.await_json(&second_result);
            second.finish();
            assert_eq!(accepted["report"]["replayed_count"], 0);
            assert_eq!(accepted["report"]["inbox_count"], 2);
            assert_eq!(accepted["spool_state"], "database_only");
            assert!(!rejected["error"].as_str().unwrap().is_empty());
            assert_eq!(rejected["expected_epoch"], old_epoch);
            assert_eq!(rejected["current_epoch"], old_epoch + 1);
            let current = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
            assert_eq!(current.owner_epoch, old_epoch + 1);
            assert_eq!(current.observation_gap_count, old_gap + 1);
            assert_eq!(events(&case).len(), 3);
        }

        fn exceptional_case_with_concurrent_reads() -> Case {
            let case = Case::new();
            let mode: String = case
                .fixture
                .conn
                .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            case
        }

        struct PersistGate {
            run: RunRecord,
            lock: Arc<RunLock>,
            paths: RuntimePaths,
            marker: PathBuf,
            release: PathBuf,
            written: AtomicBool,
            boundary: WriterBoundary,
        }
        impl WriterHooks for PersistGate {
            fn before(&self, boundary: WriterBoundary, deadline: Instant) -> TestResult<()> {
                if std::mem::discriminant(&boundary) != std::mem::discriminant(&self.boundary) {
                    return Ok(());
                }
                if !self.written.swap(true, Ordering::AcqRel) {
                    let observed = crate::db::connect_read_only(&format!(
                        "sqlite:///{}",
                        self.run.database_identity_json["canonical_path"]
                            .as_str()
                            .unwrap()
                    ))?;
                    self.lock.validate(&observed, &self.run.owner())?;
                    let bytes = std::fs::read(&self.paths.outbox).unwrap();
                    write_json(
                        &self.marker,
                        &json!({"run_id":self.run.run_id,"owner_epoch":self.run.owner_epoch,"owner_pid":self.run.owner_pid,"owner_start_identity":self.run.owner_start_identity,"event_seq":1,"spool_bytes":bytes.len(),"spool_digest":sha256_hex(&bytes)}),
                    );
                }
                while Instant::now() < deadline {
                    if self.release.exists() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(CafleetError::DeadlineExceeded)
            }
        }

        pub(crate) fn runner_recovery_owned_process_child_path() -> &'static str {
            test_path!(runner_recovery_owned_process_child)
        }

        #[test]
        #[ignore = "owned isolated recovery process helper"]
        fn runner_recovery_owned_process_child() {
            let config: Value = serde_json::from_slice(
                &std::fs::read(std::env::var("CAFLEET_TEST_RECOVERY_CONFIG").unwrap()).unwrap(),
            )
            .unwrap();
            let mode = ChildMode::parse(config["mode"].as_str().unwrap());
            let database_url = config["database_url"].as_str().unwrap();
            let run_id = config["run_id"].as_i64().unwrap();
            let member_id = config["member_id"].as_i64().unwrap();
            let marker = PathBuf::from(config["marker"].as_str().unwrap());
            let release = PathBuf::from(config["release"].as_str().unwrap());
            let result = PathBuf::from(config["result"].as_str().unwrap());
            let mut conn = crate::db::connect(database_url).unwrap();
            let started = Instant::now();
            let deadline = started + Duration::from_secs(4);
            let journal_mode: String = conn
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap();
            if matches!(mode, ChildMode::Writer | ChildMode::Unsynced) {
                let original = storage::get_run(&conn, run_id).unwrap();
                let lock = Arc::new(RunLock::acquire(&conn, &original.owner()).unwrap().unwrap());
                let owner = lock
                    .transfer_to_runner(
                        &mut conn,
                        &original.owner(),
                        ProcessIdentity::current().unwrap(),
                    )
                    .unwrap();
                conn.execute(
                    "UPDATE member_runs SET status='running',started_at=?1 WHERE run_id=?2",
                    rusqlite::params![NOW, run_id],
                )
                .unwrap();
                storage::reserve_event_sequences(&mut conn, &owner).unwrap();
                let run = storage::get_run(&conn, run_id).unwrap();
                let paths = validate_run_paths(&conn, &run).unwrap();
                let hooks = Arc::new(PersistGate {
                    run: run.clone(),
                    lock: lock.clone(),
                    paths: paths.clone(),
                    marker,
                    release,
                    written: AtomicBool::new(false),
                    boundary: if mode == ChildMode::Writer {
                        WriterBoundary::Persist
                    } else {
                        WriterBoundary::Append
                    },
                });
                let mut writer = EventWriter::start_with_hooks(
                    database_url.into(),
                    run.clone(),
                    lock,
                    paths.outbox,
                    hooks,
                )
                .unwrap();
                writer.submit(opening(&run, 1, 1)).unwrap();
                let status = writer.finish(deadline);
                panic!("the selected writer should be killed after fsync: {status:?}");
            }
            let settings = Settings::for_database(
                std::path::Path::new(crate::db::database_path(database_url).unwrap()),
                None,
            );
            let mut host = Host::new(deadline);
            let claim;
            let attestation = if matches!(
                mode,
                ChildMode::Transaction | ChildMode::ExceptionalFirst | ChildMode::ExceptionalStale
            ) {
                let marker = marker.clone();
                let release = release.clone();
                if mode != ChildMode::ExceptionalStale {
                    let changed = Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
                        String,
                        std::collections::BTreeSet<i64>,
                    >::new()));
                    let updates = changed.clone();
                    conn.update_hook(Some(
                        move |_: rusqlite::hooks::Action, _: &str, table: &str, rowid: i64| {
                            updates
                                .lock()
                                .expect("owned transaction update trace")
                                .entry(table.into())
                                .or_default()
                                .insert(rowid);
                        },
                    ))
                    .unwrap();
                    let hook_journal_mode = journal_mode.clone();
                    conn.commit_hook(Some(move || {
            let trace=changed.lock().expect("owned transaction commit trace");
            let count=|table:&str|trace.get(table).map_or(0,std::collections::BTreeSet::len);
            let state=json!({"transaction":"before-exceptional-commit","event_rows":count("member_runtime_events"),"run_rows":count("member_runs"),"interaction_rows":count("member_interactions"),"journal_mode":hook_journal_mode,"helper_elapsed_ms":started.elapsed().as_millis(),"remaining_ms":deadline.saturating_duration_since(Instant::now()).as_millis()});
            drop(trace);
            write_json(&marker,&state);
            while Instant::now()<deadline && !release.exists(){std::thread::sleep(Duration::from_millis(1));}
            !release.exists()
        })).unwrap();
                }
                claim = OwnerTerminationAttestation {
                    expected_run_id: run_id,
                    expected_owner_epoch: config["expected_epoch"].as_i64().unwrap(),
                    evidence_ref: "audit:owned-transaction-fixture".into(),
                };
                host.process = Evidence::Unknown;
                Some(&claim)
            } else {
                assert!(matches!(mode, ChildMode::Hold | ChildMode::Recover));
                None
            };
            if mode == ChildMode::Hold {
                host.action = Some(Box::new(move |boundary, run, until| {
                    if boundary == "after_takeover" {
                        write_json(&marker, &json!({"epoch":run.owner_epoch}));
                        while Instant::now() < until && !release.exists() {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        if !release.exists() {
                            return Err(CafleetError::DeadlineExceeded);
                        }
                    }
                    Ok(())
                }));
            } else if mode == ChildMode::ExceptionalStale {
                let mut written = false;
                host.action = Some(Box::new(move |boundary, run, _| {
                    if boundary == "after_snapshot" && !written {
                        write_json(
                            &marker,
                            &json!({"epoch":run.owner_epoch,"journal_mode":journal_mode,"helper_elapsed_ms":started.elapsed().as_millis()}),
                        );
                        written = true;
                    }
                    Ok(())
                }));
            }
            let recovery = recover_member_with_host(
                &mut conn,
                &settings,
                member_id,
                attestation,
                deadline,
                &mut host,
            );
            if mode == ChildMode::ExceptionalStale {
                let error = match recovery {
                    Ok(_) => {
                        panic!(
                            "the original exceptional epoch becomes stale after the competing commit"
                        )
                    }
                    Err(error) => error,
                };
                let current = storage::get_run(&conn, run_id).unwrap();
                write_json(
                    &result,
                    &json!({"error":error.message(),"expected_epoch":config["expected_epoch"],"current_epoch":current.owner_epoch}),
                );
                return;
            }
            let report = recovery.unwrap();
            let run = storage::get_run(&conn, run_id).unwrap();
            let actual = ProcessIdentity::current().unwrap();
            let spool_state =
                if matches!(mode, ChildMode::Transaction | ChildMode::ExceptionalFirst) {
                    "database_only"
                } else if run.owner_pid == actual.pid
                    && run.owner_start_identity == actual.start_identity
                {
                    let held = RunLock::acquire_released(&conn, &run.owner())
                        .unwrap()
                        .unwrap();
                    held.validate(&conn, &run.owner()).unwrap();
                    let paths = validate_run_paths(&conn, &run).unwrap();
                    match std::fs::read(paths.outbox) {
                        Ok(bytes) => {
                            assert!(bytes.is_empty());
                            "cleared"
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "cleared",
                        Err(error) => panic!("owned final spool read failed: {error}"),
                    }
                } else {
                    "not_owner"
                };
            write_json(
                &result,
                &json!({"report":crate::presentation::runtime_recovery(&report),"spool_state":spool_state}),
            );
        }
    }

    mod identity {
        use super::*;
        use crate::input::PaneLock;

        #[test]
        fn runner_recovery_matching_held_original_lock_defers_without_mutating_rows_or_spool() {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let before = rows(&case.fixture.conn);
            let bytes = case.spool_bytes().unwrap();
            let mut host = new_host();
            let report = recover_member_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                None,
                host.deadline,
                &mut host,
            )
            .unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
            assert_eq!(counts(&report), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), before);
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            assert!(!host.calls.iter().any(|name| name == "after_takeover"
                || name == "cleanup"
                || name == "before_replay"));
        }

        #[test]
        fn runner_recovery_copied_registry_identity_fails_before_touching_the_original_owner_namespace()
         {
            let case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let original_rows = rows(&case.fixture.conn);
            let original_spool = case.spool_bytes().unwrap();
            let copy = case.fixture._dir.path().join("copied.db");
            std::fs::copy(case.fixture.conn.path().unwrap(), &copy).unwrap();
            let mut conn = Connection::open(&copy).unwrap();
            let copied_before = rows(&conn);
            let settings = Settings::for_database(&copy, None);
            let mut host = new_host();
            assert!(
                recover_member_with_host(
                    &mut conn,
                    &settings,
                    case.fixture.run.member_id,
                    None,
                    host.deadline,
                    &mut host
                )
                .is_err()
            );
            assert_eq!(rows(&conn), copied_before);
            assert_eq!(rows(&case.fixture.conn), original_rows);
            assert_eq!(case.spool_bytes().unwrap(), original_spool);
        }

        #[test]
        fn runner_recovery_creator_free_precreated_lock_requires_complete_startup_evidence() {
            for evidence in [Evidence::Alive, Evidence::Unknown] {
                let mut case = Case::creator();
                let before = rows(&case.fixture.conn);
                let mut host = new_host();
                host.startup = evidence;
                let report = case.recover(&mut host).unwrap();
                match evidence {
                    Evidence::Alive => {
                        assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable))
                    }
                    Evidence::Unknown => assert!(matches!(
                        report.outcome,
                        RecoveryOutcome::OperatorReconciliationRequired
                    )),
                    Evidence::Terminated => unreachable!(),
                }
                assert!(host.calls.iter().any(|name| name == "startup"));
                assert_eq!(counts(&report), (0, 0, 0));
                assert_eq!(rows(&case.fixture.conn), before);
            }
        }

        #[test]
        fn runner_recovery_creator_positive_complete_startup_termination_allows_one_bookkeeping_loss()
         {
            let mut case = Case::creator();
            let epoch = case.fixture.run.owner_epoch;
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(host.calls.iter().any(|name| name == "startup"));
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            assert_eq!(
                events(&case)
                    .iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                    .count(),
                1
            );
            case.refresh();
            assert_eq!(case.fixture.run.owner_epoch, epoch + 1);
        }

        #[test]
        fn runner_recovery_borrowed_original_guard_reaches_actual_replay_without_reacquiring_pane()
        {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            case.release();
            let mut host = new_host();
            let completion = case.recover_held(&expected, &held, &mut host).unwrap();
            assert_eq!(counts(&completion.report), (1, 3, 0));
            assert_eq!(completion.run.owner_epoch, expected.owner_epoch + 1);
            assert_eq!(
                completion.run,
                storage::get_run(&case.fixture.conn, expected.run_id).unwrap()
            );
            assert!(
                PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                    .unwrap()
                    .is_none()
            );
            assert!(host.calls.iter().any(|name| name == "before_replay"));
        }

        #[test]
        fn runner_recovery_borrowed_call_rejects_pre_entry_foreign_epoch_instead_of_recapturing_it()
        {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            case.release();
            case.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [expected.run_id],
                )
                .unwrap();
            let before = rows(&case.fixture.conn);
            let bytes = case.spool_bytes().unwrap();
            let mut host = new_host();
            assert!(case.recover_held(&expected, &held, &mut host).is_err());
            assert_eq!(rows(&case.fixture.conn), before);
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|name| name == "after_takeover" || name == "cleanup")
            );
        }

        #[test]
        fn runner_recovery_borrowed_terminal_no_work_validates_fixed_starting_identity() {
            let mut case = Case::new();
            case.fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at=?1 WHERE run_id=?2",rusqlite::params![NOW,case.fixture.run.run_id]).unwrap();
            case.refresh();
            let expected = case.fixture.run.clone();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            case.release();
            case.fixture
                .conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [expected.run_id],
                )
                .unwrap();
            let before = rows(&case.fixture.conn);
            let mut host = new_host();
            assert!(case.recover_held(&expected, &held, &mut host).is_err());
            assert_eq!(rows(&case.fixture.conn), before);
            case.refresh();
            let matching = case.fixture.run.clone();
            let mut host = new_host();
            let completion = case.recover_held(&matching, &held, &mut host).unwrap();
            assert_eq!(counts(&completion.report), (0, 0, 0));
            assert_eq!(completion.run, matching);
            assert_eq!(rows(&case.fixture.conn), before);
        }

        #[test]
        fn runner_recovery_borrowed_expectations_reject_stable_run_and_concrete_placement_mismatches()
         {
            for field in [
                "run",
                "member",
                "fleet",
                "director",
                "backend",
                "nonce",
                "epoch",
                "owner_pid",
                "owner_start",
                "directory",
                "lock",
                "runtime_path",
                "runner",
                "child",
                "database",
                "pane",
                "session",
                "window",
                "coding_agent",
            ] {
                let mut case = Case::new();
                let mut expected = case.fixture.run.clone();
                let mut placement = case.placement.clone();
                match field {
                    "run" => expected.run_id += 1,
                    "member" => expected.member_id += 1,
                    "fleet" => expected.fleet_id += 1,
                    "director" => expected.director_member_id += 1,
                    "backend" => expected.backend = "claude".into(),
                    "nonce" => expected.launch_nonce = "b".repeat(64),
                    "epoch" => expected.owner_epoch += 1,
                    "owner_pid" => expected.owner_pid += 1,
                    "owner_start" => expected.owner_start_identity.push_str("-foreign"),
                    "directory" => {
                        expected.runtime_dir_identity_json = Some(json!({"device":0,"inode":0}))
                    }
                    "lock" => expected.run_lock_identity_json = Some(json!({"device":0,"inode":0})),
                    "runtime_path" => expected.runtime_path.push_str("-foreign"),
                    "runner" => expected.runner_start_identity = Some("foreign-generation".into()),
                    "child" => {
                        expected.child_pid = Some(1);
                        expected.child_start_identity = Some("foreign-generation".into());
                    }
                    "database" => {
                        expected.database_identity_json["instance_id"] = json!("0".repeat(32))
                    }
                    "pane" => placement.mux_pane_id = Some("%foreign".into()),
                    "session" => placement.mux_session.push_str("-foreign"),
                    "window" => placement.mux_window_id.push_str("-foreign"),
                    "coding_agent" => placement.coding_agent = "claude".into(),
                    _ => unreachable!(),
                }
                let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                    .unwrap()
                    .unwrap();
                case.release();
                let before = rows(&case.fixture.conn);
                let mut host = new_host();
                assert!(
                    recover_member_with_held_pane_lock(
                        &mut case.fixture.conn,
                        &case.settings,
                        case.fixture.run.member_id,
                        &expected,
                        &placement,
                        None,
                        host.deadline,
                        &held,
                        &mut host
                    )
                    .is_err(),
                    "{field}"
                );
                assert_eq!(rows(&case.fixture.conn), before, "{field}");
            }
        }

        #[test]
        fn runner_recovery_borrowed_fixed_snapshot_accepts_changed_mutable_health_and_current_counters()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            storage::reserve_event_sequences(&mut case.fixture.conn, &expected.owner()).unwrap();
            case.fixture.conn.execute("UPDATE member_runs SET last_health_at=?1,reachability='suspect',observation_gap_count=observation_gap_count+1 WHERE run_id=?2",rusqlite::params![NOW,expected.run_id]).unwrap();
            let ceiling = storage::get_run(&case.fixture.conn, expected.run_id)
                .unwrap()
                .event_seq_reserved_through;
            case.release();
            let mut host = new_host();
            let completion = case.recover_held(&expected, &held, &mut host).unwrap();
            assert_eq!(counts(&completion.report), (1, 3, 0));
            assert!(
                events(&case)[1..]
                    .iter()
                    .all(|row| row.event.event_seq > ceiling)
            );
        }

        #[test]
        fn runner_recovery_after_snapshot_precedes_real_pane_wait_and_revalidates_stable_owner() {
            use std::sync::mpsc;
            let mut case = Case::new();
            let pane = PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                .unwrap()
                .unwrap();
            let original_epoch = case.fixture.run.owner_epoch;
            let run_id = case.fixture.run.run_id;
            let path = case.fixture.conn.path().unwrap().to_owned();
            let (send, receive) = mpsc::sync_channel(1);
            let mut send = Some(send);
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary == "after_snapshot"
                    && let Some(send) = send.take()
                {
                    assert_eq!(run.owner_epoch, original_epoch);
                    send.send(()).unwrap();
                }
                if boundary == "after_takeover" {
                    assert!(run.owner_epoch >= original_epoch + 2);
                }
                Ok(())
            }));
            std::thread::scope(|scope| {
                let mutation = scope.spawn(move || {
                    receive.recv_timeout(Duration::from_secs(1)).unwrap();
                    let connection = Connection::open(path).unwrap();
                    connection
                        .execute(
                            "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                            [run_id],
                        )
                        .unwrap();
                    drop(pane);
                });
                let result = case.recover(&mut host);
                mutation.join().unwrap();
                assert!(host.calls.iter().any(|name| name == "after_snapshot"));
                match result {
                    Ok(report) => assert_eq!(report.status, RunStatus::Lost),
                    Err(_) => assert!(events(&case).is_empty()),
                }
            });
        }
    }

    mod clock {
        use super::*;
        use crate::input::PaneLock;

        fn clock_case(borrowed: bool, cap: Duration, delay: Duration) {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = borrowed.then(|| {
                PaneLock::try_acquire(&case.settings.database_url, &case.placement)
                    .unwrap()
                    .unwrap()
            });
            case.release();
            let until = Instant::now() + cap;
            let mut host = Host::with_clock(until, delay);
            let report = if let Some(held) = held.as_ref() {
                with_database_deadline(&mut case.fixture.conn, until, |conn| {
                    std::thread::sleep(Duration::from_millis(100));
                    host.clock.as_mut().unwrap().before_entry = Instant::now();
                    recover_member_with_held_pane_lock(
                        conn,
                        &case.settings,
                        expected.member_id,
                        &expected,
                        &case.placement,
                        None,
                        until,
                        held,
                        &mut host,
                    )
                    .map(|completion| completion.report)
                })
                .unwrap()
            } else {
                host.clock.as_mut().unwrap().before_entry = Instant::now();
                recover_member_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    expected.member_id,
                    None,
                    until,
                    &mut host,
                )
                .unwrap()
            };
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert!(host.clock.as_ref().unwrap().observed.is_some());
            assert_eq!(counts(&report), (1, 3, 0));
        }

        #[test]
        fn runner_recovery_standalone_cleanup_uses_original_entry_clock_after_wait_and_replay() {
            clock_case(false, Duration::from_secs(7), Duration::from_millis(150));
        }

        #[test]
        fn runner_recovery_borrowed_cleanup_keeps_entry_clock_and_original_outer_cap() {
            clock_case(
                true,
                Duration::from_millis(3300),
                Duration::from_millis(150),
            );
        }

        #[test]
        fn runner_recovery_short_outer_cap_clips_every_actual_cleanup_boundary() {
            for borrowed in [false, true] {
                clock_case(
                    borrowed,
                    Duration::from_millis(500),
                    Duration::from_millis(80),
                );
            }
        }
    }

    mod completion {
        use super::*;
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Mutex, mpsc};

        type CompletedSnapshot = Arc<Mutex<Option<Vec<TableSnapshot>>>>;

        fn complete(
            conn: &Connection,
            run_id: i64,
            status: RunStatus,
            foreign_owner: bool,
        ) -> Vec<TableSnapshot> {
            assert_eq!(conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='complete',exit_code=?2,exit_signal=NULL,ended_at=?3,cleanup_diagnostics_json=?4,owner_epoch=owner_epoch+?5 WHERE run_id=?6",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, json!([{"kind":"same_owner_completion","status":status.as_str()}]).to_string(), i64::from(foreign_owner), run_id],
    ).unwrap(), 1);
            rows(conn)
        }

        fn prepare(exceptional: bool) -> (Case, Option<OwnerTerminationAttestation>) {
            let mut case = Case::new();
            let claim = exceptional.then(|| OwnerTerminationAttestation {
                expected_run_id: case.fixture.run.run_id,
                expected_owner_epoch: case.fixture.run.owner_epoch,
                evidence_ref: "audit:completion-race".into(),
            });
            case.release();
            if exceptional {
                std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            }
            (case, claim)
        }

        fn assert_completed(
            case: &Case,
            host: &Host,
            report: &RecoveryReport,
            status: RunStatus,
            expected: &CompletedSnapshot,
        ) {
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, status);
            assert_eq!(counts(report), (0, 0, 0));
            assert_eq!(
                &rows(&case.fixture.conn),
                expected.lock().unwrap().as_ref().unwrap(),
                "same-owner terminal completion remains immutable"
            );
            assert!(!host.calls.iter().any(|name| matches!(
                name.as_str(),
                "after_takeover" | "before_replay" | "cleanup"
            )));
            let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
            assert_eq!(run.owner(), case.fixture.run.owner());
            assert_eq!(
                run.observation_gap_count,
                case.fixture.run.observation_gap_count
            );
            assert_eq!(
                run.exit_code,
                if status == RunStatus::Exited {
                    Some(7)
                } else {
                    None
                }
            );
            assert_eq!(
                &report.cleanup_diagnostics,
                run.cleanup_diagnostics_json
                    .as_ref()
                    .unwrap()
                    .as_array()
                    .unwrap()
            );
            assert!(events(case).is_empty());
        }

        fn immediate_completion(exceptional: bool, foreign_owner: bool) {
            for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
                let (mut case, claim) = prepare(exceptional);
                let expected: CompletedSnapshot = Arc::new(Mutex::new(None));
                let published = expected.clone();
                let path = case.fixture.conn.path().unwrap().to_owned();
                let mut host = new_host();
                if exceptional {
                    host.process = Evidence::Unknown;
                }
                host.action = Some(Box::new(move |boundary, run, _| {
                    if boundary == "before_takeover" {
                        let conn = Connection::open(&path).unwrap();
                        assert_eq!(
                            storage::get_run(&conn, run.run_id).unwrap().owner(),
                            run.owner()
                        );
                        *published.lock().unwrap() =
                            Some(complete(&conn, run.run_id, status, foreign_owner));
                    }
                    Ok(())
                }));
                let result = case.recover_with(&mut host, claim.as_ref());
                if foreign_owner {
                    assert!(
                        result.is_err(),
                        "a terminal shortcut still validates the fixed owner and attestation"
                    );
                    assert_eq!(
                        &rows(&case.fixture.conn),
                        expected.lock().unwrap().as_ref().unwrap()
                    );
                } else {
                    assert_completed(&case, &host, &result.unwrap(), status, &expected);
                }
            }
        }

        #[test]
        fn runner_recovery_same_owner_completion_before_ordinary_takeover_has_zero_effects() {
            immediate_completion(false, false);
        }

        #[test]
        fn runner_recovery_same_owner_completion_before_exceptional_takeover_has_zero_effects() {
            immediate_completion(true, false);
        }

        #[test]
        fn runner_recovery_completed_foreign_owner_still_rejects_fixed_snapshot_and_attestation() {
            immediate_completion(false, true);
            immediate_completion(true, true);
        }

        fn completion_behind_write_lock(exceptional: bool) {
            for status in [RunStatus::Exited, RunStatus::Failed, RunStatus::Lost] {
                let (mut case, claim) = prepare(exceptional);
                let expected: CompletedSnapshot = Arc::new(Mutex::new(None));
                let published = expected.clone();
                let path = case.fixture.conn.path().unwrap().to_owned();
                let run_id = case.fixture.run.run_id;
                let (start, started) = mpsc::sync_channel(1);
                let (held, is_held) = mpsc::sync_channel(1);
                let (begin, beginning) = mpsc::sync_channel(1);
                let armed = Arc::new(AtomicBool::new(false));
                let observed_begin = Arc::new(AtomicBool::new(false));
                let enabled = armed.clone();
                let observed = observed_begin.clone();
                let mut begin = Some(begin);
                case.fixture
                    .conn
                    .authorizer(Some(move |context: AuthContext<'_>| {
                        if enabled.load(Ordering::Acquire)
                            && matches!(
                                context.action,
                                AuthAction::Transaction {
                                    operation: TransactionOperation::Begin
                                }
                            )
                            && let Some(begin) = begin.take()
                        {
                            observed.store(true, Ordering::Release);
                            begin.send(()).unwrap();
                        }
                        Authorization::Allow
                    }))
                    .unwrap();
                let mut host = new_host();
                if exceptional {
                    host.process = Evidence::Unknown;
                }
                let until = host.deadline;
                let mut start = Some(start);
                host.action = Some(Box::new(move |boundary, _, deadline| {
                    if boundary == "before_takeover"
                        && let Some(start) = start.take()
                    {
                        start.send(()).unwrap();
                        is_held
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                            .unwrap();
                        armed.store(true, Ordering::Release);
                    }
                    Ok(())
                }));
                let result = std::thread::scope(|scope| {
                    let writer = scope.spawn(move || {
                        started
                            .recv_timeout(until.saturating_duration_since(Instant::now()))
                            .unwrap();
                        let conn = Connection::open(path).unwrap();
                        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
                        *published.lock().unwrap() = Some(complete(&conn, run_id, status, false));
                        held.send(()).unwrap();
                        beginning
                            .recv_timeout(until.saturating_duration_since(Instant::now()))
                            .unwrap();
                        conn.execute_batch("COMMIT").unwrap();
                    });
                    let result = case.recover_with(&mut host, claim.as_ref());
                    writer.join().unwrap();
                    result
                });
                case.fixture
                    .conn
                    .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                    .unwrap();
                assert!(
                    observed_begin.load(Ordering::Acquire),
                    "the caller attempted its real immediate transaction while completion held the write lock"
                );
                assert_completed(&case, &host, &result.unwrap(), status, &expected);
            }
        }

        #[test]
        fn runner_recovery_ordinary_completion_is_rechecked_after_actual_write_lock_wait() {
            completion_behind_write_lock(false);
        }

        #[test]
        fn runner_recovery_exceptional_completion_is_rechecked_after_actual_write_lock_wait() {
            completion_behind_write_lock(true);
        }
    }

    mod missing_paths {
        use super::super::test_support::missing_paths::*;
        use super::*;
        #[test]
        fn runner_recovery_missing_path_unknown_identity_preserves_open_episode_and_requires_operator()
         {
            let mut case = Case::new();
            persist_opening(&mut case);
            missing_lock(&mut case);
            let before = rows(&case.fixture.conn);
            let mut host = new_host();
            host.process = Evidence::Unknown;
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(
                report.outcome,
                RecoveryOutcome::OperatorReconciliationRequired
            ));
            assert_eq!(counts(&report), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), before);
            assert!(
                events(&case)[0]
                    .interaction
                    .as_ref()
                    .unwrap()
                    .closed_at
                    .is_none()
            );
            assert!(!case.artifacts.paths.owner_lock.exists());
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|name| name == "before_replay" || name == "cleanup")
            );
        }

        #[test]
        fn runner_recovery_missing_path_rejects_wrong_snapshot_and_blank_operator_evidence_without_effects()
         {
            for invalid in ["run", "epoch", "zero_run", "zero_epoch", "reference"] {
                let mut case = Case::new();
                persist_opening(&mut case);
                missing_lock(&mut case);
                let mut attestation = claim(&case.fixture.run);
                match invalid {
                    "run" => attestation.expected_run_id += 1,
                    "epoch" => attestation.expected_owner_epoch += 1,
                    "zero_run" => attestation.expected_run_id = 0,
                    "zero_epoch" => attestation.expected_owner_epoch = 0,
                    "reference" => attestation.evidence_ref = " \t".into(),
                    _ => unreachable!(),
                }
                let before = rows(&case.fixture.conn);
                let mut host = new_host();
                host.process = Evidence::Unknown;
                assert!(
                    case.recover_with(&mut host, Some(&attestation)).is_err(),
                    "{invalid}"
                );
                assert_eq!(rows(&case.fixture.conn), before, "{invalid}");
            }
        }

        #[test]
        fn runner_recovery_live_unlinked_original_lock_is_not_replaced_or_treated_as_termination() {
            use std::os::unix::fs::PermissionsExt;
            let mut case = Case::new();
            persist_opening(&mut case);
            let original_guard = case.held.as_ref().unwrap().clone();
            case.release();
            std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            std::fs::write(
                &case.artifacts.paths.owner_lock,
                b"foreign replacement inode",
            )
            .unwrap();
            std::fs::set_permissions(
                &case.artifacts.paths.owner_lock,
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
            let before = rows(&case.fixture.conn);
            let mut host = new_host();
            host.process = Evidence::Alive;
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
            assert_eq!(rows(&case.fixture.conn), before);
            assert_eq!(
                std::fs::read(&case.artifacts.paths.owner_lock).unwrap(),
                b"foreign replacement inode"
            );
            assert!(
                original_guard
                    .validate(&case.fixture.conn, &case.fixture.run.owner())
                    .is_err()
            );
        }
        #[test]
        fn runner_recovery_positive_pid_lock_or_strict_health_independently_rejects_termination_claim()
         {
            for witness in ["pid", "lock", "health"] {
                let mut case = Case::new();
                persist_opening(&mut case);
                let attestation = claim(&case.fixture.run);
                if witness != "lock" {
                    missing_lock(&mut case);
                }
                let before = rows(&case.fixture.conn);
                let mut host = new_host();
                host.process = if witness == "pid" {
                    Evidence::Alive
                } else {
                    Evidence::Unknown
                };
                host.matching_health = witness == "health";
                let result = recover_member_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    case.fixture.run.member_id,
                    Some(&attestation),
                    host.deadline,
                    &mut host,
                );
                if let Ok(report) = result {
                    assert!(
                        matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable),
                        "{witness}"
                    );
                }
                assert_eq!(rows(&case.fixture.conn), before, "{witness}");
                assert!(
                    !host
                        .calls
                        .iter()
                        .any(|name| name == "after_takeover" || name == "cleanup")
                );
            }
        }
    }

    mod relationships {
        use super::super::test_support::relationships::*;
        #[test]
        fn runner_recovery_ordinary_takeover_refuses_actual_member_fleet_change_atomically() {
            refusal(Stage::OrdinaryTakeover, Relationship::MemberFleet);
        }
        #[test]
        fn runner_recovery_ordinary_takeover_refuses_actual_director_fleet_change_atomically() {
            refusal(Stage::OrdinaryTakeover, Relationship::DirectorFleet);
        }
        #[test]
        fn runner_recovery_ordinary_takeover_refuses_actual_fleet_director_change_atomically() {
            refusal(Stage::OrdinaryTakeover, Relationship::FleetDirector);
        }
        #[test]
        fn runner_recovery_valid_historical_ordinary_takeover_keeps_truth_and_existing_inbox() {
            historical_control(Stage::OrdinaryTakeover);
        }
    }
}
