use super::*;
use crate::broker::members::db_err;
use crate::broker::records::{Placement, RegisteredMember};
use crate::broker::runtime::{self as storage, *};
use crate::broker::{self, NewPlacement};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::time::Instant;

pub(crate) struct PendingMemberSpec {
    pub fleet_id: i64,
    pub name: String,
    pub description: String,
    pub placement: NewPlacement,
}
pub(crate) struct RunSettings {
    pub database_identity_json: Value,
    pub creator: ProcessIdentity,
    pub launch_nonce: String,
    pub requested_mode: Option<String>,
    pub resolved_mode: String,
    pub mode_source: ModeSource,
    pub permission_argv_json: Value,
    pub event_evidence: EventEvidence,
    pub native_version: Option<String>,
    pub profile_version: Option<String>,
    pub coverage: Coverage,
}
pub(crate) struct PendingRegistration {
    pub member: RegisteredMember,
    pub run: RunRecord,
    pub paths: RuntimePaths,
}

pub(crate) fn register_pending_run(
    conn: &mut Connection,
    member: &PendingMemberSpec,
    settings: &RunSettings,
    deadline: Instant,
) -> Result<PendingRegistration> {
    with_database_deadline(conn, deadline, |conn| {
        validate_database(conn, &settings.database_identity_json)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let fleet = broker::fleets::require_live_fleet(&tx, member.fleet_id)?;
        let director = fleet
            .director_member_id
            .ok_or_else(|| error("live fleet requires a Director"))?;
        let registered = broker::members::register_member_rows(
            &tx,
            member.fleet_id,
            &member.name,
            &member.description,
            Some(&member.placement),
        )?;
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1",
                [],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        let paths = RuntimePaths::for_run(&settings.database_identity_json, next)?;
        let run = storage::insert_run(
            &tx,
            &NewRun {
                member_id: registered.member_id,
                fleet_id: member.fleet_id,
                director_member_id: director,
                backend: member.placement.coding_agent.clone(),
                database_identity_json: settings.database_identity_json.clone(),
                launch_nonce: settings.launch_nonce.clone(),
                runtime_path: paths.run_dir.to_string_lossy().into_owned(),
                requested_mode: settings.requested_mode.clone(),
                resolved_mode: settings.resolved_mode.clone(),
                mode_source: settings.mode_source,
                permission_argv_json: settings.permission_argv_json.clone(),
                event_evidence: settings.event_evidence,
                native_version: settings.native_version.clone(),
                profile_version: settings.profile_version.clone(),
                coverage: settings.coverage,
                owner_pid: settings.creator.pid,
                owner_start_identity: settings.creator.start_identity.clone(),
            },
        )?;
        ensure(
            run.run_id == next,
            "atomic run allocation changed its reserved ID",
        )?;
        tx.commit().map_err(db_err)?;
        Ok(PendingRegistration {
            member: registered,
            run,
            paths,
        })
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControlIdentity {
    pub database_identity_json: Value,
    pub fleet_id: i64,
    pub member_id: i64,
    pub director_member_id: i64,
    pub run_id: i64,
    pub owner_epoch: i64,
    pub launch_nonce: String,
}
impl ControlIdentity {
    pub(crate) fn from_run(run: &RunRecord) -> Self {
        Self {
            database_identity_json: run.database_identity_json.clone(),
            fleet_id: run.fleet_id,
            member_id: run.member_id,
            director_member_id: run.director_member_id,
            run_id: run.run_id,
            owner_epoch: run.owner_epoch,
            launch_nonce: run.launch_nonce.clone(),
        }
    }
    pub(crate) fn validate(&self, run: &RunRecord) -> Result<()> {
        ensure(
            *self == Self::from_run(run),
            "control run/database/nonce/epoch identity mismatch",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Prepared {
    pub identity: ControlIdentity,
    pub runner: ProcessIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Started {
    pub identity: ControlIdentity,
    pub runner: ProcessIdentity,
    pub child: ProcessIdentity,
}
pub(crate) trait LaunchHost {
    fn now(&self) -> Instant;
    fn current_executable(&self) -> Result<PathBuf>;
    fn split_runner(
        &mut self,
        argv: &[String],
        environment: &[(String, String)],
        deadline: Instant,
    ) -> Result<String>;
    fn wait_prepared(
        &mut self,
        descriptor: &LaunchDescriptor,
        deadline: Instant,
    ) -> Result<Prepared>;
    fn start(
        &mut self,
        prepared: &Prepared,
        placement: &Placement,
        deadline: Instant,
    ) -> Result<Started>;
    fn shutdown_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome>;
    fn close_pane(&mut self, pane_id: &str, deadline: Instant) -> Result<()>;
}
pub(crate) struct LaunchOutcome {
    pub member: RegisteredMember,
    pub run: RunRecord,
    pub placement: Placement,
    pub started: Started,
}

fn pending_failure(
    conn: &mut Connection,
    run: &RunRecord,
    clock: ShutdownClock,
    diagnostics: &[String],
) -> Result<()> {
    with_database_deadline(conn, clock.storage_deadline(), |conn| {
        validate_database(conn, &run.database_identity_json)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_err)?;
        tx.execute("UPDATE member_runs SET status='failed',ended_at=COALESCE(ended_at,?1),cleanup_state=?2,cleanup_diagnostics_json=?3 WHERE run_id=?4 AND launch_nonce=?5",params![crate::time::format_utc(crate::time::now_utc()),if diagnostics.is_empty(){"complete"}else{"incomplete"},serde_json::to_string(diagnostics).map_err(io_error)?,run.run_id,run.launch_nonce]).map_err(db_err)?;
        broker::members::deregister_member_rows(&tx, run.member_id)?;
        tx.commit().map_err(db_err)
    })
}
fn remove_pending_descriptor(conn: &Connection, run: &RunRecord) -> Result<()> {
    if run.runtime_dir_identity_json.is_none() {
        return Ok(());
    }
    RunArtifacts::from_run(conn, run)?.remove_descriptor()
}
struct PendingGuard<'a> {
    conn: &'a mut Connection,
    run: RunRecord,
    armed: bool,
}
impl PendingGuard<'_> {
    fn rollback(
        &mut self,
        primary: CafleetError,
        clock: ShutdownClock,
        mut diagnostics: Vec<String>,
    ) -> CafleetError {
        self.armed = false;
        let current = match storage::get_run(self.conn, self.run.run_id) {
            Ok(run) => run,
            Err(cause) => {
                diagnostics.push(cause.message().into_owned());
                self.run.clone()
            }
        };
        if let Err(cause) = remove_pending_descriptor(self.conn, &current) {
            diagnostics.push(cause.message().into_owned());
        }
        if let Err(cause) = pending_failure(self.conn, &current, clock, &diagnostics) {
            diagnostics.push(cause.message().into_owned());
        }
        diagnostics
            .into_iter()
            .fold(primary, |primary, note| primary.with_cleanup(note))
    }
}
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let cause = self.rollback(
                error("pending creation was interrupted"),
                ShutdownClock::new(Instant::now()),
                Vec::new(),
            );
            eprintln!("{cause}");
        }
    }
}
pub(crate) fn create_registered(
    conn: &mut Connection,
    member: &PendingMemberSpec,
    settings: &RunSettings,
    prepare: impl FnOnce(&mut Connection, &PendingRegistration) -> Result<(RunArtifacts, SpawnSpec)>,
    host: &mut impl LaunchHost,
    deadline: Instant,
) -> Result<LaunchOutcome> {
    let pending = register_pending_run(conn, member, settings, deadline)?;
    let mut guard = PendingGuard {
        conn,
        run: pending.run.clone(),
        armed: true,
    };
    let (mut artifacts, spawn) = match prepare(guard.conn, &pending) {
        Ok(value) => value,
        Err(cause) => {
            return Err(guard.rollback(cause, ShutdownClock::new(host.now()), Vec::new()));
        }
    };
    guard.armed = false;
    launch_registered(guard.conn, pending, &mut artifacts, &spawn, host, deadline)
}

pub(crate) fn launch_registered(
    conn: &mut Connection,
    pending: PendingRegistration,
    artifacts: &mut RunArtifacts,
    spawn: &SpawnSpec,
    host: &mut impl LaunchHost,
    deadline: Instant,
) -> Result<LaunchOutcome> {
    let mut guard = PendingGuard {
        conn,
        run: pending.run.clone(),
        armed: true,
    };
    let mut pane = None;
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_deadline(host.now(), deadline)?;
        let run = storage::get_run(guard.conn, pending.run.run_id)?;
        ensure(
            pending.paths.run_dir == artifacts.paths.run_dir,
            "prepared artifacts belong to a different pending run",
        )?;
        validate_run_paths(guard.conn, &run)?;
        let descriptor = LaunchDescriptor::new(&run, spawn.clone())?;
        if artifacts.paths.descriptor.try_exists().map_err(io_error)? {
            let existing = load_descriptor(guard.conn, &artifacts.paths.descriptor)?;
            ensure(
                serde_json::to_value(existing).map_err(io_error)?
                    == serde_json::to_value(&descriptor).map_err(io_error)?,
                "pending launch descriptor differs from prepared inputs",
            )?;
        } else {
            artifacts.write_descriptor(&descriptor)?;
        }
        let executable = host.current_executable()?;
        ensure(
            executable.is_absolute(),
            "absolute CAFleet executable required",
        )?;
        let argv = vec![
            executable
                .to_str()
                .ok_or_else(|| error("CAFleet executable requires UTF-8 path"))?
                .into(),
            "__member-run".into(),
            "--descriptor".into(),
            artifacts
                .paths
                .descriptor
                .to_str()
                .ok_or_else(|| error("descriptor requires UTF-8 path"))?
                .into(),
        ];
        let environment = vec![(
            "CAFLEET_DATABASE_URL".into(),
            format!(
                "sqlite:///{}",
                run.database_identity_json["canonical_path"]
                    .as_str()
                    .expect("validated database path")
            ),
        )];
        pane = Some(host.split_runner(&argv, &environment, deadline)?);
        check_deadline(host.now(), deadline)?;
        let prepared = host.wait_prepared(&descriptor, deadline)?;
        check_deadline(host.now(), deadline)?;
        let current = with_database_deadline(guard.conn, deadline, |conn| {
            storage::get_run(conn, run.run_id)
        })?;
        prepared.identity.validate(&current)?;
        ensure(
            current.owner_epoch
                == run
                    .owner_epoch
                    .checked_add(1)
                    .ok_or_else(|| error("owner epoch exhausted"))?
                && current.owner_kind == OwnerKind::Runner
                && current.owner_pid == prepared.runner.pid
                && current.owner_start_identity == prepared.runner.start_identity
                && current.child_pid.is_none(),
            "prepared owner or child state mismatch",
        )?;
        let placement = with_database_deadline(guard.conn, deadline, |conn| {
            validate_run_paths(conn, &current)?;
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(db_err)?;
            ensure(
                broker::active_member_fleet(&tx, run.member_id)? == Some(run.fleet_id),
                "pending member became inactive",
            )?;
            let changed=tx.execute("UPDATE member_runs SET status='prepared',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='starting'",params![crate::time::format_utc(crate::time::now_utc()),run.run_id,current.owner_epoch]).map_err(db_err)?;
            ensure(changed == 1, "prepared publication lost run ownership")?;
            let changed=tx.execute("UPDATE member_placements SET mux_pane_id=?1 WHERE member_id=?2 AND mux_pane_id IS NULL",params![pane.as_ref().expect("created pane"),run.member_id]).map_err(db_err)?;
            ensure(
                changed == 1,
                "pending placement vanished or changed before start",
            )?;
            tx.commit().map_err(db_err)?;
            broker::get_member(conn, run.member_id, run.fleet_id)?
                .and_then(|member| member.placement)
                .ok_or_else(|| error("published placement vanished"))
        })?;
        let started = host.start(&prepared, &placement, deadline)?;
        check_deadline(host.now(), deadline)?;
        let current = with_database_deadline(guard.conn, deadline, |conn| {
            storage::get_run(conn, run.run_id)
        })?;
        started.identity.validate(&current)?;
        ensure(
            started.runner == prepared.runner
                && current.runner_pid == Some(started.runner.pid)
                && current.runner_start_identity.as_deref()
                    == Some(started.runner.start_identity.as_str())
                && current.child_pid == Some(started.child.pid)
                && current.child_start_identity.as_deref()
                    == Some(started.child.start_identity.as_str())
                && current.started_at.is_some()
                && (matches!(current.status, RunStatus::Running | RunStatus::Degraded)
                    || current.status == RunStatus::Exited),
            "started reply lacks a matching completed native spawn",
        )?;
        artifacts.remove_descriptor()?;
        Ok((current, placement, started))
    }));
    let (primary, unwind) = match attempt {
        Ok(Ok((run, placement, started))) => {
            guard.armed = false;
            return Ok(LaunchOutcome {
                member: pending.member,
                run,
                placement,
                started,
            });
        }
        Ok(Err(primary)) => (primary, None),
        Err(payload) => (
            error("native launch unwound; performing owned compensation"),
            Some(payload),
        ),
    };
    let clock = ShutdownClock::new(host.now());
    let mut notes = Vec::new();
    match storage::get_run(guard.conn, pending.run.run_id) {
        Ok(current) if current.owner_kind == OwnerKind::Runner => {
            let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                host.shutdown_owned(&current, clock)
            }));
            match cleanup {
                Ok(Ok(outcome)) => {
                    notes.extend(outcome.diagnostics);
                    if outcome.cleanup_state == CleanupState::Incomplete && notes.is_empty() {
                        notes.push("owned child cleanup remains unconfirmed".into());
                    }
                }
                Ok(Err(cause)) => notes.push(cause.message().into_owned()),
                Err(_) => notes
                    .push("owned child cleanup panicked; explicit reconciliation required".into()),
            }
        }
        Err(cause) => notes.push(cause.message().into_owned()),
        _ => {}
    }
    if let Some(pane) = pane {
        let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.close_pane(&pane, clock.storage_deadline())
        }));
        match cleanup {
            Ok(Ok(())) => {}
            Ok(Err(cause)) => notes.push(cause.message().into_owned()),
            Err(_) => {
                notes.push("owned pane cleanup panicked; explicit reconciliation required".into())
            }
        }
    }
    let failure = guard.rollback(primary, clock, notes);
    if let Some(payload) = unwind {
        eprintln!("{failure}");
        std::panic::resume_unwind(payload);
    }
    Err(failure)
}

pub(crate) fn check_deadline(now: Instant, deadline: Instant) -> Result<()> {
    if now >= deadline {
        Err(crate::error::CafleetError::DeadlineExceeded)
    } else {
        Ok(())
    }
}
