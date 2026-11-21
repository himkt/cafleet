mod events;
mod inputs;

use crate::broker::MemberStatus;
pub(crate) use crate::broker::runtime::events::*;
pub(crate) use crate::broker::runtime::inputs::*;
use crate::broker::stored_enum;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::time::Deadline;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
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

pub(super) fn rfc3339(value: &str, field: &str) -> Result<(), CafleetError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(drop)
        .map_err(|_| invalid(field, "RFC 3339 timestamp required"))
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
        coverage: row.get("coverage")?,
        status: row.get("status")?,
        native_version: row.get("native_version")?,
        owner_epoch: row.get("owner_epoch")?,
        owner_kind: row.get("owner_kind")?,
        owner_pid: row.get("owner_pid")?,
        owner_start_identity: row.get("owner_start_identity")?,
        cleanup_state: row.get("cleanup_state")?,
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
    validate_identity(&run.database_identity_json)?;
    require(
        run.permission_argv_json
            .as_array()
            .expect("decoded array")
            .iter()
            .all(Value::is_string),
        "member_runs.permission_argv_json",
        "string arguments required",
    )?;
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
            && fleet.director_member_id == run.director_member_id
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
        fleet.director_member_id == new.director_member_id
            && new.member_id != new.director_member_id
            && crate::broker::members::active_member_fleet(conn, new.member_id)?
                == Some(new.fleet_id),
        "member_runs.ownership",
        "member/fleet/Director mismatch",
    )?;
    conn.execute(
            "INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,database_identity_json,
             launch_nonce,runtime_path,requested_mode,resolved_mode,mode_source,permission_argv_json,
             coverage,status,native_version,owner_epoch,
             owner_kind,owner_pid,owner_start_identity,cleanup_state)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'starting',?13,1,'creator',?14,?15,'incomplete')",
            params![new.member_id,new.fleet_id,new.director_member_id,new.backend,new.database_identity_json.to_string(),
                new.launch_nonce,new.runtime_path,new.requested_mode,new.resolved_mode,new.mode_source.as_str(),
                new.permission_argv_json.to_string(),new.coverage.as_str(),
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
    Degraded => "degraded", Recovering => "recovering",
    Exited => "exited", Failed => "failed", Lost => "lost"
});
stored_enum!(ModeSource, "member_runs.mode_source", { Default => "default", Explicit => "explicit" });
stored_enum!(OwnerKind, "member_runs.owner_kind", { Creator => "creator", Runner => "runner", Reconciler => "reconciler" });
stored_enum!(CleanupState, "member_runs.cleanup_state", { Complete => "complete", Incomplete => "incomplete" });
stored_enum!(Coverage, "member_runs.coverage", { Tested => "tested", Unknown => "unknown" });
stored_enum!(test_values InteractionClass, "member_interactions.classification", {
    PermissionRequest => "permission_request", Question => "question",
    UnknownInteraction => "unknown_interaction"
});
stored_enum!(test_values InteractionCloseReason, "member_interactions.close_reason", {
    NotVisible => "not_visible", Superseded => "superseded",
    ProcessExited => "process_exited", MonitorLost => "monitor_lost"
});
stored_enum!(RuntimeEventKind,"member_runtime_events.kind", {
    CoverageUnknown => "coverage_unknown", InteractionOpened => "interaction_opened",
    InteractionClosed => "interaction_closed",
    InputDeliveryResult => "input_delivery_result", InputDeliveryStalled => "input_delivery_stalled",
    RunnerDegraded => "runner_degraded",
    RunnerLost => "runner_lost", RunnerExited => "runner_exited"
});
stored_enum!(RuntimeCauseKind, "cause.kind", {
    CoverageUnknown => "coverage_unknown", NativeExit => "native_exit",
    StoragePressure => "storage_pressure", StorageRecovered => "storage_recovered",
    ObserverError => "observer_error", MonitorLost => "monitor_lost",
    RuntimePathMissing => "runtime_path_missing", EventRejected => "event_rejected"
});
stored_enum!(NotificationState, "member_runtime_events.notification_state", { Inbox => "inbox", Archived => "archived" });
stored_enum!(test_values ArchiveReason, "member_runtime_events.archive_reason", {
    MemberInactive => "member_inactive", FleetDeleted => "fleet_deleted", DirectorInactive => "director_inactive",
    HeadNotReady => "head_not_ready", DuplicateDelivery => "duplicate_delivery"
});
stored_enum!(InputKind, "member_input_operations.kind", { Prompt => "prompt", Exec => "exec" });
stored_enum!(InputState, "member_input_operations.state", {
    Ready => "ready", InProgress => "in_progress", Accepted => "accepted",
    Uncertain => "uncertain", Settled => "settled"
});
stored_enum!(test_values InputDisposition, "member_input_operations.disposition", {
    Submitted => "submitted", NotSubmitted => "not_submitted", AbandonedUncertain => "abandoned_uncertain"
});
stored_enum!(StopCause, "member_input_operations.stop_cause", {
    InteractionCandidate => "interaction_candidate", InteractionOpen => "interaction_open",
    DeadlineExpired => "deadline_expired",
    SubmitUnconfirmed => "submit_unconfirmed", ShortWrite => "short_write",
    WriteFailed => "write_failed", RunnerStopped => "runner_stopped"
});

