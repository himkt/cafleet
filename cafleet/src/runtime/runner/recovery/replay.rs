use super::*;

pub(super) fn reconcile(
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
fn count(
    result: &mut RecoveryCompletion,
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
        increment(&mut result.report.replayed_count)?;
    }
    match event.notification_state {
        NotificationState::Inbox => increment(&mut result.report.inbox_count),
        NotificationState::Archived => increment(&mut result.report.archived_count),
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
        Some(observation) => storage::persist_transition_with_effective_policy_counted(
            conn,
            &result.run.owner(),
            transition,
            observation,
            PersistenceMode::Recovery,
        ),
        None => storage::persist_transition_counted(
            conn,
            &result.run.owner(),
            transition,
            PersistenceMode::Recovery,
        ),
    })?;
    if inserted {
        count(result, &event, original)?;
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
    let now = crate::time::format_utc(crate::time::now_utc());
    let mut transitions = Vec::new();
    if let Some(interaction_id) = snapshot.current_interaction_id {
        let interaction = storage::get_event(conn, conn.query_row(
            "SELECT event_id FROM member_runtime_events WHERE interaction_id=?1 ORDER BY event_seq LIMIT 1",
            [interaction_id], |row| row.get::<_, i64>(0)).map_err(db_err)?)?.interaction
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
    let evidence = storage::evidence::EvidencePolicy::metadata().apply(&json!({
        "backend":run.backend,"native_version":run.native_version,"profile_version":run.profile_version,
        "rule_id":"runtime-recovery","screen_seq":0,"operation":null,"choices":null,"native_request_id":null,
        "excerpt":null,"truncated":false,"sensitive":false
    }))?;
    transitions.push(ObservedTransition {
        run_id: run.run_id,
        event_seq: next,
        episode_seq: None,
        kind: RuntimeEventKind::RunnerLost,
        classification: None,
        observed_at: now,
        evidence_json: evidence,
        close_reason: None,
        cause_json: Some(json!({"kind":if missing {"runtime_path_missing"} else {"monitor_lost"}})),
    });
    Ok(transitions)
}
pub(super) fn terminal_outcome(conn: &Connection, run: &RunRecord) -> Result<Option<RunStatus>> {
    let terminal: Option<String> = conn.query_row(
        "SELECT kind FROM member_runtime_events WHERE run_id=?1 AND kind IN ('runner_exited','runner_lost') ORDER BY event_seq DESC LIMIT 1",
        [run.run_id], |row| row.get(0)).optional().map_err(db_err)?;
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
    let mut outbox = Outbox::open_recovery(conn, &result.run, lock.clone(), until)?;
    let scan = outbox.scan_recovery(conn, until)?;
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
                outbox.append_recovery(conn, &transition, until)?;
                persist(conn, result, &transition, None, false, host, until)?;
            }
            RunStatus::Lost
        }
    };
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
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        check_snapshot(&tx, &result.run, pinned)?;
        lock.validate(&tx, &result.run.owner())?;
        storage::validate_owner(&tx, &result.run.owner())?;
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2),cleanup_diagnostics_json=?3 WHERE run_id=?4 AND owner_epoch=?5",
            params![target.as_str(),crate::time::format_utc(crate::time::now_utc()),serde_json::to_string(&result.report.cleanup_diagnostics).map_err(io_error)?,result.run.run_id,result.run.owner_epoch]).map_err(db_err)?;
        let current = storage::get_run(&tx, result.run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit().map_err(db_err)?;
        Ok(current)
    })?;
    result.report.status = result.run.status;
    result.report.outcome = RecoveryOutcome::Recovered;
    Ok(())
}

pub(super) fn count_exceptional(
    report: &mut RecoveryReport,
    event: &RuntimeEventRecord,
) -> Result<()> {
    let counter = match event.notification_state {
        NotificationState::Inbox => &mut report.inbox_count,
        NotificationState::Archived => &mut report.archived_count,
    };
    *counter = counter
        .checked_add(1)
        .ok_or_else(|| error("recovery commit count overflow"))?;
    Ok(())
}
