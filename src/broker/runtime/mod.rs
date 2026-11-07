mod events;
pub(crate) mod evidence;
mod inputs;

use crate::broker::MemberStatus;
pub(crate) use crate::broker::runtime::events::original as original_transition;
pub(crate) use crate::broker::runtime::events::validate_transition as validate_observed_transition;
pub(crate) use crate::broker::runtime::events::*;
pub(crate) use crate::broker::runtime::inputs::*;
use crate::broker::stored_enum;
use crate::coding_agent::catalog::AnswerKey;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::time::Deadline;
use crate::time::now_stamp;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::ops::RangeInclusive;
use std::os::unix::fs::MetadataExt;

pub(super) fn invalid(field: &str, description: impl Into<String>) -> CafleetError {
    CafleetError::InvalidStoredValue {
        field: field.into(),
        value: description.into(),
    }
}

pub(super) fn require(condition: bool, field: &str, description: &str) -> Result<(), CafleetError> {
    if condition {
        Ok(())
    } else {
        Err(invalid(field, description))
    }
}

pub(super) fn nonblank(value: &str, field: &str) -> Result<(), CafleetError> {
    require(!value.trim().is_empty(), field, "required nonempty value")
}

pub(crate) fn hex(value: &str, length: usize, field: &str) -> Result<(), CafleetError> {
    require(
        value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        field,
        "invalid hexadecimal identity",
    )
}

pub(super) fn json_column(
    row: &rusqlite::Row<'_>,
    field: &str,
    array: bool,
) -> rusqlite::Result<Value> {
    let text: String = row.get(field)?;
    let decoded = serde_json::from_str::<Value>(&text).ok().filter(|value| {
        if array {
            value.is_array()
        } else {
            value.is_object()
        }
    });
    decoded.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            row.as_ref()
                .column_index(field)
                .expect("a selected field has an index"),
            rusqlite::types::Type::Text,
            Box::new(invalid(field, "invalid JSON shape")),
        )
    })
}

fn optional_json(
    row: &rusqlite::Row<'_>,
    field: &str,
    array: bool,
) -> rusqlite::Result<Option<Value>> {
    let present: Option<String> = row.get(field)?;
    present.map(|_| json_column(row, field, array)).transpose()
}

pub(crate) fn database_identity(conn: &Connection) -> Result<Value, CafleetError> {
    let path = conn
        .path()
        .ok_or_else(|| invalid("database_identity", "file-backed database required"))?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| CafleetError::App(format!("cannot identify runtime database: {error}")))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| CafleetError::App(format!("cannot inspect runtime database: {error}")))?;
    let instance: String = conn.query_row(
        "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    hex(&instance, 32, "runtime_database_identity.instance_id")?;
    let canonical = canonical
        .to_str()
        .ok_or_else(|| invalid("database_identity.canonical_path", "UTF-8 path required"))?;
    Ok(
        json!({"canonical_path":canonical,"instance_id":instance,"device":metadata.dev(),"inode":metadata.ino()}),
    )
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        run_id: row.get("run_id")?,
        member_id: row.get("member_id")?,
        fleet_id: row.get("fleet_id")?,
        director_member_id: row.get("director_member_id")?,
        backend: row.get("backend")?,
        database_identity_json: json_column(row, "database_identity_json", false)?,
        launch_nonce: row.get("launch_nonce")?,
        runtime_path: row.get("runtime_path")?,
        requested_mode: row.get("requested_mode")?,
        resolved_mode: row.get("resolved_mode")?,
        mode_source: row.get("mode_source")?,
        permission_argv_json: json_column(row, "permission_argv_json", true)?,
        event_evidence: row.get("event_evidence")?,
        coverage: row.get("coverage")?,
        status: row.get("status")?,
        created_at: row.get("created_at")?,
        native_version: row.get("native_version")?,
        effective_policy_json: optional_json(row, "effective_policy_json", false)?,
        owner_epoch: row.get("owner_epoch")?,
        owner_kind: row.get("owner_kind")?,
        owner_pid: row.get("owner_pid")?,
        owner_start_identity: row.get("owner_start_identity")?,
        reachability: row.get("reachability")?,
        cleanup_state: row.get("cleanup_state")?,
        event_seq_reserved_through: row.get("event_seq_reserved_through")?,
        observation_gap_count: row.get("observation_gap_count")?,
        runtime_dir_identity_json: optional_json(row, "runtime_dir_identity_json", false)?,
        run_lock_identity_json: optional_json(row, "run_lock_identity_json", false)?,
        runner_pid: row.get("runner_pid")?,
        runner_start_identity: row.get("runner_start_identity")?,
        child_pid: row.get("child_pid")?,
        child_start_identity: row.get("child_start_identity")?,
        started_at: row.get("started_at")?,
        last_health_at: row.get("last_health_at")?,
        ended_at: row.get("ended_at")?,
        exit_code: row.get("exit_code")?,
        exit_signal: row.get("exit_signal")?,
        cleanup_diagnostics_json: optional_json(row, "cleanup_diagnostics_json", true)?,
        recovery_origin_status: row.get("recovery_origin_status")?,
    })
}

pub(crate) fn get_run(conn: &Connection, run_id: i64) -> Result<RunRecord, CafleetError> {
    let run = conn
        .query_row(
            "SELECT * FROM member_runs WHERE run_id=?1",
            [run_id],
            map_run,
        )
        .optional()?
        .ok_or_else(|| CafleetError::App(format!("Run {run_id} not found")))?;
    nonblank(
        &run.owner_start_identity,
        "member_runs.owner_start_identity",
    )?;
    nonblank(&run.resolved_mode, "member_runs.resolved_mode")?;
    require(
        crate::coding_agent::coding_agent(&run.backend).is_some(),
        "member_runs.backend",
        "unknown backend",
    )?;
    require(
        run.owner_epoch > 0 && run.owner_pid > 0,
        "member_runs.owner_epoch",
        "positive owner identity required",
    )?;
    require(
        run.event_seq_reserved_through >= 0 && run.observation_gap_count >= 0,
        "member_runs.event_seq_reserved_through",
        "nonnegative counters required",
    )?;
    validate_identity(&run.database_identity_json)?;
    hex(&run.launch_nonce, 64, "member_runs.launch_nonce")?;
    require(
        run.permission_argv_json
            .as_array()
            .expect("decoded array")
            .iter()
            .all(Value::is_string),
        "member_runs.permission_argv_json",
        "string arguments required",
    )?;
    events::validate(&run)?;
    Ok(run)
}