impl StopCause {
    pub(crate) fn is_check_stop(self) -> bool {
        matches!(
            self,
            Self::InteractionCandidate
                | Self::InteractionOpen
                | Self::DeadlineExpired
                | Self::SubmitUnconfirmed
        )
    }
}

impl RunStatus {
    pub(crate) fn terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed | Self::Lost)
    }
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
    pub(crate) coverage: Coverage,
    pub(crate) status: RunStatus,
    pub(crate) native_version: Option<String>,
    pub(crate) owner_epoch: i64,
    pub(crate) owner_kind: OwnerKind,
    pub(crate) owner_pid: i64,
    pub(crate) owner_start_identity: String,
    pub(crate) cleanup_state: CleanupState,
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
    pub(crate) disposition: Option<InputDisposition>,
    pub(crate) evidence_ref: Option<String>,
    pub(crate) delivery_method: Option<DeliveryMethod>,
    pub(crate) delivery_reason: Option<String>,
    pub(crate) deferred_at: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) stop_cause: Option<StopCause>,
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

stored_enum!(values PermissionDecision, "response.decision", { Allow => "allow", Deny => "deny" });

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

impl InputOperation {
    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.input_id == other.input_id
            && self.run_id == other.run_id
            && self.operation_key == other.operation_key
            && self.kind == other.kind
            && self.payload_digest == other.payload_digest
            && self.created_at == other.created_at
    }
}

