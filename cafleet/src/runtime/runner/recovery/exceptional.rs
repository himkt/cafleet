use super::*;

pub(super) fn reconcile(
    conn: &mut Connection,
    expected: &RunRecord,
    pinned: Option<&Placement>,
    process: &ProcessIdentity,
    attestation: Option<&OwnerTerminationAttestation>,
    until: Instant,
    check: impl FnOnce(&Connection) -> Result<()>,
) -> Result<RecoveryCompletion> {
    with_database_deadline(conn, until, |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        check(&tx)?;
        let current = check_snapshot(&tx, expected, pinned)?;
        validate_attestation(&current, attestation)?;
        if current.status.terminal() && current.cleanup_state == CleanupState::Complete {
            check_deadline(Instant::now(), until)?;
            tx.commit().map_err(db_err)?;
            return completion(current, RecoveryOutcome::Recovered);
        }
        let established = super::replay::terminal_outcome(&tx, &current)?;
        let mut run = advance_recovery_owner(&tx, &current, process)?;
        let mut report = completion(run.clone(), RecoveryOutcome::Recovering)?.report;
        report
            .cleanup_diagnostics
            .push(json!("runtime_path_missing"));
        if let Some(claim) = attestation {
            report.cleanup_diagnostics.push(json!({"kind":"owner_termination_attestation",
                "run_id":current.run_id,"owner_epoch":current.owner_epoch,"owner_pid":current.owner_pid,
                "owner_start_identity":current.owner_start_identity,"evidence_ref":claim.evidence_ref,
                "recorded_at":crate::time::format_utc(crate::time::now_utc()),"source":"operator_via_director"}));
        }
        tx.execute("UPDATE member_runs SET observation_gap_count=observation_gap_count+1,cleanup_diagnostics_json=?1 WHERE run_id=?2",
            params![serde_json::to_string(&report.cleanup_diagnostics).map_err(io_error)?,run.run_id]).map_err(db_err)?;
        let target = match established {
            Some(status) => status,
            None => {
                let maximum: Option<i64> = tx
                    .query_row(
                        "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
                        [run.run_id],
                        |row| row.get(0),
                    )
                    .map_err(db_err)?;
                let ceiling = maximum.map_or(run.event_seq_reserved_through, |value| {
                    value.max(run.event_seq_reserved_through)
                });
                let first = ceiling
                    .checked_add(1)
                    .ok_or_else(|| error("recovery sequence overflow"))?;
                let reserved = ceiling
                    .checked_add(256)
                    .ok_or_else(|| error("recovery sequence overflow"))?;
                tx.execute(
                    "UPDATE member_runs SET event_seq_reserved_through=?1 WHERE run_id=?2",
                    params![reserved, run.run_id],
                )
                .map_err(db_err)?;
                run = storage::get_run(&tx, run.run_id)?;
                for transition in super::replay::loss_transitions(&tx, &run, first, true)? {
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
                    super::replay::count_exceptional(&mut report, &event)?;
                }
                RunStatus::Lost
            }
        };
        tx.execute("UPDATE member_runs SET status=?1,cleanup_state='complete',ended_at=COALESCE(ended_at,?2) WHERE run_id=?3",
            params![target.as_str(),crate::time::format_utc(crate::time::now_utc()),run.run_id]).map_err(db_err)?;
        let run = storage::get_run(&tx, run.run_id)?;
        check_deadline(Instant::now(), until)?;
        tx.commit().map_err(db_err)?;
        report.outcome = RecoveryOutcome::Recovered;
        report.status = run.status;
        Ok(RecoveryCompletion { report, run })
    })
}
