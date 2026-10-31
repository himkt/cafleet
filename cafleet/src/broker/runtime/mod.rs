pub(crate) use events::{
    original as original_transition, validate_transition as validate_observed_transition,
};
mod effective_policy;
mod events;
pub(crate) mod evidence;
mod inputs;
mod permissions;
mod records;

#[cfg(test)]
pub(crate) use effective_policy::persist_event_with_effective_policy;
pub(crate) use effective_policy::{
    persist_transition_with_effective_policy, record_effective_policy,
};
#[cfg(test)]
pub(crate) use events::persist_event;
pub(crate) use events::persist_transition_in_transaction;
pub(crate) use events::{
    get_event, input_delivery_result_cause, latest_event_seq, persist_transition,
    update_interaction_evidence,
};
#[cfg(test)]
pub(crate) use inputs::input_payload_digest;
pub(crate) use inputs::{
    InputReservation, NewReservation, UNRESOLVED_INPUT, begin_deferred_attempt,
    has_unresolved_ordinary_attempt, requeue_zero_byte_immediate, reserve_input_with_provenance,
    settle_lost_deferred, settle_lost_deferred_in_transaction, settle_zero_byte_deferred,
    unpublished_delivery_results, versioned_payload_digest,
};
pub(crate) use inputs::{
    find_input, get_input, record_input_state, record_input_uncertain, settle_preview,
};
#[cfg(test)]
pub(crate) use inputs::{reserve_input, settle_input};
#[cfg(test)]
pub(crate) use permissions::release_permission_claim;
pub(crate) use permissions::{
    ZeroByteRetirement, get_permission_response, lookup_permission_response,
    permission_response_digest, reserve_permission_response,
};
pub(crate) use permissions::{rearm_permission_response, settle_any_input_in_transaction};
pub(crate) use records::*;

use std::ops::RangeInclusive;
use std::os::unix::fs::MetadataExt;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use super::records::MemberStatus;
use crate::error::CafleetError;
use crate::input::{Deadline, with_database_budget};
use crate::time::now_stamp;

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

pub(crate) fn get_run(conn: &Connection, run_id: i64) -> Result<RunRecord, CafleetError> {
    let run = conn
        .query_row(
            "SELECT * FROM member_runs WHERE run_id=?1",
            [run_id],
            map_run,
        )
        .optional()?
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
    effective_policy::validate(&run)?;
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
        super::members::member_fleet_status(conn, member_id)?
            .ok_or(CafleetError::from(rusqlite::Error::QueryReturnedNoRows))
    };
    let member = present(run.member_id)?;
    let director = present(run.director_member_id)?;
    let fleet = super::fleets::get_fleet(conn, run.fleet_id)?
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

#[cfg(test)]
pub(crate) fn create_run(conn: &mut Connection, new: &NewRun) -> Result<RunRecord, CafleetError> {
    let deadline = Deadline::inherited_or_new();
    with_database_budget(conn, deadline, |conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = insert_run(&tx, new)?;
        deadline.remaining()?;
        tx.commit()?;
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
    let now = now_stamp();
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
        )?;
    let run = get_run(conn, conn.last_insert_rowid())?;
    Ok(run)
}

pub(crate) fn reserve_event_sequences(
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
        )?;
        require(
            changed == 1,
            "member_runs.owner",
            "sequence reservation lost ownership",
        )?;
        Ok(first..=last)
    })
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

#[cfg(test)]
pub(crate) mod test_support {
    use std::ops::RangeInclusive;

    use rusqlite::Connection;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{self as storage, *};
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::db::test_support::{migrated_conn, table_rows};
    use crate::output::format_json;

    pub(crate) const NOW: &str = "2026-10-26T10:00:00.000000+00:00";
    pub(crate) const SECRET: &str = "fixture-secret-194-unique";

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
            let run = storage::create_run(&mut conn, &spec).unwrap();
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
                profile_version: None,
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

        pub(crate) fn reserve(&mut self, owner: &RunOwner) -> RangeInclusive<i64> {
            storage::reserve_event_sequences(&mut self.conn, owner).unwrap()
        }