#[cfg(test)]
pub(crate) mod test_support {
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
        owned_transaction(conn, owner, |conn, run| {
            settle_input_in_transaction(conn, run, input_id, expected, disposition, evidence_ref)
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
                native_version: Some("0.163.0".into()),
                coverage: Coverage::Unknown,
                owner_pid: i64::from(std::process::id()),
                owner_start_identity: "fixture-start-identity".into(),
            }
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

        pub(crate) fn open(&mut self, owner: &RunOwner) -> RuntimeEventRecord {
            self.open_as(owner, InteractionClass::PermissionRequest)
        }

        pub(crate) fn open_as(
            &mut self,
            owner: &RunOwner,
            classification: InteractionClass,
        ) -> RuntimeEventRecord {
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
        ) -> InputOperation {
            reserve_input(
                &mut self.conn,
                owner,
                &NewInput {
                    operation_key: key.into(),
                    kind,
                    payload_digest: "d".repeat(64),
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
            "excerpt":format!("request {text}"),
            "semantic_fingerprint":format!("fingerprint-{text}"),
            "truncated":false,})
    }

    pub(crate) fn metadata() -> Value {
        crate::broker::runtime::metadata_evidence(&raw_evidence(SECRET)).unwrap()
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
    use serde_json::json;

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::test_support::*;
    use crate::broker::runtime::*;

    #[test]
    fn run_creation_records_the_default_or_explicit_mode_and_unreached_phases() {
        struct Case {
            name: &'static str,
            requested: Option<&'static str>,
            resolved: &'static str,
            source: ModeSource,
        }
        const CASES: &[Case] = &[
            Case {
                name: "the backend default",
                requested: None,
                resolved: "never",
                source: ModeSource::Default,
            },
            Case {
                name: "an explicit mode",
                requested: Some("on-request"),
                resolved: "on-request",
                source: ModeSource::Explicit,
            },
        ];
        for case in CASES {
            let name = case.name;
            let mut fixture = Fixture::new();
            let member = register(&mut fixture.conn, fixture.fleet, "worker-run", None);
            let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
            spec.requested_mode = case.requested.map(Into::into);
            spec.resolved_mode = case.resolved.into();
            spec.mode_source = case.source;
            spec.permission_argv_json = json!(["--ask-for-approval", case.resolved]);
            let run = create_run(&mut fixture.conn, &spec).unwrap();
            let run = storage::get_run(&fixture.conn, run.run_id).unwrap();
            assert_eq!(run.requested_mode.as_deref(), case.requested, "{name}");
            assert_eq!(run.resolved_mode, case.resolved, "{name}");
            assert_eq!(run.mode_source, case.source, "{name}");
            assert_eq!(
                run.permission_argv_json, spec.permission_argv_json,
                "{name}"
            );
            assert_eq!(run.status, RunStatus::Starting, "{name}");
            assert_eq!(run.owner_epoch, 1, "{name}");
            assert_eq!(run.owner_kind, OwnerKind::Creator, "{name}");
            assert_eq!(run.cleanup_state, CleanupState::Incomplete, "{name}");
            assert_eq!(run.child_pid, None, "{name}");
            assert_eq!(run.exit_code, None, "{name}");
            assert_eq!(run.coverage, Coverage::Unknown, "{name}");
        }
    }

    #[test]
    fn corrupt_or_missing_required_storage_fails_its_typed_reader() {
        const CASES: &[(&str, &str, &str)] = &[
            (
                "status",
                "PRAGMA ignore_check_constraints=ON; UPDATE member_runs SET status='invented'",
                "status",
            ),
            (
                "database identity",
                "PRAGMA ignore_check_constraints=ON; UPDATE member_runs SET database_identity_json='[]'",
                "database_identity_json",
            ),
            (
                "instance identity",
                "DELETE FROM runtime_database_identity",
                "",
            ),
        ];
        for &(name, corruption, field) in CASES {
            let fixture = Fixture::new();
            fixture.conn.execute_batch(corruption).unwrap();
            let error = if field.is_empty() {
                storage::database_identity(&fixture.conn).unwrap_err()
            } else {
                storage::get_run(&fixture.conn, fixture.run.run_id).unwrap_err()
            };
            assert!(error.message().contains(field), "{name}: {error:?}");
        }
    }

    #[test]
    fn a_stale_owner_kind_epoch_or_database_identity_fences_every_owner_operation() {
        type Stale = fn(&mut RunOwner);
        const STALE: &[(&str, Stale)] = &[
            ("epoch", |owner| owner.owner_epoch += 1),
            ("kind", |owner| owner.owner_kind = OwnerKind::Reconciler),
            ("database", |owner| {
                owner.database_identity_json["instance_id"] = json!("0".repeat(32))
            }),
        ];
        let mut fixture = Fixture::new();
        let owner = fixture.activate();
        assert_eq!(
            validate_owner(&fixture.conn, &owner).unwrap().run_id,
            owner.run_id
        );
        for &(name, make_stale) in STALE {
            let mut stale = owner.clone();
            make_stale(&mut stale);
            let before = durable_state(&fixture.conn);
            assert!(validate_owner(&fixture.conn, &stale).is_err(), "{name}");
            assert!(
                storage::persist_transition(
                    &mut fixture.conn,
                    &stale,
                    &opening(owner.run_id, 1, 1, metadata()),
                    PersistenceMode::Live
                )
                .is_err(),
                "{name}"
            );
            assert!(
                reserve_input(
                    &mut fixture.conn,
                    &stale,
                    &NewInput {
                        operation_key: "stale".into(),
                        kind: InputKind::Prompt,
                        payload_digest: "d".repeat(64),
                    }
                )
                .is_err(),
                "{name}"
            );
            assert_eq!(durable_state(&fixture.conn), before, "{name}");
        }
        let mut stale = owner.clone();
        stale.owner_epoch += 1;
        assert!(
            validate_owner(&fixture.conn, &stale)
                .unwrap_err()
                .message()
                .contains("stale ownership token")
        );
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

    mod runner_diagnostics {
        use crate::broker::runtime::tests::*;

        type Row = (String, String, String, String, i64);

        fn at(second: u32) -> String {
            format!(
                "2026-11-21T12:{:02}:{:02}.000000+00:00",
                second / 60,
                second % 60
            )
        }

        fn entry(
            source: RunnerDiagnosticSource,
            message: &str,
            first: u32,
            last: u32,
            count: i64,
        ) -> RunnerDiagnosticEntry {
            RunnerDiagnosticEntry {
                source,
                message: message.into(),
                first_at: at(first),
                last_at: at(last),
                count,
            }
        }

        fn write(
            fixture: &mut Fixture,
            run_id: i64,
            entries: &[RunnerDiagnosticEntry],
            dropped: u64,
        ) {
            let tx = fixture.conn.transaction().unwrap();
            write_runner_diagnostics(&tx, run_id, entries, dropped).unwrap();
            tx.commit().unwrap();
        }

        fn rows(fixture: &Fixture, run_id: i64) -> Vec<Row> {
            fixture
                .conn
                .prepare("SELECT source,message,first_at,last_at,count FROM member_runner_diagnostics WHERE run_id=?1 ORDER BY diagnostic_id")
                .unwrap()
                .query_map([run_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }

        fn second_run(fixture: &mut Fixture) -> i64 {
            let member = register(
                &mut fixture.conn,
                fixture.fleet,
                "second-worker",
                Some("%3"),
            );
            let mut spec = Fixture::spec(&fixture.conn, fixture.fleet, fixture.director, member);
            spec.launch_nonce = "b".repeat(64);
            spec.runtime_path = spec.runtime_path.replace("/fixture/", "/second/");
            create_run(&mut fixture.conn, &spec).unwrap().run_id
        }

        const SOURCES: &[(RunnerDiagnosticSource, &str)] = &[
            (RunnerDiagnosticSource::EventWriter, "event_writer"),
            (RunnerDiagnosticSource::Observation, "observation"),
            (RunnerDiagnosticSource::DeferredInput, "deferred_input"),
            (
                RunnerDiagnosticSource::QueueHousekeeping,
                "queue_housekeeping",
            ),
            (RunnerDiagnosticSource::PreviewSweep, "preview_sweep"),
            (RunnerDiagnosticSource::InputResult, "input_result"),
            (
                RunnerDiagnosticSource::PermissionAnswer,
                "permission_answer",
            ),
            (RunnerDiagnosticSource::Shutdown, "shutdown"),
        ];

        #[test]
        fn every_runner_diagnostic_source_is_stored_under_its_name() {
            for &(source, name) in SOURCES {
                let mut fixture = Fixture::new();
                let run_id = fixture.run.run_id;
                write(
                    &mut fixture,
                    run_id,
                    &[entry(source, "label: fixture diagnostic", 1, 1, 1)],
                    0,
                );
                assert_eq!(source.as_str(), name);
                assert_eq!(rows(&fixture, run_id)[0].0, name, "{name}");
                assert_eq!(
                    latest_runner_diagnostics(&fixture.conn, run_id).unwrap()[0].source,
                    source,
                    "{name}"
                );
            }
        }

        #[test]
        fn a_repeated_source_and_message_adds_its_count_and_moves_last_at() {
            let mut fixture = Fixture::new();
            let run_id = fixture.run.run_id;
            write(
                &mut fixture,
                run_id,
                &[
                    entry(RunnerDiagnosticSource::EventWriter, "label: busy", 1, 2, 1),
                    entry(RunnerDiagnosticSource::Observation, "label: busy", 1, 3, 2),
                ],
                0,
            );
            write(
                &mut fixture,
                run_id,
                &[entry(
                    RunnerDiagnosticSource::EventWriter,
                    "label: busy",
                    10,
                    12,
                    3,
                )],
                0,
            );
            assert_eq!(
                rows(&fixture, run_id),
                [
                    (
                        "event_writer".into(),
                        "label: busy".into(),
                        at(1),
                        at(12),
                        4
                    ),
                    ("observation".into(), "label: busy".into(), at(1), at(3), 2),
                ]
            );
        }

        #[test]
        fn dropped_entries_add_one_overflow_row_naming_the_count() {
            for dropped in [0_u64, 3] {
                let mut fixture = Fixture::new();
                let run_id = fixture.run.run_id;
                write(
                    &mut fixture,
                    run_id,
                    &[entry(
                        RunnerDiagnosticSource::Shutdown,
                        "label: exit",
                        1,
                        1,
                        1,
                    )],
                    dropped,
                );
                let overflow: Vec<Row> = rows(&fixture, run_id)
                    .into_iter()
                    .filter(|row| row.1.starts_with("diagnostic buffer overflow"))
                    .collect();
                if dropped == 0 {
                    assert!(overflow.is_empty(), "{overflow:?}");
                } else {
                    let [row] = overflow.as_slice() else {
                        panic!("one overflow row, got {overflow:?}");
                    };
                    assert_eq!(row.0, "event_writer");
                    assert_eq!(row.1, "diagnostic buffer overflow: 3 entries dropped");
                }
            }
        }

        struct PruneCase {
            name: &'static str,
            total: u32,
            distinct_last_at: bool,
        }

        const PRUNE_CASES: &[PruneCase] = &[
            PruneCase {
                name: "older last_at rows are deleted first",
                total: 260,
                distinct_last_at: true,
            },
            PruneCase {
                name: "a last_at tie deletes the lower diagnostic_id first",
                total: 258,
                distinct_last_at: false,
            },
        ];

        #[test]
        fn each_run_keeps_only_its_newest_256_rows() {
            for case in PRUNE_CASES {
                let name = case.name;
                let mut fixture = Fixture::new();
                let run_id = fixture.run.run_id;
                let other = second_run(&mut fixture);
                write(
                    &mut fixture,
                    other,
                    &[entry(
                        RunnerDiagnosticSource::Shutdown,
                        "label: other run",
                        0,
                        0,
                        1,
                    )],
                    0,
                );
                let entries: Vec<_> = (0..case.total)
                    .map(|index| {
                        let last = if case.distinct_last_at { index + 1 } else { 1 };
                        entry(
                            RunnerDiagnosticSource::EventWriter,
                            &format!("label: entry {index:03}"),
                            0,
                            last,
                            1,
                        )
                    })
                    .collect();
                write(&mut fixture, run_id, &entries, 0);
                let kept = rows(&fixture, run_id);
                assert_eq!(kept.len(), 256, "{name}");
                let removed = case.total - 256;
                assert_eq!(
                    kept.first().unwrap().1,
                    format!("label: entry {removed:03}"),
                    "{name}: the oldest rows are pruned"
                );
                assert_eq!(
                    kept.last().unwrap().1,
                    format!("label: entry {:03}", case.total - 1),
                    "{name}"
                );
                assert_eq!(
                    rows(&fixture, other).len(),
                    1,
                    "{name}: other runs keep theirs"
                );
            }
        }

        #[test]
        fn latest_runner_diagnostics_returns_the_runs_newest_five_by_last_at_then_id() {
            let mut fixture = Fixture::new();
            let run_id = fixture.run.run_id;
            let other = second_run(&mut fixture);
            write(
                &mut fixture,
                other,
                &[entry(
                    RunnerDiagnosticSource::Shutdown,
                    "label: other run",
                    99,
                    99,
                    1,
                )],
                0,
            );
            write(
                &mut fixture,
                run_id,
                &[
                    entry(RunnerDiagnosticSource::EventWriter, "label: a", 1, 1, 1),
                    entry(RunnerDiagnosticSource::Observation, "label: b", 1, 7, 2),
                    entry(RunnerDiagnosticSource::DeferredInput, "label: c", 2, 3, 1),
                    entry(RunnerDiagnosticSource::PreviewSweep, "label: d", 4, 5, 1),
                    entry(RunnerDiagnosticSource::InputResult, "label: e", 5, 5, 3),
                    entry(RunnerDiagnosticSource::Shutdown, "label: f", 6, 6, 1),
                    entry(RunnerDiagnosticSource::EventWriter, "label: g", 2, 2, 1),
                ],
                0,
            );
            assert_eq!(
                latest_runner_diagnostics(&fixture.conn, run_id).unwrap(),
                [
                    entry(RunnerDiagnosticSource::Observation, "label: b", 1, 7, 2),
                    entry(RunnerDiagnosticSource::Shutdown, "label: f", 6, 6, 1),
                    entry(RunnerDiagnosticSource::InputResult, "label: e", 5, 5, 3),
                    entry(RunnerDiagnosticSource::PreviewSweep, "label: d", 4, 5, 1),
                    entry(RunnerDiagnosticSource::DeferredInput, "label: c", 2, 3, 1),
                ]
            );
            assert_eq!(
                latest_runner_diagnostics(&fixture.conn, other).unwrap(),
                [entry(
                    RunnerDiagnosticSource::Shutdown,
                    "label: other run",
                    99,
                    99,
                    1
                )]
            );
        }
    }
}