pub(crate) fn validate_identity(identity: &Value) -> Result<(), CafleetError> {
    let object = identity
        .as_object()
        .ok_or_else(|| invalid("database_identity_json", "object required"))?;
    let path = object
        .get("canonical_path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("database_identity_json", "canonical_path required"))?;
    require(
        std::path::Path::new(path).is_absolute(),
        "database_identity_json",
        "absolute canonical path required",
    )?;
    let instance = object
        .get("instance_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("database_identity_json", "instance_id required"))?;
    hex(instance, 32, "database_identity_json")?;
    for field in ["device", "inode"] {
        require(
            object.get(field).and_then(Value::as_u64).is_some(),
            "database_identity_json",
            "filesystem identity required",
        )?;
    }
    Ok(())
}

pub(super) fn historical_ownership(
    conn: &Connection,
    run: &RunRecord,
) -> Result<Option<ArchiveReason>, CafleetError> {
    let present = |member_id| {
        crate::broker::members::member_fleet_status(conn, member_id)?
            .ok_or(CafleetError::from(rusqlite::Error::QueryReturnedNoRows))
    };
    let member = present(run.member_id)?;
    let director = present(run.director_member_id)?;
    let fleet = crate::broker::fleets::get_fleet(conn, run.fleet_id)?
        .ok_or_else(|| invalid("member_runs.fleet_id", "fleet missing"))?;
    require(
        member.0 == run.fleet_id
            && director.0 == run.fleet_id
            && fleet.director_member_id == Some(run.director_member_id)
            && run.member_id != run.director_member_id,
        "member_runs.ownership",
        "member/fleet/Director mismatch",
    )?;
    Ok(if fleet.deleted_at.is_some() {
        Some(ArchiveReason::FleetDeleted)
    } else if member.1 != MemberStatus::Active {
        Some(ArchiveReason::MemberInactive)
    } else if director.1 != MemberStatus::Active {
        Some(ArchiveReason::DirectorInactive)
    } else {
        None
    })
}

pub(crate) fn validate_owner(
    conn: &Connection,
    owner: &RunOwner,
) -> Result<RunRecord, CafleetError> {
    let run = get_run(conn, owner.run_id)?;
    require(
        run.owner() == *owner,
        "member_runs.owner",
        "stale ownership token",
    )?;
    require(
        database_identity(conn)? == run.database_identity_json,
        "member_runs.database_identity_json",
        "database identity changed",
    )?;
    historical_ownership(conn, &run)?;
    Ok(run)
}

pub(super) fn owned_transaction<T>(
    conn: &mut Connection,
    owner: &RunOwner,
    operation: impl FnOnce(&Connection, &RunRecord) -> Result<T, CafleetError>,
) -> Result<T, CafleetError> {
    let deadline = Deadline::inherited_or_new();
    with_database_budget(conn, deadline, |conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = validate_owner(&tx, owner)?;
        let result = operation(&tx, &run)?;
        deadline.remaining()?;
        tx.commit()?;
        Ok(result)
    })
}

pub(crate) fn insert_run(conn: &Connection, new: &NewRun) -> Result<RunRecord, CafleetError> {
    validate_identity(&new.database_identity_json)?;
    nonblank(&new.owner_start_identity, "owner_start_identity")?;
    nonblank(&new.resolved_mode, "resolved_mode")?;
    hex(&new.launch_nonce, 64, "launch_nonce")?;
    require(
        std::path::Path::new(&new.runtime_path).is_absolute(),
        "runtime_path",
        "absolute path required",
    )?;
    require(
        new.permission_argv_json.is_array(),
        "permission_argv_json",
        "array required",
    )?;
    require(
        database_identity(conn)? == new.database_identity_json,
        "database_identity_json",
        "database identity mismatch",
    )?;
    let fleet = crate::broker::fleets::require_live_fleet(conn, new.fleet_id)?;
    require(
        fleet.director_member_id == Some(new.director_member_id)
            && new.member_id != new.director_member_id
            && crate::broker::members::active_member_fleet(conn, new.member_id)?
                == Some(new.fleet_id),
        "member_runs.ownership",
        "member/fleet/Director mismatch",
    )?;
    let now = now_stamp();
    conn.execute(
            "INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,database_identity_json,
             launch_nonce,runtime_path,requested_mode,resolved_mode,mode_source,permission_argv_json,
             event_evidence,coverage,status,created_at,native_version,owner_epoch,
             owner_kind,owner_pid,owner_start_identity,reachability,cleanup_state,event_seq_reserved_through,observation_gap_count)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'starting',?14,?15,1,'creator',?16,?17,'reachable','incomplete',0,0)",
            params![new.member_id,new.fleet_id,new.director_member_id,new.backend,new.database_identity_json.to_string(),
                new.launch_nonce,new.runtime_path,new.requested_mode,new.resolved_mode,new.mode_source.as_str(),
                new.permission_argv_json.to_string(),new.event_evidence.as_str(),new.coverage.as_str(),now,
                new.native_version,new.owner_pid,new.owner_start_identity],
        )?;
    let run = get_run(conn, conn.last_insert_rowid())?;
    Ok(run)
}

pub(crate) fn native_session(
    conn: &Connection,
    run_id: i64,
) -> Result<Option<crate::coding_agent::hermes::NativeSession>, CafleetError> {
    let stored: Option<String> = conn.query_row(
        "SELECT native_session_json FROM member_runs WHERE run_id=?1",
        [run_id],
        |row| row.get(0),
    )?;
    stored
        .map(|text| {
            serde_json::from_str(&text).map_err(|cause| {
                CafleetError::App(format!(
                    "member_runs.native_session_json of run {run_id} is invalid: {cause}"
                ))
            })
        })
        .transpose()
}

pub(crate) fn set_native_session(
    conn: &Connection,
    run_id: i64,
    session: &crate::coding_agent::hermes::NativeSession,
) -> Result<(), CafleetError> {
    let changed = conn.execute(
        "UPDATE member_runs SET native_session_json=?1 WHERE run_id=?2",
        params![
            serde_json::to_string(session).expect("a native session serializes"),
            run_id
        ],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(CafleetError::App(format!("member run {run_id} is missing")))
    }
}

pub(crate) fn reserve_event_sequences(
    conn: &mut Connection,
    owner: &RunOwner,
) -> Result<RangeInclusive<i64>, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        reserve_event_sequences_in_transaction(conn, run, SequenceReservation::CurrentOwnerBlock)
    })
}

pub(crate) enum SequenceReservation {
    CurrentOwnerBlock,
    RecoveryRange { ceiling: i64, count: i64 },
}