        pub(crate) fn open(&mut self, owner: &RunOwner) -> RuntimeEventRecord {
            self.reserve(owner);
            let transition = opening(self.run.run_id, 1, 1, metadata());
            storage::persist_event(&mut self.conn, owner, &transition, PersistenceMode::Live)
                .unwrap()
        }

        pub(crate) fn input(
            &mut self,
            owner: &RunOwner,
            key: &str,
            kind: InputKind,
            message_id: Option<i64>,
        ) -> InputOperation {
            storage::reserve_input(
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
            "backend":"codex","native_version":"0.163.0","profile_version":"fixture-v1",
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

    use crate::coding_agent::{AnswerCapability, CodingAgent};
    use std::time::{Duration, Instant};

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
        AnswerCapability::fixture(
            CodingAgent::Codex,
            "0.163.0",
            "answer-fixture-v1",
            "native-tool-permission",
            "single-request-choice",
            decision,
            &["synthetic-storage-fixture".into()],
        )
        .unwrap()
    }

    impl ResponseFixture {
        pub(crate) fn new() -> Self {
            let mut base = Fixture::new();
            base.conn.execute("UPDATE member_runs SET coverage='tested',profile_version='fixture-v1' WHERE run_id=?1",[base.run.run_id]).unwrap();
            let owner = base.activate();
            let event = base.open(&owner);
            let request = NewPermissionResponse {
                operation_key: "permission:fixture-1".into(),
                metadata: PermissionResponseMetadata {
                    event_id: event.event_id,
                    interaction_id: event.interaction_id.unwrap(),
                    expected_owner_epoch: owner.owner_epoch,
                    request_token: "a".repeat(64),
                    decision: PermissionDecision::Allow,
                    director_member_id: base.director,
                    director_mode: DirectorMode::Auto,
                    decision_source: DecisionSource::DirectorAuto,
                    mode_evidence_ref: "fixture:host-mode".into(),
                    decision_evidence_ref: "fixture:decision".into(),
                    answer_profile_version: "answer-fixture-v1".into(),
                    answer_rule_id: "single-request-choice".into(),
                },
            };
            Self {
                base,
                owner,
                event,
                request,
                capability: capability(PermissionDecision::Allow),
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

    pub(crate) mod automatic_review {
        use super::*;
        pub(crate) fn annotation_raw(rule: &str, operation: Option<&str>) -> Value {
            json!({
                "backend":"codex","native_version":"0.163.0","profile_version":"fixture-v1",
                "rule_id":rule,"screen_seq":140,
                "operation":operation,"operation_rows":operation.map(|text| text.split('\n').count()),
                "choices":null,"native_request_id":null,
                "excerpt":format!("notice {SECRET}"),
                "truncated":false,"sensitive":false,
            })
        }

        pub(crate) fn refusal_raw(operation: &str) -> Value {
            annotation_raw("codex-auto-review-refusal", Some(operation))
        }

        pub(crate) fn unavailable_raw() -> Value {
            annotation_raw("codex-auto-review-unavailable", None)
        }

        pub(crate) fn redacted(raw: &Value) -> Value {
            evidence::EvidencePolicy::redacted(
                &json!({"schema_version":1,"patterns":[SECRET],"sensitive_rules":[]}),
                &["codex-auto-review-refusal", "codex-auto-review-unavailable"],
            )
            .unwrap()
            .apply(raw)
            .unwrap()
        }

        pub(crate) fn annotation_event(
            run: i64,
            event_seq: i64,
            kind: RuntimeEventKind,
            evidence: Value,
        ) -> ObservedTransition {
            ObservedTransition {
                run_id: run,
                event_seq,
                episode_seq: None,
                kind,
                classification: None,
                observed_at: NOW.into(),
                evidence_json: evidence,
                close_reason: None,
                cause_json: None,
            }
        }

        pub(crate) fn persisted(
            fixture: &mut Fixture,
            kind: RuntimeEventKind,
            evidence: Value,
        ) -> RuntimeEventRecord {
            let owner = fixture.activate();
            fixture.reserve(&owner);
            storage::persist_event(
                &mut fixture.conn,
                &owner,
                &annotation_event(owner.run_id, 1, kind, evidence),
                PersistenceMode::Live,
            )
            .unwrap()
        }

        pub(crate) fn redacted_fixture() -> Fixture {
            let fixture = Fixture::new();
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET event_evidence='redacted' WHERE run_id=?1",
                    [fixture.run.run_id],
                )
                .unwrap();
            fixture
        }

        pub(crate) fn inspect_argv(event: &RuntimeEventRecord) -> Value {
            json!([
                "cafleet",
                "event",
                "show",
                event.event_id.to_string(),
                "--json"
            ])
        }

        pub(crate) fn assert_runner_screen_event_without_interaction(
            fixture: &Fixture,
            event: &RuntimeEventRecord,
            kind: &str,
        ) {
            let payload = &event.payload_json;
            assert_eq!(payload["schema_version"], 2);
            assert_eq!(payload["kind"], kind);
            assert_eq!(payload["interaction_id"], Value::Null);
            assert_eq!(payload["classification"], Value::Null);
            assert_eq!(payload["source"], "runner_screen");
            assert_eq!(payload["native_request_id"], Value::Null);
            assert_eq!(payload["decision_outcome"], "unknown");
            assert_eq!(event.interaction_id, None);
            assert_eq!(event.notification_state, NotificationState::Inbox);
            assert_key_order(
                &payload["director_action"],
                &["kind", "instruction", "inspect_argv"],
            );
            assert_eq!(
                payload["director_action"]["inspect_argv"],
                inspect_argv(event)
            );
            let message =
                crate::broker::get_message(&fixture.conn, event.message_id.unwrap()).unwrap();
            assert_eq!(message.from_member_id, fixture.member);
            assert_eq!(message.to_member_id, fixture.director);
            assert_eq!(message.text, payload.to_string());
            assert!(rows(&fixture.conn, "member_interactions").is_empty());
            assert!(rows(&fixture.conn, "member_input_operations").is_empty());
            let view = storage::get_event(&fixture.reopen(), event.event_id).unwrap();
            assert!(view.interaction.is_none());
            assert_eq!(view.event.payload_json, *payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use serde_json::json;

    use super::test_support::*;
    use super::{self as storage, *};
    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;

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
        for mismatch in ["member", "fleet", "director", "database"] {
            let mut fixture = Fixture::new();
            let (other_fleet, other_director) = create_fleet_fixture(&mut fixture.conn, "foreign");
            let member = register(&mut fixture.conn, fixture.fleet, "new-worker", None);
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
                storage::create_run(&mut fixture.conn, &spec).is_err(),
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
        let run = storage::create_run(&mut fixture.conn, &spec).unwrap();
        assert_eq!(run.requested_mode.as_deref(), Some("on-request"));
        assert_eq!(run.mode_source, ModeSource::Explicit);
        assert_eq!(run.permission_argv_json, spec.permission_argv_json);
        assert_eq!(run.effective_policy_json, None);
    }

    #[test]
    fn typed_runtime_readers_fail_on_corrupt_required_values() {
        for (field, value) in [
            ("status", "invented"),
            ("owner_kind", "model"),
            ("coverage", "guaranteed"),
            ("database_identity_json", "[]"),
            ("permission_argv_json", "{}"),
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
            storage::persist_event(&mut other, &owner, &transition, PersistenceMode::Live).is_ok()
        );
    }

    #[test]
    fn stale_owner_kind_epoch_or_database_identity_fences_every_mutation() {
        for mismatch in ["epoch", "kind", "database"] {
            let mut fixture = Fixture::new();
            let owner = fixture.activate();
            fixture.reserve(&owner);
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
                storage::persist_event(
                    &mut fixture.conn,
                    &stale,
                    &opening(owner.run_id, 1, 1, metadata()),
                    PersistenceMode::Live
                )
                .is_err(),
                "{mismatch}"
            );
            assert!(
                storage::reserve_input(
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
}
