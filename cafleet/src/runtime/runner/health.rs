use super::*;
use crate::broker::members::db_err;
use crate::broker::runtime::{self as storage, OwnerKind, RunRecord, RunStatus};
use rusqlite::{Connection, params};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HealthSnapshot {
    pub identity: ControlIdentity,
    pub runner: ProcessIdentity,
    pub child: Option<ProcessIdentity>,
    pub output_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationHealth>,
}
impl HealthSnapshot {
    pub(crate) fn decode(value: Value, run: &RunRecord) -> Result<Self> {
        let result: Self = serde_json::from_value(value).map_err(io_error)?;
        result.validate(run)?;
        Ok(result)
    }
    pub(crate) fn validate(&self, run: &RunRecord) -> Result<()> {
        self.identity.validate(run)?;
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        ensure(
            run.runner_pid == Some(self.runner.pid)
                && run.runner_start_identity.as_deref()
                    == Some(self.runner.start_identity.as_str())
                && run.owner_pid == self.runner.pid
                && run.owner_start_identity == self.runner.start_identity
                && self.runner.matches()?,
            "health runner generation mismatch",
        )?;
        match &self.child {
            Some(child) => ensure(
                run.child_pid == Some(child.pid)
                    && run.child_start_identity.as_deref() == Some(child.start_identity.as_str())
                    && child.matches()?,
                "health child generation mismatch",
            ),
            None => ensure(
                run.child_pid.is_none(),
                "health omitted native child identity",
            ),
        }
    }
}
pub(crate) trait HealthProbe {
    fn probe(&mut self, run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot>;
}
pub(crate) struct HealthCheck {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "wrapped delivery integration consumes the matching health result"
        )
    )]
    pub matched: bool,
}
struct SocketProbe;
impl HealthProbe for SocketProbe {
    fn probe(&mut self, run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot> {
        probe_health(run, deadline)
    }
}
pub(crate) fn probe_health(run: &RunRecord, deadline: Instant) -> Result<HealthSnapshot> {
    let reply = super::service::request(run, "health", deadline)?;
    HealthSnapshot::decode(reply, run)
}
pub(crate) fn check_input_health(
    conn: &mut Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<HealthCheck> {
    check_input_health_with_probe(conn, run_id, deadline, &mut SocketProbe)
}
pub(crate) fn check_input_health_with_probe(
    conn: &mut Connection,
    run_id: i64,
    deadline: Instant,
    probe: &mut impl HealthProbe,
) -> Result<HealthCheck> {
    check_deadline(Instant::now(), deadline)?;
    let run = with_database_deadline(conn, deadline, |conn| storage::get_run(conn, run_id))?;
    let probed = probe.probe(&run, deadline);
    let matched = probed
        .and_then(|health| {
            health.validate(&run)?;
            validate_run_paths(conn, &run)?;
            ensure(
                run.owner_kind == OwnerKind::Runner
                    && matches!(run.status, RunStatus::Running | RunStatus::Degraded),
                "runtime is not accepting input health checks",
            )?;
            Ok(())
        })
        .is_ok();
    with_database_deadline(conn, deadline, |conn| {
        validate_database(conn, &run.database_identity_json)?;
        let changed=conn.execute("UPDATE member_runs SET reachability=?1,last_health_at=?2 WHERE run_id=?3 AND owner_epoch=?4 AND owner_kind=?5 AND owner_pid=?6 AND owner_start_identity=?7",params![if matched{"reachable"}else{"suspect"},crate::time::format_utc(crate::time::now_utc()),run_id,run.owner_epoch,run.owner_kind.as_str(),run.owner_pid,run.owner_start_identity]).map_err(db_err)?;
        ensure(changed == 1, "input health owner changed during probe")
    })?;
    Ok(HealthCheck { matched })
}
pub(crate) struct RuntimeDiagnosis {
    pub diagnostics: Vec<String>,
}
pub(crate) fn diagnose_runtime(
    conn: &Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    let source = conn
        .path()
        .ok_or_else(|| error("runtime diagnosis requires a file-backed registry"))?;
    let mut observed = crate::db::connect_read_only(&format!("sqlite:///{source}"))?;
    with_database_deadline(&mut observed, deadline, |conn| {
        diagnose_runtime_observed(conn, run_id, deadline)
    })
}
fn diagnose_runtime_observed(
    conn: &Connection,
    run_id: i64,
    deadline: Instant,
) -> Result<RuntimeDiagnosis> {
    check_deadline(Instant::now(), deadline)?;
    let run = storage::get_run(conn, run_id)?;
    let mut diagnostics = Vec::new();
    match validate_run_paths(conn, &run) {
        Ok(paths) => {
            if paths.descriptor.try_exists().map_err(io_error)? {
                diagnostics.push(
                    "leftover launch descriptor; use member recover after ownership validation"
                        .into(),
                );
            }
            match RunLock::acquire(conn,&run.owner()){
                Ok(Some(_))=>diagnostics.push("matching owner lock is currently available; explicit recovery establishes loss".into()),
                Ok(None)=>diagnostics.push("matching owner lock held: live_or_unreachable".into()),
                Err(cause)=>diagnostics.push(cause.message().into_owned()),
            }
            if Instant::now() < deadline {
                match probe_health(&run, deadline) {
                    Ok(_) => diagnostics.push("matching runtime health reply observed".into()),
                    Err(cause) => diagnostics.push(format!(
                        "runtime endpoint unavailable: {cause}; inspect with member recover"
                    )),
                }
            }
        }
        Err(cause) => diagnostics.push(format!("{cause}; explicit member recover required")),
    }
    Ok(RuntimeDiagnosis { diagnostics })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationHealth {
    pub visibility: Visibility,
    pub eligibility: InputEligibility,
    pub pending_count: u64,
    pub pending_evidence: bool,
    pub gap_count: u64,
    pub degraded: bool,
    pub diagnostic: Option<String>,
}
impl ObservationHealth {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure(
            self.pending_count <= 130,
            "health pending count exceeds capacity",
        )?;
        ensure(
            serde_json::to_vec(self).map_err(io_error)?.len() <= 1024,
            "health observation exceeds encoded limit",
        )?;
        if let Some(code) = &self.diagnostic {
            ensure(
                code.len() <= 128
                    && code.is_ascii()
                    && [
                        "parser_unavailable",
                        "detector_unavailable",
                        "sequence_unavailable",
                        "sink_unavailable",
                        "observation_gap",
                    ]
                    .contains(&code.as_str()),
                "invalid health diagnostic",
            )?;
        }
        if self.eligibility == InputEligibility::ComposerEmpty {
            ensure(
                self.visibility == Visibility::NotVisible
                    && !self.degraded
                    && self.pending_count == 0
                    && !self.pending_evidence,
                "health composer proof conflicts with held state",
            )?;
        }
        Ok(())
    }
}
