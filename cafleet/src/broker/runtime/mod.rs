pub(crate) use events::{
    original as original_transition, validate_transition as validate_observed_transition,
};
mod events;
pub mod evidence;
mod inputs;
mod permissions;
mod records;

pub use events::{get_event, persist_transition, update_interaction_evidence};
pub(crate) use events::{persist_transition_counted, persist_transition_in_transaction};
pub(crate) use inputs::{InputReservation, reserve_input_with_provenance};
pub use inputs::{
    find_input, get_input, record_input_state, reserve_input, settle_input, settle_preview,
};
pub use permissions::{
    ZeroByteRetirement, get_permission_response, lookup_permission_response,
    permission_response_digest, release_permission_claim, reserve_permission_response,
};
pub use records::*;

use std::ops::RangeInclusive;
use std::os::unix::fs::MetadataExt;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use super::members::db_err;
use super::records::MemberStatus;
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::time::{format_utc, now_utc};

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

pub fn database_identity(conn: &Connection) -> Result<Value, CafleetError> {
    let path = conn
        .path()
        .ok_or_else(|| invalid("database_identity", "file-backed database required"))?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| CafleetError::App(format!("cannot identify runtime database: {error}")))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| CafleetError::App(format!("cannot inspect runtime database: {error}")))?;
    let instance: String = conn
        .query_row(
            "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1",
            [],
            |row| row.get(0),
        )
        .map_err(db_err)?;
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
        profile_version: row.get("profile_version")?,
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

pub fn get_run(conn: &Connection, run_id: i64) -> Result<RunRecord, CafleetError> {
    let run = conn
        .query_row(
            "SELECT * FROM member_runs WHERE run_id=?1",
            [run_id],
            map_run,
        )
        .optional()
        .map_err(db_err)?
        .ok_or_else(|| CafleetError::Value(format!("Run {run_id} not found")))?;
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
    let member: (i64, MemberStatus) = conn
        .query_row(
            "SELECT fleet_id,status FROM members WHERE member_id=?1",
            [run.member_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_err)?;
    let director: (i64, MemberStatus) = conn
        .query_row(
            "SELECT fleet_id,status FROM members WHERE member_id=?1",
            [run.director_member_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_err)?;
    let fleet = super::fleets::fetch_fleet(conn, run.fleet_id)?
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
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let run = validate_owner(&tx, owner)?;
        let result = operation(&tx, &run)?;
        deadline.remaining()?;
        tx.commit().map_err(db_err)?;
        Ok(result)
    })
}

pub fn create_run(conn: &mut Connection, new: &NewRun) -> Result<RunRecord, CafleetError> {
    let deadline = Deadline::inherited_or_new();
    with_database_budget(conn, deadline, |conn| {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let run = insert_run(&tx, new)?;
        deadline.remaining()?;
        tx.commit().map_err(db_err)?;
        Ok(run)
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
    let fleet = super::fleets::require_live_fleet(conn, new.fleet_id)?;
    require(
        fleet.director_member_id == Some(new.director_member_id)
            && new.member_id != new.director_member_id
            && super::members::active_member_fleet(conn, new.member_id)? == Some(new.fleet_id),
        "member_runs.ownership",
        "member/fleet/Director mismatch",
    )?;
    let now = format_utc(now_utc());
    conn.execute(
            "INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,database_identity_json,
             launch_nonce,runtime_path,requested_mode,resolved_mode,mode_source,permission_argv_json,
             event_evidence,coverage,status,created_at,native_version,profile_version,owner_epoch,
             owner_kind,owner_pid,owner_start_identity,reachability,cleanup_state,event_seq_reserved_through,observation_gap_count)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'starting',?14,?15,?16,1,'creator',?17,?18,'reachable','incomplete',0,0)",
            params![new.member_id,new.fleet_id,new.director_member_id,new.backend,new.database_identity_json.to_string(),
                new.launch_nonce,new.runtime_path,new.requested_mode,new.resolved_mode,new.mode_source.as_str(),
                new.permission_argv_json.to_string(),new.event_evidence.as_str(),new.coverage.as_str(),now,
                new.native_version,new.profile_version,new.owner_pid,new.owner_start_identity],
        ).map_err(db_err)?;
    let run = get_run(conn, conn.last_insert_rowid())?;
    Ok(run)
}

pub fn reserve_event_sequences(
    conn: &mut Connection,
    owner: &RunOwner,
) -> Result<RangeInclusive<i64>, CafleetError> {
    owned_transaction(conn, owner, |conn, run| {
        let first = run
            .event_seq_reserved_through
            .checked_add(1)
            .ok_or_else(|| invalid("event_seq_reserved_through", "sequence overflow"))?;
        let last = run
            .event_seq_reserved_through
            .checked_add(256)
            .ok_or_else(|| invalid("event_seq_reserved_through", "sequence overflow"))?;
        let changed = conn.execute(
            "UPDATE member_runs SET event_seq_reserved_through=?1 WHERE run_id=?2 AND owner_epoch=?3 AND event_seq_reserved_through=?4",
            params![last,run.run_id,run.owner_epoch,run.event_seq_reserved_through],
        ).map_err(db_err)?;
        require(
            changed == 1,
            "member_runs.owner",
            "sequence reservation lost ownership",
        )?;
        Ok(first..=last)
    })
}

pub fn runtime_snapshot(conn: &Connection, run_id: i64) -> Result<RuntimeSnapshot, CafleetError> {
    let run = get_run(conn, run_id)?;
    let current_interaction_id = conn
        .query_row(
            "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND closed_at IS NULL",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    let mut statement = conn.prepare(
        "SELECT input_id FROM member_input_operations WHERE run_id=?1 AND state IN ('in_progress','accepted','uncertain') ORDER BY input_id",
    ).map_err(db_err)?;
    let unresolved_input_ids = statement
        .query_map([run_id], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(RuntimeSnapshot {
        run,
        current_interaction_id,
        unresolved_input_ids,
    })
}

pub fn member_runtime(
    conn: &Connection,
    member_id: i64,
) -> Result<Option<RuntimeSnapshot>, CafleetError> {
    let id = conn
        .query_row(
            "SELECT run_id FROM member_runs WHERE member_id=?1 ORDER BY run_id DESC LIMIT 1",
            [member_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    id.map(|id| runtime_snapshot(conn, id)).transpose()
}