pub(crate) fn reserve_event_sequences_in_transaction(
    conn: &Connection,
    run: &RunRecord,
    reservation: SequenceReservation,
) -> Result<RangeInclusive<i64>, CafleetError> {
    assert!(
        !conn.is_autocommit(),
        "sequence reservation requires its caller transaction"
    );
    let (ceiling, count) = match &reservation {
        SequenceReservation::CurrentOwnerBlock => (run.event_seq_reserved_through, 256),
        SequenceReservation::RecoveryRange { ceiling, count } => {
            assert!(
                *count > 0,
                "recovery reservation requires a nonempty result count"
            );
            (*ceiling, *count)
        }
    };
    let overflow = || match &reservation {
        SequenceReservation::CurrentOwnerBlock => {
            invalid("event_seq_reserved_through", "sequence overflow")
        }
        SequenceReservation::RecoveryRange { .. } => {
            CafleetError::App("recovery sequence overflow".into())
        }
    };
    let first = ceiling.checked_add(1).ok_or_else(overflow)?;
    let last = ceiling.checked_add(count).ok_or_else(overflow)?;
    match reservation {
        SequenceReservation::CurrentOwnerBlock => {
            let changed = conn.execute(
                "UPDATE member_runs SET event_seq_reserved_through=?1 WHERE run_id=?2 AND owner_epoch=?3 AND event_seq_reserved_through=?4",
                params![last, run.run_id, run.owner_epoch, run.event_seq_reserved_through],
            )?;
            require(
                changed == 1,
                "member_runs.owner",
                "sequence reservation lost ownership",
            )?;
        }
        SequenceReservation::RecoveryRange { .. } => {
            conn.execute(
                "UPDATE member_runs SET event_seq_reserved_through=?1 WHERE run_id=?2",
                params![last, run.run_id],
            )?;
        }
    }
    Ok(first..=last)
}

