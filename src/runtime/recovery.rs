use crate::broker::Placement;
use crate::broker::runtime as storage;
use crate::broker::runtime::*;
use crate::broker::stored_enum;
use crate::config::Settings;
use crate::pane_lock::PaneLock;
use crate::runtime::*;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

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
stored_enum!(RecoveryOutcome, "recovery.outcome", {
    Recovered => "recovered",
    Recovering => "recovering",
    LiveOrUnreachable => "live_or_unreachable",
    OperatorReconciliationRequired => "operator_reconciliation_required",
});
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

fn snapshot(
    conn: &mut Connection,
    member_id: i64,
    until: Instant,
) -> Result<(RunRecord, Option<Placement>)> {
    with_database_deadline(conn, until, |conn| {
        let run = storage::member_runtime(conn, member_id)?
            .ok_or_else(|| crate::broker::members::missing_run(member_id))?
            .run;
        ensure_same_database(conn, &run.database_identity_json)?;
        Ok((
            run,
            crate::runtime::artifacts::placement_of(conn, member_id)?,
        ))
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
    ensure_same_database(conn, &expected.database_identity_json)?;
    ensure(
        stable_run_identity(&current) == stable_run_identity(expected)
            && crate::runtime::artifacts::placement_of(conn, expected.member_id)?.as_ref()
                == expected_placement,
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
    ensure_same_database(conn, &expected.database_identity_json)?;
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
#[cfg(test)]
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
    let borrowed = BorrowedPane {
        run: expected_run,
        placement: expected_placement,
        guard: held,
    };
    recover(
        conn,
        settings,
        member_id,
        attestation,
        deadline,
        Some(borrowed),
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
    let borrowed = BorrowedPane {
        run: expected_run,
        placement: expected_placement,
        guard: held,
    };
    recover(
        conn,
        settings,
        member_id,
        None,
        deadline,
        Some(borrowed),
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
    if let Some(expected) = &borrowed {
        ensure(
            member_id == expected.run.member_id,
            "borrowed recovery member identity mismatch",
        )?;
    }
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
    let unborrowed_pane = pinned
        .as_ref()
        .filter(|item| borrowed.is_none() && item.mux_pane_id.is_some());
    let acquired_pane = match unborrowed_pane {
        Some(target) => loop {
            check_deadline(Instant::now(), until)?;
            if let Some(guard) = PaneLock::try_acquire(&settings.database_url, target)? {
                break Some(guard);
            }
            if matches!(
                with_database_deadline(conn, until, |conn| RunLock::acquire(conn, &run.owner())),
                Ok(None)
            ) {
                return completion(run, RecoveryOutcome::LiveOrUnreachable);
            }
            std::thread::sleep(
                Duration::from_millis(5).min(until.saturating_duration_since(Instant::now())),
            );
        },
        None => None,
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
    if run.is_settled() {
        return completion(run, RecoveryOutcome::Recovered);
    }
    let owned = with_database_deadline(conn, until, |conn| RunLock::acquire(conn, &run.owner()));
    let (lock, path_failure) = match owned {
        Ok(Some(lock)) => (Some(Arc::new(lock)), None),
        Ok(None) => return completion(run, RecoveryOutcome::LiveOrUnreachable),
        Err(cause) => (None, Some(cause.message().into_owned())),
    };
    let process = run.owner_process();
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
    let recheck = |conn: &Connection| -> Result<()> {
        check_pane()?;
        let current = check_snapshot(conn, &run, pinned.as_ref())?;
        validate_attestation(&current, attestation)
    };
    if path_failure.is_some() || unknown {
        return reconcile_missing_runtime(
            conn,
            &run,
            pinned.as_ref(),
            &process,
            attestation.filter(|_| unknown),
            until,
            recheck,
        );
    }
    let lock = lock.expect("verified ordinary recovery lock");
    let current = lock.transfer_to_reconciler_checked(conn, &run, process, until, recheck)?;
    if current.is_settled() {
        return completion(current, RecoveryOutcome::Recovered);
    }
    with_database_deadline(conn, until, |conn| {
        storage::settle_lost_deferred(conn, &current.owner())
    })?;
    reconcile_retained_runtime(conn, current, pinned, lock, host, clock, until)
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
        match crate::runtime::control::request(run, LifecycleRequest::health(run), until) {
            Ok(value) => match HealthSnapshot::decode(value, run) {
                Ok(health) => Ok(RecoveryHealth::Matching(health)),
                Err(cause) => Ok(RecoveryHealth::Mismatched(cause.message().into_owned())),
            },
            Err(cause) => Ok(RecoveryHealth::Unavailable(cause.message().into_owned())),
        }
    }
    fn cleanup_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome> {
        crate::runtime::process::recover_descendants(run, clock)
    }
    fn checkpoint(&mut self, _: RecoveryBoundary, _: &RunRecord, until: Instant) -> Result<()> {
        check_deadline(Instant::now(), until)
    }
}

fn delivery_result_transition(
    run_id: i64,
    event_seq: i64,
    receipt: &storage::UnpublishedDeliveryResult,
    observed_at: String,
    evidence_json: Value,
) -> ObservedTransition {
    let &(input_id, stage, cause) = receipt;
    ObservedTransition {
        run_id,
        event_seq,
        episode_seq: None,
        kind: RuntimeEventKind::InputDeliveryResult,
        classification: None,
        observed_at,
        evidence_json,
        close_reason: None,
        cause_json: Some(storage::input_delivery_result_cause(input_id, stage, cause)),
    }
}

fn reconcile_missing_runtime(
    conn: &mut Connection,
    expected: &RunRecord,
    pinned: Option<&Placement>,
    process: &ProcessIdentity,
    attestation: Option<&OwnerTerminationAttestation>,
    until: Instant,
    check: impl FnOnce(&Connection) -> Result<()>,
) -> Result<RecoveryCompletion> {
    with_database_deadline(conn, until, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check(&tx)?;
        let current = check_snapshot(&tx, expected, pinned)?;
        validate_attestation(&current, attestation)?;
        if current.is_settled() {
            check_deadline(Instant::now(), until)?;
            tx.commit()?;
            return completion(current, RecoveryOutcome::Recovered);
        }
        let established = terminal_outcome(&tx, &current)?;
        let mut run = advance_recovery_owner(&tx, &current, process)?;
        let mut report = completion(run.clone(), RecoveryOutcome::Recovering)?.report;
        report
            .cleanup_diagnostics
            .push(json!("runtime_path_missing"));
        if let Some(claim) = attestation {
            report.cleanup_diagnostics.push(json!({"kind":"owner_termination_attestation",
                "run_id":current.run_id,"owner_epoch":current.owner_epoch,"owner_pid":current.owner_pid,
                "owner_start_identity":current.owner_start_identity,"evidence_ref":claim.evidence_ref,
                "recorded_at":crate::time::now_stamp(),"source":"operator_via_director"}));
        }
        tx.execute("UPDATE member_runs SET observation_gap_count=observation_gap_count+1,cleanup_diagnostics_json=?1 WHERE run_id=?2",
            params![serde_json::to_string(&report.cleanup_diagnostics).map_err(io_error)?,run.run_id])?;
        let target = match established {
            Some(status) => status,
            None => {
                let first = *reserve_recovery_range(&tx, &run, 256)?.start();
                run = storage::get_run(&tx, run.run_id)?;
                for transition in loss_transitions(&tx, &run, first, true)? {
                    let (event, inserted) = storage::persist_transition_in_transaction(
                        &tx,
                        &run,
                        &transition,
                        PersistenceMode::Recovery,
                    )?;
                    ensure(
                        inserted,
                        "exceptional recovery requires fresh reserved transitions",
                    )?;
                    count_commit(&mut report, &event, false)?;
                }
                RunStatus::Lost
            }
        };
        storage::settle_lost_deferred_in_transaction(&tx, &run)?;
        let unpublished = storage::unpublished_delivery_results(&tx, run.run_id)?;
        if !unpublished.is_empty() {
            let count = i64::try_from(unpublished.len()).expect("bounded unresolved inputs");
            let sequences = reserve_recovery_range(&tx, &run, count)?;
            run = storage::get_run(&tx, run.run_id)?;
            let evidence = storage::evidence::run_metadata(&run, "runtime-recovery", 0)?;
            for (event_seq, receipt) in sequences.zip(unpublished) {
                let transition = delivery_result_transition(
                    run.run_id,
                    event_seq,
                    &receipt,
                    crate::time::now_stamp(),
                    evidence.clone(),
                );
                let (event, inserted) = storage::persist_transition_in_transaction(
                    &tx,
                    &run,
                    &transition,
                    PersistenceMode::Recovery,
                )?;
                if inserted {
                    count_commit(&mut report, &event, false)?;
                }
            }
        }
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2) WHERE run_id=?3",
            params![target.as_str(),crate::time::now_stamp(),run.run_id])?;
        let run = storage::get_run(&tx, run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit()?;
        report.outcome = RecoveryOutcome::Recovered;
        report.status = run.status;
        Ok(RecoveryCompletion { report, run })
    })
}

/// `count` recovery sequences above both the reserved ceiling and the latest committed event.
fn reserve_recovery_range(
    conn: &Connection,
    run: &RunRecord,
    count: i64,
) -> Result<std::ops::RangeInclusive<i64>> {
    let ceiling = storage::latest_event_seq(conn, run.run_id)?
        .map_or(run.event_seq_reserved_through, |value| {
            value.max(run.event_seq_reserved_through)
        });
    storage::reserve_event_sequences_in_transaction(
        conn,
        run,
        storage::SequenceReservation::RecoveryRange { ceiling, count },
    )
}

fn reconcile_retained_runtime(
    conn: &mut Connection,
    run: RunRecord,
    pinned: Option<Placement>,
    lock: Arc<RunLock>,
    host: &mut impl RecoveryHost,
    clock: ShutdownClock,
    until: Instant,
) -> Result<RecoveryCompletion> {
    let mut result = completion(run, RecoveryOutcome::Recovering)?;
    let attempt = perform(conn, &mut result, pinned.as_ref(), lock, host, clock, until);
    match attempt {
        Ok(()) => Ok(result),
        Err(cause)
            if matches!(cause, CafleetError::DeadlineExceeded) || Instant::now() >= until =>
        {
            result
                .report
                .cleanup_diagnostics
                .push(json!(cause.message()));
            Ok(result)
        }
        Err(cause) => Err(cause),
    }
}
fn refresh(conn: &mut Connection, result: &mut RecoveryCompletion, until: Instant) -> Result<()> {
    result.run = with_database_deadline(conn, until, |conn| {
        storage::get_run(conn, result.run.run_id)
    })?;
    result.report.status = result.run.status;
    Ok(())
}
pub(super) fn count_commit(
    report: &mut RecoveryReport,
    event: &RuntimeEventRecord,
    original: bool,
) -> Result<()> {
    fn increment(value: &mut u64) -> Result<()> {
        *value = value
            .checked_add(1)
            .ok_or_else(|| error("recovery commit count overflow"))?;
        Ok(())
    }
    if original {
        increment(&mut report.replayed_count)?;
    }
    match event.notification_state {
        NotificationState::Inbox => increment(&mut report.inbox_count),
        NotificationState::Archived => increment(&mut report.archived_count),
    }
}
fn persist(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    transition: &ObservedTransition,
    effective_policy: Option<&storage::EffectivePolicyObservation>,
    original: bool,
    host: &mut impl RecoveryHost,
    until: Instant,
) -> Result<()> {
    let (event, inserted) = with_database_deadline(conn, until, |conn| match effective_policy {
        Some(observation) => storage::persist_transition_with_effective_policy(
            conn,
            &result.run.owner(),
            transition,
            observation,
            PersistenceMode::Recovery,
        ),
        None => storage::persist_transition(
            conn,
            &result.run.owner(),
            transition,
            PersistenceMode::Recovery,
        ),
    })?;
    if inserted {
        count_commit(&mut result.report, &event, original)?;
    }
    refresh(conn, result, until)?;
    host.checkpoint(RecoveryBoundary::AfterCommit, &result.run, until)
}
pub(super) fn loss_transitions(
    conn: &Connection,
    run: &RunRecord,
    first: i64,
    missing: bool,
) -> Result<Vec<ObservedTransition>> {
    let snapshot = storage::runtime_snapshot(conn, run.run_id)?;
    let mut next = first;
    let now = crate::time::now_stamp();
    let mut transitions = Vec::new();
    if let Some(interaction_id) = snapshot.current_interaction_id {
        let interaction = storage::get_event(conn, conn.query_row(
            "SELECT event_id FROM member_runtime_events WHERE interaction_id=?1 ORDER BY event_seq LIMIT 1",
            [interaction_id], |row| row.get::<_, i64>(0))?)?.interaction
            .ok_or_else(|| error("pending recovery interaction missing"))?;
        transitions.push(ObservedTransition {
            run_id: run.run_id,
            event_seq: next,
            episode_seq: Some(interaction.episode_seq),
            kind: RuntimeEventKind::InteractionClosed,
            classification: Some(interaction.classification),
            observed_at: now.clone(),
            evidence_json: interaction.evidence_json,
            close_reason: Some(InteractionCloseReason::MonitorLost),
            cause_json: None,
        });
        next = next
            .checked_add(1)
            .ok_or_else(|| error("recovery sequence overflow"))?;
    }
    let evidence = storage::evidence::run_metadata(run, "runtime-recovery", 0)?;
    transitions.push(ObservedTransition {
        run_id: run.run_id,
        event_seq: next,
        episode_seq: None,
        kind: RuntimeEventKind::RunnerLost,
        classification: None,
        observed_at: now,
        evidence_json: evidence,
        close_reason: None,
        cause_json: Some(json!({"kind":if missing {
            RuntimeCauseKind::RuntimePathMissing
        } else {
            RuntimeCauseKind::MonitorLost
        }.as_str()})),
    });
    Ok(transitions)
}
fn publish_delivery_results<S: OutboxIo>(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    outbox: &mut Outbox<S>,
    host: &mut impl RecoveryHost,
    until: Instant,
) -> Result<()> {
    let unpublished = with_database_deadline(conn, until, |conn| {
        storage::unpublished_delivery_results(conn, result.run.run_id)
    })?;
    for chunk in unpublished.chunks(256) {
        let sequences = with_database_deadline(conn, until, |conn| {
            storage::reserve_event_sequences(conn, &result.run.owner())
        })?;
        refresh(conn, result, until)?;
        let evidence = storage::evidence::run_metadata(&result.run, "runtime-recovery", 0)?;
        for (event_seq, receipt) in sequences.zip(chunk) {
            let transition = delivery_result_transition(
                result.run.run_id,
                event_seq,
                receipt,
                crate::time::now_stamp(),
                evidence.clone(),
            );
            outbox.append_synced(conn, &transition, None, until)?;
            persist(conn, result, &transition, None, false, host, until)?;
        }
    }
    Ok(())
}
pub(super) fn terminal_outcome(conn: &Connection, run: &RunRecord) -> Result<Option<RunStatus>> {
    let terminal: Option<String> = conn.query_row(
        "SELECT kind FROM member_runtime_events WHERE run_id=?1 AND kind IN ('runner_exited','runner_lost') ORDER BY event_seq DESC LIMIT 1",
        [run.run_id], |row| row.get(0)).optional()?;
    match terminal.as_deref() {
        Some("runner_exited") => Ok(Some(RunStatus::Exited)),
        Some("runner_lost") => Ok(Some(RunStatus::Lost)),
        Some(_) => Err(error("unexpected terminal recovery event")),
        None => Ok(run
            .recovery_origin_status
            .filter(|status| status.terminal())
            .or_else(|| run.status.terminal().then_some(run.status))),
    }
}
fn perform(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    pinned: Option<&Placement>,
    lock: Arc<RunLock>,
    host: &mut impl RecoveryHost,
    clock: ShutdownClock,
    until: Instant,
) -> Result<()> {
    host.checkpoint(RecoveryBoundary::AfterTakeover, &result.run, until)?;
    let mut descriptor = RecoveryDescriptor::pin(conn, &result.run, &lock, until)?;
    let (mut outbox, scan) = Outbox::open_recovery(conn, &result.run, lock.clone(), until)?;
    if let Some(tail) = scan.incomplete_tail {
        result.report.cleanup_diagnostics.push(json!(tail));
    }
    for frame in scan.frames {
        with_database_deadline(conn, until, |conn| {
            check_snapshot(conn, &result.run, pinned)
        })?;
        host.checkpoint(RecoveryBoundary::BeforeReplay, &result.run, until)?;
        persist(
            conn,
            result,
            &frame.transition,
            frame.effective_policy.as_ref(),
            true,
            host,
            until,
        )?;
    }
    let existing = with_database_deadline(conn, until, |conn| terminal_outcome(conn, &result.run))?;
    let target = match existing {
        Some(status) => status,
        None => {
            let sequences = with_database_deadline(conn, until, |conn| {
                storage::reserve_event_sequences(conn, &result.run.owner())
            })?;
            refresh(conn, result, until)?;
            let transitions = with_database_deadline(conn, until, |conn| {
                loss_transitions(conn, &result.run, *sequences.start(), false)
            })?;
            for transition in transitions {
                outbox.append_synced(conn, &transition, None, until)?;
                persist(conn, result, &transition, None, false, host, until)?;
            }
            RunStatus::Lost
        }
    };
    publish_delivery_results(conn, result, &mut outbox, host, until)?;
    let cleanup = host.cleanup_owned(&result.run, clock)?;
    for diagnostic in &cleanup.diagnostics {
        result.report.cleanup_diagnostics.push(json!(diagnostic));
    }
    if cleanup.cleanup_state != CleanupState::Complete {
        result
            .report
            .cleanup_diagnostics
            .push(json!("owned recovery cleanup remains incomplete"));
        return Ok(());
    }
    host.checkpoint(RecoveryBoundary::BeforeCompact, &result.run, until)?;
    outbox.compact_committed(conn, &result.run.owner(), until)?;
    host.checkpoint(RecoveryBoundary::BeforeFinish, &result.run, until)?;
    with_database_deadline(conn, until, |conn| {
        check_snapshot(conn, &result.run, pinned)
    })?;
    descriptor.remove(conn, &result.run, &lock, until, |path, deadline| {
        host.unlink_descriptor(path, deadline)
    })?;
    result.run = with_database_deadline(conn, until, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_snapshot(&tx, &result.run, pinned)?;
        lock.validate(&tx, &result.run.owner())?;
        storage::validate_owner(&tx, &result.run.owner())?;
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2),cleanup_diagnostics_json=?3 WHERE run_id=?4 AND owner_epoch=?5",
            params![target.as_str(),crate::time::now_stamp(),serde_json::to_string(&result.report.cleanup_diagnostics).map_err(io_error)?,result.run.run_id,result.run.owner_epoch])?;
        let current = storage::get_run(&tx, result.run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit()?;
        Ok(current)
    })?;
    result.report.status = result.run.status;
    result.report.outcome = RecoveryOutcome::Recovered;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::Value;
    use serde_json::json;

    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::db::test_support::TableSnapshot;
    use crate::error::CafleetError;
    use crate::runtime::recovery::test_support::*;
    use crate::runtime::*;
    use std::time::Duration;
    use std::time::Instant;

    mod unpublished_history {
        use crate::runtime::recovery::test_support::descriptor::identity as metadata;
        use crate::runtime::recovery::test_support::relationships::retained_history;
        use crate::runtime::recovery::tests::*;
        use std::path::PathBuf;

        struct ArtifactWitness {
            path: PathBuf,
            identity: (u64, u64, u64, i64, i64),
        }
        fn lose_artifacts(case: &mut Case, directory: bool) -> ArtifactWitness {
            let identity = metadata(&case.artifacts.paths.outbox);
            case.release();
            let path = if directory {
                let retained = case.fixture._dir.path().join("retained-runtime");
                std::fs::rename(&case.artifacts.paths.run_dir, &retained).unwrap();
                retained.join("events.outbox")
            } else {
                std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
                case.artifacts.paths.outbox.clone()
            };
            ArtifactWitness { path, identity }
        }
        fn original_terminal(status: RunStatus) -> Case {
            let mut case = Case::new();
            let event = ObservedTransition {
                run_id: case.fixture.run.run_id,
                event_seq: 1,
                episode_seq: None,
                kind: RuntimeEventKind::CoverageUnknown,
                classification: None,
                observed_at: NOW.into(),
                evidence_json: evidence(&case.fixture.run, 1),
                close_reason: None,
                cause_json: Some(json!({"kind":"coverage_unknown"})),
            };
            storage::persist_transition(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &event,
                PersistenceMode::Live,
            )
            .unwrap();
            case.fixture.conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='incomplete',exit_code=?2,ended_at=?3,cleanup_diagnostics_json=?4 WHERE run_id=?5",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, json!([{"kind":"retained_terminal_history"}]).to_string(), case.fixture.run.run_id],
    ).unwrap();
            case.refresh();
            case
        }
        fn recover_missing(case: &mut Case, operator: bool) -> TestResult<RecoveryReport> {
            let claim = operator.then(|| {
                claim(
                    &case.fixture.run,
                    Some("audit:established-terminal-history"),
                )
            });
            let mut host = new_host();
            if operator {
                host.process = Evidence::Unknown;
            }
            let result = recover_member_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                claim.as_ref(),
                host.deadline,
                &mut host,
            );
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|name| matches!(name.as_str(), "before_replay" | "cleanup"))
            );
            result
        }
        fn assert_terminal_bookkeeping(
            case: &Case,
            report: &RecoveryReport,
            before: &RunRecord,
            status: RunStatus,
            prior: &[TableSnapshot],
            artifact: &ArtifactWitness,
            operator: bool,
        ) {
            let current = storage::get_run(&case.fixture.conn, before.run_id).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, status);
            assert_eq!(current.status, status);
            assert_eq!(counts(report), (0, 0, 0));
            assert_eq!(current.owner_epoch, before.owner_epoch + 1);
            assert_eq!(current.owner_kind, OwnerKind::Reconciler);
            let process = ProcessIdentity::current().unwrap();
            assert_eq!(current.owner_pid, process.pid);
            assert_eq!(current.owner_start_identity, process.start_identity);
            assert_eq!(
                current.observation_gap_count,
                before.observation_gap_count + 1
            );
            assert_eq!(
                current.event_seq_reserved_through,
                before.event_seq_reserved_through
            );
            assert_eq!(current.exit_code, before.exit_code);
            assert_eq!(current.exit_signal, before.exit_signal);
            assert_eq!(current.cleanup_state, CleanupState::Complete);
            assert_eq!(
                current.recovery_origin_status,
                before.recovery_origin_status.or(Some(before.status))
            );
            assert_eq!(retained_history(&case.fixture.conn), prior);
            assert_eq!(
                metadata(&artifact.path),
                artifact.identity,
                "database-only bookkeeping preserves surviving spool identity and metadata"
            );
            let diagnostics = current
                .cleanup_diagnostics_json
                .as_ref()
                .unwrap()
                .to_string();
            assert!(diagnostics.contains("runtime_path_missing"));
            assert_eq!(diagnostics.contains("operator_via_director"), operator);
            if operator {
                assert!(diagnostics.contains("audit:established-terminal-history"));
            }
        }

        fn original_history_cases(status: RunStatus, operator: bool) {
            for directory in [false, true] {
                let mut case = original_terminal(status);
                let before = case.fixture.run.clone();
                let prior = retained_history(&case.fixture.conn);
                let artifact = lose_artifacts(&mut case, directory);
                let report = recover_missing(&mut case, operator).unwrap();
                assert_terminal_bookkeeping(
                    &case, &report, &before, status, &prior, &artifact, operator,
                );
                assert!(
                    storage::get_run(&case.fixture.conn, before.run_id)
                        .unwrap()
                        .cleanup_diagnostics_json
                        .unwrap()
                        .to_string()
                        .contains("retained_terminal_history")
                );
            }
        }
        #[test]
        fn runner_recovery_machine_missing_paths_preserve_original_exited_and_failed_history() {
            original_history_cases(RunStatus::Exited, false);
        }
        #[test]
        fn runner_recovery_operator_missing_paths_preserve_original_exited_and_failed_history() {
            original_history_cases(RunStatus::Failed, true);
        }

        #[test]
        fn runner_recovery_machine_missing_paths_keep_committed_native_exit_code_signal_and_message()
         {
            for directory in [false, true] {
                let mut case = Case::new();
                case.exited_pending_cleanup(None, Some(15));
                let before = case.fixture.run.clone();
                let prior = retained_history(&case.fixture.conn);
                let artifact = lose_artifacts(&mut case, directory);
                let report = recover_missing(&mut case, false).unwrap();
                assert_terminal_bookkeeping(
                    &case,
                    &report,
                    &before,
                    RunStatus::Exited,
                    &prior,
                    &artifact,
                    false,
                );
                assert_eq!(events(&case).len(), 1);
            }
        }

        #[test]
        fn runner_recovery_machine_artifact_loss_after_committed_runner_lost_creates_no_duplicate()
        {
            for directory in [false, true] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let mut first = new_host();
                first.action = Some(Box::new(|boundary, _, _| {
                    if boundary == "before_finish" {
                        return Err(CafleetError::App(
                            "owned interruption after committed terminal history".into(),
                        ));
                    }
                    Ok(())
                }));
                assert!(case.recover(&mut first).is_err());
                assert!(first.calls.iter().any(|name| name == "before_finish"));
                let committed = events(&case);
                assert_eq!(
                    committed
                        .iter()
                        .map(|view| view.event.kind)
                        .collect::<Vec<_>>(),
                    vec![
                        RuntimeEventKind::InteractionOpened,
                        RuntimeEventKind::InteractionClosed,
                        RuntimeEventKind::RunnerLost
                    ]
                );
                case.refresh();
                assert_eq!(case.fixture.run.cleanup_state, CleanupState::Incomplete);
                let before = case.fixture.run.clone();
                let prior = retained_history(&case.fixture.conn);
                let artifact = lose_artifacts(&mut case, directory);
                let report = recover_missing(&mut case, false).unwrap();
                assert_terminal_bookkeeping(
                    &case,
                    &report,
                    &before,
                    RunStatus::Lost,
                    &prior,
                    &artifact,
                    false,
                );
                assert_eq!(events(&case).len(), 3);
                let complete = rows(&case.fixture.conn);
                assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
                assert_eq!(rows(&case.fixture.conn), complete);
            }
        }

        #[test]
        fn runner_recovery_terminal_bookkeeping_faults_rollback_atomically_then_retry_without_events()
         {
            for fault in ["owner", "diagnostic", "finish", "commit"] {
                let mut case = original_terminal(RunStatus::Failed);
                let before_run = case.fixture.run.clone();
                let prior = retained_history(&case.fixture.conn);
                let artifact = lose_artifacts(&mut case, false);
                let trigger = match fault {
                    "owner" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF owner_epoch ON member_runs BEGIN SELECT RAISE(ABORT,'owned history owner fault'); END",
                    ),
                    "diagnostic" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs BEGIN SELECT RAISE(ABORT,'owned history diagnostic fault'); END",
                    ),
                    "finish" => Some(
                        "CREATE TRIGGER fail_history BEFORE UPDATE OF cleanup_state ON member_runs WHEN NEW.cleanup_state='complete' BEGIN SELECT RAISE(ABORT,'owned history finish fault'); END",
                    ),
                    "commit" => None,
                    _ => unreachable!(),
                };
                fail_next(&mut case.fixture.conn, trigger);
                let before = rows(&case.fixture.conn);
                let result = recover_missing(&mut case, true);
                case.fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
                assert!(result.is_err(), "{fault}");
                assert_eq!(rows(&case.fixture.conn), before, "{fault}");
                assert_eq!(metadata(&artifact.path), artifact.identity);
                if trigger.is_some() {
                    case.fixture
                        .conn
                        .execute_batch("DROP TRIGGER fail_history")
                        .unwrap();
                }
                if fault == "commit" {
                    let report = recover_missing(&mut case, true).unwrap();
                    assert_terminal_bookkeeping(
                        &case,
                        &report,
                        &before_run,
                        RunStatus::Failed,
                        &prior,
                        &artifact,
                        true,
                    );
                }
            }
        }
    }

    mod unpublished_missing_paths {
        use crate::runtime::recovery::test_support::missing_paths::*;
        use crate::runtime::recovery::tests::*;
        #[test]
        fn runner_recovery_missing_path_machine_termination_commits_one_atomic_database_only_loss()
        {
            let mut case = Case::new();
            persist_opening(&mut case);
            let original = case.fixture.run.clone();
            missing_lock(&mut case);
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert_loss(&case, &report, &original);
            assert!(!host.calls.iter().any(|name| name == "before_replay"));
            assert!(!case.artifacts.paths.owner_lock.exists());
            let diagnostics = storage::get_run(&case.fixture.conn, original.run_id)
                .unwrap()
                .cleanup_diagnostics_json
                .unwrap()
                .to_string();
            assert!(!diagnostics.contains("operator_via_director"));
        }

        fn deferred_receipt(case: &Case, key: &str, settled_reason: Option<&str>) -> i64 {
            InputRow {
                key,
                state: if settled_reason.is_some() {
                    "settled"
                } else {
                    "ready"
                },
                disposition: settled_reason.map(|_| "not_submitted"),
                delivery_reason: settled_reason,
                deferred: true,
                ..InputRow::default()
            }
            .insert(case)
        }

        #[test]
        fn runner_recovery_missing_path_settles_queued_receipts_runner_lost_and_publishes_durable_results_once()
         {
            let mut case = Case::new();
            persist_opening(&mut case);
            let queued = deferred_receipt(&case, "prompt:queued", None);
            let unpublished = deferred_receipt(&case, "prompt:unpublished", Some("runner_stopped"));
            let uncertain = InputRow {
                key: "prompt:uncertain",
                state: "uncertain",
                delivery_method: Some("profile"),
                uncertain_cause: Some("runner_stopped"),
                deferred: true,
                ..InputRow::default()
            }
            .insert(&case);
            missing_lock(&mut case);
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            let lost = storage::get_input(&case.fixture.conn, queued).unwrap();
            assert_eq!(lost.state, InputState::Settled);
            assert_eq!(lost.disposition, Some(InputDisposition::NotSubmitted));
            assert_eq!(lost.delivery_reason.as_deref(), Some("runner_lost"));
            assert_eq!(
                storage::get_input(&case.fixture.conn, unpublished)
                    .unwrap()
                    .delivery_reason
                    .as_deref(),
                Some("runner_stopped")
            );
            for input_id in [queued, unpublished, uncertain] {
                assert_eq!(
                    delivery_results(&case, input_id).len(),
                    1,
                    "input {input_id}"
                );
            }
            let published_cause = |input_id: i64| -> Option<String> {
                delivery_results(&case, input_id)[0]["cause"]["kind"]
                    .as_str()
                    .map(str::to_owned)
            };
            assert_eq!(
                published_cause(uncertain).as_deref(),
                Some("runner_stopped")
            );
            assert_eq!(published_cause(queued), None);
            assert_eq!(published_cause(unpublished), None);
            case.recover(&mut new_host()).unwrap();
            for input_id in [queued, unpublished, uncertain] {
                assert_eq!(
                    delivery_results(&case, input_id).len(),
                    1,
                    "a repeated recovery publishes input {input_id} once"
                );
            }
        }

        #[test]
        fn runner_recovery_missing_path_uses_current_lifecycle_for_atomic_archived_closure_and_loss()
         {
            use crate::runtime::recovery::test_support::relationships::LIFECYCLES;
            use crate::runtime::recovery::test_support::relationships::end_lifecycle;
            for lifecycle in LIFECYCLES {
                let mut case = Case::new();
                let opening = persist_opening(&mut case);
                end_lifecycle(&mut case, lifecycle);
                missing_lock(&mut case);
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(counts(&report), (0, 0, 2));
                let committed = events(&case);
                assert_eq!(committed[0].event, opening);
                assert_eq!(committed.len(), 3);
                assert!(committed[1..].iter().all(|row| row.event.notification_state
                    == NotificationState::Archived
                    && row.event.message_id.is_none()
                    && row.event.archive_reason == Some(lifecycle.archive_reason())));
            }
        }

        #[test]
        fn runner_recovery_missing_directory_keeps_surviving_suspect_artifacts_untouched() {
            use std::os::unix::fs::MetadataExt;
            let mut case = Case::new();
            persist_opening(&mut case);
            case.seed(&[obscured(&case.fixture.run, 2, 1)]);
            let original = case.fixture.run.clone();
            let snapshot = case.spool_bytes().unwrap();
            let spool_meta = std::fs::metadata(&case.artifacts.paths.outbox).unwrap();
            let sentinel = case
                .artifacts
                .paths
                .run_dir
                .join("unrelated-suspect-artifact");
            std::fs::write(&sentinel, b"surviving owned fixture artifact").unwrap();
            case.release();
            let retained = case.artifacts.paths.run_dir.with_extension("retained");
            std::fs::rename(&case.artifacts.paths.run_dir, &retained).unwrap();
            let report = case.recover(&mut new_host()).unwrap();
            assert_loss(&case, &report, &original);
            assert!(!case.artifacts.paths.run_dir.exists());
            assert_eq!(
                std::fs::read(retained.join("unrelated-suspect-artifact")).unwrap(),
                b"surviving owned fixture artifact"
            );
            let after = std::fs::metadata(retained.join("events.outbox")).unwrap();
            assert_eq!(
                (
                    after.dev(),
                    after.ino(),
                    after.len(),
                    after.modified().unwrap()
                ),
                (
                    spool_meta.dev(),
                    spool_meta.ino(),
                    u64::try_from(snapshot.len()).unwrap(),
                    spool_meta.modified().unwrap()
                )
            );
            assert_eq!(
                events(&case).len(),
                3,
                "database-only recovery does not replay the surviving spool"
            );
        }

        #[test]
        fn runner_recovery_attestation_records_attributed_original_provenance_and_current_terminal_retries()
         {
            let mut case = Case::new();
            persist_opening(&mut case);
            let original = case.fixture.run.clone();
            let original_claim = claim(&original, None);
            missing_lock(&mut case);
            let earliest = chrono::Utc::now();
            let mut host = new_host();
            host.process = Evidence::Unknown;
            let report = case.recover_with(&mut host, Some(&original_claim)).unwrap();
            let latest = chrono::Utc::now();
            assert_loss(&case, &report, &original);
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            let diagnostics = current.cleanup_diagnostics_json.as_ref().unwrap();
            let text = diagnostics.to_string();
            for value in [
                "owner_termination_attestation",
                "operator_via_director",
                original_claim.evidence_ref.as_str(),
                original.owner_start_identity.as_str(),
            ] {
                assert!(text.contains(value), "{text}");
            }
            let mut leaves = Vec::new();
            fn values<'a>(value: &'a Value, leaves: &mut Vec<&'a Value>) {
                match value {
                    Value::Object(map) => {
                        for value in map.values() {
                            values(value, leaves)
                        }
                    }
                    Value::Array(array) => {
                        for value in array {
                            values(value, leaves)
                        }
                    }
                    _ => leaves.push(value),
                }
            }
            values(diagnostics, &mut leaves);
            for expected in [original.run_id, original.owner_epoch, original.owner_pid] {
                assert!(leaves.iter().any(|value| value.as_i64() == Some(expected)));
            }
            assert!(
                leaves
                    .iter()
                    .filter_map(|value| value.as_str())
                    .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .any(|value| value >= earliest && value <= latest)
            );
            let committed = rows(&case.fixture.conn);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            assert!(
                case.recover_with(&mut retry, Some(&original_claim))
                    .is_err()
            );
            assert_eq!(rows(&case.fixture.conn), committed);
            let matching = claim(&current, None);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            let no_work = case.recover_with(&mut retry, Some(&matching)).unwrap();
            assert_eq!(counts(&no_work), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), committed);
            bump_epoch(&case.fixture.conn, original.run_id);
            let changed = rows(&case.fixture.conn);
            assert!(case.recover_with(&mut new_host(), Some(&matching)).is_err());
            assert_eq!(rows(&case.fixture.conn), changed);
            case.refresh();
            let latest_claim = claim(&case.fixture.run, None);
            let mut retry = new_host();
            retry.process = Evidence::Alive;
            assert_eq!(
                counts(&case.recover_with(&mut retry, Some(&latest_claim)).unwrap()),
                (0, 0, 0)
            );
            assert_eq!(rows(&case.fixture.conn), changed);
            assert_eq!(
                storage::get_run(&case.fixture.conn, original.run_id)
                    .unwrap()
                    .cleanup_diagnostics_json,
                current.cleanup_diagnostics_json
            );
        }
        #[test]
        fn runner_recovery_missing_path_faults_rollback_owner_provenance_counters_episode_and_events_together()
         {
            for fault in [
                "owner",
                "provenance",
                "reservation",
                "closure",
                "loss",
                "terminal",
                "commit",
            ] {
                let mut case = Case::new();
                persist_opening(&mut case);
                missing_lock(&mut case);
                let statement = match fault {
                    "owner" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF owner_epoch ON member_runs BEGIN SELECT RAISE(ABORT,'injected owner failure'); END",
                    ),
                    "provenance" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF cleanup_diagnostics_json ON member_runs BEGIN SELECT RAISE(ABORT,'injected provenance failure'); END",
                    ),
                    "reservation" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF event_seq_reserved_through ON member_runs BEGIN SELECT RAISE(ABORT,'injected reservation failure'); END",
                    ),
                    "closure" => Some(
                        "CREATE TRIGGER fault BEFORE INSERT ON member_runtime_events WHEN NEW.kind='interaction_closed' BEGIN SELECT RAISE(ABORT,'injected closure failure'); END",
                    ),
                    "loss" => Some(
                        "CREATE TRIGGER fault BEFORE INSERT ON member_runtime_events WHEN NEW.kind='runner_lost' BEGIN SELECT RAISE(ABORT,'injected loss failure'); END",
                    ),
                    "terminal" => Some(
                        "CREATE TRIGGER fault BEFORE UPDATE OF status ON member_runs WHEN NEW.status='lost' BEGIN SELECT RAISE(ABORT,'injected terminal failure'); END",
                    ),
                    "commit" => None,
                    _ => unreachable!(),
                };
                fail_next(&mut case.fixture.conn, statement);
                let before = rows(&case.fixture.conn);
                let mut host = new_host();
                host.process = Evidence::Unknown;
                let attestation = claim(&case.fixture.run, None);
                let result = case.recover_with(&mut host, Some(&attestation));
                case.fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
                assert!(result.is_err(), "{fault}");
                assert_eq!(rows(&case.fixture.conn), before, "{fault}");
                assert!(!host.calls.iter().any(|name| name == "before_replay"));
            }
        }

        #[test]
        fn runner_recovery_attestation_revalidates_epoch_after_obtaining_the_actual_write_lock() {
            use std::sync::mpsc;
            let mut case = Case::new();
            persist_opening(&mut case);
            missing_lock(&mut case);
            let original = case.fixture.run.clone();
            let attestation = claim(&original, None);
            let run_id = original.run_id;
            let path = case.fixture.conn.path().unwrap().to_owned();
            let (start_tx, start_rx) = mpsc::sync_channel(1);
            let (held_tx, held_rx) = mpsc::sync_channel(1);
            let mut start_tx = Some(start_tx);
            let mut host = new_host();
            host.process = Evidence::Unknown;
            host.action = Some(Box::new(move |boundary, _, deadline| {
                if boundary == "before_takeover"
                    && let Some(start) = start_tx.take()
                {
                    start.send(()).unwrap();
                    held_rx
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .unwrap();
                }
                Ok(())
            }));
            std::thread::scope(|scope| {
                let writer = scope.spawn(move || {
                    start_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                    let writer = Connection::open(path).unwrap();
                    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
                    bump_epoch(&writer, run_id);
                    held_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(80));
                    writer.execute_batch("COMMIT").unwrap();
                });
                let result = case.recover_with(&mut host, Some(&attestation));
                writer.join().unwrap();
                assert!(result.is_err());
            });
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            assert_eq!(current.owner_epoch, original.owner_epoch + 1);
            assert_eq!(current.owner_kind, original.owner_kind);
            assert_eq!(current.status, original.status);
            assert_eq!(
                current.cleanup_diagnostics_json,
                original.cleanup_diagnostics_json
            );
            assert_eq!(events(&case).len(), 1);
        }
    }

    mod unpublished_relationships {
        use crate::runtime::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_exceptional_zero_event_takeover_refuses_actual_member_fleet_change_atomically()
         {
            refusal(Stage::ExceptionalTakeover);
        }
        #[test]
        fn runner_recovery_valid_historical_exceptional_zero_event_bookkeeping_keeps_history() {
            historical_control(Stage::ExceptionalTakeover);
        }
    }

    use std::io::Write;

    use std::sync::Arc;

    mod replay_history {
        use crate::runtime::recovery::test_support::relationships::LIFECYCLES;
        use crate::runtime::recovery::test_support::relationships::end_lifecycle;
        use crate::runtime::recovery::tests::*;

        #[test]
        fn runner_recovery_archives_each_original_and_synthesized_event_by_current_lifecycle() {
            for lifecycle in LIFECYCLES {
                let mut case = Case::new();
                case.seed(&[
                    opening(&case.fixture.run, 1, 1),
                    obscured(&case.fixture.run, 2, 1),
                ]);
                end_lifecycle(&mut case, lifecycle);
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(counts(&report), (2, 0, 4), "{lifecycle:?}");
                let events = events(&case);
                assert_eq!(events.len(), 4);
                assert!(events.iter().all(|row| row.event.notification_state
                    == NotificationState::Archived
                    && row.event.message_id.is_none()
                    && row.event.archive_reason == Some(lifecycle.archive_reason())));
                assert_eq!(
            case.fixture
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE from_member_id=?1 AND to_member_id=?2",
                    rusqlite::params![
                        case.fixture.run.member_id,
                        case.fixture.run.director_member_id
                    ],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
            }
        }

        #[test]
        fn runner_recovery_lifecycle_change_between_actual_commits_selects_each_disposition_atomically()
         {
            let mut case = Case::new();
            case.seed(&[
                opening(&case.fixture.run, 1, 1),
                obscured(&case.fixture.run, 2, 1),
            ]);
            let mutation = Connection::open(case.fixture.conn.path().unwrap()).unwrap();
            let member = case.fixture.run.member_id;
            let mut changed = false;
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, _, _| {
                if boundary == "after_commit" && !changed {
                    mutation.execute("UPDATE members SET status='deregistered',deregistered_at=?1 WHERE member_id=?2",rusqlite::params![NOW,member]).unwrap();
                    changed = true;
                }
                Ok(())
            }));
            let report = case.recover(&mut host).unwrap();
            assert_eq!(counts(&report), (2, 1, 3));
            let rows = events(&case);
            assert_eq!(rows[0].event.notification_state, NotificationState::Inbox);
            assert!(rows[0].event.message_id.is_some());
            assert!(rows[1..].iter().all(|row| row.event.notification_state
                == NotificationState::Archived
                && row.event.message_id.is_none()
                && row.event.archive_reason == Some(ArchiveReason::MemberInactive)));
        }

        #[test]
        fn runner_recovery_preserves_latest_evidence_receipts_response_audits_and_episode_claims() {
            use crate::coding_agent::AnswerCapability;
            use crate::coding_agent::CodingAgent;
            let mut case = Case::new();
            case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET native_version='0.162.0',coverage='tested' WHERE run_id=?1",
            [case.fixture.run.run_id],
        )
        .unwrap();
            case.refresh();
            let owner = case.fixture.run.owner();
            let event = persist_opening(&mut case);
            let latest = evidence(&case.fixture.run, 77);
            storage::update_interaction_evidence(
                &mut case.fixture.conn,
                &owner,
                &InteractionEvidenceUpdate {
                    run_id: owner.run_id,
                    episode_seq: 1,
                    output_seq: 77,
                    evidence_json: latest.clone(),
                },
                1,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
            let input = storage::test_support::reserve_input(
                &mut case.fixture.conn,
                &owner,
                &NewInput {
                    operation_key: "prompt:retained-uncertainty".into(),
                    kind: InputKind::Prompt,
                    payload_digest: "d".repeat(64),
                    message_id: None,
                },
            )
            .unwrap();
            assert_eq!(input.state, InputState::InProgress);
            storage::record_input_uncertain(
                &mut case.fixture.conn,
                &owner,
                input.input_id,
                storage::UncertainCause::WriteFailed,
            )
            .unwrap();
            let capability = AnswerCapability::fixture(
                CodingAgent::Codex,
                case.fixture
                    .run
                    .native_version
                    .as_deref()
                    .expect("fixture run records its native version"),
                "recovery-fixture",
                "single-request-choice",
                PermissionDecision::Allow,
                &["synthetic-storage-fixture".into()],
            )
            .unwrap();
            let request = NewPermissionResponse {
                operation_key: "permission:retained-claim".into(),
                metadata: PermissionResponseMetadata {
                    event_id: event.event_id,
                    interaction_id: event.interaction_id.unwrap(),
                    expected_owner_epoch: owner.owner_epoch,
                    request_token: "a".repeat(64),
                    key: crate::coding_agent::catalog::AnswerKey::Decision(
                        PermissionDecision::Allow,
                    ),
                    director_member_id: case.fixture.run.director_member_id,
                    director_mode: DirectorMode::Auto,
                    decision_source: DecisionSource::DirectorAuto,
                    mode_evidence_ref: "fixture:mode".into(),
                    decision_evidence_ref: "fixture:decision".into(),
                    answer_rule_id: "single-request-choice".into(),
                },
            };
            assert!(matches!(
                storage::reserve_permission_response(
                    &mut case.fixture.conn,
                    &owner,
                    &request,
                    Some(&capability),
                    Instant::now() + Duration::from_secs(1)
                )
                .unwrap(),
                PermissionReservation::Reserved(_)
            ));
            let snapshot = || {
                rows(&case.fixture.conn)
                    .into_iter()
                    .filter(|table| {
                        table.name == "member_input_operations"
                            || table.name.starts_with("member_permission_")
                    })
                    .collect::<Vec<_>>()
            };
            let before = snapshot();
            assert!(before.iter().all(|table| !table.rows.is_empty()));
            case.recover(&mut new_host()).unwrap();
            let after = rows(&case.fixture.conn)
                .into_iter()
                .filter(|table| {
                    table.name == "member_input_operations"
                        || table.name.starts_with("member_permission_")
                })
                .collect::<Vec<_>>();
            assert_eq!(after, before);
            let interaction = events(&case)[0].interaction.clone().unwrap();
            assert_eq!(interaction.evidence_json, latest);
            assert_eq!(
                interaction.close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            assert_eq!(
                storage::get_input(&case.fixture.conn, input.input_id)
                    .unwrap()
                    .state,
                InputState::Uncertain
            );
            assert_eq!(events(&case)[0].event.payload_json, event.payload_json);
        }
    }

    mod replay {
        use crate::runtime::recovery::tests::*;

        fn unavailable(run: &RunRecord, sequence: i64) -> ObservedTransition {
            ObservedTransition {
                run_id: run.run_id,
                event_seq: sequence,
                episode_seq: None,
                kind: RuntimeEventKind::AutomaticReviewUnavailable,
                classification: None,
                observed_at: NOW.into(),
                evidence_json: storage::evidence::run_metadata(
                    run,
                    "codex-auto-review-unavailable",
                    u64::try_from(sequence).unwrap(),
                )
                .unwrap(),
                close_reason: None,
                cause_json: None,
            }
        }

        fn seed_unavailable(
            case: &mut Case,
        ) -> (ObservedTransition, storage::EffectivePolicyObservation) {
            let value = unavailable(&case.fixture.run, 1);
            let lost = storage::EffectivePolicyObservation {
                automatic_review: storage::AutomaticReview::Unavailable,
                observed_mode: None,
                rule_id: "codex-auto-review-unavailable".into(),
                observed_at: NOW.into(),
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut spool = Outbox::open_owned(
                &mut case.fixture.conn,
                &case.fixture.run,
                case.held.as_ref().unwrap().clone(),
                deadline,
            )
            .unwrap();
            spool
                .append_synced(&mut case.fixture.conn, &value, Some(&lost), deadline)
                .unwrap();
            (value, lost)
        }

        #[test]
        fn runner_recovery_replays_an_unavailable_frame_with_its_effective_policy_record() {
            let mut case = Case::new();
            let (value, _) = seed_unavailable(&mut case);
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            let rows = events(&case);
            assert_eq!(
                rows[0].event.kind,
                RuntimeEventKind::AutomaticReviewUnavailable
            );
            assert_eq!(rows[0].event.event_seq, value.event_seq);
            assert_eq!(rows[0].event.notification_state, NotificationState::Inbox);
            assert_eq!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .effective_policy_json,
                Some(json!({
                    "schema_version":1,"automatic_review":"unavailable","observed_mode":null,
                    "rule_id":"codex-auto-review-unavailable","event_id":rows[0].event.event_id,
                    "observed_at":NOW,
                }))
            );
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
        }

        #[test]
        fn runner_recovery_replays_a_committed_unavailable_frame_idempotently() {
            let mut case = Case::new();
            let (value, lost) = seed_unavailable(&mut case);
            let committed = storage::persist_transition_with_effective_policy(
                &mut case.fixture.conn,
                &case.fixture.run.owner(),
                &value,
                &lost,
                PersistenceMode::Live,
            )
            .unwrap()
            .0;
            let record = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                .unwrap()
                .effective_policy_json;
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            let rows = events(&case);
            assert_eq!(rows[0].event.event_id, committed.event_id);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::AutomaticReviewUnavailable)
                    .count(),
                1
            );
            assert_eq!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .effective_policy_json,
                record
            );
        }

        #[test]
        fn runner_recovery_replays_opening_and_obscurity_before_one_monitor_closure_and_loss() {
            let mut case = Case::new();
            let opening = opening(&case.fixture.run, 1, 1);
            let obscured = obscured(&case.fixture.run, 2, 1);
            case.seed(&[opening.clone(), obscured.clone()]);
            let high_water = case.fixture.run.event_seq_reserved_through;
            let mut host = new_host();
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            assert_eq!(counts(&report), (2, 4, 0));
            let rows = events(&case);
            assert_eq!(
                rows.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
                vec![
                    RuntimeEventKind::InteractionOpened,
                    RuntimeEventKind::InteractionObscured,
                    RuntimeEventKind::InteractionClosed,
                    RuntimeEventKind::RunnerLost
                ]
            );
            assert_eq!(rows[0].event.event_seq, opening.event_seq);
            assert_eq!(rows[1].event.event_seq, obscured.event_seq);
            assert!(rows[2].event.event_seq > high_water);
            assert!(rows[3].event.event_seq > rows[2].event.event_seq);
            assert_eq!(
                rows[2].interaction.as_ref().unwrap().close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            assert!(rows.iter().all(|row| row.event.notification_state
                == NotificationState::Inbox
                && row.event.message_id.is_some()));
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
            assert!(host.calls.iter().any(|name| name == "before_compact"));
        }

        #[test]
        fn runner_recovery_counts_only_new_commits_across_prefix_and_identical_duplicate_frames() {
            let mut case = Case::new();
            let prefix = opening(&case.fixture.run, 1, 1);
            persist_opening(&mut case);
            let second = obscured(&case.fixture.run, 2, 1);
            let third = obscured(&case.fixture.run, 3, 1);
            let mut bytes = frame(&prefix);
            bytes.extend(frame(&second));
            bytes.extend(frame(&second));
            bytes.extend(frame(&third));
            case.replace_owned_bytes(&bytes);
            let prefix_view = events(&case)[0].clone();
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (2, 4, 0));
            let rows = events(&case);
            assert_eq!(rows.len(), 5);
            assert_eq!(rows[0].event, prefix_view.event);
            assert_eq!(
                rows.iter().filter(|row| row.event.event_seq == 2).count(),
                1
            );
            assert_eq!(
                rows.iter()
                    .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                    .count(),
                1
            );
            let again = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&again), (0, 0, 0));
            assert_eq!(events(&case), rows);
        }

        #[test]
        fn runner_recovery_validates_all_complete_frames_before_any_replay_and_preserves_corruption()
         {
            for damage in ["conflict", "foreign_run"] {
                let mut case = Case::new();
                let first = opening(&case.fixture.run, 1, 1);
                let second = obscured(&case.fixture.run, 2, 1);
                let mut bytes = frame(&first);
                match damage {
                    "conflict" => {
                        let mut conflict = first.clone();
                        conflict.observed_at = "2026-10-26T20:00:01.000000+00:00".into();
                        bytes.extend(frame(&conflict));
                    }
                    "foreign_run" => {
                        let mut foreign = second.clone();
                        foreign.run_id += 1;
                        bytes.extend(frame(&foreign));
                    }
                    _ => unreachable!(),
                }
                case.replace_owned_bytes(&bytes);
                assert!(case.recover(&mut new_host()).is_err(), "{damage}");
                assert!(
                    events(&case).is_empty(),
                    "complete validation precedes replay: {damage}"
                );
                assert_eq!(case.spool_bytes().unwrap(), bytes, "{damage}");
            }
        }

        #[test]
        fn runner_recovery_incomplete_final_tail_replays_complete_originals_and_retains_a_diagnostic()
         {
            let mut case = Case::new();
            let first = opening(&case.fixture.run, 1, 1);
            let mut bytes = frame(&first);
            bytes.extend_from_slice(&[0, 0, 0, 40, 1, 2, 3]);
            case.replace_owned_bytes(&bytes);
            let report = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&report), (1, 3, 0));
            assert_eq!(
                events(&case)[0].event.kind,
                RuntimeEventKind::InteractionOpened
            );
            assert!(!report.cleanup_diagnostics.is_empty());
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
        }

        #[test]
        fn runner_recovery_replayed_native_exit_preserves_exact_code_or_signal_without_fabricated_loss()
         {
            for (code, signal) in [(Some(7), None), (None, Some(15))] {
                let mut case = Case::new();
                case.seed(&[exited(&case.fixture.run, 1, code, signal)]);
                let report = case.recover(&mut new_host()).unwrap();
                assert_eq!(report.status, RunStatus::Exited);
                assert_eq!(counts(&report), (1, 1, 0));
                case.refresh();
                assert_eq!(case.fixture.run.exit_code, code);
                assert_eq!(case.fixture.run.exit_signal, signal);
                assert_eq!(
                    events(&case)
                        .iter()
                        .map(|row| row.event.kind)
                        .collect::<Vec<_>>(),
                    vec![RuntimeEventKind::RunnerExited]
                );
            }
        }

        #[test]
        fn runner_recovery_expiry_after_a_commit_retains_counts_and_next_pass_counts_only_new_work()
        {
            let mut case = Case::new();
            case.seed(&[
                opening(&case.fixture.run, 1, 1),
                obscured(&case.fixture.run, 2, 1),
            ]);
            let mut host = Host::new(Instant::now() + Duration::from_millis(300));
            host.action = Some(Box::new(|boundary, _, deadline| {
                if boundary == "after_commit" {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    return Err(CafleetError::DeadlineExceeded);
                }
                Ok(())
            }));
            let first = case.recover(&mut host).unwrap();
            assert!(Instant::now() >= host.deadline);
            assert!(matches!(first.outcome, RecoveryOutcome::Recovering));
            assert_eq!(counts(&first), (1, 1, 0));
            assert_eq!(events(&case).len(), 1);
            assert!(case.spool_bytes().is_some_and(|bytes| !bytes.is_empty()));
            let epoch = storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                .unwrap()
                .owner_epoch;
            let second = case.recover(&mut new_host()).unwrap();
            assert_eq!(counts(&second), (1, 3, 0));
            assert_eq!(events(&case).len(), 4);
            assert!(
                storage::get_run(&case.fixture.conn, case.fixture.run.run_id)
                    .unwrap()
                    .owner_epoch
                    > epoch
            );
        }

        #[test]
        fn runner_recovery_interruption_boundaries_preserve_origin_and_idempotent_event_associations()
         {
            for stop in [
                "after_takeover",
                "before_replay",
                "after_commit",
                "before_compact",
                "before_finish",
            ] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let original = case.fixture.run.status;
                let original_bytes = case.spool_bytes().unwrap();
                let mut host = new_host();
                let mut fired = false;
                host.action = Some(Box::new(move |boundary, _, _| {
                    if boundary == stop && !fired {
                        fired = true;
                        return Err(CafleetError::App("injected bounded interruption".into()));
                    }
                    Ok(())
                }));
                let interrupted = case.recover(&mut host);
                assert!(host.calls.iter().any(|name| name == stop), "{stop}");
                let prior = events(&case);
                if stop == "before_compact" {
                    assert!(
                        interrupted.is_err()
                            || interrupted.is_ok_and(|report| matches!(
                                report.outcome,
                                RecoveryOutcome::Recovering
                            ))
                    );
                    assert!(
                        case.spool_bytes().unwrap().starts_with(&original_bytes),
                        "an interrupted compaction retains the original frames"
                    );
                    assert!(!prior.is_empty());
                }
                let result = case.recover(&mut new_host()).unwrap();
                assert_eq!(result.status, RunStatus::Lost, "{stop}");
                let final_rows = events(&case);
                assert_eq!(final_rows.len(), 3, "{stop}");
                if stop == "before_compact" {
                    assert_eq!(
                        final_rows.iter().map(|row| &row.event).collect::<Vec<_>>(),
                        prior.iter().map(|row| &row.event).collect::<Vec<_>>(),
                        "the committed delivery is complete before compaction"
                    );
                }
                for earlier in prior {
                    assert!(final_rows.iter().any(|row| row.event == earlier.event));
                }
                case.refresh();
                assert_eq!(case.fixture.run.recovery_origin_status, Some(original));
                assert_eq!(
                    final_rows
                        .iter()
                        .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                        .count(),
                    1
                );
            }
        }

        #[test]
        fn runner_recovery_fsynced_synthesis_from_an_interrupted_pass_counts_as_next_pass_input() {
            for failed_kind in ["interaction_closed", "runner_lost"] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                case.fixture.conn.execute_batch(&format!("CREATE TRIGGER refuse_synthesis BEFORE INSERT ON member_runtime_events WHEN NEW.kind='{failed_kind}' BEGIN SELECT RAISE(ABORT,'injected synthesis persistence refusal'); END")).unwrap();
                let first = case.recover(&mut new_host());
                let committed = events(&case);
                let expected_commits = if failed_kind == "interaction_closed" {
                    1
                } else {
                    2
                };
                assert_eq!(committed.len(), expected_commits);
                if let Ok(report) = first {
                    assert!(matches!(report.outcome, RecoveryOutcome::Recovering));
                    assert_eq!(
                        counts(&report),
                        (1, u64::try_from(expected_commits).unwrap(), 0)
                    );
                }
                let last = crate::runtime::writer::test_support::frame_bodies(
                    &case.spool_bytes().unwrap(),
                )
                .pop()
                .unwrap();
                assert_eq!(last["kind"], failed_kind);
                assert!(last["event_seq"].as_i64().unwrap() > 256);
                case.fixture
                    .conn
                    .execute_batch("DROP TRIGGER refuse_synthesis")
                    .unwrap();
                let next = case.recover(&mut new_host()).unwrap();
                assert_eq!(
                    counts(&next),
                    if failed_kind == "interaction_closed" {
                        (1, 2, 0)
                    } else {
                        (1, 1, 0)
                    }
                );
                let final_rows = events(&case);
                assert_eq!(final_rows.len(), 3);
                for earlier in committed {
                    assert!(final_rows.iter().any(|row| row.event == earlier.event));
                }
                assert_eq!(
                    final_rows
                        .iter()
                        .filter(|row| row.event.kind == RuntimeEventKind::RunnerLost)
                        .count(),
                    1
                );
            }
        }
    }

    mod terminal_tail {
        use crate::pane_lock::PaneLock;
        use crate::runtime::recovery::tests::*;

        fn terminal_case(status: RunStatus) -> Case {
            let mut case = Case::new();
            case.fixture.conn.execute(
        "UPDATE member_runs SET status=?1,cleanup_state='incomplete',exit_code=?2,ended_at=?3 WHERE run_id=?4",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, case.fixture.run.run_id],
    ).unwrap();
            case.refresh();
            case
        }

        fn exclusive_host(case: &Case) -> Host {
            let mut host = new_host();
            let database_url = case.settings.database_url.clone();
            let path = case.fixture.conn.path().unwrap().to_owned();
            let placement = case.placement.clone();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary == "before_compact" {
                    let conn = Connection::open(&path).unwrap();
                    assert!(RunLock::acquire(&conn, &run.owner()).unwrap().is_none());
                    assert!(
                        PaneLock::try_acquire(&database_url, &placement)
                            .unwrap()
                            .is_none()
                    );
                }
                Ok(())
            }));
            host
        }

        fn assert_diagnosed_tail(report: &RecoveryReport) {
            assert!(report.cleanup_diagnostics.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|text| text.contains("incomplete outbox"))
            }));
        }

        #[test]
        fn runner_recovery_terminal_empty_incomplete_suffix_is_retired_without_new_events() {
            for (status, suffix) in [
                (RunStatus::Exited, vec![0, 0]),
                (RunStatus::Lost, vec![0, 0, 0, 40, 1, 2, 3]),
                (RunStatus::Failed, vec![]),
            ] {
                let mut case = terminal_case(status);
                let original = case.fixture.run.clone();
                case.replace_owned_bytes(&suffix);
                let mut host = exclusive_host(&case);
                let report = case.recover(&mut host).unwrap();
                assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
                assert_eq!(report.status, status);
                assert_eq!(counts(&report), (0, 0, 0));
                if !suffix.is_empty() {
                    assert_diagnosed_tail(&report);
                }
                assert!(events(&case).is_empty());
                case.refresh();
                assert_eq!(case.fixture.run.status, status);
                assert_eq!(case.fixture.run.recovery_origin_status, Some(status));
                assert_eq!(case.fixture.run.exit_code, original.exit_code);
                assert_eq!(case.fixture.run.exit_signal, original.exit_signal);
                assert_eq!(case.fixture.run.cleanup_state, CleanupState::Complete);
                assert!(host.calls.iter().any(|name| name == "before_compact"));
                assert!(
                    case.spool_bytes().is_none_or(|bytes| bytes.is_empty()),
                    "terminal {status:?} suffix remains after reported completion"
                );
                let committed = rows(&case.fixture.conn);
                assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
                assert_eq!(rows(&case.fixture.conn), committed);
            }
        }

        #[test]
        fn runner_recovery_terminal_committed_originals_before_tail_keep_zero_new_counts_and_truthful_exit()
         {
            let (code, signal) = (None, Some(15));
            let mut case = Case::new();
            let (original, event) = case.exited_pending_cleanup(code, signal);
            let message =
                crate::broker::get_message(&case.fixture.conn, event.message_id.unwrap()).unwrap();
            let mut bytes = frame(&original);
            bytes.extend(frame(&original));
            bytes.extend([0, 0, 0, 40, 1, 2, 3]);
            case.replace_owned_bytes(&bytes);
            let mut host = exclusive_host(&case);
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Exited);
            assert_eq!(counts(&report), (0, 0, 0));
            assert_diagnosed_tail(&report);
            let committed = events(&case);
            assert_eq!(committed.len(), 1);
            assert_eq!(committed[0].event, event);
            assert_eq!(
                crate::broker::get_message(&case.fixture.conn, event.message_id.unwrap())
                    .unwrap()
                    .text,
                message.text
            );
            case.refresh();
            assert_eq!(case.fixture.run.exit_code, code);
            assert_eq!(case.fixture.run.exit_signal, signal);
            assert_eq!(case.fixture.run.cleanup_state, CleanupState::Complete);
            assert!(host.calls.iter().any(|name| name == "before_compact"));
            assert!(
                case.spool_bytes().is_none_or(|bytes| bytes.is_empty()),
                "committed originals and incomplete suffix remain after reported completion"
            );
        }

        #[test]
        fn runner_recovery_terminal_complete_corruption_before_tail_preserves_every_byte_and_commit()
         {
            let mut case = Case::new();
            let (original, event) = case.exited_pending_cleanup(Some(7), None);
            let mut bytes = frame(&original);
            let mut damaged = frame(&original);
            damaged[4] ^= 1;
            bytes.extend(damaged);
            bytes.extend([0, 0]);
            case.replace_owned_bytes(&bytes);
            assert!(case.recover(&mut new_host()).is_err());
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            let committed = events(&case);
            assert_eq!(committed.len(), 1);
            assert_eq!(committed[0].event, event);
        }
    }

    mod descriptor {
        use crate::runtime::recovery::test_support::descriptor::*;
        use crate::runtime::recovery::tests::*;
        #[test]
        fn runner_recovery_verified_creator_recovery_removes_its_private_descriptor() {
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
        fn runner_recovery_historical_descriptor_survives_reconciler_retries_until_verified_removal()
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
        fn runner_recovery_foreign_epoch_before_unlink_preserves_descriptor() {
            let mut case = described_creator();
            let original = owned_bytes(&case);
            let database = case.fixture.conn.path().unwrap().to_owned();
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary != "before_finish" {
                    return Ok(());
                }
                let conn = Connection::open(&database).unwrap();
                assert!(RunLock::acquire(&conn, &run.owner()).unwrap().is_none());
                bump_epoch(&conn, run.run_id);
                Ok(())
            }));
            let result = case.recover(&mut host);
            assert!(host.calls.iter().any(|name| name == "before_finish"));
            incomplete(&case, result);
            assert!(
                owned_bytes(&case) == original,
                "the refused descriptor remains unchanged"
            );
        }

        #[test]
        fn runner_recovery_invalid_descriptor_identity_epoch_shape_or_permissions_fails_privately()
        {
            for damage in ["nonce", "symlink"] {
                let mut case = described_creator();
                let mut value: Value = serde_json::from_slice(&owned_bytes(&case)).unwrap();
                if damage == "nonce" {
                    value["launch_nonce"] = json!("b".repeat(64));
                }
                let bytes = serde_json::to_vec(&value).unwrap();
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&case.artifacts.paths.descriptor)
                    .unwrap();
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                if damage == "symlink" {
                    let retained = case.artifacts.paths.run_dir.join("invalid-target.json");
                    std::fs::rename(&case.artifacts.paths.descriptor, &retained).unwrap();
                    std::os::unix::fs::symlink(&retained, &case.artifacts.paths.descriptor)
                        .unwrap();
                }
                let before = identity(&case.artifacts.paths.descriptor);
                let result = recover_released(&mut case);
                incomplete(&case, result);
                assert_eq!(identity(&case.artifacts.paths.descriptor), before);
            }
        }

        #[test]
        fn runner_recovery_exceptional_database_only_reconciliation_preserves_suspect_descriptor() {
            let mut case = described_creator();
            let before = identity(&case.artifacts.paths.descriptor);
            missing_lock(&mut case);
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(identity(&case.artifacts.paths.descriptor), before);
            secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
        }
    }

    mod descriptor_cleanup {
        use crate::runtime::recovery::test_support::descriptor::*;
        use crate::runtime::recovery::tests::*;
        #[test]
        fn runner_recovery_coordinator_propagates_actual_unlink_failure_and_retries_historical_epoch()
         {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let mut failed =
                Host::with_unlink(&case, Instant::now() + Duration::from_secs(2), true);
            let result = case.recover(&mut failed);
            assert_eq!(
                failed.unlink.as_ref().unwrap().calls,
                1,
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
            let mut retry =
                Host::with_unlink(&case, Instant::now() + Duration::from_secs(2), false);
            let report = case.recover(&mut retry).unwrap();
            assert_eq!(retry.unlink.as_ref().unwrap().calls, 1);
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
        fn runner_recovery_expired_original_budget_prevents_actual_unlink_dispatch() {
            let mut case = described_creator();
            let bytes = owned_bytes(&case);
            let mut host =
                Host::with_unlink(&case, Instant::now() + Duration::from_millis(500), false);
            host.action = Some(Box::new(|boundary, _, deadline| {
                if boundary == "before_finish" {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
                Ok(())
            }));
            let result = case.recover(&mut host);
            assert!(host.calls.iter().any(|name| name == "before_finish"));
            assert!(Instant::now() >= host.deadline);
            assert_eq!(
                host.unlink.as_ref().unwrap().calls,
                0,
                "the expired deadline is checked before forwarding the unlink seam"
            );
            incomplete(&case, result);
            assert!(
                owned_bytes(&case) == bytes,
                "expiry preserves the selected descriptor"
            );
        }
    }

    mod replay_relationships {
        use crate::runtime::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_finalization_after_replay_refuses_actual_member_fleet_change_atomically()
        {
            refusal(Stage::Finalization);
        }
        #[test]
        fn runner_recovery_valid_historical_lifecycle_change_after_replay_allows_finalization() {
            historical_control(Stage::Finalization);
        }
    }

    mod delivery_results {
        use crate::runtime::recovery::tests::*;

        fn uncertain_deferred(case: &Case, key: &str, cause: Option<&str>) -> i64 {
            InputRow {
                key,
                state: "uncertain",
                delivery_method: Some("profile"),
                uncertain_cause: cause,
                deferred: true,
                ..InputRow::default()
            }
            .insert(case)
        }

        fn published_causes(case: &Case, input_id: i64) -> Vec<Option<Value>> {
            delivery_results(case, input_id)
                .into_iter()
                .map(|payload| Some(payload["cause"].clone()).filter(|cause| !cause.is_null()))
                .collect()
        }

        #[test]
        fn runner_recovery_replay_publishes_each_uncertain_result_with_its_receipt_cause() {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let caused = uncertain_deferred(&case, "prompt:caused", Some("write_failed"));
            let causeless = uncertain_deferred(&case, "prompt:causeless", None);
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(
                published_causes(&case, caused),
                vec![Some(json!({"kind":"write_failed"}))]
            );
            assert_eq!(published_causes(&case, causeless), vec![None]);
            case.recover(&mut new_host()).unwrap();
            assert_eq!(published_causes(&case, caused).len(), 1, "published once");
        }

        #[test]
        fn runner_recovery_replay_publishes_a_causeless_result_for_a_receipt_reconciled_before_publication()
         {
            for (disposition, status, stage) in [
                ("submitted", "submitted", "submitted"),
                ("not_submitted", "not_submitted", "zero_bytes"),
            ] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let reconciled =
                    uncertain_deferred(&case, "prompt:reconciled", Some("write_failed"));
                case.fixture
                    .conn
                    .execute(
                        "UPDATE member_input_operations SET state='settled',disposition=?1,evidence_ref='audit:inspected' WHERE input_id=?2",
                        rusqlite::params![disposition, reconciled],
                    )
                    .unwrap();
                assert_eq!(
                    storage::get_input(&case.fixture.conn, reconciled)
                        .unwrap()
                        .uncertain_cause,
                    Some(storage::UncertainCause::WriteFailed),
                    "{disposition}: reconciliation keeps the recorded cause"
                );
                let report = case.recover(&mut new_host()).unwrap();
                assert!(
                    matches!(report.outcome, RecoveryOutcome::Recovered),
                    "{disposition}"
                );
                let published = delivery_results(&case, reconciled)
                    .into_iter()
                    .map(|payload| {
                        (
                            payload["delivery"]["status"].as_str().unwrap().to_owned(),
                            payload["stage"].as_str().unwrap().to_owned(),
                            payload["cause"].as_str().map(str::to_owned),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    published,
                    vec![(status.to_owned(), stage.to_owned(), None)],
                    "{disposition}: a reconciled result publishes once without an uncertain cause"
                );
            }
        }
    }

    use crate::config::Settings;

    use crate::digest::sha256_hex;

    use crate::runtime::system::test_support::TestChild;

    mod crash {
        use crate::runtime::recovery::tests::*;
        use std::os::unix::fs::OpenOptionsExt;
        use std::path::Path;
        use std::path::PathBuf;
        use std::process::Command;
        use std::process::Stdio;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;

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
            persist_opening(&mut case);
            missing_lock(&mut case);
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
            let attestation = claim(&case.fixture.run, Some("audit:owned-transaction-fixture"));
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
            persist_opening(&mut case);
            missing_lock(&mut case);
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
            let settings = Settings::for_database(std::path::Path::new(
                crate::db::database_path(database_url).unwrap(),
            ));
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
                &json!({"report":crate::output::json::runtime_recovery(&report),"spool_state":spool_state}),
            );
        }
    }

    mod identity {
        use crate::pane_lock::PaneLock;
        use crate::runtime::recovery::tests::*;

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
            let settings = Settings::for_database(&copy);
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
            assert!(!case.artifacts.paths.descriptor.exists());
            let before = rows(&case.fixture.conn);
            assert_eq!(counts(&case.recover(&mut new_host()).unwrap()), (0, 0, 0));
            assert_eq!(rows(&case.fixture.conn), before);
        }

        #[test]
        fn runner_recovery_borrowed_original_guard_reaches_actual_replay_without_reacquiring_pane()
        {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = case.pane_lock();
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
            let held = case.pane_lock();
            case.release();
            bump_epoch(&case.fixture.conn, expected.run_id);
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
            let held = case.pane_lock();
            case.release();
            bump_epoch(&case.fixture.conn, expected.run_id);
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
            for field in ["nonce", "member", "pane"] {
                let mut case = Case::new();
                let mut expected = case.fixture.run.clone();
                let mut placement = case.placement.clone();
                match field {
                    "nonce" => expected.launch_nonce = "b".repeat(64),
                    "member" => expected.member_id += 1,
                    "pane" => placement.mux_pane_id = Some("%foreign".into()),
                    _ => unreachable!(),
                }
                let held = case.pane_lock();
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
        fn stable_run_identity_changes_with_each_stable_field_and_ignores_mutable_state() {
            let run = crate::runtime::test_support::Fixture::new().run;
            let stable = stable_run_identity(&run);
            type Change = (&'static str, fn(&mut RunRecord));
            let changes: [Change; 18] = [
                ("database", |run| {
                    run.database_identity_json["instance_id"] = json!("0".repeat(32))
                }),
                ("fleet", |run| run.fleet_id += 1),
                ("member", |run| run.member_id += 1),
                ("director", |run| run.director_member_id += 1),
                ("backend", |run| run.backend.push_str("-foreign")),
                ("run", |run| run.run_id += 1),
                ("nonce", |run| run.launch_nonce = "b".repeat(64)),
                ("epoch", |run| run.owner_epoch += 1),
                ("owner_kind", |run| {
                    run.owner_kind = if run.owner_kind == OwnerKind::Reconciler {
                        OwnerKind::Runner
                    } else {
                        OwnerKind::Reconciler
                    }
                }),
                ("owner_pid", |run| run.owner_pid += 1),
                ("owner_start", |run| {
                    run.owner_start_identity.push_str("-foreign")
                }),
                ("runtime_path", |run| run.runtime_path.push_str("-foreign")),
                ("directory", |run| {
                    run.runtime_dir_identity_json = Some(json!({"device":0,"inode":0}))
                }),
                ("lock", |run| {
                    run.run_lock_identity_json = Some(json!({"device":0,"inode":0}))
                }),
                ("runner_pid", |run| run.runner_pid = Some(1)),
                ("runner_start", |run| {
                    run.runner_start_identity = Some("foreign-generation".into())
                }),
                ("child_pid", |run| run.child_pid = Some(1)),
                ("child_start", |run| {
                    run.child_start_identity = Some("foreign-generation".into())
                }),
            ];
            for (field, change) in changes {
                let mut changed = run.clone();
                change(&mut changed);
                assert_ne!(stable_run_identity(&changed), stable, "{field}");
            }
            let mut mutable = run.clone();
            mutable.last_health_at = Some("2000-01-01T00:00:00Z".into());
            mutable.reachability = Reachability::Suspect;
            mutable.cleanup_diagnostics_json = Some(json!(["old bounded diagnostic"]));
            mutable.event_seq_reserved_through += 256;
            mutable.observation_gap_count += 1;
            assert_eq!(stable_run_identity(&mutable), stable);
        }

        #[test]
        fn runner_recovery_borrowed_fixed_snapshot_accepts_changed_mutable_health_and_current_counters()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = case.pane_lock();
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
            let pane = case.pane_lock();
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
                    bump_epoch(&connection, run_id);
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
        use crate::runtime::recovery::tests::*;

        fn clock_case(borrowed: bool, cap: Duration, delay: Duration) {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let expected = case.fixture.run.clone();
            let held = borrowed.then(|| case.pane_lock());
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
    }

    mod completion {
        use crate::runtime::recovery::tests::*;
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use rusqlite::hooks::TransactionOperation;
        use std::sync::Mutex;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;

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
            let claim =
                exceptional.then(|| claim(&case.fixture.run, Some("audit:completion-race")));
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

        fn immediate_completion(exceptional: bool, foreign_owner: bool, status: RunStatus) {
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

        #[test]
        fn runner_recovery_same_owner_completion_before_ordinary_takeover_has_zero_effects() {
            immediate_completion(false, false, RunStatus::Exited);
        }

        #[test]
        fn runner_recovery_same_owner_completion_before_exceptional_takeover_has_zero_effects() {
            immediate_completion(true, false, RunStatus::Failed);
        }

        #[test]
        fn runner_recovery_completed_foreign_owner_still_rejects_fixed_snapshot_and_attestation() {
            immediate_completion(false, true, RunStatus::Lost);
            immediate_completion(true, true, RunStatus::Exited);
        }

        fn completion_behind_write_lock(exceptional: bool, status: RunStatus) {
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

        #[test]
        fn runner_recovery_ordinary_completion_is_rechecked_after_actual_write_lock_wait() {
            completion_behind_write_lock(false, RunStatus::Exited);
        }

        #[test]
        fn runner_recovery_exceptional_completion_is_rechecked_after_actual_write_lock_wait() {
            completion_behind_write_lock(true, RunStatus::Lost);
        }
    }

    mod missing_paths {
        use crate::runtime::recovery::tests::*;
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
                let mut attestation = claim(&case.fixture.run, None);
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
            missing_lock(&mut case);
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
                let attestation = claim(&case.fixture.run, None);
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
        use crate::runtime::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_ordinary_takeover_refuses_actual_member_fleet_change_atomically() {
            refusal(Stage::OrdinaryTakeover);
        }
        #[test]
        fn runner_recovery_valid_historical_ordinary_takeover_keeps_truth_and_existing_inbox() {
            historical_control(Stage::OrdinaryTakeover);
        }
    }

    mod deferred_inputs {
        use crate::runtime::recovery::tests::*;

        fn insert(case: &Case, key: &str, state: &str, deferred: bool) -> i64 {
            InputRow {
                key,
                state,
                deferred,
                ..InputRow::default()
            }
            .insert(case)
        }

        #[test]
        fn runner_recovery_settles_a_deferred_ready_receipt_not_submitted_without_replay() {
            let mut case = Case::creator();
            let deferred = insert(&case, "prompt:deferred", "ready", true);
            let attempted = insert(&case, "prompt:attempted", "in_progress", false);
            let attempted_before = storage::get_input(&case.fixture.conn, attempted).unwrap();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            let settled = storage::get_input(&case.fixture.conn, deferred).unwrap();
            assert_eq!(settled.state, InputState::Settled);
            assert_eq!(settled.disposition, Some(InputDisposition::NotSubmitted));
            assert_eq!(settled.delivery_method, None);
            assert_eq!(settled.delivery_reason.as_deref(), Some("runner_lost"));
            assert_eq!(settled.defer_reason.as_deref(), Some("interaction_open"));
            assert_eq!(
                storage::get_input(&case.fixture.conn, attempted).unwrap(),
                attempted_before,
                "an attempt that may have written bytes keeps its reconciliation state"
            );
            let results = |case: &Case| delivery_results(case, deferred).len();
            assert_eq!(
                results(&case),
                1,
                "the recovery settlement publishes its deferred result"
            );
            case.recover(&mut new_host()).unwrap();
            assert_eq!(
                storage::get_input(&case.fixture.conn, deferred).unwrap(),
                settled,
                "a repeated recovery leaves the settled receipt unchanged"
            );
            assert_eq!(results(&case), 1, "the result is published once");
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use rusqlite::Connection;
    use serde_json::json;

    use crate::broker::Placement;
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::config::Settings;
    use crate::db::test_support::TableSnapshot;
    use crate::error::CafleetError;
    use crate::runtime::test_support::*;
    use crate::runtime::*;
    use serde_json::Value;
    use std::io::Write;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;
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
                Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()));
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
            let mut conn = Connection::open(self.fixture.conn.path().unwrap()).unwrap();
            let mut spool =
                Outbox::open_owned(&mut conn, &self.fixture.run, held.clone(), deadline).unwrap();
            for transition in transitions {
                spool
                    .append_synced(&mut conn, transition, None, deadline)
                    .unwrap();
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
            let event = storage::persist_transition(
                &mut self.fixture.conn,
                &self.fixture.run.owner(),
                &transition,
                PersistenceMode::Live,
            )
            .unwrap()
            .0;
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
            held: &crate::pane_lock::PaneLock,
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

        /// Runs `read` while the current process holds the run's validated original lock,
        /// reacquiring it when the case released it.
        pub(crate) fn with_owner_lock<T>(&self, read: impl FnOnce() -> T) -> T {
            let run = storage::get_run(&self.fixture.conn, self.fixture.run.run_id).unwrap();
            let current = ProcessIdentity::current().unwrap();
            assert_eq!(
                run.owner_pid, current.pid,
                "the current process owns the run"
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
            read()
        }

        pub(crate) fn pane_lock(&self) -> crate::pane_lock::PaneLock {
            crate::pane_lock::PaneLock::try_acquire(&self.settings.database_url, &self.placement)
                .unwrap()
                .unwrap()
        }

        pub(crate) fn spool_bytes(&self) -> Option<Vec<u8>> {
            self.with_owner_lock(|| match std::fs::read(&self.artifacts.paths.outbox) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("owned spool snapshot failed: {error}"),
            })
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
    pub(crate) use crate::runtime::writer::test_support::frame;
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

    /// Makes the next write fail: `Some` installs that trigger, `None` refuses the next commit.
    pub(crate) fn fail_next(conn: &mut Connection, trigger: Option<&str>) {
        match trigger {
            Some(trigger) => conn.execute_batch(trigger).unwrap(),
            None => conn.commit_hook(Some(|| true)).unwrap(),
        }
    }

    pub(crate) fn rows(conn: &Connection) -> Vec<TableSnapshot> {
        crate::db::test_support::table_snapshot(conn, |_| true)
    }

    /// A prompt receipt on the case's run; `deferred` stamps `deferred_at`/`defer_reason`.
    #[derive(Default)]
    pub(crate) struct InputRow<'a> {
        pub(crate) key: &'a str,
        pub(crate) state: &'a str,
        pub(crate) disposition: Option<&'a str>,
        pub(crate) delivery_reason: Option<&'a str>,
        pub(crate) delivery_method: Option<&'a str>,
        pub(crate) uncertain_cause: Option<&'a str>,
        pub(crate) deferred: bool,
    }

    impl InputRow<'_> {
        pub(crate) fn insert(&self, case: &Case) -> i64 {
            case.fixture
                .conn
                .execute(
                    "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,disposition,delivery_reason,delivery_method,uncertain_cause,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'prompt',?3,?4,?5,?6,?7,?8,?9,?9,?10,?11)",
                    rusqlite::params![
                        case.fixture.run.run_id,
                        self.key,
                        "d".repeat(64),
                        self.state,
                        self.disposition,
                        self.delivery_reason,
                        self.delivery_method,
                        self.uncertain_cause,
                        NOW,
                        self.deferred.then_some(NOW),
                        self.deferred.then_some("interaction_open"),
                    ],
                )
                .unwrap();
            case.fixture.conn.last_insert_rowid()
        }
    }

    pub(crate) fn delivery_results(case: &Case, input_id: i64) -> Vec<Value> {
        case.fixture
            .conn
            .prepare(
                "SELECT payload_json FROM member_runtime_events WHERE run_id=?1 AND kind='input_delivery_result' AND json_extract(payload_json,'$.input_id')=?2 ORDER BY event_seq",
            )
            .unwrap()
            .query_map(rusqlite::params![case.fixture.run.run_id, input_id], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .map(|text| serde_json::from_str(&text.unwrap()).unwrap())
            .collect()
    }

    pub(crate) fn persist_opening(case: &mut Case) -> RuntimeEventRecord {
        let transition = opening(&case.fixture.run, 1, 1);
        storage::persist_transition(
            &mut case.fixture.conn,
            &case.fixture.run.owner(),
            &transition,
            PersistenceMode::Live,
        )
        .unwrap()
        .0
    }

    pub(crate) fn missing_lock(case: &mut Case) {
        case.release();
        std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
    }

    pub(crate) fn claim(
        run: &RunRecord,
        evidence_ref: Option<&str>,
    ) -> OwnerTerminationAttestation {
        OwnerTerminationAttestation {
            expected_run_id: run.run_id,
            expected_owner_epoch: run.owner_epoch,
            evidence_ref: evidence_ref
                .unwrap_or("audit:owned-fixture-termination")
                .into(),
        }
    }

    pub(crate) fn bump_epoch(conn: &Connection, run_id: i64) {
        assert_eq!(
            conn.execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [run_id],
            )
            .unwrap(),
            1
        );
    }

    pub(crate) mod missing_paths {
        use crate::runtime::recovery::test_support::*;
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
        use crate::runtime::recovery::test_support::*;
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::OpenOptionsExt;
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
            case.with_owner_lock(|| std::fs::read(&case.artifacts.paths.descriptor).unwrap())
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
        /// Recovers right after the case's owner lock is released. A child forked by a concurrent
        /// test shares that flock until its exec closes it, which recovery reads as a live owner,
        /// so retry that side-effect-free outcome briefly.
        pub(crate) fn recover_released(case: &mut Case) -> TestResult<RecoveryReport> {
            let until = Instant::now() + crate::pane_lock::PaneLock::RELEASED_LOCK_GRACE;
            loop {
                let result = case.recover(&mut new_host());
                let contended = matches!(
                    &result,
                    Ok(report) if matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable)
                );
                if !contended || Instant::now() >= until {
                    return result;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        pub(crate) fn incomplete(case: &Case, result: TestResult<RecoveryReport>) {
            match result {
                Err(error) => secret_free(&error.message()),
                Ok(report) => {
                    assert!(
                        matches!(report.outcome, RecoveryOutcome::Recovering),
                        "descriptor failure remains an error or incomplete outcome, got {:?}",
                        report.outcome
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
        use crate::runtime::recovery::test_support::*;
        use std::sync::Mutex;

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
                    missing_lock(&mut case);
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

        fn move_member_to(conn: &Connection, run: &RunRecord, fleet_id: i64) {
            let changed = conn
                .execute(
                    "UPDATE members SET fleet_id=?1 WHERE member_id=?2",
                    rusqlite::params![fleet_id, run.member_id],
                )
                .unwrap();
            assert_eq!(changed, 1);
        }

        pub(crate) fn refusal(stage: Stage) {
            let mut case = prepare(stage);
            let original = case.fixture.run.clone();
            let target = create_fleet_fixture(&mut case.fixture.conn, "other-relationship-fleet").0;
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
                move_member_to(&mutation, run, target);
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
                "relationship refusal preserves the exact database after the injected change: {stage:?} changed {changed:?}"
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
                    crate::broker::members::test_support::deregister_member(
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
            let lifecycle = Lifecycle::DirectorInactive;
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
