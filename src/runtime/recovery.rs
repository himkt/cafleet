use crate::broker::Placement;
use crate::broker::runtime as storage;
use crate::broker::runtime::*;
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
    Unknown,
}
#[derive(Debug)]
pub(crate) enum RecoveryHealth {
    Matching,
    Unavailable(String),
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecoveryBoundary {
    AfterSnapshot,
    BeforeTakeover,
    AfterTakeover,
    AfterCommit,
    BeforeFinish,
}
pub(crate) trait RecoveryHost {
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock>;
    fn current_process(&mut self) -> Result<ProcessIdentity>;
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        deadline: Instant,
    ) -> Result<ProcessEvidence>;
    fn health(&mut self, run: &RunRecord, deadline: Instant) -> Result<RecoveryHealth>;
    fn cleanup_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownDeadlines,
    ) -> Result<ShutdownOutcome>;
    fn unlink_descriptor(&mut self, path: &std::path::Path, deadline: Instant) -> Result<()> {
        unlink_descriptor_file(path, deadline, &*self.clock())
    }
    #[cfg(test)]
    fn checkpoint(&mut self, _: RecoveryBoundary, _: &RunRecord, _: Instant) -> Result<()> {
        Ok(())
    }
}

fn unlink_descriptor_file(
    path: &std::path::Path,
    deadline: Instant,
    clock: &dyn crate::time::Clock,
) -> Result<()> {
    check_deadline(clock.now(), deadline)?;
    std::fs::remove_file(path).map_err(|cause| error(format!("descriptor unlink failed: {cause}")))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryOutcome {
    Recovered,
    Recovering,
    LiveOrUnreachable,
}
#[derive(Debug)]
pub(crate) struct RecoveryReport {
    pub(crate) member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) outcome: RecoveryOutcome,
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
    deadline: Instant,
    host: &mut impl RecoveryHost,
) -> Result<RecoveryReport> {
    recover(conn, settings, member_id, deadline, None, host).map(|value| value.report)
}