pub(crate) fn runtime_snapshot(
    conn: &Connection,
    run_id: i64,
) -> Result<RuntimeSnapshot, CafleetError> {
    let run = get_run(conn, run_id)?;
    let current_interaction_id = conn
        .query_row(
            "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND closed_at IS NULL",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    let mut statement = conn.prepare(&format!(
        "SELECT input_id FROM member_input_operations WHERE run_id=?1 AND {UNRESOLVED_INPUT} ORDER BY input_id"
    ))?;
    let unresolved_input_ids = statement
        .query_map([run_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuntimeSnapshot {
        run,
        current_interaction_id,
        unresolved_input_ids,
    })
}

pub(crate) fn member_runtime(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<RuntimeSnapshot>, CafleetError> {
    let id = conn
        .query_row(
            "SELECT run_id FROM member_runs WHERE member_id=?1 ORDER BY run_id DESC LIMIT 1",
            [member_id],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|id| runtime_snapshot(conn, id)).transpose()
}

stored_enum!(RunStatus, "member_runs.status", {
    Starting => "starting", Prepared => "prepared", Running => "running",
    Degraded => "degraded", Stopping => "stopping", Recovering => "recovering",
    Exited => "exited", Failed => "failed", Lost => "lost"
});
stored_enum!(ModeSource, "member_runs.mode_source", { Default => "default", Explicit => "explicit" });
stored_enum!(OwnerKind, "member_runs.owner_kind", { Creator => "creator", Runner => "runner", Reconciler => "reconciler" });
stored_enum!(Reachability, "member_runs.reachability", { Reachable => "reachable", Suspect => "suspect" });
stored_enum!(CleanupState, "member_runs.cleanup_state", { Complete => "complete", Incomplete => "incomplete" });
stored_enum!(Coverage, "member_runs.coverage", { Tested => "tested", Unknown => "unknown" });
stored_enum!(EventEvidence, "member_runs.event_evidence", { Metadata => "metadata", Redacted => "redacted" });
stored_enum!(AutomaticReview, "effective_policy.automatic_review", { Available => "available", Unavailable => "unavailable" });
stored_enum!(test_values InteractionClass, "member_interactions.classification", {
    PermissionRequest => "permission_request", Question => "question", Trust => "trust",
    Update => "update", UnknownInteraction => "unknown_interaction"
});
stored_enum!(test_values InteractionCloseReason, "member_interactions.close_reason", {
    NotVisible => "not_visible", Superseded => "superseded",
    ProcessExited => "process_exited", MonitorLost => "monitor_lost"
});
stored_enum!(RuntimeEventKind,"member_runtime_events.kind", {
    CoverageUnknown => "coverage_unknown", InteractionOpened => "interaction_opened",
    InteractionObscured => "interaction_obscured", InteractionClosed => "interaction_closed",
    PermissionDeniedObserved => "permission_denied_observed",
    AutomaticRefusalObserved => "automatic_refusal_observed",
    AutomaticReviewUnavailable => "automatic_review_unavailable",
    InputDeliveryResult => "input_delivery_result", RunnerDegraded => "runner_degraded",
    RunnerLost => "runner_lost", RunnerExited => "runner_exited"
});
stored_enum!(RuntimeCauseKind, "cause.kind", {
    CoverageUnknown => "coverage_unknown", NativeExit => "native_exit",
    StoragePressure => "storage_pressure", StorageRecovered => "storage_recovered",
    ObserverError => "observer_error", MonitorLost => "monitor_lost",
    RuntimePathMissing => "runtime_path_missing"
});
stored_enum!(NotificationState, "member_runtime_events.notification_state", { Inbox => "inbox", Archived => "archived" });
stored_enum!(test_values ArchiveReason, "member_runtime_events.archive_reason", {
    MemberInactive => "member_inactive", FleetDeleted => "fleet_deleted", DirectorInactive => "director_inactive"
});
stored_enum!(InputKind, "member_input_operations.kind", { Preview => "preview", Prompt => "prompt", Exec => "exec", PermissionResponse => "permission_response" });
stored_enum!(InputState, "member_input_operations.state", {
    Ready => "ready", InProgress => "in_progress", Accepted => "accepted",
    Uncertain => "uncertain", Settled => "settled"
});
stored_enum!(test_values InputDisposition, "member_input_operations.disposition", {
    Submitted => "submitted", NotSubmitted => "not_submitted", AbandonedUncertain => "abandoned_uncertain"
});
stored_enum!(UncertainCause, "member_input_operations.uncertain_cause", {
    ManualInput => "manual_input", ManualAccountingFailed => "manual_accounting_failed",
    InteractionCandidate => "interaction_candidate", InteractionOpen => "interaction_open",
    ObserverUnavailable => "observer_unavailable", ShortWrite => "short_write",
    WriteFailed => "write_failed", RunnerStopped => "runner_stopped"
});

impl RunStatus {
    pub(crate) fn terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed | Self::Lost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EffectivePolicyObservation {
    pub(crate) automatic_review: AutomaticReview,
    pub(crate) observed_mode: Option<String>,
    pub(crate) rule_id: String,
    pub(crate) observed_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NewRun {
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) backend: String,
    pub(crate) database_identity_json: Value,
    pub(crate) launch_nonce: String,
    pub(crate) runtime_path: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
    pub(crate) event_evidence: EventEvidence,
    pub(crate) native_version: Option<String>,
    pub(crate) coverage: Coverage,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunRecord {
    pub(crate) run_id: i64,
    pub(crate) member_id: i64,
    pub(crate) fleet_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) backend: String,
    pub(crate) database_identity_json: Value,
    pub(crate) launch_nonce: String,
    pub(crate) runtime_path: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
    pub(crate) event_evidence: EventEvidence,
    pub(crate) coverage: Coverage,
    pub(crate) status: RunStatus,
    pub(crate) created_at: String,
    pub(crate) native_version: Option<String>,
    pub(crate) effective_policy_json: Option<Value>,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_kind: OwnerKind,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
    pub(crate) reachability: Reachability,
    pub(crate) cleanup_state: CleanupState,
    pub(crate) event_seq_reserved_through: i64,
    pub(crate) observation_gap_count: i64,
    pub(crate) runtime_dir_identity_json: Option<Value>,
    pub(crate) run_lock_identity_json: Option<Value>,
    pub(crate) runner_pid: Option<i64>,
    pub(crate) runner_start_identity: Option<String>,
    pub(crate) child_pid: Option<i64>,
    pub(crate) child_start_identity: Option<String>,
    pub(crate) started_at: Option<String>,
    pub(crate) last_health_at: Option<String>,
    pub(crate) ended_at: Option<String>,
    pub(crate) exit_code: Option<i64>,
    pub(crate) exit_signal: Option<i64>,
    pub(crate) cleanup_diagnostics_json: Option<Value>,
    pub(crate) recovery_origin_status: Option<RunStatus>,
}

impl RunRecord {
    pub(crate) fn owner(&self) -> RunOwner {
        RunOwner {
            run_id: self.run_id,
            database_identity_json: self.database_identity_json.clone(),
            owner_epoch: self.owner_epoch,
            owner_kind: self.owner_kind,
            owner_pid: self.owner_pid,
            owner_start_identity: self.owner_start_identity.clone(),
        }
    }
    pub(crate) fn owner_process(&self) -> crate::runtime::ProcessIdentity {
        crate::runtime::ProcessIdentity {
            pid: self.owner_pid,
            start_identity: self.owner_start_identity.clone(),
        }
    }
    pub(crate) fn child_process(
        &self,
    ) -> Result<Option<crate::runtime::ProcessIdentity>, CafleetError> {
        match (self.child_pid, &self.child_start_identity) {
            (None, None) => Ok(None),
            (Some(pid), Some(start_identity)) => Ok(Some(crate::runtime::ProcessIdentity {
                pid,
                start_identity: start_identity.clone(),
            })),
            _ => Err(CafleetError::App(
                "recorded child generation is incomplete".into(),
            )),
        }
    }
    pub(crate) fn is_settled(&self) -> bool {
        self.status.terminal() && self.cleanup_state == CleanupState::Complete
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunOwner {
    pub(crate) run_id: i64,
    pub(crate) database_identity_json: Value,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_kind: OwnerKind,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) run: RunRecord,
    pub(crate) current_interaction_id: Option<i64>,
    pub(crate) unresolved_input_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionRecord {
    pub(crate) interaction_id: i64,
    pub(crate) run_id: i64,
    pub(crate) episode_seq: i64,
    pub(crate) classification: InteractionClass,
    pub(crate) evidence_digest: String,
    pub(crate) opened_at: String,
    pub(crate) evidence_json: Value,
    pub(crate) closed_at: Option<String>,
    pub(crate) close_reason: Option<InteractionCloseReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObservedTransition {
    pub(crate) run_id: i64,
    pub(crate) event_seq: i64,
    pub(crate) episode_seq: Option<i64>,
    pub(crate) kind: RuntimeEventKind,
    pub(crate) classification: Option<InteractionClass>,
    pub(crate) observed_at: String,
    pub(crate) evidence_json: Value,
    pub(crate) close_reason: Option<InteractionCloseReason>,
    pub(crate) cause_json: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceMode {
    Live,
    Recovery,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeEventRecord {
    pub(crate) event_id: i64,
    pub(crate) run_id: i64,
    pub(crate) event_seq: i64,
    pub(crate) interaction_id: Option<i64>,
    pub(crate) kind: RuntimeEventKind,
    pub(crate) observed_at: String,
    pub(crate) committed_at: String,
    pub(crate) payload_json: Value,
    pub(crate) notification_state: NotificationState,
    pub(crate) message_id: Option<i64>,
    pub(crate) archive_reason: Option<ArchiveReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RuntimeEventView {
    pub(crate) event: RuntimeEventRecord,
    pub(crate) run: RuntimeSnapshot,
    pub(crate) interaction: Option<InteractionRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewInput {
    pub(crate) operation_key: String,
    pub(crate) kind: InputKind,
    pub(crate) payload_digest: String,
    pub(crate) message_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InputOperation {
    pub(crate) input_id: i64,
    pub(crate) run_id: i64,
    pub(crate) operation_key: String,
    pub(crate) kind: InputKind,
    pub(crate) payload_digest: String,
    pub(crate) state: InputState,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) message_id: Option<i64>,
    pub(crate) disposition: Option<InputDisposition>,
    pub(crate) evidence_ref: Option<String>,
    pub(crate) permission_response: Option<PermissionResponseRecord>,
    pub(crate) delivery_method: Option<DeliveryMethod>,
    pub(crate) delivery_reason: Option<String>,
    pub(crate) deferred_at: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) uncertain_cause: Option<UncertainCause>,
}

stored_enum!(DeliveryStatus, "delivery.status", {
    Submitted => "submitted", Queued => "queued", Held => "held", Uncertain => "uncertain",
    NotSubmitted => "not_submitted", NotApplicable => "not_applicable"
});
stored_enum!(DeliveryMethod, "member_input_operations.delivery_method", {
    Profile => "profile", Fallback => "fallback"
});

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryProjection {
    pub(crate) status: DeliveryStatus,
    pub(crate) method: Option<DeliveryMethod>,
    pub(crate) reason: Option<String>,
}
impl DeliveryProjection {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"status":self.status.as_str(),
            "method":self.method.map(DeliveryMethod::as_str),"reason":self.reason})
    }
}

stored_enum!(values PermissionDecision, "member_permission_responses.decision", { Allow => "allow", Deny => "deny" });
stored_enum!(values DirectorMode, "member_permission_responses.director_mode", { Auto => "auto", Manual => "manual", Unknown => "unknown" });
stored_enum!(values DecisionSource,"member_permission_responses.decision_source", { DirectorAuto => "director-auto", User => "user" });

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseMetadata {
    pub(crate) event_id: i64,
    pub(crate) interaction_id: i64,
    pub(crate) expected_owner_epoch: i64,
    pub(crate) request_token: String,
    pub(crate) key: AnswerKey,
    pub(crate) director_member_id: i64,
    pub(crate) director_mode: DirectorMode,
    pub(crate) decision_source: DecisionSource,
    pub(crate) mode_evidence_ref: String,
    pub(crate) decision_evidence_ref: String,
    pub(crate) answer_rule_id: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseRecord {
    pub(crate) input_id: i64,
    pub(crate) metadata: PermissionResponseMetadata,
    pub(crate) decided_at: String,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionResponseClaim {
    pub(crate) interaction_id: i64,
    pub(crate) input_id: i64,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionReceipt {
    pub(crate) input: InputOperation,
    pub(crate) response: PermissionResponseRecord,
    pub(crate) claim: Option<PermissionResponseClaim>,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PermissionReceiptIdentity {
    pub(crate) database_identity_json: Value,
    pub(crate) run_id: i64,
}
impl PermissionReceiptIdentity {
    pub(crate) fn for_run(run: &RunRecord) -> Self {
        Self {
            database_identity_json: run.database_identity_json.clone(),
            run_id: run.run_id,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewPermissionResponse {
    pub(crate) operation_key: String,
    pub(crate) metadata: PermissionResponseMetadata,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PermissionReservation {
    Retained(PermissionReceipt),
    Reserved(PermissionReceipt),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionEvidenceUpdate {
    pub(crate) run_id: i64,
    pub(crate) episode_seq: i64,
    pub(crate) output_seq: u64,
    pub(crate) evidence_json: Value,
}

impl InputOperation {
    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.input_id == other.input_id
            && self.run_id == other.run_id
            && self.operation_key == other.operation_key
            && self.kind == other.kind
            && self.payload_digest == other.payload_digest
            && self.message_id == other.message_id
            && self.created_at == other.created_at
            && self.permission_response == other.permission_response
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::ops::RangeInclusive;

    use rusqlite::Connection;
    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::db::test_support::migrated_conn;
    use crate::db::test_support::table_rows;
    use crate::output::format_json;

    pub(crate) const NOW: &str = "2026-10-26T10:00:00.000000+00:00";
    pub(crate) const SECRET: &str = "fixture-secret-194-unique";

    pub(crate) fn create_run(
        conn: &mut Connection,
        new: &NewRun,
    ) -> Result<RunRecord, CafleetError> {
        let deadline = Deadline::inherited_or_new();
        with_database_budget(conn, deadline, |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let run = insert_run(&tx, new)?;
            deadline.remaining()?;
            tx.commit()?;
            Ok(run)
        })
    }

    pub(crate) fn reserve_input(
        conn: &mut Connection,
        owner: &RunOwner,
        new: &NewInput,
    ) -> Result<InputOperation, CafleetError> {
        match reserve_input_with_provenance(conn, owner, new, NewReservation::PLAIN)? {
            InputReservation::Inserted(operation) | InputReservation::Existing(operation) => {
                Ok(operation)
            }
        }
    }

    pub(crate) fn settle_input(
        conn: &mut Connection,
        owner: &RunOwner,
        input_id: i64,
        expected: InputState,
        disposition: InputDisposition,
        evidence_ref: &str,
    ) -> Result<InputOperation, CafleetError> {
        owned_transaction(conn, owner, |conn, _run| {
            settle_input_in_transaction(conn, owner, input_id, expected, disposition, evidence_ref)
        })
    }

    pub(crate) fn release_permission_claim(
        conn: &mut Connection,
        owner: &RunOwner,
        input_id: i64,
        proof: &ZeroByteRetirement<'_>,
        deadline: std::time::Instant,
    ) -> Result<PermissionReceipt, CafleetError> {
        crate::runtime::with_database_deadline(conn, deadline, |conn| {
            owned_transaction(conn, owner, |conn, _| {
                release_permission_claim_in_transaction(conn, owner, input_id, proof, deadline)
            })
        })
    }

    pub(crate) struct Fixture {
        pub(crate) _dir: TempDir,
        pub(crate) conn: Connection,
        pub(crate) fleet: i64,
        pub(crate) director: i64,
        pub(crate) member: i64,
        pub(crate) run: RunRecord,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "runtime-contract");
            let member = register(&mut conn, fleet, "worker", Some("%2"));
            let spec = Self::spec(&conn, fleet, director, member);
            let run = create_run(&mut conn, &spec).unwrap();
            Self {
                _dir: dir,
                conn,
                fleet,
                director,
                member,
                run,
            }
        }

        pub(crate) fn spec(conn: &Connection, fleet: i64, director: i64, member: i64) -> NewRun {
            NewRun {
                member_id: member,
                fleet_id: fleet,
                director_member_id: director,
                backend: "codex".into(),
                database_identity_json: storage::database_identity(conn).unwrap(),
                launch_nonce: "a".repeat(64),
                runtime_path: std::path::Path::new(conn.path().unwrap())
                    .parent()
                    .unwrap()
                    .join(".cafleet-runs/fixture/run")
                    .to_str()
                    .unwrap()
                    .into(),
                requested_mode: None,
                resolved_mode: "never".into(),
                mode_source: ModeSource::Default,
                permission_argv_json: json!([
                    "--ask-for-approval",
                    "never",
                    "--sandbox",
                    "workspace-write"
                ]),
                event_evidence: EventEvidence::Metadata,
                native_version: Some("0.163.0".into()),
                coverage: Coverage::Unknown,
                owner_pid: i64::from(std::process::id()),
                owner_start_identity: "fixture-start-identity".into(),
            }
        }

        /// A second running, tested run for a new member `name` of this fleet, owned by its
        /// runner at epoch 2 with reserved event sequences.
        pub(crate) fn running_sibling(&mut self, name: &str) -> RunOwner {
            let member = crate::broker::members::test_support::register(
                &mut self.conn,
                self.fleet,
                name,
                Some("%3"),
            );
            let spec = Self::spec(&self.conn, self.fleet, self.director, member);
            let run = create_run(&mut self.conn, &spec).unwrap();
            self.conn
                .execute(
                    "UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2,coverage='tested' WHERE run_id=?1",
                    [run.run_id],
                )
                .unwrap();
            let owner = crate::broker::runtime::get_run(&self.conn, run.run_id)
                .unwrap()
                .owner();
            crate::broker::runtime::reserve_event_sequences(&mut self.conn, &owner).unwrap();
            owner
        }

        /// A fixture whose run records redacted event evidence.
        pub(crate) fn redacted() -> Self {
            let fixture = Self::new();
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
            fixture
        }

        pub(crate) fn activate(&mut self) -> RunOwner {
            self.conn.execute("UPDATE member_runs SET status='running',owner_kind='runner',owner_epoch=2 WHERE run_id=?1",[self.run.run_id]).unwrap();
            self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
            self.run.owner()
        }

        pub(crate) fn activate_next_epoch(&mut self) -> RunOwner {
            self.conn
                .execute(
                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                    [self.run.run_id],
                )
                .unwrap();
            self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
            self.run.owner()
        }

        pub(crate) fn recoverer(&mut self) -> RunOwner {
            self.conn.execute("UPDATE member_runs SET status='recovering',owner_kind='reconciler',owner_epoch=owner_epoch+1 WHERE run_id=?1",[self.run.run_id]).unwrap();
            self.run = storage::get_run(&self.conn, self.run.run_id).unwrap();
            self.run.owner()
        }

        pub(crate) fn reserve(&mut self, owner: &RunOwner) -> RangeInclusive<i64> {
            storage::reserve_event_sequences(&mut self.conn, owner).unwrap()
        }

        pub(crate) fn open(&mut self, owner: &RunOwner) -> RuntimeEventRecord {
            self.open_as(owner, InteractionClass::PermissionRequest)
        }

        pub(crate) fn open_as(
            &mut self,
            owner: &RunOwner,
            classification: InteractionClass,
        ) -> RuntimeEventRecord {
            self.reserve(owner);
            let transition = ObservedTransition {
                classification: Some(classification),
                ..opening(self.run.run_id, 1, 1, metadata())
            };
            storage::persist_transition(&mut self.conn, owner, &transition, PersistenceMode::Live)
                .unwrap()
                .0
        }

        pub(crate) fn input(
            &mut self,
            owner: &RunOwner,
            key: &str,
            kind: InputKind,
            message_id: Option<i64>,
        ) -> InputOperation {
            reserve_input(
                &mut self.conn,
                owner,
                &NewInput {
                    operation_key: key.into(),
                    kind,
                    payload_digest: "d".repeat(64),
                    message_id,
                },
            )
            .unwrap()
        }

        pub(crate) fn reopen(&self) -> Connection {
            crate::db::connect(&format!("sqlite:///{}", self.conn.path().unwrap())).unwrap()
        }
    }

    pub(crate) fn raw_evidence(text: &str) -> Value {
        json!({
            "backend":"codex","native_version":"0.163.0",
            "rule_id":"native-tool-permission","screen_seq":120,
            "operation":format!("run {text}"),"operation_rows":1,"choices":[format!("allow {text}"),"deny"],
            "native_request_id":format!("native-{text}"),"excerpt":format!("request {text}"),
            "semantic_fingerprint":format!("fingerprint-{text}"),
            "truncated":false,"sensitive":false,
        })
    }

    pub(crate) fn metadata() -> Value {
        evidence::EvidencePolicy::metadata()
            .apply(&raw_evidence(SECRET))
            .unwrap()
    }

    pub(crate) fn opening(
        run: i64,
        event_seq: i64,
        episode_seq: i64,
        evidence: Value,
    ) -> ObservedTransition {
        ObservedTransition {
            run_id: run,
            event_seq,
            episode_seq: Some(episode_seq),
            kind: RuntimeEventKind::InteractionOpened,
            classification: Some(InteractionClass::PermissionRequest),
            observed_at: NOW.into(),
            evidence_json: evidence,
            close_reason: None,
            cause_json: None,
        }
    }

    pub(crate) fn closure(run: i64, event_seq: i64, episode_seq: i64) -> ObservedTransition {
        ObservedTransition {
            kind: RuntimeEventKind::InteractionClosed,
            close_reason: Some(InteractionCloseReason::NotVisible),
            ..opening(run, event_seq, episode_seq, metadata())
        }
    }

    pub(crate) fn rows(conn: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
        table_rows(conn, &format!("SELECT * FROM {table} ORDER BY 1"))
    }

    pub(crate) fn durable_state(conn: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "member_runs",
            "member_interactions",
            "member_runtime_events",
            "member_input_operations",
            "messages",
        ]
        .map(|table| rows(conn, table))
        .into()
    }

    pub(crate) fn assert_no_secret(conn: &Connection, secret: &str) {
        for table in
            crate::db::test_support::table_snapshot(conn, |name| !name.starts_with("sqlite_"))
        {
            for row in table.rows {
                for value in row {
                    match value {
                        rusqlite::types::Value::Text(text) => {
                            assert!(!text.contains(secret), "secret in {}: {text}", table.name)
                        }
                        rusqlite::types::Value::Blob(bytes) => assert!(
                            !bytes
                                .windows(secret.len())
                                .any(|part| part == secret.as_bytes()),
                            "secret in {} blob",
                            table.name
                        ),
                        _ => {}
                    }
                }
            }
        }
    }

    pub(crate) fn assert_key_order(value: &Value, expected: &[&str]) {
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected
        );
        let text = format_json(value);
        let mut previous = 0;
        for key in expected {
            let needle = format!("\"{key}\":");
            let position = text[previous..]
                .find(&needle)
                .unwrap_or_else(|| panic!("missing ordered {key} in {text}"))
                + previous;
            previous = position + needle.len();
        }
    }

    /// Registers a member and records its run at the run's own artifact directory.
    pub(crate) fn register_member_run(
        conn: &mut Connection,
        fleet: i64,
        director: i64,
        name: &str,
    ) -> RunRecord {
        let member = register(conn, fleet, name, Some("%2"));
        let next_run: i64 = conn
            .query_row(
                "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut spec = Fixture::spec(conn, fleet, director, member);
        spec.runtime_path =
            crate::runtime::RuntimePaths::for_run(&spec.database_identity_json, next_run)
                .unwrap()
                .run_dir
                .to_str()
                .unwrap()
                .into();
        create_run(conn, &spec).unwrap()
    }

    use crate::coding_agent::AnswerCapability;
    use crate::coding_agent::CodingAgent;
    use crate::coding_agent::catalog::AnswerKey;
    use std::time::Duration;
    use std::time::Instant;

    pub(crate) struct ResponseFixture {
        pub(crate) base: Fixture,
        pub(crate) owner: RunOwner,
        pub(crate) event: RuntimeEventRecord,
        pub(crate) request: NewPermissionResponse,
        pub(crate) capability: AnswerCapability,
    }

    pub(crate) fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    pub(crate) fn capability(decision: PermissionDecision) -> AnswerCapability {
        keyed_capability(AnswerKey::Decision(decision))
    }

    pub(crate) fn keyed_capability(key: AnswerKey) -> AnswerCapability {
        AnswerCapability::fixture_key(
            CodingAgent::Codex,
            "0.163.0",
            "native-tool-permission",
            "single-request-choice",
            key,
            &["synthetic-storage-fixture".into()],
        )
        .unwrap()
    }

    impl ResponseFixture {
        pub(crate) fn new() -> Self {
            Self::opened(
                InteractionClass::PermissionRequest,
                AnswerKey::Decision(PermissionDecision::Allow),
            )
        }

        /// A response fixture whose open interaction is a question, answered by `choice`.
        pub(crate) fn question(choice: &str) -> Self {
            Self::opened(InteractionClass::Question, AnswerKey::Choice(choice.into()))
        }

        pub(crate) fn opened(classification: InteractionClass, key: AnswerKey) -> Self {
            let mut base = Fixture::new();
            base.conn
                .execute(
                    "UPDATE member_runs SET coverage='tested' WHERE run_id=?1",
                    [base.run.run_id],
                )
                .unwrap();
            let owner = base.activate();
            let event = base.open_as(&owner, classification);
            let request = NewPermissionResponse {
                operation_key: "permission:fixture-1".into(),
                metadata: PermissionResponseMetadata {
                    event_id: event.event_id,
                    interaction_id: event.interaction_id.unwrap(),
                    expected_owner_epoch: owner.owner_epoch,
                    request_token: "a".repeat(64),
                    key: key.clone(),
                    director_member_id: base.director,
                    director_mode: DirectorMode::Auto,
                    decision_source: DecisionSource::DirectorAuto,
                    mode_evidence_ref: "fixture:host-mode".into(),
                    decision_evidence_ref: "fixture:decision".into(),
                    answer_rule_id: "single-request-choice".into(),
                },
            };
            Self {
                base,
                owner,
                event,
                request,
                capability: keyed_capability(key),
            }
        }

        pub(crate) fn identity(&self) -> PermissionReceiptIdentity {
            PermissionReceiptIdentity {
                database_identity_json: self.owner.database_identity_json.clone(),
                run_id: self.owner.run_id,
            }
        }

        pub(crate) fn reserve(&mut self) -> PermissionReceipt {
            match storage::reserve_permission_response(
                &mut self.base.conn,
                &self.owner,
                &self.request,
                Some(&self.capability),
                deadline(),
            )
            .unwrap()
            {
                PermissionReservation::Reserved(receipt) => receipt,
                PermissionReservation::Retained(receipt) => {
                    panic!("fresh fixture returned retained receipt: {receipt:?}")
                }
            }
        }
    }
    pub(crate) fn effective_policy(
        automatic_review: &str,
        observed_mode: Option<&str>,
        event_id: Option<i64>,
    ) -> Value {
        json!({
            "schema_version":1,"automatic_review":automatic_review,"observed_mode":observed_mode,
            "rule_id":"codex-auto-review-unavailable","event_id":event_id,
            "observed_at":"2026-10-29T02:30:00Z",
        })
    }

    /// The redaction policy for `patterns` that recognizes the native permission request.
    pub(crate) fn redacted_policy(patterns: &[&str]) -> evidence::EvidencePolicy {
        evidence::EvidencePolicy::redacted(
            &json!({"schema_version":1,"patterns":patterns,"sensitive_rules":[]}),
            &["native-tool-permission"],
        )
        .unwrap()
    }

    pub(crate) fn store_effective_policy(fixture: &Fixture, record: &Value) {
        fixture
            .conn
            .execute(
                "UPDATE member_runs SET event_evidence='redacted',effective_policy_json=?1 WHERE run_id=?2",
                params![record.to_string(), fixture.run.run_id],
            )
            .unwrap();
    }
    pub(crate) fn replace_public_payload(
        conn: &mut Connection,
        event: &RuntimeEventRecord,
        payload: Value,
    ) {
        let tx = conn.transaction().unwrap();
        tx.execute(
            "UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2",
            params![payload.to_string(), event.event_id],
        )
        .unwrap();
        if let Some(message_id) = event.message_id {
            tx.execute(
                "UPDATE messages SET text=?1 WHERE message_id=?2",
                params![payload.to_string(), message_id],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use serde_json::json;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::*;

    #[test]
    fn new_run_records_launch_intent_and_unreached_phases_without_inventing_policy() {
        let fixture = Fixture::new();
        let run = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Starting);
        assert_eq!(run.owner_epoch, 1);
        assert_eq!(run.owner_kind, OwnerKind::Creator);
        assert_eq!(run.cleanup_state, CleanupState::Incomplete);
        assert_eq!(run.event_seq_reserved_through, 0);
        assert_eq!(run.observation_gap_count, 0);
        assert_eq!(run.mode_source, ModeSource::Default);
        assert_eq!(run.requested_mode, None);
        assert_eq!(run.resolved_mode, "never");
        assert_eq!(run.effective_policy_json, None);
        assert_eq!(run.child_pid, None);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.exit_signal, None);
        assert_eq!(run.coverage, Coverage::Unknown);
    }

    #[test]
    fn run_creation_validates_member_fleet_and_pinned_director_before_allocating() {
        let mut fixture = Fixture::new();
        let (other_fleet, other_director) = create_fleet_fixture(&mut fixture.conn, "foreign");
        let member = register(&mut fixture.conn, fixture.fleet, "new-worker", None);
        for mismatch in ["member", "fleet", "director", "database"] {
            let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
            match mismatch {
                "member" => spec.member_id = other_director,
                "fleet" => spec.fleet_id = other_fleet,
                "director" => spec.director_member_id = other_director,
                "database" => spec.database_identity_json["instance_id"] = json!("0".repeat(32)),
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                crate::broker::runtime::test_support::create_run(&mut fixture.conn, &spec).is_err(),
                "{mismatch}"
            );
            assert_eq!(durable_state(&fixture.conn), before, "{mismatch}");
        }
    }

    #[test]
    fn explicit_mode_is_distinct_from_the_default_and_does_not_prove_effective_policy() {
        let mut fixture = Fixture::new();
        let member = register(&mut fixture.conn, fixture.fleet, "explicit", None);
        let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
        spec.requested_mode = Some("on-request".into());
        spec.resolved_mode = "on-request".into();
        spec.mode_source = ModeSource::Explicit;
        spec.permission_argv_json = json!([
            "--ask-for-approval",
            "on-request",
            "--sandbox",
            "workspace-write"
        ]);
        let run =
            crate::broker::runtime::test_support::create_run(&mut fixture.conn, &spec).unwrap();
        assert_eq!(run.requested_mode.as_deref(), Some("on-request"));
        assert_eq!(run.mode_source, ModeSource::Explicit);
        assert_eq!(run.permission_argv_json, spec.permission_argv_json);
        assert_eq!(run.effective_policy_json, None);
    }

    #[test]
    fn typed_runtime_readers_fail_on_corrupt_required_values() {
        for (field, value) in [
            ("status", "invented"),
            ("database_identity_json", "[]"),
            ("owner_start_identity", ""),
        ] {
            let fixture = Fixture::new();
            fixture
                .conn
                .execute_batch("PRAGMA ignore_check_constraints=ON")
                .unwrap();
            fixture
                .conn
                .execute(
                    &format!("UPDATE member_runs SET {field}=?1 WHERE run_id=?2"),
                    params![value, fixture.run.run_id],
                )
                .unwrap();
            let error = storage::get_run(&fixture.conn, fixture.run.run_id).unwrap_err();
            assert!(error.message().contains(field), "{field}: {error:?}");
        }
    }

    #[test]
    fn current_owner_reserves_disjoint_ranges_of_256_across_reconnections() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        assert_eq!(fixture.reserve(&owner), 1..=256);
        let mut other = fixture.reopen();
        assert_eq!(
            storage::reserve_event_sequences(&mut other, &owner).unwrap(),
            257..=512
        );
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .event_seq_reserved_through,
            512
        );
        let transition = opening(owner.run_id, 257, 1, metadata());
        assert!(
            storage::persist_transition(&mut other, &owner, &transition, PersistenceMode::Live)
                .is_ok()
        );
    }

    #[test]
    fn stale_owner_kind_epoch_or_database_identity_fences_every_mutation() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.reserve(&owner);
        for mismatch in ["epoch", "kind", "database"] {
            let mut stale = owner.clone();
            match mismatch {
                "epoch" => stale.owner_epoch += 1,
                "kind" => stale.owner_kind = OwnerKind::Reconciler,
                "database" => stale.database_identity_json["instance_id"] = json!("0".repeat(32)),
                _ => unreachable!(),
            }
            let before = durable_state(&fixture.conn);
            assert!(
                storage::reserve_event_sequences(&mut fixture.conn, &stale).is_err(),
                "{mismatch}"
            );
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &stale,
                    &opening(owner.run_id, 1, 1, metadata()),
                    PersistenceMode::Live
                )
                .is_err(),
                "{mismatch}"
            );
            assert!(
                crate::broker::runtime::test_support::reserve_input(
                    &mut fixture.conn,
                    &stale,
                    &NewInput {
                        operation_key: "stale".into(),
                        kind: InputKind::Prompt,
                        payload_digest: "d".repeat(64),
                        message_id: None
                    }
                )
                .is_err(),
                "{mismatch}"
            );
            assert_eq!(durable_state(&fixture.conn), before, "{mismatch}");
        }
    }

    #[test]
    fn failed_sequence_reservation_rolls_back_the_high_water_mark() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        fixture.conn.execute_batch("CREATE TRIGGER fail_reservation BEFORE UPDATE OF event_seq_reserved_through ON member_runs BEGIN SELECT RAISE(ABORT,'injected reservation failure'); END").unwrap();
        assert!(storage::reserve_event_sequences(&mut fixture.conn, &owner).is_err());
        assert_eq!(
            storage::get_run(&fixture.conn, owner.run_id)
                .unwrap()
                .event_seq_reserved_through,
            0
        );
        fixture
            .conn
            .execute_batch("DROP TRIGGER fail_reservation")
            .unwrap();
        assert_eq!(fixture.reserve(&owner), 1..=256);
    }
    #[test]
    fn concurrent_sequence_reservations_commit_disjoint_ranges() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        let url = format!("sqlite:///{}", fixture.conn.path().unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let url = url.clone();
                let owner = owner.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut conn = crate::db::connect(&url).unwrap();
                    barrier.wait();
                    storage::reserve_event_sequences(&mut conn, &owner).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let mut ranges = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        ranges.sort_by_key(|range| *range.start());
        assert_eq!(ranges, [1..=256, 257..=512]);
    }
    #[test]
    fn missing_database_instance_identity_fails_at_the_identity_read_boundary() {
        let fixture = Fixture::new();
        fixture
            .conn
            .execute("DELETE FROM runtime_database_identity", [])
            .unwrap();
        assert!(storage::database_identity(&fixture.conn).is_err());
    }

    mod runner_registry {
        use crate::broker::runtime as storage;
        use crate::broker::runtime::RunStatus;
        use crate::runtime::control::test_support::SilentServer;
        use crate::runtime::control::test_support::activate;
        use crate::runtime::test_support::Fixture;
        use rusqlite::Connection;
        use std::time::Duration;
        use std::time::Instant;

        fn counts(conn: &Connection) -> (i64, i64, i64) {
            let scalar = |table| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
            };
            (
                scalar("member_interactions"),
                scalar("member_runtime_events"),
                scalar("messages"),
            )
        }
        #[test]
        fn member_runtime_returns_historical_health_without_contacting_a_live_endpoint() {
            let mut fixture = Fixture::new();
            let (artifacts, _held) = activate(&mut fixture);
            let _server = SilentServer::start(&artifacts.paths.control_socket);
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET last_health_at='2000-01-01T00:00:00Z' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
            let before = counts(&fixture.conn);
            let start = Instant::now();
            let snapshot = storage::member_runtime(&fixture.conn, fixture.run.member_id)
                .unwrap()
                .expect("the member has a recorded run");
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(
                snapshot.run.last_health_at.as_deref(),
                Some("2000-01-01T00:00:00Z")
            );
            assert_eq!(counts(&fixture.conn), before);
            assert_eq!(
                storage::get_run(&fixture.conn, fixture.run.run_id)
                    .unwrap()
                    .status,
                RunStatus::Running
            );
        }
    }

    #[test]
    fn historical_ownership_requires_one_fleet_relationship_and_selects_the_archive_reason() {
        let mut fixture = Fixture::new();
        let run = fixture.run.clone();
        assert_eq!(historical_ownership(&fixture.conn, &run).unwrap(), None);
        let (other_fleet, other_director) = create_fleet_fixture(&mut fixture.conn, "other");
        let foreign_member = register(&mut fixture.conn, other_fleet, "foreign", None);
        let peer = register(&mut fixture.conn, run.fleet_id, "peer", None);
        for (conjunct, change) in [
            (
                "member fleet",
                RunRecord {
                    member_id: foreign_member,
                    ..run.clone()
                },
            ),
            (
                "director fleet",
                RunRecord {
                    director_member_id: other_director,
                    ..run.clone()
                },
            ),
            (
                "fleet director",
                RunRecord {
                    director_member_id: peer,
                    ..run.clone()
                },
            ),
            (
                "member is director",
                RunRecord {
                    member_id: run.director_member_id,
                    ..run.clone()
                },
            ),
        ] {
            let failure = historical_ownership(&fixture.conn, &change).unwrap_err();
            assert!(
                failure.message().contains("member/fleet/Director mismatch"),
                "{conjunct}: {}",
                failure.message()
            );
        }
        for (sql, reason) in [
            (
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                (run.director_member_id, ArchiveReason::DirectorInactive),
            ),
            (
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                (run.member_id, ArchiveReason::MemberInactive),
            ),
            (
                "UPDATE fleets SET deleted_at=created_at WHERE fleet_id=?1",
                (run.fleet_id, ArchiveReason::FleetDeleted),
            ),
        ] {
            fixture.conn.execute(sql, [reason.0]).unwrap();
            assert_eq!(
                historical_ownership(&fixture.conn, &run).unwrap(),
                Some(reason.1)
            );
        }
    }

    #[test]
    fn validate_owner_requires_the_current_token_and_database_identity() {
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        assert_eq!(
            validate_owner(&fixture.conn, &owner).unwrap().run_id,
            owner.run_id
        );
        let mut stale = owner.clone();
        stale.owner_epoch += 1;
        assert!(
            validate_owner(&fixture.conn, &stale)
                .unwrap_err()
                .message()
                .contains("stale ownership token")
        );
        let mut foreign = owner.clone();
        foreign.database_identity_json["instance_id"] = json!("0".repeat(32));
        assert!(validate_owner(&fixture.conn, &foreign).is_err());
    }
}
