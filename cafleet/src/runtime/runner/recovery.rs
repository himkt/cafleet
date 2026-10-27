use super::*;
use crate::broker::members::db_err;
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
        check_deadline(Instant::now(), deadline)?;
        std::fs::remove_file(path)
            .map_err(|cause| error(format!("descriptor unlink failed: {cause}")))
    }
    fn checkpoint(
        &mut self,
        boundary: RecoveryBoundary,
        run: &RunRecord,
        deadline: Instant,
    ) -> Result<()>;
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
        [member_id], |row| Placement::from_row(row, 0)).optional().map_err(db_err)
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
        params![epoch,process.pid,process.start_identity,current.run_id,current.owner_epoch]).map_err(db_err)?;
    ensure(changed == 1, "recovery takeover lost CAS")?;
    storage::get_run(conn, expected.run_id)
}

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
    recover_member_with_host(
        conn,
        settings,
        member_id,
        attestation,
        deadline,
        &mut NativeRecoveryHost,
    )
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
        match super::service::request(run, "health", until) {
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