struct BorrowedPane<'a> {
    run: &'a RunRecord,
    placement: &'a Placement,
    guard: &'a PaneLock,
}
#[cfg(test)]
pub(crate) fn recover_member_with_held_pane_lock(
    conn: &mut Connection,
    settings: &Settings,
    expected_run: &RunRecord,
    expected_placement: &Placement,
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
        expected_run.member_id,
        deadline,
        Some(borrowed),
        host,
    )
}
pub(crate) fn recover_member_under_pane_lock(
    conn: &mut Connection,
    settings: &Settings,
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
        expected_run.member_id,
        deadline,
        Some(borrowed),
        &mut NativeRecoveryHost,
    )
}
fn recover(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    until: Instant,
    borrowed: Option<BorrowedPane<'_>>,
    host: &mut impl RecoveryHost,
) -> Result<RecoveryCompletion> {
    let clock = ShutdownDeadlines::starting_now(&*host.clock()).clipped(until);
    check_deadline(host.clock().now(), until)?;
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
    check_deadline(host.clock().now(), until)?;
    #[cfg(test)]
    host.checkpoint(RecoveryBoundary::AfterSnapshot, &run, until)?;
    let unborrowed_pane = pinned
        .as_ref()
        .filter(|item| borrowed.is_none() && item.mux_pane_id.is_some());
    let acquired_pane = match unborrowed_pane {
        Some(target) => loop {
            check_deadline(host.clock().now(), until)?;
            if let Some(guard) = PaneLock::try_acquire(&settings.database_url, target)? {
                break Some(guard);
            }
            if matches!(
                with_database_deadline(conn, until, |conn| RunLock::acquire(conn, &run.owner())),
                Ok(None)
            ) {
                return completion(run, RecoveryOutcome::LiveOrUnreachable);
            }
            host.clock().sleep(
                Duration::from_millis(5).min(until.saturating_duration_since(host.clock().now())),
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
    let health = host.health(&run, until)?;
    let health_live = match &health {
        RecoveryHealth::Matching => true,
        RecoveryHealth::Unavailable(reason) => {
            ensure(
                !reason.is_empty(),
                "health uncertainty requires a diagnostic",
            )?;
            false
        }
    };
    if !matches!(owner, ProcessEvidence::Terminated)
        || run.owner_kind == OwnerKind::Creator
        || health_live
    {
        return completion(run, RecoveryOutcome::LiveOrUnreachable);
    }
    check_deadline(host.clock().now(), until)?;
    #[cfg(test)]
    host.checkpoint(RecoveryBoundary::BeforeTakeover, &run, until)?;
    let process = host.current_process()?;
    let recheck = |conn: &Connection| -> Result<()> {
        check_pane()?;
        check_snapshot(conn, &run, pinned.as_ref()).map(|_| ())
    };
    if path_failure.is_some() {
        return reconcile_missing_runtime(
            conn,
            &run,
            pinned.as_ref(),
            &process,
            until,
            recheck,
            &*host.clock(),
        );
    }
    let lock = lock.expect("verified ordinary recovery lock");
    let current =
        lock.transfer_to_reconciler_checked(conn, &run, process, until, recheck, &*host.clock())?;
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
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
        std::sync::Arc::new(crate::time::SystemClock)
    }
    fn current_process(&mut self) -> Result<ProcessIdentity> {
        ProcessIdentity::current()
    }
    fn process_evidence(
        &mut self,
        process: &ProcessIdentity,
        until: Instant,
    ) -> Result<ProcessEvidence> {
        check_deadline(self.clock().now(), until)?;
        match process.is_alive() {
            Ok(true) => Ok(ProcessEvidence::Alive),
            Ok(false) => Ok(ProcessEvidence::Terminated),
            Err(_) => Ok(ProcessEvidence::Unknown),
        }
    }
    fn health(&mut self, run: &RunRecord, until: Instant) -> Result<RecoveryHealth> {
        match crate::runtime::control::request(
            run,
            LifecycleRequest::health(run),
            until,
            &*self.clock(),
        ) {
            Ok(value) => match HealthSnapshot::decode(value, run) {
                Ok(_) => Ok(RecoveryHealth::Matching),
                Err(cause) => Ok(RecoveryHealth::Unavailable(cause.message().into_owned())),
            },
            Err(cause) => Ok(RecoveryHealth::Unavailable(cause.message().into_owned())),
        }
    }
    fn cleanup_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownDeadlines,
    ) -> Result<ShutdownOutcome> {
        crate::runtime::process::recover_descendants(run, clock, &*self.clock())
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
    until: Instant,
    check: impl FnOnce(&Connection) -> Result<()>,
    clock: &dyn crate::time::Clock,
) -> Result<RecoveryCompletion> {
    with_database_deadline(conn, until, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check(&tx)?;
        let current = check_snapshot(&tx, expected, pinned)?;
        if current.is_settled() {
            check_deadline(clock.now(), until)?;
            tx.commit()?;
            return completion(current, RecoveryOutcome::Recovered);
        }
        let established = terminal_outcome(&tx, &current)?;
        let run = advance_recovery_owner(&tx, &current, process)?;
        let mut report = completion(run.clone(), RecoveryOutcome::Recovering)?.report;
        report
            .cleanup_diagnostics
            .push(json!("runtime_path_missing"));
        tx.execute(
            "UPDATE member_runs SET cleanup_diagnostics_json=?1 WHERE run_id=?2",
            params![
                serde_json::to_string(&report.cleanup_diagnostics).map_err(io_error)?,
                run.run_id
            ],
        )?;
        let target = match established {
            Some(status) => status,
            None => {
                let first = next_event_seq(&tx, &run)?;
                for transition in loss_transitions(&tx, &run, first, true, clock)? {
                    let (_, inserted) = storage::persist_transition_in_transaction(
                        &tx,
                        &run,
                        &transition,
                        PersistenceMode::Recovery,
                    )?;
                    ensure(inserted, "missing-path recovery requires fresh transitions")?;
                }
                RunStatus::Lost
            }
        };
        storage::settle_lost_deferred_in_transaction(&tx, &run)?;
        let unpublished = storage::unpublished_delivery_results(&tx, run.run_id)?;
        if !unpublished.is_empty() {
            let evidence = storage::run_evidence(&run, "runtime-recovery", 0)?;
            for (event_seq, receipt) in (next_event_seq(&tx, &run)?..).zip(unpublished) {
                let transition = delivery_result_transition(
                    run.run_id,
                    event_seq,
                    &receipt,
                    clock.wall_stamp(),
                    evidence.clone(),
                );
                storage::persist_transition_in_transaction(
                    &tx,
                    &run,
                    &transition,
                    PersistenceMode::Recovery,
                )?;
            }
        }
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2) WHERE run_id=?3",
            params![target.as_str(),clock.wall_stamp(),run.run_id])?;
        let run = storage::get_run(&tx, run.run_id)?;
        check_deadline(clock.now(), until)?;
        tx.commit()?;
        report.outcome = RecoveryOutcome::Recovered;
        report.status = run.status;
        Ok(RecoveryCompletion { report, run })
    })
}

fn next_event_seq(conn: &Connection, run: &RunRecord) -> Result<i64> {
    Ok(storage::latest_event_seq(conn, run.run_id)?.unwrap_or(0) + 1)
}

fn reconcile_retained_runtime(
    conn: &mut Connection,
    run: RunRecord,
    pinned: Option<Placement>,
    lock: Arc<RunLock>,
    host: &mut impl RecoveryHost,
    clock: ShutdownDeadlines,
    until: Instant,
) -> Result<RecoveryCompletion> {
    let mut result = completion(run, RecoveryOutcome::Recovering)?;
    let attempt = perform(conn, &mut result, pinned.as_ref(), lock, host, clock, until);
    match attempt {
        Ok(()) => Ok(result),
        Err(cause)
            if matches!(cause, CafleetError::DeadlineExceeded) || host.clock().now() >= until =>
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
fn persist(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    transition: &ObservedTransition,
    host: &mut impl RecoveryHost,
    until: Instant,
) -> Result<()> {
    with_database_deadline(conn, until, |conn| {
        storage::persist_transition(
            conn,
            &result.run.owner(),
            transition,
            PersistenceMode::Recovery,
        )
    })?;
    refresh(conn, result, until)?;
    check_deadline(host.clock().now(), until)?;
    #[cfg(test)]
    host.checkpoint(RecoveryBoundary::AfterCommit, &result.run, until)?;
    Ok(())
}
fn loss_transitions(
    conn: &Connection,
    run: &RunRecord,
    first: i64,
    missing: bool,
    clock: &dyn crate::time::Clock,
) -> Result<Vec<ObservedTransition>> {
    let snapshot = storage::runtime_snapshot(conn, run.run_id)?;
    let mut next = first;
    let now = clock.wall_stamp();
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
    let evidence = storage::run_evidence(run, "runtime-recovery", 0)?;
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
fn publish_delivery_results(
    conn: &mut Connection,
    result: &mut RecoveryCompletion,
    host: &mut impl RecoveryHost,
    until: Instant,
) -> Result<()> {
    let (unpublished, first) = with_database_deadline(conn, until, |conn| {
        Ok((
            storage::unpublished_delivery_results(conn, result.run.run_id)?,
            next_event_seq(conn, &result.run)?,
        ))
    })?;
    let evidence = storage::run_evidence(&result.run, "runtime-recovery", 0)?;
    for (event_seq, receipt) in (first..).zip(&unpublished) {
        let transition = delivery_result_transition(
            result.run.run_id,
            event_seq,
            receipt,
            host.clock().wall_stamp(),
            evidence.clone(),
        );
        persist(conn, result, &transition, host, until)?;
    }
    Ok(())
}
fn terminal_outcome(conn: &Connection, run: &RunRecord) -> Result<Option<RunStatus>> {
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
    clock: ShutdownDeadlines,
    until: Instant,
) -> Result<()> {
    check_deadline(host.clock().now(), until)?;
    #[cfg(test)]
    host.checkpoint(RecoveryBoundary::AfterTakeover, &result.run, until)?;
    let mut descriptor = RecoveryDescriptor::pin(conn, &result.run, &lock, until, &*host.clock())?;
    let existing = with_database_deadline(conn, until, |conn| terminal_outcome(conn, &result.run))?;
    let target = match existing {
        Some(status) => status,
        None => {
            let transitions = with_database_deadline(conn, until, |conn| {
                loss_transitions(
                    conn,
                    &result.run,
                    next_event_seq(conn, &result.run)?,
                    false,
                    &*host.clock(),
                )
            })?;
            for transition in transitions {
                persist(conn, result, &transition, host, until)?;
            }
            RunStatus::Lost
        }
    };
    publish_delivery_results(conn, result, host, until)?;
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
    check_deadline(host.clock().now(), until)?;
    #[cfg(test)]
    host.checkpoint(RecoveryBoundary::BeforeFinish, &result.run, until)?;
    with_database_deadline(conn, until, |conn| {
        check_snapshot(conn, &result.run, pinned)
    })?;
    let clock_handle = host.clock();
    descriptor.remove(
        conn,
        &result.run,
        &lock,
        until,
        clock_handle.as_ref(),
        |path, deadline| host.unlink_descriptor(path, deadline),
    )?;
    result.run = with_database_deadline(conn, until, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        check_snapshot(&tx, &result.run, pinned)?;
        lock.validate(&tx, &result.run.owner())?;
        storage::validate_owner(&tx, &result.run.owner())?;
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2),cleanup_diagnostics_json=?3 WHERE run_id=?4 AND owner_epoch=?5",
            params![target.as_str(),host.clock().wall_stamp(),serde_json::to_string(&result.report.cleanup_diagnostics).map_err(io_error)?,result.run.run_id,result.run.owner_epoch])?;
        let current = storage::get_run(&tx, result.run.run_id)?;
        check_deadline(host.clock().now(), until)?;
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
            let launch = LaunchDescriptor::new(&case.fixture.run, case.fixture.spec()).unwrap();
            case.artifacts.write_descriptor(&launch).unwrap();
            let descriptor = case.artifacts.paths.descriptor.clone();
            let identity = metadata(&descriptor);
            case.release();
            let path = if directory {
                let retained = case.fixture._dir.path().join("retained-runtime");
                std::fs::rename(&case.artifacts.paths.run_dir, &retained).unwrap();
                retained.join(descriptor.file_name().unwrap())
            } else {
                std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
                descriptor
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
        fn recover_missing(case: &mut Case) -> TestResult<RecoveryReport> {
            let mut host = new_host();
            let result = recover_member_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.member_id,
                host.deadline,
                &mut host,
            );
            assert!(!host.calls.iter().any(|name| name == "cleanup"));
            result
        }
        fn assert_terminal_bookkeeping(
            case: &Case,
            report: &RecoveryReport,
            before: &RunRecord,
            status: RunStatus,
            prior: &[TableSnapshot],
            artifact: &ArtifactWitness,
        ) {
            let current = storage::get_run(&case.fixture.conn, before.run_id).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, status);
            assert_eq!(current.status, status);
            assert_eq!(current.owner_epoch, before.owner_epoch + 1);
            assert_eq!(current.owner_kind, OwnerKind::Reconciler);
            let process = ProcessIdentity::current().unwrap();
            assert_eq!(current.owner_pid, process.pid);
            assert_eq!(current.owner_start_identity, process.start_identity);
            assert_eq!(current.exit_code, before.exit_code);
            assert_eq!(current.cleanup_state, CleanupState::Complete);
            assert_eq!(
                current.recovery_origin_status,
                before.recovery_origin_status.or(Some(before.status))
            );
            assert_eq!(retained_history(&case.fixture.conn), prior);
            assert_eq!(
                metadata(&artifact.path),
                artifact.identity,
                "database-only bookkeeping preserves surviving runtime artifact identity and metadata"
            );
            let diagnostics = current
                .cleanup_diagnostics_json
                .as_ref()
                .unwrap()
                .to_string();
            assert!(diagnostics.contains("runtime_path_missing"));
        }

        fn original_history_cases(status: RunStatus) {
            for directory in [false, true] {
                let mut case = original_terminal(status);
                let before = case.fixture.run.clone();
                let prior = retained_history(&case.fixture.conn);
                let artifact = lose_artifacts(&mut case, directory);
                let report = recover_missing(&mut case).unwrap();
                assert_terminal_bookkeeping(&case, &report, &before, status, &prior, &artifact);
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
            original_history_cases(RunStatus::Exited);
            original_history_cases(RunStatus::Failed);
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
                let report = recover_missing(&mut case).unwrap();
                assert_terminal_bookkeeping(
                    &case,
                    &report,
                    &before,
                    RunStatus::Exited,
                    &prior,
                    &artifact,
                );
                assert_eq!(events(&case).len(), 1);
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
                let result = recover_missing(&mut case);
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
                    let report = recover_missing(&mut case).unwrap();
                    assert_terminal_bookkeeping(
                        &case,
                        &report,
                        &before_run,
                        RunStatus::Failed,
                        &prior,
                        &artifact,
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
            assert!(!case.artifacts.paths.owner_lock.exists());
        }

        #[test]
        fn runner_recovery_of_a_released_runner_closes_its_interaction_and_records_one_runner_lost()
        {
            let mut case = Case::new();
            persist_opening(&mut case);
            case.release();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(
                matches!(report.outcome, RecoveryOutcome::Recovered),
                "{:?}",
                report.outcome
            );
            assert_eq!(report.status, RunStatus::Lost);
            let kinds = |case: &Case| {
                events(case)
                    .iter()
                    .map(|row| row.event.kind)
                    .collect::<Vec<_>>()
            };
            let expected = [
                RuntimeEventKind::InteractionOpened,
                RuntimeEventKind::InteractionClosed,
                RuntimeEventKind::RunnerLost,
            ];
            assert_eq!(kinds(&case), expected);
            assert_eq!(
                events(&case)[1].interaction.as_ref().unwrap().close_reason,
                Some(InteractionCloseReason::MonitorLost)
            );
            let _ = case.recover(&mut new_host());
            assert_eq!(kinds(&case), expected, "a second pass records nothing new");
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
                stop_cause: Some("runner_stopped"),
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
                case.recover(&mut new_host()).unwrap();
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
        fn runner_recovery_missing_path_faults_rollback_owner_provenance_counters_episode_and_events_together()
         {
            for fault in [
                "owner",
                "provenance",
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
                let result = case.recover(&mut host);
                case.fixture.conn.commit_hook(None::<fn() -> bool>).unwrap();
                assert!(result.is_err(), "{fault}");
                assert_eq!(rows(&case.fixture.conn), before, "{fault}");
            }
        }

        #[test]
        fn runner_recovery_missing_path_revalidates_epoch_after_obtaining_the_actual_write_lock() {
            use std::sync::mpsc;
            let mut case = Case::new();
            persist_opening(&mut case);
            missing_lock(&mut case);
            let original = case.fixture.run.clone();
            let run_id = original.run_id;
            let path = case.fixture.conn.path().unwrap().to_owned();
            let (start_tx, start_rx) = mpsc::sync_channel(1);
            let (held_tx, held_rx) = mpsc::sync_channel(1);
            let mut start_tx = Some(start_tx);
            let mut host = new_host();
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
                let result = case.recover(&mut host);
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
        fn runner_recovery_missing_path_zero_event_takeover_refuses_actual_member_fleet_change_atomically()
         {
            refusal(Stage::MissingPathTakeover);
        }
        #[test]
        fn runner_recovery_valid_historical_missing_path_zero_event_bookkeeping_keeps_history() {
            historical_control(Stage::MissingPathTakeover);
        }
    }

    use std::io::Write;

    use std::sync::Arc;

    mod descriptor {
        use crate::runtime::recovery::test_support::descriptor::*;
        use crate::runtime::recovery::tests::*;
        #[test]
        fn runner_recovery_verified_recovery_removes_its_private_descriptor() {
            let mut case = described_run();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert_eq!(report.status, RunStatus::Lost);
            secret_free(&serde_json::to_string(&report.cleanup_diagnostics).unwrap());
            assert!(
                !case.artifacts.paths.descriptor.exists(),
                "successful creator cleanup removes its launch descriptor"
            );
        }

        #[test]
        fn runner_recovery_historical_descriptor_survives_reconciler_retries_until_verified_removal()
         {
            let mut case = described_run();
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
            assert_eq!(events(&case).len(), 1);
            assert!(
                !case.artifacts.paths.descriptor.exists(),
                "a positive historical epoch remains removable after legitimate takeovers"
            );
        }

        #[test]
        fn runner_recovery_foreign_epoch_before_unlink_preserves_descriptor() {
            let mut case = described_run();
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
                let mut case = described_run();
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
        fn runner_recovery_missing_path_database_only_reconciliation_preserves_suspect_descriptor()
        {
            let mut case = described_run();
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
            let mut case = described_run();
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
            let mut case = described_run();
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

    mod finalization_relationships {
        use crate::runtime::recovery::test_support::relationships::*;
        #[test]
        fn runner_recovery_finalization_refuses_actual_member_fleet_change_atomically() {
            refusal(Stage::Finalization);
        }
        #[test]
        fn runner_recovery_valid_historical_lifecycle_change_allows_finalization() {
            historical_control(Stage::Finalization);
        }
    }

    mod identity {
        use crate::runtime::recovery::tests::*;

        #[test]
        fn runner_recovery_creator_free_precreated_lock_is_live_or_unreachable() {
            let mut case = Case::creator();
            let before = rows(&case.fixture.conn);
            let report = case.recover(&mut new_host()).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
            assert_eq!(rows(&case.fixture.conn), before);
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
                        &expected,
                        &placement,
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
            mutable.cleanup_diagnostics_json = Some(json!(["old bounded diagnostic"]));
            assert_eq!(stable_run_identity(&mutable), stable);
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
                        &expected,
                        &case.placement,
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
                    until,
                    &mut host,
                )
                .unwrap()
            };
            assert!(matches!(report.outcome, RecoveryOutcome::Recovered));
            assert!(host.clock.as_ref().unwrap().observed.is_some());
        }

        #[test]
        fn runner_recovery_standalone_cleanup_uses_original_entry_clock_after_wait() {
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
        use crate::runtime::recovery::test_support::descriptor::recover_released_with_host;
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
        "UPDATE member_runs SET status=?1,cleanup_state='complete',exit_code=?2,ended_at=?3,cleanup_diagnostics_json=?4,owner_epoch=owner_epoch+?5 WHERE run_id=?6",
        rusqlite::params![status.as_str(), if status == RunStatus::Exited { Some(7) } else { None }, NOW, json!([{"kind":"same_owner_completion","status":status.as_str()}]).to_string(), i64::from(foreign_owner), run_id],
    ).unwrap(), 1);
            rows(conn)
        }

        fn prepare(missing_path: bool) -> Case {
            let mut case = Case::new();
            case.release();
            if missing_path {
                std::fs::remove_file(&case.artifacts.paths.owner_lock).unwrap();
            }
            case
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
            assert_eq!(
                &rows(&case.fixture.conn),
                expected.lock().unwrap().as_ref().unwrap(),
                "same-owner terminal completion remains immutable"
            );
            assert!(
                !host
                    .calls
                    .iter()
                    .any(|name| matches!(name.as_str(), "after_takeover" | "cleanup"))
            );
            let run = storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
            assert_eq!(run.owner(), case.fixture.run.owner());
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

        fn immediate_completion(missing_path: bool, foreign_owner: bool, status: RunStatus) {
            let mut case = prepare(missing_path);
            let mut inherited_lock = (foreign_owner && !missing_path).then(|| {
                RunLock::acquire(&case.fixture.conn, &case.fixture.run.owner())
                    .unwrap()
                    .unwrap()
            });
            let mut initial_snapshot_seen = false;
            let expected: CompletedSnapshot = Arc::new(Mutex::new(None));
            let published = expected.clone();
            let path = case.fixture.conn.path().unwrap().to_owned();
            let mut host = new_host();
            host.action = Some(Box::new(move |boundary, run, _| {
                if boundary == "after_snapshot" && inherited_lock.is_some() {
                    if initial_snapshot_seen {
                        inherited_lock.take();
                    }
                    initial_snapshot_seen = true;
                }
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
            let result = recover_released_with_host(&mut case, &mut host);
            if foreign_owner {
                assert!(
                    result.is_err(),
                    "a terminal shortcut still validates the fixed owner"
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
        fn runner_recovery_same_owner_completion_before_missing_path_takeover_has_zero_effects() {
            immediate_completion(true, false, RunStatus::Failed);
        }

        #[test]
        fn runner_recovery_completed_foreign_owner_still_rejects_the_fixed_snapshot() {
            immediate_completion(false, true, RunStatus::Lost);
            immediate_completion(true, true, RunStatus::Exited);
        }

        fn completion_behind_write_lock(missing_path: bool, status: RunStatus) {
            let mut case = prepare(missing_path);
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
                let result = case.recover(&mut host);
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
        fn runner_recovery_missing_path_completion_is_rechecked_after_actual_write_lock_wait() {
            completion_behind_write_lock(true, RunStatus::Lost);
        }
    }

    mod missing_paths {
        use crate::runtime::recovery::tests::*;
        #[test]
        fn runner_recovery_missing_path_unknown_identity_preserves_open_episode_as_live_or_unreachable()
         {
            let mut case = Case::new();
            persist_opening(&mut case);
            missing_lock(&mut case);
            let before = rows(&case.fixture.conn);
            let mut host = new_host();
            host.process = Evidence::Unknown;
            let report = case.recover(&mut host).unwrap();
            assert!(matches!(report.outcome, RecoveryOutcome::LiveOrUnreachable));
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
            assert!(!host.calls.iter().any(|name| name == "cleanup"));
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
        fn runner_recovery_positive_pid_lock_or_strict_health_independently_keeps_the_owner_live() {
            for witness in ["pid", "lock", "health"] {
                let mut case = Case::new();
                persist_opening(&mut case);
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
                delivery_method: (state != "ready").then_some("profile"),
                ..InputRow::default()
            }
            .insert(case)
        }

        #[test]
        fn runner_recovery_settles_a_deferred_ready_receipt_not_submitted() {
            let mut case = Case::new();
            let deferred = insert(&case, "prompt:deferred", "ready", true);
            let attempted = insert(&case, "prompt:attempted", "in_progress", false);
            let attempted_before = storage::get_input(&case.fixture.conn, attempted).unwrap();
            let report = case.recover(&mut new_host()).unwrap();
            assert!(
                matches!(report.outcome, RecoveryOutcome::Recovered),
                "{:?}",
                report.outcome
            );
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

    mod native_process_evidence {
        use crate::runtime::recovery::NativeRecoveryHost;
        use crate::runtime::recovery::ProcessEvidence;
        use crate::runtime::recovery::RecoveryHost;
        use crate::runtime::recovery::tests::*;
        use crate::runtime::test_support::zombie;

        fn evidence(process: &ProcessIdentity) -> ProcessEvidence {
            NativeRecoveryHost
                .process_evidence(process, Instant::now() + Duration::from_secs(5))
                .unwrap()
        }

        #[test]
        fn a_running_owner_with_its_recorded_identity_is_alive() {
            let observed = evidence(&ProcessIdentity::current().unwrap());
            assert!(matches!(observed, ProcessEvidence::Alive), "{observed:?}");
        }

        #[test]
        fn an_unreaped_zombie_owner_is_terminated() {
            let (mut child, identity) = zombie();
            let observed = evidence(&identity);
            child.wait().unwrap();
            assert!(
                matches!(observed, ProcessEvidence::Terminated),
                "{observed:?}"
            );
        }

        #[test]
        fn a_reaped_owner_is_terminated() {
            let (mut child, identity) = zombie();
            child.wait().unwrap();
            let observed = evidence(&identity);
            assert!(
                matches!(observed, ProcessEvidence::Terminated),
                "{observed:?}"
            );
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
            .placement;
            Self {
                fixture,
                artifacts,
                held: Some(held),
                settings,
                placement,
            }
        }

        pub(crate) fn seed(&mut self, transitions: &[ObservedTransition]) {
            let owner = self.fixture.run.owner();
            for transition in transitions {
                storage::persist_transition(
                    &mut self.fixture.conn,
                    &owner,
                    transition,
                    PersistenceMode::Live,
                )
                .unwrap();
            }
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
            self.release();
            recover_member_with_host(
                &mut self.fixture.conn,
                &self.settings,
                self.fixture.run.member_id,
                host.deadline,
                host,
            )
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
                expected,
                &self.placement,
                host.deadline,
                held,
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
                Self::Unknown => ProcessEvidence::Unknown,
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
        pub(crate) observed: Option<ShutdownDeadlines>,
    }
    pub(crate) struct UnlinkProbe {
        pub(crate) path: std::path::PathBuf,
        pub(crate) calls: usize,
        pub(crate) fail: bool,
    }
    pub(crate) struct Host {
        pub(crate) deadline: Instant,
        pub(crate) process: Evidence,
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
        fn assert_entry_clock(&mut self, clock: ShutdownDeadlines) {
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
        fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
            std::sync::Arc::new(crate::time::SystemClock)
        }
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
        fn health(&mut self, run: &RunRecord, deadline: Instant) -> TestResult<RecoveryHealth> {
            self.check(deadline)?;
            self.calls.push("health".into());
            if self.matching_health {
                let snapshot = json!({
                    "identity": ControlIdentity::from_run(run),
                    "runner": {
                        "pid": run.runner_pid.unwrap(),
                        "start_identity": run.runner_start_identity.clone().unwrap(),
                    },
                    "child": null,
                    "output_seq": 1,
                });
                HealthSnapshot::decode(snapshot, run)?;
                Ok(RecoveryHealth::Matching)
            } else {
                Ok(RecoveryHealth::Unavailable(
                    "bounded injected endpoint unavailable".into(),
                ))
            }
        }
        fn cleanup_owned(
            &mut self,
            _: &RunRecord,
            clock: ShutdownDeadlines,
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
                RecoveryBoundary::AfterCommit => "after_commit",
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
                return crate::runtime::recovery::unlink_descriptor_file(
                    path,
                    deadline,
                    &crate::time::SystemClock,
                );
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
        crate::broker::runtime::run_evidence(
            run,
            "recovery-fixture",
            u64::try_from(sequence).unwrap(),
        )
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
        pub(crate) stop_cause: Option<&'a str>,
        pub(crate) deferred: bool,
    }

    impl InputRow<'_> {
        pub(crate) fn insert(&self, case: &Case) -> i64 {
            case.fixture
                .conn
                .execute(
                    "INSERT INTO member_input_operations(run_id,operation_key,kind,payload_digest,state,disposition,delivery_reason,delivery_method,stop_cause,created_at,updated_at,deferred_at,defer_reason) VALUES(?1,?2,'prompt',?3,?4,?5,?6,?7,?8,?9,?9,?10,?11)",
                    rusqlite::params![
                        case.fixture.run.run_id,
                        self.key,
                        "d".repeat(64),
                        self.state,
                        self.disposition,
                        self.delivery_reason,
                        self.delivery_method,
                        self.stop_cause,
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
            let current = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            assert_eq!(current.owner_epoch, original.owner_epoch + 1);
            assert_eq!(current.owner_kind, OwnerKind::Reconciler);
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
            assert_eq!(
                events
                    .iter()
                    .map(|row| row.event.event_seq)
                    .collect::<Vec<_>>(),
                [1, 2, 3]
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

        pub(crate) fn described_run() -> Case {
            let case = Case::new();
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
            recover_released_with_host(case, &mut new_host())
        }

        pub(crate) fn recover_released_with_host(
            case: &mut Case,
            host: &mut Host,
        ) -> TestResult<RecoveryReport> {
            let until = host
                .deadline
                .min(Instant::now() + crate::pane_lock::PaneLock::RELEASED_LOCK_GRACE);
            loop {
                let result = case.recover(host);
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
            MissingPathTakeover,
            Finalization,
        }
        impl Stage {
            pub(crate) fn boundary(self) -> &'static str {
                match self {
                    Self::OrdinaryTakeover | Self::MissingPathTakeover => "before_takeover",
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
                Stage::MissingPathTakeover => {
                    let mut case = established_terminal();
                    missing_lock(&mut case);
                    case
                }
                Stage::Finalization => established_terminal(),
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
                if matches!(stage, Stage::Finalization) {
                    assert_eq!(run.status, RunStatus::Recovering);
                    assert_eq!(run.recovery_origin_status, Some(RunStatus::Exited));
                } else {
                    assert_eq!(run.status, RunStatus::Exited);
                }
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
                assert!(
                    !host
                        .calls
                        .iter()
                        .any(|call| matches!(call.as_str(), "after_takeover" | "cleanup"))
                );
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
                        "UPDATE members SET status='deregistered' WHERE member_id=?1",
                        [if matches!(self, Self::MemberInactive) {
                            run.member_id
                        } else {
                            run.director_member_id
                        }],
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
            let run = storage::get_run(&case.fixture.conn, original.run_id).unwrap();
            assert_eq!(run.owner_epoch, original.owner_epoch + 1);
            assert_eq!(run.owner_kind, OwnerKind::Reconciler);
            assert_eq!(run.status, RunStatus::Exited);
            assert_eq!(run.exit_code, Some(7));
            assert_eq!(run.cleanup_state, CleanupState::Complete);
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
            if matches!(stage, Stage::MissingPathTakeover) {
                assert!(!host.calls.iter().any(|call| call == "cleanup"));
            }
        }
    }
}
