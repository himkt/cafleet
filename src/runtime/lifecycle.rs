use crate::broker;
use crate::broker::NewPlacement;
use crate::broker::Placement;
use crate::broker::RegisteredMember;
use crate::broker::runtime as storage;
use crate::broker::runtime::CleanupState;
use crate::broker::runtime::RunRecord;
use crate::broker::runtime::*;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MultiplexerContext;
use crate::runtime::control::ControlClient;
use crate::runtime::control::pause;
use crate::runtime::control::request;
use crate::runtime::*;
use rusqlite::Connection;
use rusqlite::params;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ChildExit {
    Code(i32),
    Signal(i32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShutdownClock {
    start: Instant,
    unit: Duration,
    limit: Option<Instant>,
}

impl ShutdownClock {
    pub(crate) fn new(start: Instant) -> Self {
        Self::with_unit(start, Duration::from_secs(1))
    }
    pub(crate) fn with_unit(start: Instant, unit: Duration) -> Self {
        Self {
            start,
            unit,
            limit: None,
        }
    }
    pub(crate) fn clipped(self, deadline: Instant) -> Self {
        Self {
            limit: Some(self.limit.map_or(deadline, |prior| prior.min(deadline))),
            ..self
        }
    }
    fn boundary(&self, units: u32) -> Instant {
        let original = self.start + self.unit * units;
        self.limit.map_or(original, |limit| original.min(limit))
    }
    pub(crate) fn child_deadline(&self) -> Instant {
        self.boundary(2)
    }
    pub(crate) fn display_deadline(&self) -> Instant {
        self.boundary(3)
    }
    pub(crate) fn storage_deadline(&self) -> Instant {
        self.boundary(4)
    }
    pub(crate) fn remaining_storage_at(&self, now: Instant) -> Duration {
        self.storage_deadline().saturating_duration_since(now)
    }
}

pub(crate) fn runner_exit_status(native: Option<ChildExit>, complete: bool) -> Result<i32> {
    let status = match native {
        Some(ChildExit::Code(code)) => {
            ensure(code >= 0, "negative native exit code")?;
            code
        }
        Some(ChildExit::Signal(signal)) => {
            ensure(signal > 0, "positive native signal required")?;
            128i32
                .checked_add(signal)
                .ok_or_else(|| error("native exit signal overflow"))?
        }
        None => {
            ensure(!complete, "complete cleanup requires actual native status")?;
            1
        }
    };
    Ok(if !complete && status == 0 { 1 } else { status })
}

pub(crate) use crate::db::with_database_deadline;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ShutdownOutcome {
    pub(crate) native_exit: Option<ChildExit>,
    pub(crate) cleanup_state: CleanupState,
    pub(crate) diagnostics: Vec<String>,
    pub(crate) dropped_display_bytes: u64,
    pub(crate) pending_event_frames: u64,
    pub(crate) unsaved_observation_gap: u64,
}

impl ShutdownOutcome {
    /// An outcome whose cleanup is complete exactly when no diagnostic was recorded.
    pub(crate) fn new(native_exit: Option<ChildExit>, diagnostics: Vec<String>) -> Self {
        Self {
            native_exit,
            cleanup_state: if diagnostics.is_empty() {
                CleanupState::Complete
            } else {
                CleanupState::Incomplete
            },
            diagnostics,
            dropped_display_bytes: 0,
            pending_event_frames: 0,
            unsaved_observation_gap: 0,
        }
    }

    pub(crate) fn complete(native_exit: Option<ChildExit>) -> Self {
        Self::new(native_exit, Vec::new())
    }

    /// Records `diagnostic` and marks the cleanup incomplete.
    pub(crate) fn fail(&mut self, diagnostic: impl Into<String>) {
        self.diagnostics.push(diagnostic.into());
        self.cleanup_state = CleanupState::Incomplete;
    }
}

/// Runs `cleanup` through an unwind guard, recording its error or `panicked` in `notes`.
fn guarded<T>(
    notes: &mut Vec<String>,
    panicked: &str,
    cleanup: impl FnOnce() -> Result<T>,
) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(cleanup)) {
        Ok(result) => note(notes, result),
        Err(_) => {
            notes.push(panicked.into());
            None
        }
    }
}

/// The value of `result`, or `None` after recording its error in `diagnostics`.
pub(crate) fn note<T>(diagnostics: &mut Vec<String>, result: Result<T>) -> Option<T> {
    result
        .map_err(|cause| diagnostics.push(cause.message().into_owned()))
        .ok()
}

pub(crate) trait ShutdownHost {
    fn now(&self) -> Instant;
    fn gate_programmatic_input(&mut self);
    fn term_verified_descendants(&mut self) -> Result<()>;
    fn drain_and_reap_until(&mut self, deadline: Instant) -> Result<Option<ChildExit>>;
    fn kill_verified_survivors(&mut self) -> Result<()>;
    fn close_pty(&mut self) -> Result<()>;
    fn final_native_exit(&mut self) -> Result<Option<ChildExit>>;
    fn native_was_spawned(&self) -> bool;
    fn flush_display_until(&mut self, deadline: Instant) -> Result<u64>;
    fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64)>;
    fn restore_terminal(&mut self) -> Result<()>;
}

pub(crate) fn shutdown(
    host: &mut impl ShutdownHost,
    clock: ShutdownClock,
    native: Option<ChildExit>,
) -> ShutdownOutcome {
    let mut outcome = ShutdownOutcome::complete(native);
    fn reconcile_exit(outcome: &mut ShutdownOutcome, exit: Option<ChildExit>, conflict: &str) {
        let Some(exit) = exit else {
            return;
        };
        match outcome.native_exit {
            Some(recorded) if recorded != exit => outcome.diagnostics.push(conflict.into()),
            Some(_) => {}
            None => outcome.native_exit = Some(exit),
        }
    }
    host.gate_programmatic_input();
    note(&mut outcome.diagnostics, host.term_verified_descendants());
    let drained = note(
        &mut outcome.diagnostics,
        host.drain_and_reap_until(clock.child_deadline()),
    )
    .flatten();
    reconcile_exit(&mut outcome, drained, "conflicting native exit status");
    note(&mut outcome.diagnostics, host.kill_verified_survivors());
    note(&mut outcome.diagnostics, host.close_pty());
    let final_exit = note(&mut outcome.diagnostics, host.final_native_exit()).flatten();
    reconcile_exit(
        &mut outcome,
        final_exit,
        "conflicting final native exit status",
    );
    if let Some(dropped) = note(
        &mut outcome.diagnostics,
        host.flush_display_until(clock.display_deadline()),
    ) {
        outcome.dropped_display_bytes = dropped;
    }
    if let Some((pending, gap)) = note(
        &mut outcome.diagnostics,
        host.finish_events_until(clock.storage_deadline()),
    ) {
        outcome.pending_event_frames = pending;
        outcome.unsaved_observation_gap = gap;
    }
    note(&mut outcome.diagnostics, host.restore_terminal());
    if host.now() > clock.storage_deadline() {
        outcome
            .diagnostics
            .push("shutdown exceeded its original storage deadline".into());
    }
    if outcome.dropped_display_bytes > 0 {
        outcome.diagnostics.push(format!(
            "display truncated: {} undelivered bytes",
            outcome.dropped_display_bytes
        ));
    }
    if outcome.pending_event_frames > 0 || outcome.unsaved_observation_gap > 0 {
        outcome.diagnostics.push(format!(
            "pending events={} unsaved observation gap={}",
            outcome.pending_event_frames, outcome.unsaved_observation_gap
        ));
    }
    if outcome.native_exit.is_none() && host.native_was_spawned() {
        outcome
            .diagnostics
            .push("native exit remains unconfirmed".into());
    }
    if !outcome.diagnostics.is_empty() {
        outcome.cleanup_state = CleanupState::Incomplete;
    }
    outcome
}

pub(crate) struct PendingMemberSpec {
    pub(crate) fleet_id: i64,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) placement: NewPlacement,
}
pub(crate) struct RunSettings {
    pub(crate) database_identity_json: Value,
    pub(crate) creator: ProcessIdentity,
    pub(crate) launch_nonce: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
    pub(crate) event_evidence: EventEvidence,
    pub(crate) native_version: Option<String>,
    pub(crate) coverage: Coverage,
}
pub(crate) struct PendingRegistration {
    pub(crate) member: RegisteredMember,
    pub(crate) run: RunRecord,
    pub(crate) paths: RuntimePaths,
}

pub(super) fn register_pending_run(
    conn: &mut Connection,
    member: &PendingMemberSpec,
    settings: &RunSettings,
    deadline: Instant,
) -> Result<PendingRegistration> {
    with_database_deadline(conn, deadline, |conn| {
        ensure_same_database(conn, &settings.database_identity_json)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let fleet = broker::fleets::require_live_fleet(&tx, member.fleet_id)?;
        let director = fleet.director_member_id;
        let registered = broker::members::register_member_rows(
            &tx,
            member.fleet_id,
            &member.name,
            &member.description,
            &member.placement,
        )?;
        let next: i64 = tx.query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1",
            [],
            |row| row.get(0),
        )?;
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
                coverage: settings.coverage,
                owner_pid: settings.creator.pid,
                owner_start_identity: settings.creator.start_identity.clone(),
            },
        )?;
        ensure(
            run.run_id == next,
            "atomic run allocation changed its reserved ID",
        )?;
        tx.commit()?;
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
    pub(crate) database_identity_json: Value,
    pub(crate) fleet_id: i64,
    pub(crate) member_id: i64,
    pub(crate) director_member_id: i64,
    pub(crate) run_id: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) launch_nonce: String,
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
    pub(crate) identity: ControlIdentity,
    pub(crate) runner: ProcessIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Started {
    pub(crate) identity: ControlIdentity,
    pub(crate) runner: ProcessIdentity,
    pub(crate) child: ProcessIdentity,
}
pub(crate) trait LaunchHost {
    fn now(&self) -> Instant;
    fn current_executable(&self) -> Result<PathBuf>;
    fn split_runner(&mut self, argv: &[String], deadline: Instant) -> Result<String>;
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
    fn settle_hermes(
        &mut self,
        _conn: &Connection,
        run: &RunRecord,
        _pane_id: &str,
        _deadline: Instant,
    ) -> Result<()> {
        Err(error(format!(
            "hermes settlement of run {} requires the member's multiplexer",
            run.run_id
        )))
    }
}
pub(crate) struct LaunchOutcome {
    pub(crate) member: RegisteredMember,
    pub(crate) run: RunRecord,
    pub(crate) placement: Placement,
    pub(crate) started: Started,
}

fn pending_failure(
    conn: &mut Connection,
    run: &RunRecord,
    clock: ShutdownClock,
    diagnostics: &[String],
) -> Result<()> {
    with_database_deadline(conn, clock.storage_deadline(), |conn| {
        ensure_same_database(conn, &run.database_identity_json)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE member_runs SET status='failed',ended_at=COALESCE(ended_at,?1),cleanup_state=?2,cleanup_diagnostics_json=?3 WHERE run_id=?4 AND launch_nonce=?5",params![crate::time::now_stamp(),if diagnostics.is_empty(){"complete"}else{"incomplete"},serde_json::to_string(diagnostics).map_err(io_error)?,run.run_id,run.launch_nonce])?;
        broker::members::deregister_member_rows(&tx, run.member_id)?;
        tx.commit().map_err(crate::error::CafleetError::from)
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
        let current = note(
            &mut diagnostics,
            storage::get_run(self.conn, self.run.run_id),
        )
        .unwrap_or_else(|| self.run.clone());
        note(
            &mut diagnostics,
            remove_pending_descriptor(self.conn, &current),
        );
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
    launch_registered(&mut guard, pending, &mut artifacts, &spawn, host, deadline)
}

fn launch_registered(
    guard: &mut PendingGuard<'_>,
    pending: PendingRegistration,
    artifacts: &mut RunArtifacts,
    spawn: &SpawnSpec,
    host: &mut impl LaunchHost,
    deadline: Instant,
) -> Result<LaunchOutcome> {
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
            "--database-url".into(),
            format!(
                "sqlite:///{}",
                run.database_identity_json["canonical_path"]
                    .as_str()
                    .expect("validated database path")
            ),
            "__member-run".into(),
            "--descriptor".into(),
            artifacts
                .paths
                .descriptor
                .to_str()
                .ok_or_else(|| error("descriptor requires UTF-8 path"))?
                .into(),
        ];
        pane = Some(host.split_runner(&argv, deadline)?);
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
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            ensure(
                broker::active_member_fleet(&tx, run.member_id)? == Some(run.fleet_id),
                "pending member became inactive",
            )?;
            let changed=tx.execute("UPDATE member_runs SET status='prepared',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='starting'",params![crate::time::now_stamp(),run.run_id,current.owner_epoch])?;
            ensure(changed == 1, "prepared publication lost run ownership")?;
            let changed=tx.execute("UPDATE member_placements SET mux_pane_id=?1 WHERE member_id=?2 AND mux_pane_id IS NULL",params![pane.as_ref().expect("created pane"),run.member_id])?;
            ensure(
                changed == 1,
                "pending placement vanished or changed before start",
            )?;
            tx.commit()?;
            broker::get_member(conn, run.member_id, run.fleet_id)?
                .map(|member| member.placement)
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
                && matches!(
                    current.status,
                    RunStatus::Running | RunStatus::Degraded | RunStatus::Exited
                ),
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
            if current.backend == crate::coding_agent::CodingAgent::Hermes.name()
                && current.child_pid.is_some()
                && let Some(pane_id) = pane.as_deref()
            {
                let until = crate::coding_agent::hermes::settlement_deadline().instant();
                note(
                    &mut notes,
                    host.settle_hermes(guard.conn, &current, pane_id, until),
                );
            }
            let panicked = "owned child cleanup panicked; explicit reconciliation required";
            if let Some(outcome) = guarded(&mut notes, panicked, || {
                host.shutdown_owned(&current, clock)
            }) {
                notes.extend(outcome.diagnostics);
                if outcome.cleanup_state == CleanupState::Incomplete && notes.is_empty() {
                    notes.push("owned child cleanup remains unconfirmed".into());
                }
            }
        }
        Err(cause) => notes.push(cause.message().into_owned()),
        _ => {}
    }
    if let Some(pane) = pane {
        let panicked = "owned pane cleanup panicked; explicit reconciliation required";
        guarded(&mut notes, panicked, || {
            host.close_pane(&pane, clock.storage_deadline())
        });
    }
    let failure = guard.rollback(primary, clock, notes);
    if let Some(payload) = unwind {
        eprintln!("{failure}");
        std::panic::resume_unwind(payload);
    }
    Err(failure)
}

pub(crate) struct NativeLaunchHost<'a> {
    mux: &'a dyn Multiplexer,
    context: &'a MultiplexerContext,
    client: Option<ControlClient>,
}

impl<'a> NativeLaunchHost<'a> {
    pub(crate) fn new(mux: &'a dyn Multiplexer, context: &'a MultiplexerContext) -> Self {
        Self {
            mux,
            context,
            client: None,
        }
    }
}

impl LaunchHost for NativeLaunchHost<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn current_executable(&self) -> Result<std::path::PathBuf> {
        std::env::current_exe().map_err(io_error)
    }
    fn split_runner(&mut self, argv: &[String], deadline: Instant) -> Result<String> {
        self.mux
            .bounded(crate::time::Deadline::until(deadline))
            .split_window(self.context, argv)
            .map_err(|cause| error(format!("runner pane spawn failed: {cause}")))
    }
    fn wait_prepared(
        &mut self,
        descriptor: &LaunchDescriptor,
        deadline: Instant,
    ) -> Result<Prepared> {
        let paths = RuntimePaths::for_run(
            &descriptor.identity.database_identity_json,
            descriptor.identity.run_id,
        )?;
        let mut client = ControlClient::connect_starting(&paths.control_socket, deadline)?;
        let request = LifecycleRequest::Prepare {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: descriptor.identity.clone(),
            remaining_ms: deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(10000) as u64,
        };
        let result = client.exchange(&request, deadline)?;
        let prepared = serde_json::from_value(result).map_err(io_error)?;
        self.client = Some(client);
        Ok(prepared)
    }
    fn start(
        &mut self,
        prepared: &Prepared,
        placement: &Placement,
        deadline: Instant,
    ) -> Result<Started> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| error("prepared control connection required"))?;
        let request = LifecycleRequest::Start {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: prepared.identity.clone(),
            pane_id: placement.mux_pane_id.clone(),
        };
        let value = client.exchange(&request, deadline)?;
        serde_json::from_value(value).map_err(io_error)
    }
    fn settle_hermes(
        &mut self,
        conn: &Connection,
        run: &RunRecord,
        pane_id: &str,
        deadline: Instant,
    ) -> Result<()> {
        crate::coding_agent::hermes::settle_run(
            conn,
            self.mux,
            run,
            pane_id,
            crate::time::Deadline::until(deadline),
        )
    }
    fn shutdown_owned(&mut self, run: &RunRecord, clock: ShutdownClock) -> Result<ShutdownOutcome> {
        if let Ok(value) = request(
            run,
            LifecycleRequest::shutdown(run),
            clock.storage_deadline(),
        ) {
            return serde_json::from_value(value).map_err(io_error);
        }
        let runner = run.owner_process();
        if runner.matches()? {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(i32::try_from(runner.pid).map_err(io_error)?),
                nix::sys::signal::Signal::SIGTERM,
            )
            .map_err(io_error)?;
            while runner.matches()? && Instant::now() < clock.storage_deadline() {
                pause(clock.storage_deadline())?;
            }
        }
        let mut notes=vec!["runner control shutdown unavailable; descendant cleanup requires explicit reconciliation".into()];
        if runner.matches()? {
            notes.push("owned runner termination remains unconfirmed".into());
        }
        Ok(ShutdownOutcome::new(None, notes))
    }
    fn close_pane(&mut self, pane: &str, deadline: Instant) -> Result<()> {
        self.mux
            .bounded(crate::time::Deadline::until(deadline))
            .kill_pane(pane, true)
            .map_err(|cause| error(format!("runner pane cleanup failed: {cause}")))
    }
}

#[cfg(test)]
mod tests {
    mod launch_readiness_tests {
        use std::io::ErrorKind;

        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        use std::os::unix::net::UnixStream;

        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;
        use std::time::Duration;
        use std::time::Instant;

        use nix::sys::socket::Backlog;

        use nix::sys::socket::listen;

        use crate::error::CafleetError;
        use crate::multiplexer::MultiplexerContext;
        use crate::multiplexer::TmuxMultiplexer;
        use crate::multiplexer::test_support;
        use crate::runtime::control::test_support::Endpoint;
        use crate::runtime::control::test_support::StartupInterval;
        use crate::runtime::control::test_support::bound_socket;
        use crate::runtime::control::test_support::independent_frame as raw_frame;
        use crate::runtime::lifecycle::*;
        use crate::runtime::nap_until as pause_until;
        use crate::runtime::test_support::read_fixture_bytes;
        use crate::runtime::test_support::write_fixture_bytes;
        use serde_json::Value;

        fn with_native_host<T>(operation: impl FnOnce(&mut NativeLaunchHost<'_>) -> T) -> T {
            let runner = test_support::FakeRunner::without_binaries();
            let mux = TmuxMultiplexer::for_server(runner.clone(), "/tmp/tmux-1000/default");
            let context = MultiplexerContext {
                session: "readiness-fixture".into(),
                window_id: "@1".into(),
                pane_id: "%1".into(),
            };
            let result = operation(&mut NativeLaunchHost::new(&mux, &context));
            assert!(runner.events().is_empty(), "{:?}", runner.events());
            result
        }

        fn accept_before(
            listener: &UnixListener,
            finished: &AtomicBool,
            deadline: Instant,
        ) -> Option<UnixStream> {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(true).unwrap();
                        return Some(stream);
                    }
                    Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                    Err(cause) => panic!("fixture accept failed: {cause}"),
                }
                if finished.load(Ordering::Acquire) || Instant::now() >= deadline {
                    return None;
                }
                pause_until(deadline);
            }
        }

        fn read_prepare(
            stream: &mut UnixStream,
            identity: &ControlIdentity,
            deadline: Instant,
        ) -> Value {
            let mut header = [0; 4];
            read_fixture_bytes(stream, &mut header, deadline);
            let length = usize::try_from(u32::from_be_bytes(header)).unwrap();
            assert!(
                length > 0 && length <= 65536,
                "invalid request length {length}"
            );
            let mut bytes = vec![0; length];
            read_fixture_bytes(stream, &mut bytes, deadline);
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["protocol_version"], 1);
            assert_eq!(request["kind"], "prepare");
            assert_eq!(request["identity"], serde_json::to_value(identity).unwrap());
            assert!(request["remaining_ms"].as_u64().unwrap() > 0);
            request
        }

        fn readiness_success(interval: StartupInterval) {
            let endpoint = Endpoint::new();
            let listener = match interval {
                StartupInterval::Bound => {
                    let listener = bound_socket(endpoint.path(), 0o600, false);
                    assert_eq!(
                        UnixStream::connect(endpoint.path()).unwrap_err().kind(),
                        ErrorKind::ConnectionRefused
                    );
                    listener
                }
                StartupInterval::BeforePrivateMode => bound_socket(endpoint.path(), 0o644, true),
            };
            let finished = AtomicBool::new(false);
            let started = Instant::now();
            let deadline = started + Duration::from_secs(1);
            let ready_at = started + Duration::from_millis(100);
            let path = endpoint.path().to_path_buf();
            let expected_identity = ControlIdentity::from_run(&endpoint.fixture.run);
            let reply = serde_json::json!({"protocol_version":1,"result":endpoint.prepared()});
            let (result, request, premature_connections) = std::thread::scope(|scope| {
                let server = scope.spawn(|| {
                    let mut premature_connections = 0;
                    while Instant::now() < ready_at {
                        if matches!(interval, StartupInterval::BeforePrivateMode) {
                            match listener.accept() {
                                Ok((stream, _)) => {
                                    premature_connections += 1;
                                    drop(stream);
                                }
                                Err(cause) if cause.kind() == ErrorKind::WouldBlock => {}
                                Err(cause) => panic!("pre-private fixture accept failed: {cause}"),
                            }
                        }
                        if finished.load(Ordering::Acquire) {
                            return (None, premature_connections);
                        }
                        pause_until(ready_at);
                    }
                    match interval {
                        StartupInterval::Bound => {
                            listen(&listener, Backlog::new(8).unwrap()).unwrap()
                        }
                        StartupInterval::BeforePrivateMode => {
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                                .unwrap()
                        }
                    }
                    let Some(mut stream) = accept_before(&listener, &finished, deadline) else {
                        return (None, premature_connections);
                    };
                    let request = read_prepare(&mut stream, &expected_identity, deadline);
                    write_fixture_bytes(
                        &mut stream,
                        &raw_frame(&serde_json::to_vec(&reply).unwrap()),
                        deadline,
                    );
                    (Some(request), premature_connections)
                });
                let result =
                    with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
                finished.store(true, Ordering::Release);
                let (request, premature_connections) = server.join().unwrap();
                (result, request, premature_connections)
            });
            assert_eq!(premature_connections, 0);
            let prepared =
                result.expect("startup interval must become ready within its original deadline");
            assert_eq!(prepared.identity, endpoint.prepared().identity);
            assert!(started.elapsed() >= Duration::from_millis(100));
            assert!(Instant::now() < deadline);
            assert!(request.unwrap()["remaining_ms"].as_u64().unwrap() <= 900);
        }

        #[test]
        fn runner_readiness_bound_socket_becomes_listening_within_the_original_deadline() {
            readiness_success(StartupInterval::Bound);
        }

        #[test]
        fn runner_readiness_pre_private_socket_waits_without_sending_prepare() {
            readiness_success(StartupInterval::BeforePrivateMode);
        }

        fn readiness_expiry(interval: StartupInterval) {
            let endpoint = Endpoint::new();
            let listener = match interval {
                StartupInterval::Bound => bound_socket(endpoint.path(), 0o600, false),
                StartupInterval::BeforePrivateMode => bound_socket(endpoint.path(), 0o644, true),
            };
            let started = Instant::now();
            let deadline = started + Duration::from_millis(120);
            let result =
                with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
            assert!(
                matches!(result, Err(CafleetError::DeadlineExceeded)),
                "{result:?}"
            );
            assert!(Instant::now() >= deadline);
            assert!(started.elapsed() < Duration::from_secs(1));
            if matches!(interval, StartupInterval::BeforePrivateMode) {
                assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
            }
        }

        #[test]
        fn runner_readiness_bound_unready_socket_expires_at_the_supplied_deadline() {
            readiness_expiry(StartupInterval::Bound);
        }

        fn assert_identity_rejected(endpoint: &Endpoint) {
            let started = Instant::now();
            let result = with_native_host(|host| {
                host.wait_prepared(&endpoint.descriptor, started + Duration::from_secs(1))
            });
            let cause = result.expect_err("invalid endpoint identity must fail explicitly");
            assert!(
                cause
                    .to_string()
                    .contains("private control socket identity required"),
                "{cause}"
            );
            assert!(started.elapsed() < Duration::from_millis(500));
        }

        #[test]
        fn runner_readiness_symlink_to_a_private_socket_is_an_identity_failure() {
            let endpoint = Endpoint::new();
            let target = endpoint.fixture._dir.path().join("linked.sock");
            let listener = bound_socket(&target, 0o600, true);
            symlink(target, endpoint.path()).unwrap();
            assert_identity_rejected(&endpoint);
            assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
        }

        fn assert_prepare_not_reconnected(reply_body: Option<&[u8]>) {
            let endpoint = Endpoint::new();
            let listener = bound_socket(endpoint.path(), 0o600, true);
            let finished = AtomicBool::new(false);
            let started = Instant::now();
            let deadline = started + Duration::from_secs(1);
            let expected_identity = ControlIdentity::from_run(&endpoint.fixture.run);
            let (result, connections) = std::thread::scope(|scope| {
                let server = scope.spawn(|| {
                    let mut stream = accept_before(&listener, &finished, deadline)
                        .expect("actual startup client must connect");
                    read_prepare(&mut stream, &expected_identity, deadline);
                    if let Some(body) = reply_body {
                        write_fixture_bytes(&mut stream, &raw_frame(body), deadline);
                    }
                    drop(stream);
                    let mut connections = 1;
                    while let Some(stream) = accept_before(&listener, &finished, deadline) {
                        connections += 1;
                        drop(stream);
                    }
                    connections
                });
                let result =
                    with_native_host(|host| host.wait_prepared(&endpoint.descriptor, deadline));
                finished.store(true, Ordering::Release);
                (result, server.join().unwrap())
            });
            assert!(result.is_err(), "{result:?}");
            assert!(
                !matches!(&result, Err(CafleetError::DeadlineExceeded)),
                "{result:?}"
            );
            assert!(started.elapsed() < Duration::from_millis(500));
            assert_eq!(connections, 1, "a written prepare must not be reconnected");
        }

        #[test]
        fn runner_readiness_lost_reply_after_written_prepare_is_not_reconnected() {
            for body in [
                None,
                Some(br#"{"protocol_version":2,"result":null}"#.as_slice()),
                Some(br#"{"protocol_version":1,"result":{"identity":null}}"#),
            ] {
                assert_prepare_not_reconnected(body);
            }
        }
    }

    use std::result::Result;

    use rusqlite::Connection;
    use serde_json::json;

    use crate::broker::MemberStatus;
    use crate::broker::Placement;
    use crate::broker::members::test_support::placement;
    use crate::broker::runtime as storage;
    use crate::error::CafleetError;
    use crate::runtime::lifecycle::*;
    use rusqlite::hooks::AuthAction;
    use rusqlite::hooks::AuthContext;
    use rusqlite::hooks::Authorization;
    use rusqlite::params;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    #[derive(Clone, Copy)]
    enum PublicationFailure {
        Missing,
        Rejected,
    }

    struct LaunchFixtureHost {
        database_url: String,
        now: Instant,
        delay: Duration,
        calls: Vec<&'static str>,
        deadlines: Vec<(&'static str, Instant)>,
        argv: Vec<String>,
        child_argv: Vec<String>,
        spawn_count: usize,
        fail: Option<&'static str>,
        failure_is_usage: bool,
        publication_failure: Option<PublicationFailure>,
        panic_at: Option<&'static str>,
        corrupt: Option<&'static str>,
        fast_exit: bool,
        cleanup_failure: bool,
        hermes_failure: Option<&'static str>,
        lock: Option<RunLock>,
    }

    impl LaunchFixtureHost {
        fn new(fixture: &Fixture) -> Self {
            Self {
                database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
                now: Instant::now(),
                delay: Duration::ZERO,
                calls: Vec::new(),
                deadlines: Vec::new(),
                argv: Vec::new(),
                child_argv: Vec::new(),
                spawn_count: 0,
                fail: None,
                failure_is_usage: false,
                publication_failure: None,
                panic_at: None,
                corrupt: None,
                fast_exit: false,
                cleanup_failure: false,
                hermes_failure: None,
                lock: None,
            }
        }
        fn phase(&mut self, name: &'static str, deadline: Instant) -> Result<(), CafleetError> {
            self.calls.push(name);
            self.deadlines.push((name, deadline));
            self.now += self.delay;
            if self.fail == Some(name) {
                let message = format!("fixture {name} failure");
                Err(if self.failure_is_usage {
                    CafleetError::Usage(message)
                } else {
                    CafleetError::App(message)
                })
            } else {
                Ok(())
            }
        }
        fn identity(run: &RunRecord) -> ControlIdentity {
            ControlIdentity::from_run(run)
        }
    }

    impl LaunchHost for LaunchFixtureHost {
        fn now(&self) -> Instant {
            self.now
        }
        fn current_executable(&self) -> Result<PathBuf, CafleetError> {
            Ok("/absolute/cafleet".into())
        }
        fn split_runner(
            &mut self,
            argv: &[String],
            deadline: Instant,
        ) -> Result<String, CafleetError> {
            self.phase("split", deadline)?;
            self.argv = argv.to_vec();
            assert_eq!(argv[0], "/absolute/cafleet");
            assert_eq!(argv[1], "--database-url");
            assert_eq!(
                argv[2],
                format!(
                    "sqlite:///{}",
                    std::fs::canonicalize(crate::db::database_path(&self.database_url).unwrap())
                        .unwrap()
                        .display()
                )
            );
            assert_eq!(argv[3], "__member-run");
            assert_eq!(argv[4], "--descriptor");
            assert!(std::path::Path::new(&argv[5]).is_absolute());
            assert_eq!(argv.len(), 6);
            Ok("%9".into())
        }
        fn wait_prepared(
            &mut self,
            descriptor: &LaunchDescriptor,
            deadline: Instant,
        ) -> Result<Prepared, CafleetError> {
            self.phase("prepared", deadline)?;
            assert_ne!(self.panic_at, Some("prepared"), "fixture prepared unwind");
            self.child_argv = descriptor.spawn.argv.clone();
            let mut conn = crate::db::connect(&self.database_url).unwrap();
            let before = storage::get_run(&conn, descriptor.identity.run_id).unwrap();
            assert_eq!(before.child_pid, None);
            assert_eq!(self.spawn_count, 0);
            let lock = RunLock::acquire(&conn, &before.owner()).unwrap().unwrap();
            let process = ProcessIdentity::current().unwrap();
            let owner = lock
                .transfer_to_runner(&mut conn, &before.owner(), process.clone())
                .unwrap();
            conn.execute(
                "UPDATE member_runs SET runner_pid=?1,runner_start_identity=?2 WHERE run_id=?3",
                params![process.pid, process.start_identity, owner.run_id],
            )
            .unwrap();
            let run = storage::get_run(&conn, owner.run_id).unwrap();
            self.lock = Some(lock);
            match self.publication_failure {
                Some(PublicationFailure::Missing) => {
                    conn.execute(
                        "DELETE FROM member_placements WHERE member_id=?1",
                        [run.member_id],
                    )
                    .unwrap();
                }
                Some(PublicationFailure::Rejected) => {
                    conn.execute_batch(
                        "CREATE TRIGGER reject_pending_publication
                     BEFORE UPDATE OF mux_pane_id ON member_placements
                     BEGIN SELECT RAISE(ABORT,'fixture publication blocked'); END",
                    )
                    .unwrap();
                }
                None => {}
            }
            let mut identity = Self::identity(&run);
            match self.corrupt {
                Some("prepared-nonce") => identity.launch_nonce = "b".repeat(64),
                Some("prepared-epoch") => identity.owner_epoch += 1,
                Some("prepared-run") => identity.run_id += 1,
                Some("prepared-director") => identity.director_member_id += 1,
                _ => {}
            }
            Ok(Prepared {
                identity,
                runner: process,
            })
        }
        fn start(
            &mut self,
            prepared: &Prepared,
            placement: &Placement,
            deadline: Instant,
        ) -> Result<Started, CafleetError> {
            self.phase("start", deadline)?;
            let mut conn = crate::db::connect(&self.database_url).unwrap();
            let run = storage::get_run(&conn, prepared.identity.run_id).unwrap();
            assert_eq!(run.status, RunStatus::Prepared);
            let member = crate::broker::get_member(&conn, run.member_id, run.fleet_id)
                .unwrap()
                .unwrap();
            assert_eq!(member.placement.mux_pane_id.as_deref(), Some("%9"));
            assert_eq!(placement.mux_pane_id.as_deref(), Some("%9"));
            self.spawn_count += 1;
            let child = ProcessIdentity {
                pid: run.owner_pid.checked_add(1_000_000).unwrap(),
                start_identity: "fixture-child-start".into(),
            };
            conn.execute("UPDATE member_runs SET status='running',child_pid=?1,child_start_identity=?2,started_at=?3 WHERE run_id=?4",params![child.pid,child.start_identity,"2026-10-26T11:00:00.000000+00:00",run.run_id]).unwrap();
            assert_ne!(self.panic_at, Some("started"), "fixture started unwind");
            if self.fast_exit {
                let run = storage::get_run(&conn, run.run_id).unwrap();
                storage::reserve_event_sequences(&mut conn, &run.owner()).unwrap();
                let evidence=crate::broker::runtime::evidence::EvidencePolicy::metadata().apply(&json!({"backend":"codex","native_version":"0.163.0","rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"native_request_id":null,"excerpt":null,"truncated":false,"sensitive":false})).unwrap();
                storage::persist_transition(
                    &mut conn,
                    &run.owner(),
                    &ObservedTransition {
                        run_id: run.run_id,
                        event_seq: 1,
                        episode_seq: None,
                        kind: RuntimeEventKind::RunnerExited,
                        classification: None,
                        observed_at: "2026-10-26T11:00:00.000000+00:00".into(),
                        evidence_json: evidence,
                        close_reason: None,
                        cause_json: Some(
                            json!({"kind":"native_exit","exit_code":7,"exit_signal":null}),
                        ),
                    },
                    PersistenceMode::Live,
                )
                .unwrap();
            }
            if self.fail == Some("started-reply") {
                return Err(CafleetError::App(
                    "fixture started reply lost after spawn".into(),
                ));
            }
            let mut identity = Self::identity(&storage::get_run(&conn, run.run_id).unwrap());
            let mut reply_child = child;
            if self.corrupt == Some("started-epoch") {
                identity.owner_epoch += 1;
            }
            if self.corrupt == Some("started-child") {
                reply_child.pid += 1;
            }
            Ok(Started {
                identity,
                runner: prepared.runner.clone(),
                child: reply_child,
            })
        }
        fn shutdown_owned(
            &mut self,
            run: &RunRecord,
            clock: ShutdownClock,
        ) -> Result<ShutdownOutcome, CafleetError> {
            self.calls.push("shutdown");
            assert_eq!(clock.storage_deadline(), self.now + Duration::from_secs(4));
            let conn = crate::db::connect(&self.database_url).unwrap();
            let current = storage::get_run(&conn, run.run_id).unwrap();
            assert_eq!(current.run_id, run.run_id);
            if self.cleanup_failure {
                return Err(CafleetError::App(
                    "fixture owned-child cleanup failed".into(),
                ));
            }
            self.lock.take();
            Ok(ShutdownOutcome::complete(None))
        }
        fn settle_hermes(
            &mut self,
            _conn: &Connection,
            run: &RunRecord,
            pane_id: &str,
            _deadline: Instant,
        ) -> Result<(), CafleetError> {
            self.calls.push("settle");
            assert_eq!(run.backend, "hermes");
            assert_eq!(pane_id, "%9");
            match self.hermes_failure {
                Some(cause) => Err(CafleetError::App(cause.into())),
                None => Ok(()),
            }
        }
        fn close_pane(&mut self, pane: &str, deadline: Instant) -> Result<(), CafleetError> {
            self.calls.push("close");
            self.deadlines.push(("close", deadline));
            assert_eq!(pane, "%9");
            if self.cleanup_failure {
                Err(CafleetError::App("fixture pane cleanup failed".into()))
            } else {
                Ok(())
            }
        }
    }

    /// A fresh fixture, its launch host and the creation deadline ten seconds after the host's clock.
    fn launch_case() -> (Fixture, LaunchFixtureHost, Instant) {
        let fixture = Fixture::new();
        let host = LaunchFixtureHost::new(&fixture);
        let deadline = host.now + Duration::from_secs(10);
        (fixture, host, deadline)
    }

    fn inputs(fixture: &Fixture) -> (PendingMemberSpec, RunSettings) {
        inputs_for(fixture, "codex")
    }

    fn inputs_for(fixture: &Fixture, backend: &str) -> (PendingMemberSpec, RunSettings) {
        let mut placement = placement(None);
        placement.coding_agent = backend.into();
        (
            PendingMemberSpec {
                fleet_id: fixture.run.fleet_id,
                name: "pending-worker".into(),
                description: "pending assignment".into(),
                placement,
            },
            RunSettings {
                database_identity_json: fixture.run.database_identity_json.clone(),
                creator: ProcessIdentity::current().unwrap(),
                launch_nonce: "c".repeat(64),
                requested_mode: None,
                resolved_mode: "approve-for-me".into(),
                mode_source: ModeSource::Default,
                permission_argv_json: fixture.run.permission_argv_json.clone(),
                event_evidence: EventEvidence::Metadata,
                native_version: None,
                coverage: Coverage::Unknown,
            },
        )
    }

    fn launch(
        fixture: &mut Fixture,
        host: &mut LaunchFixtureHost,
        deadline: Instant,
    ) -> Result<LaunchOutcome, CafleetError> {
        launch_backend(fixture, host, deadline, "codex")
    }

    fn launch_backend(
        fixture: &mut Fixture,
        host: &mut LaunchFixtureHost,
        deadline: Instant,
        backend: &str,
    ) -> Result<LaunchOutcome, CafleetError> {
        let (member, settings) = inputs_for(fixture, backend);
        let spec = fixture.spec();
        create_registered(
            &mut fixture.conn,
            &member,
            &settings,
            |conn, pending| {
                Ok((
                    RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?,
                    spec,
                ))
            },
            host,
            deadline,
        )
    }

    fn pending_run(fixture: &Fixture) -> RunRecord {
        let id=fixture.conn.query_row("SELECT run_id FROM member_runs WHERE member_id IN (SELECT member_id FROM members WHERE name='pending-worker')",[],|row|row.get::<_,i64>(0)).unwrap();
        storage::get_run(&fixture.conn, id).unwrap()
    }

    fn assert_compensated(fixture: &Fixture, run: &RunRecord) {
        let status = fixture
            .conn
            .query_row(
                "SELECT status FROM members WHERE member_id=?1",
                [run.member_id],
                |row| row.get::<_, MemberStatus>(0),
            )
            .unwrap();
        assert_eq!(status, MemberStatus::Deregistered);
        assert_eq!(
            storage::get_run(&fixture.conn, run.run_id).unwrap().status,
            RunStatus::Failed
        );
        let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id).unwrap();
        assert!(!paths.descriptor.exists());
    }

    fn count_registration_cleanup(conn: &mut Connection) -> Arc<AtomicUsize> {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Update {
                    table_name: "members",
                    column_name: "status"
                }
            ) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }))
        .unwrap();
        attempts
    }

    fn reject_registration_cleanup(conn: &Connection) {
        conn.execute_batch(
            "CREATE TRIGGER reject_registration_cleanup BEFORE UPDATE OF status ON members
         BEGIN SELECT RAISE(ABORT,'fixture database cleanup blocked'); END",
        )
        .unwrap();
    }

    fn require_launch_error(result: Result<LaunchOutcome, CafleetError>) -> CafleetError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("fixture creation must fail"),
        }
    }

    #[test]
    fn runner_creation_allocates_member_and_run_in_one_transaction_or_rolls_both_back() {
        let mut fixture = Fixture::new();
        let (member, settings) = inputs(&fixture);
        let before = fixture
            .conn
            .query_row("SELECT COUNT(*) FROM members", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        fixture.conn.execute_batch("CREATE TRIGGER fail_pending_run BEFORE INSERT ON member_runs BEGIN SELECT RAISE(ABORT,'pending run insert failure'); END").unwrap();
        assert!(
            register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10)
            )
            .is_err()
        );
        assert_eq!(
            fixture
                .conn
                .query_row("SELECT COUNT(*) FROM members", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            fixture
                .conn
                .query_row("SELECT COUNT(*) FROM member_runs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn runner_creation_records_the_probed_version_and_coverage_on_the_pending_run() {
        for (native, coverage) in [
            (None, Coverage::Unknown),
            (Some("0.164.0"), Coverage::Tested),
        ] {
            let mut fixture = Fixture::new();
            let (member, mut settings) = inputs(&fixture);
            settings.native_version = native.map(str::to_owned);
            settings.coverage = coverage;
            let pending = register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            assert_eq!(pending.run.native_version.as_deref(), native);
            assert_eq!(pending.run.coverage, coverage);
            let reread = storage::get_run(&fixture.conn, pending.run.run_id).unwrap();
            assert_eq!(reread.native_version.as_deref(), native);
            assert_eq!(reread.coverage, coverage);
        }
    }

    #[test]
    fn runner_creation_guard_covers_preparation_failure_before_artifacts_after_allocation_and_after_descriptor()
     {
        for stage in ["before-artifacts", "after-artifacts", "after-descriptor"] {
            let (mut fixture, mut host, deadline) = launch_case();
            let (member, settings) = inputs(&fixture);
            let spec = fixture.spec();
            let result = create_registered(
                &mut fixture.conn,
                &member,
                &settings,
                |conn, pending| {
                    if stage == "before-artifacts" {
                        return Err(CafleetError::App("fixture preparation failure".into()));
                    }
                    let artifacts =
                        RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                    if stage == "after-descriptor" {
                        artifacts.write_descriptor(&LaunchDescriptor::new(
                            &storage::get_run(conn, pending.run.run_id)?,
                            spec,
                        )?)?;
                    }
                    Err(CafleetError::App("fixture preparation failure".into()))
                },
                &mut host,
                deadline,
            );
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("{stage} must compensate"),
            };
            assert!(error.message().contains("fixture preparation failure"));
            assert_compensated(&fixture, &pending_run(&fixture));
            assert!(host.calls.is_empty());
            assert_eq!(host.spawn_count, 0);
        }
    }

    #[test]
    fn runner_creation_publishes_prepared_placement_before_one_start_and_unlinks_the_descriptor() {
        let (mut fixture, mut host, deadline) = launch_case();
        let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
        let outcome = launch(&mut fixture, &mut host, deadline).unwrap();
        assert_eq!(host.calls, ["split", "prepared", "start"]);
        assert_eq!(host.spawn_count, 1);
        assert!(
            host.deadlines
                .iter()
                .all(|(_, recorded)| *recorded == deadline)
        );
        assert_eq!(outcome.run.status, RunStatus::Running);
        assert_eq!(outcome.run.owner_kind, OwnerKind::Runner);
        assert_eq!(outcome.run.owner_epoch, 2);
        assert!(
            RunLock::acquire(&fixture.conn, &outcome.run.owner())
                .unwrap()
                .is_none()
        );
        assert_eq!(outcome.placement.mux_pane_id.as_deref(), Some("%9"));
        assert_eq!(outcome.run.child_pid, Some(outcome.started.child.pid));
        assert!(
            !RuntimePaths::for_run(&outcome.run.database_identity_json, outcome.run.run_id)
                .unwrap()
                .descriptor
                .exists()
        );
        assert!(
            crate::broker::get_member(
                &fixture.conn,
                outcome.member.member_id,
                outcome.run.fleet_id
            )
            .unwrap()
            .is_some()
        );
        let member_id = outcome.member.member_id;
        let fleet_id = outcome.run.fleet_id;
        drop(outcome);
        assert!(
            crate::broker::get_member(&fixture.conn, member_id, fleet_id)
                .unwrap()
                .is_some()
        );
        assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 0);
        assert_eq!(host.calls, ["split", "prepared", "start"]);
    }

    #[test]
    fn runner_creation_missing_or_rejected_publication_compensates_the_owned_pane_once() {
        for failure in [PublicationFailure::Missing, PublicationFailure::Rejected] {
            let (mut fixture, mut host, deadline) = launch_case();
            let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
            host.publication_failure = Some(failure);
            let error = require_launch_error(launch(&mut fixture, &mut host, deadline));
            assert_eq!(error.exit_code(), 1);
            let expected = match failure {
                PublicationFailure::Missing => "placement",
                PublicationFailure::Rejected => "fixture publication blocked",
            };
            assert!(error.message().contains(expected), "{error}");
            assert_eq!(host.calls, ["split", "prepared", "shutdown", "close"]);
            assert_eq!(host.spawn_count, 0);
            assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
            assert_compensated(&fixture, &pending_run(&fixture));
        }
    }

    #[test]
    fn runner_creation_preserves_usage_exit_through_preparation_and_launch_cleanup() {
        for phase in ["preparation", "start"] {
            for cleanup_fails in [false, true] {
                let (mut fixture, mut host, deadline) = launch_case();
                let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
                if cleanup_fails {
                    reject_registration_cleanup(&fixture.conn);
                }
                let result = if phase == "preparation" {
                    let (member, settings) = inputs(&fixture);
                    create_registered(
                        &mut fixture.conn,
                        &member,
                        &settings,
                        |_, _| {
                            Err(CafleetError::Usage(
                                "fixture preparation usage failure".into(),
                            ))
                        },
                        &mut host,
                        deadline,
                    )
                } else {
                    host.fail = Some("start");
                    host.failure_is_usage = true;
                    launch(&mut fixture, &mut host, deadline)
                };
                let error = require_launch_error(result);
                assert_eq!(error.exit_code(), 2, "{error}");
                assert!(
                    error.message().starts_with(&format!("fixture {phase}")),
                    "{error}"
                );
                assert_eq!(
                    error
                        .message()
                        .matches("fixture database cleanup blocked")
                        .count(),
                    usize::from(cleanup_fails)
                );
                assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
                assert_eq!(host.spawn_count, 0);
                let run = pending_run(&fixture);
                if cleanup_fails {
                    assert!(
                        crate::broker::get_member(&fixture.conn, run.member_id, run.fleet_id)
                            .unwrap()
                            .is_some()
                    );
                } else {
                    assert_compensated(&fixture, &run);
                }
                assert_eq!(
                    host.calls.iter().filter(|call| **call == "close").count(),
                    usize::from(phase == "start")
                );
                assert_eq!(
                    host.calls
                        .iter()
                        .filter(|call| **call == "shutdown")
                        .count(),
                    usize::from(phase == "start")
                );
            }
        }
    }

    #[test]
    fn runner_creation_preserves_publication_failure_and_combined_cleanup_errors_once() {
        let (mut fixture, mut host, deadline) = launch_case();
        let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
        reject_registration_cleanup(&fixture.conn);
        host.publication_failure = Some(PublicationFailure::Rejected);
        host.cleanup_failure = true;
        let error = require_launch_error(launch(&mut fixture, &mut host, deadline));
        assert_eq!(error.exit_code(), 1);
        let text = error.message();
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains("fixture publication blocked"),
            "{text}"
        );
        for diagnostic in [
            "fixture publication blocked",
            "fixture owned-child cleanup failed",
            "fixture pane cleanup failed",
            "fixture database cleanup blocked",
        ] {
            assert_eq!(text.matches(diagnostic).count(), 1, "{text}");
        }
        assert_eq!(host.calls, ["split", "prepared", "shutdown", "close"]);
        assert_eq!(host.spawn_count, 0);
        assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
        let run = pending_run(&fixture);
        assert!(
            crate::broker::get_member(&fixture.conn, run.member_id, run.fleet_id)
                .unwrap()
                .is_some()
        );
        assert!(
            !RuntimePaths::for_run(&run.database_identity_json, run.run_id)
                .unwrap()
                .descriptor
                .exists()
        );
    }

    #[test]
    fn runner_creation_unwind_during_preparation_compensates_registration_and_descriptor_once() {
        for stage in ["before-artifacts", "after-artifacts", "after-descriptor"] {
            let (mut fixture, mut host, deadline) = launch_case();
            let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
            let (member, settings) = inputs(&fixture);
            let spec = fixture.spec();
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                create_registered(
                    &mut fixture.conn,
                    &member,
                    &settings,
                    |conn, pending| {
                        assert_ne!(stage, "before-artifacts", "fixture preparation unwind");
                        let artifacts =
                            RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                        if stage == "after-descriptor" {
                            artifacts.write_descriptor(&LaunchDescriptor::new(
                                &storage::get_run(conn, pending.run.run_id)?,
                                spec,
                            )?)?;
                        }
                        panic!("fixture preparation unwind");
                    },
                    &mut host,
                    deadline,
                )
            }));
            assert!(unwound.is_err(), "{stage}");
            assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
            assert_compensated(&fixture, &pending_run(&fixture));
            assert!(host.calls.is_empty());
            assert_eq!(host.spawn_count, 0);
        }
    }

    fn assert_launch_unwind_compensates(phase: &'static str) {
        let (mut fixture, mut host, deadline) = launch_case();
        let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
        host.panic_at = Some(phase);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            launch(&mut fixture, &mut host, deadline)
        }));
        assert!(unwound.is_err());
        let started = phase == "started";
        assert_eq!(host.spawn_count, usize::from(started));
        assert_eq!(
            host.calls
                .iter()
                .filter(|call| **call == "shutdown")
                .count(),
            usize::from(started)
        );
        assert_eq!(
            host.calls.iter().filter(|call| **call == "close").count(),
            1
        );
        assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1);
        assert_compensated(&fixture, &pending_run(&fixture));
    }

    #[test]
    fn runner_creation_unwind_after_split_closes_the_owned_pane_once() {
        assert_launch_unwind_compensates("prepared");
    }

    #[test]
    fn runner_creation_unwind_after_spawn_stops_the_owned_child_and_closes_the_pane_once() {
        assert_launch_unwind_compensates("started");
    }

    #[test]
    fn runner_creation_compensates_split_prepare_and_spawn_failures_without_leaking_registration() {
        for phase in ["split", "prepared", "start"] {
            let (mut fixture, mut host, deadline) = launch_case();
            host.fail = Some(phase);
            assert!(
                launch(&mut fixture, &mut host, deadline).is_err(),
                "{phase}"
            );
            assert_compensated(&fixture, &pending_run(&fixture));
            assert_eq!(host.spawn_count, 0);
            assert!(host.calls.iter().filter(|call| **call == "close").count() <= 1);
            assert!(
                host.calls
                    .iter()
                    .filter(|call| **call == "shutdown")
                    .count()
                    <= 1
            );
        }
    }

    #[test]
    fn runner_creation_rejects_each_stale_prepared_identity_before_starting_a_child() {
        let (mut fixture, mut host, deadline) = launch_case();
        host.corrupt = Some("prepared-nonce");
        assert!(launch(&mut fixture, &mut host, deadline).is_err());
        assert_eq!(host.spawn_count, 0);
        assert_compensated(&fixture, &pending_run(&fixture));
    }

    #[test]
    fn runner_creation_lost_started_reply_or_mismatched_child_compensates_one_uncertain_spawn() {
        for failure in ["started-reply", "started-epoch", "started-child"] {
            let (mut fixture, mut host, deadline) = launch_case();
            if failure == "started-reply" {
                host.fail = Some(failure);
            } else {
                host.corrupt = Some(failure);
            }
            assert!(
                launch(&mut fixture, &mut host, deadline).is_err(),
                "{failure}"
            );
            assert_eq!(host.spawn_count, 1);
            assert_eq!(
                host.calls
                    .iter()
                    .filter(|call| **call == "shutdown")
                    .count(),
                1
            );
            assert_eq!(
                host.calls.iter().filter(|call| **call == "close").count(),
                1
            );
            assert_compensated(&fixture, &pending_run(&fixture));
        }
    }

    #[test]
    fn runner_creation_uses_one_handshake_deadline_and_compensates_a_post_start_timeout() {
        let (mut fixture, mut host, deadline) = launch_case();
        host.delay = Duration::from_secs(4);
        assert!(launch(&mut fixture, &mut host, deadline).is_err());
        assert_eq!(host.spawn_count, 1);
        assert!(
            host.deadlines
                .iter()
                .filter(|(phase, _)| *phase != "close")
                .all(|(_, recorded)| *recorded == deadline)
        );
        assert_compensated(&fixture, &pending_run(&fixture));
    }

    #[test]
    fn runner_creation_immediate_nonzero_native_exit_is_a_successful_spawn_with_one_inbox_event() {
        let (mut fixture, mut host, deadline) = launch_case();
        host.fast_exit = true;
        let outcome = launch(&mut fixture, &mut host, deadline).unwrap();
        assert_eq!(host.spawn_count, 1);
        assert!(!host.calls.contains(&"shutdown"));
        assert_eq!(outcome.run.status, RunStatus::Exited);
        assert_eq!(outcome.run.exit_code, Some(7));
        assert_eq!(outcome.run.resolved_mode, "approve-for-me");
        let messages =
            crate::broker::poll_messages(&fixture.conn, outcome.run.director_member_id).unwrap();
        assert_eq!(messages.len(), 1);
        let payload: serde_json::Value = serde_json::from_str(&messages[0].text).unwrap();
        assert_eq!(payload["kind"], "runner_exited");
        assert_eq!(payload["cause"]["exit_code"], 7);
    }

    #[test]
    fn runner_creation_retains_the_primary_failure_and_all_owned_cleanup_diagnostics() {
        let (mut fixture, mut host, deadline) = launch_case();
        host.fail = Some("started-reply");
        host.cleanup_failure = true;
        let error = match launch(&mut fixture, &mut host, deadline) {
            Err(error) => error,
            Ok(_) => panic!("lost reply must fail"),
        };
        let text = error.message();
        assert!(text.contains("started reply lost"));
        assert!(text.contains("owned-child cleanup failed"));
        assert!(text.contains("pane cleanup failed"));
        assert_eq!(host.spawn_count, 1);
        assert_eq!(
            pending_run(&fixture).cleanup_state,
            CleanupState::Incomplete
        );
    }

    #[test]
    fn hermes_creation_compensation_settles_a_started_client_before_runner_shutdown() {
        for hermes_failure in [None, Some("fixture hermes settlement failed")] {
            let (mut fixture, mut host, deadline) = launch_case();
            host.fail = Some("started-reply");
            host.hermes_failure = hermes_failure;
            let error =
                require_launch_error(launch_backend(&mut fixture, &mut host, deadline, "hermes"));
            let text = error.message();
            assert!(
                text.lines().next().unwrap().contains("started reply lost"),
                "{text}"
            );
            assert_eq!(
                host.calls,
                ["split", "prepared", "start", "settle", "shutdown", "close"]
            );
            let run = pending_run(&fixture);
            match hermes_failure {
                None => assert_compensated(&fixture, &run),
                Some(failure) => {
                    assert_eq!(text.matches(failure).count(), 1, "{text}");
                    assert_eq!(
                        storage::get_run(&fixture.conn, run.run_id).unwrap().status,
                        RunStatus::Failed
                    );
                    assert!(
                        !RuntimePaths::for_run(&run.database_identity_json, run.run_id)
                            .unwrap()
                            .descriptor
                            .exists()
                    );
                }
            }
        }
    }

    #[test]
    fn creation_compensation_settles_only_a_hermes_run_whose_child_started() {
        for (backend, failure) in [("hermes", "start"), ("codex", "started-reply")] {
            let (mut fixture, mut host, deadline) = launch_case();
            host.fail = Some(failure);
            require_launch_error(launch_backend(&mut fixture, &mut host, deadline, backend));
            assert!(
                !host.calls.contains(&"settle"),
                "{backend} {failure}: {:?}",
                host.calls
            );
            assert_compensated(&fixture, &pending_run(&fixture));
        }
    }

    #[test]
    fn runner_creation_invalid_spawn_construction_under_the_guard_keeps_assignment_out_of_failed_rows()
     {
        let (mut fixture, mut host, deadline) = launch_case();
        let (member, settings) = inputs(&fixture);
        let mut spec = fixture.spec();
        let private = "fixture-private-assignment-194";
        spec.argv.push(private.into());
        spec.cwd = "relative".into();
        assert!(
            create_registered(
                &mut fixture.conn,
                &member,
                &settings,
                |conn, pending| {
                    let artifacts =
                        RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
                    let run = storage::get_run(conn, pending.run.run_id)?;
                    LaunchDescriptor::new(&run, spec.clone())?;
                    Ok((artifacts, spec))
                },
                &mut host,
                deadline
            )
            .is_err()
        );
        let run = pending_run(&fixture);
        assert_compensated(&fixture, &run);
        let serialized=fixture.conn.query_row("SELECT permission_argv_json || COALESCE(cleanup_diagnostics_json,'') FROM member_runs WHERE run_id=?1",[run.run_id],|row|row.get::<_,String>(0)).unwrap();
        assert!(!serialized.contains(private));
        assert!(host.calls.is_empty());
    }

    #[test]
    fn runner_creation_substitutes_once_and_keeps_the_assignment_in_one_literal_native_argument() {
        let (mut fixture, mut host, deadline) = launch_case();
        let (member, settings) = inputs(&fixture);
        let mut spec = fixture.spec();
        let template =
            "member={member_id}; database={database_url}; literal={{member_id}}; '$x' `text` 世界";
        let outcome = create_registered(
            &mut fixture.conn,
            &member,
            &settings,
            |conn, pending| {
                let assignment = crate::cli::member::substitute_spawn_placeholders(
                    template,
                    pending.run.fleet_id,
                    pending.member.member_id,
                    pending.run.director_member_id,
                    "codex",
                    "sqlite:///fixture.db",
                )?;
                spec.argv = vec!["/fixture/codex".into(), assignment];
                Ok((
                    RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?,
                    spec,
                ))
            },
            &mut host,
            deadline,
        )
        .unwrap();
        assert_eq!(
            host.child_argv,
            [
                "/fixture/codex".to_owned(),
                format!(
                    "member={}; database=sqlite:///fixture.db; literal={{member_id}}; '$x' `text` 世界",
                    outcome.member.member_id
                )
            ]
        );
        assert!(!host.argv.iter().any(|arg| arg.contains("literal=")));
    }

    use crate::runtime::lifecycle::test_support::FAST_SUM;
    use crate::runtime::lifecycle::test_support::SLOW_SUM;
    use crate::runtime::lifecycle::test_support::sum;

    use crate::runtime::test_support::Fixture;
    use std::collections::BTreeSet;

    #[test]
    fn runner_recovery_clock_clips_each_original_boundary_and_never_renews_time() {
        let start = Instant::now();
        for offset in [0, 1, 2, 3, 4, 9] {
            let until = start + Duration::from_secs(offset);
            assert_eq!(
                ShutdownClock::new(start).remaining_storage_at(until),
                Duration::from_secs(4).saturating_sub(Duration::from_secs(offset))
            );
            let clock = ShutdownClock::new(start).clipped(until);
            assert_eq!(
                clock.child_deadline(),
                (start + Duration::from_secs(2)).min(until)
            );
            assert_eq!(
                clock.display_deadline(),
                (start + Duration::from_secs(3)).min(until)
            );
            assert_eq!(
                clock.storage_deadline(),
                (start + Duration::from_secs(4)).min(until)
            );
            assert_eq!(clock.remaining_storage_at(until), Duration::ZERO);
        }
        let past = start.checked_sub(Duration::from_secs(1)).unwrap();
        let clock = ShutdownClock::new(start).clipped(past);
        assert_eq!(clock.child_deadline(), past);
        assert_eq!(clock.display_deadline(), past);
        assert_eq!(clock.storage_deadline(), past);
        assert_eq!(clock.remaining_storage_at(start), Duration::ZERO);
    }

    #[test]
    fn runner_recovery_clock_repeated_clipping_only_tightens_the_same_original_schedule() {
        let start = Instant::now();
        let first = start + Duration::from_millis(2500);
        let second = start + Duration::from_millis(1500);
        let clock = ShutdownClock::new(start)
            .clipped(first)
            .clipped(start + Duration::from_secs(20));
        assert_eq!(clock.child_deadline(), start + Duration::from_secs(2));
        assert_eq!(clock.display_deadline(), first);
        assert_eq!(clock.storage_deadline(), first);
        let clock = clock.clipped(second).clipped(first);
        assert_eq!(clock.child_deadline(), second);
        assert_eq!(clock.display_deadline(), second);
        assert_eq!(clock.storage_deadline(), second);
    }

    struct Host {
        now: Instant,
        calls: Vec<(&'static str, Instant)>,
        deadlines: Vec<(&'static str, Instant)>,
        fail: BTreeSet<&'static str>,
        native: Option<ChildExit>,
        dropped: u64,
        pending: u64,
        gap: u64,
    }

    impl Host {
        fn new(now: Instant) -> Self {
            Self {
                now,
                calls: Vec::new(),
                deadlines: Vec::new(),
                fail: BTreeSet::new(),
                native: Some(ChildExit::Code(0)),
                dropped: 0,
                pending: 0,
                gap: 0,
            }
        }
        fn call(&mut self, name: &'static str) -> Result<(), CafleetError> {
            self.calls.push((name, self.now));
            if self.fail.contains(name) {
                Err(CafleetError::App(format!("fixture {name} failure")))
            } else {
                Ok(())
            }
        }
    }

    impl ShutdownHost for Host {
        fn now(&self) -> Instant {
            self.now
        }
        fn gate_programmatic_input(&mut self) {
            self.calls.push(("gate", self.now));
        }
        fn term_verified_descendants(&mut self) -> Result<(), CafleetError> {
            self.call("term")
        }
        fn drain_and_reap_until(
            &mut self,
            deadline: Instant,
        ) -> Result<Option<ChildExit>, CafleetError> {
            self.deadlines.push(("drain", deadline));
            self.now = self.now.max(deadline);
            self.call("drain")?;
            Ok(self.native)
        }
        fn kill_verified_survivors(&mut self) -> Result<(), CafleetError> {
            self.call("kill")
        }
        fn close_pty(&mut self) -> Result<(), CafleetError> {
            self.call("close")
        }
        fn final_native_exit(&mut self) -> Result<Option<ChildExit>, CafleetError> {
            Ok(None)
        }
        fn native_was_spawned(&self) -> bool {
            true
        }
        fn flush_display_until(&mut self, deadline: Instant) -> Result<u64, CafleetError> {
            self.deadlines.push(("display", deadline));
            self.now = self.now.max(deadline);
            self.call("display")?;
            Ok(self.dropped)
        }
        fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64), CafleetError> {
            self.deadlines.push(("events", deadline));
            self.now = self.now.max(deadline);
            self.call("events")?;
            Ok((self.pending, self.gap))
        }
        fn restore_terminal(&mut self) -> Result<(), CafleetError> {
            self.call("restore")
        }
    }

    #[test]
    fn runner_shutdown_uses_one_clock_for_descendants_display_events_and_restoration() {
        let start = Instant::now();
        let mut host = Host::new(start);
        host.native = None;
        let outcome = shutdown(
            &mut host,
            ShutdownClock::new(start),
            Some(ChildExit::Code(7)),
        );
        assert_eq!(
            host.calls.iter().map(|(call, _)| *call).collect::<Vec<_>>(),
            [
                "gate", "term", "drain", "kill", "close", "display", "events", "restore"
            ]
        );
        assert_eq!(
            host.deadlines,
            [
                ("drain", start + Duration::from_secs(2)),
                ("display", start + Duration::from_secs(3)),
                ("events", start + Duration::from_secs(4))
            ]
        );
        for (name, time) in &host.calls {
            if ["kill", "close"].contains(name) {
                assert_eq!(*time, start + Duration::from_secs(2));
            }
        }
        assert_eq!(outcome.native_exit, Some(ChildExit::Code(7)));
        assert_eq!(outcome.cleanup_state, CleanupState::Complete);
    }

    #[test]
    fn runner_shutdown_collects_each_failure_and_still_restores_terminal_without_an_extra_budget() {
        let start = Instant::now();
        let mut host = Host::new(start);
        host.native = None;
        host.fail = [
            "term", "drain", "kill", "close", "display", "events", "restore",
        ]
        .into();
        let outcome = shutdown(
            &mut host,
            ShutdownClock::new(start),
            Some(ChildExit::Code(9)),
        );
        assert_eq!(host.calls.last().unwrap().0, "restore");
        assert_eq!(host.now, start + Duration::from_secs(4));
        assert_eq!(outcome.native_exit, Some(ChildExit::Code(9)));
        assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
        for failure in &host.fail {
            assert!(
                outcome
                    .diagnostics
                    .iter()
                    .any(|line| line.contains(failure)),
                "{failure}"
            );
        }
    }

    #[test]
    fn runner_shutdown_retains_display_truncation_pending_frames_and_unsaved_gaps_separately() {
        let start = Instant::now();
        let mut host = Host::new(start);
        host.dropped = 17;
        host.pending = 2;
        host.gap = 3;
        let outcome = shutdown(&mut host, ShutdownClock::new(start), None);
        assert_eq!(outcome.native_exit, Some(ChildExit::Code(0)));
        assert_eq!(outcome.dropped_display_bytes, 17);
        assert_eq!(outcome.pending_event_frames, 2);
        assert_eq!(outcome.unsaved_observation_gap, 3);
        assert_eq!(outcome.cleanup_state, CleanupState::Incomplete);
        assert!(!outcome.diagnostics.is_empty());
    }

    #[test]
    fn runner_shutdown_never_rebases_a_clock_that_was_started_before_the_current_phase() {
        let start = Instant::now();
        let mut host = Host::new(start + Duration::from_secs(3));
        host.native = None;
        let outcome = shutdown(
            &mut host,
            ShutdownClock::new(start),
            Some(ChildExit::Signal(15)),
        );
        assert_eq!(host.deadlines[0].1, start + Duration::from_secs(2));
        assert_eq!(host.deadlines[2].1, start + Duration::from_secs(4));
        assert_eq!(host.now, start + Duration::from_secs(4));
        assert_eq!(outcome.native_exit, Some(ChildExit::Signal(15)));
    }

    #[test]
    fn runner_exit_status_keeps_native_code_signal_and_cleanup_outcome_distinct() {
        for (native, complete, expected) in [
            (Some(ChildExit::Code(0)), true, 0),
            (Some(ChildExit::Code(7)), true, 7),
            (Some(ChildExit::Signal(15)), true, 143),
            (Some(ChildExit::Code(0)), false, 1),
            (Some(ChildExit::Code(7)), false, 7),
            (Some(ChildExit::Signal(15)), false, 143),
            (None, false, 1),
        ] {
            assert_eq!(runner_exit_status(native, complete).unwrap(), expected);
        }
        assert!(runner_exit_status(None, true).is_err());
    }

    #[test]
    fn runner_database_deadline_bounds_a_locked_write_and_restores_the_busy_timeout() {
        let mut fixture = Fixture::new();
        fixture
            .conn
            .execute_batch("CREATE TABLE deadline_fixture(value INTEGER)")
            .unwrap();
        fixture
            .conn
            .busy_timeout(Duration::from_millis(137))
            .unwrap();
        let other =
            crate::db::connect(&format!("sqlite:///{}", fixture.conn.path().unwrap())).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let start = Instant::now();
        let result = with_database_deadline(
            &mut fixture.conn,
            start + Duration::from_millis(80),
            |conn| {
                conn.execute("INSERT INTO deadline_fixture VALUES(1)", [])
                    .map(|_| ())
                    .map_err(|error| CafleetError::App(error.to_string()))
            },
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(750));
        assert_eq!(
            fixture
                .conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            137
        );
        other.execute_batch("ROLLBACK").unwrap();
        fixture
            .conn
            .execute("INSERT INTO deadline_fixture VALUES(2)", [])
            .unwrap();
    }

    #[test]
    fn runner_database_deadline_cancels_a_long_statement_and_removes_its_progress_handler() {
        let mut fixture = Fixture::new();
        let start = Instant::now();
        let result = with_database_deadline(
            &mut fixture.conn,
            start + Duration::from_millis(80),
            |conn| sum(conn, SLOW_SUM),
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(750));
        assert_eq!(sum(&fixture.conn, FAST_SUM).unwrap(), 50005000);
        assert_eq!(
            fixture
                .conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            5000
        );
    }

    #[test]
    fn runner_database_deadline_preserves_an_already_expired_shared_budget() {
        let mut fixture = Fixture::new();
        let mut invoked = false;
        let result = with_database_deadline(
            &mut fixture.conn,
            Instant::now() - Duration::from_secs(1),
            |_| {
                invoked = true;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!invoked);
    }

    #[derive(Debug, PartialEq)]
    struct DeadlinePanic(u32);

    fn progress_checked_sum(conn: &Connection) -> Result<i64, CafleetError> {
        sum(conn, FAST_SUM)
    }

    #[test]
    fn runner_database_deadline_review_unwind_preserves_the_panic_and_restores_connection_reuse() {
        let mut fixture = Fixture::new();
        fixture
            .conn
            .busy_timeout(Duration::from_millis(137))
            .unwrap();
        let until = Instant::now() + Duration::from_millis(40);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
                std::panic::panic_any(DeadlinePanic(194));
            })
        }));
        let payload = match unwound {
            Err(payload) => payload,
            Ok(_) => panic!("the original operation panic must propagate"),
        };
        assert_eq!(
            payload.downcast_ref::<DeadlinePanic>(),
            Some(&DeadlinePanic(194))
        );
        std::thread::sleep(
            until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
        );
        assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
        assert_eq!(
            fixture
                .conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            137
        );
    }

    #[test]
    fn runner_database_deadline_review_operation_error_stops_its_watchdog_and_restores_reuse() {
        let mut fixture = Fixture::new();
        fixture
            .conn
            .busy_timeout(Duration::from_millis(173))
            .unwrap();
        let until = Instant::now() + Duration::from_millis(100);
        let started = Instant::now();
        let error =
            with_database_deadline(&mut fixture.conn, until, |_| -> Result<(), CafleetError> {
                Err(CafleetError::Usage(
                    "fixture deadline operation error".into(),
                ))
            })
            .unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert_eq!(error.message(), "fixture deadline operation error");
        assert!(started.elapsed() < Duration::from_millis(750));
        std::thread::sleep(
            until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
        );
        assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
        assert_eq!(
            fixture
                .conn
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            173
        );
    }

    #[test]
    fn runner_database_deadline_review_inner_unwind_restores_the_live_outer_cancellation_scope() {
        assert_retired_inner_scope_preserves_outer(InnerOutcome::Panic);
    }

    #[test]
    fn runner_database_deadline_review_nested_later_deadline_keeps_the_earliest_outer_budget() {
        let mut fixture = Fixture::new();
        let started = Instant::now();
        let outer_until = started + Duration::from_millis(100);
        let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
            with_database_deadline(conn, started + Duration::from_secs(1), |conn| {
                sum(conn, SLOW_SUM)
            })
        });
        assert!(result.is_err());
        assert!(started.elapsed() >= Duration::from_millis(70));
        assert!(started.elapsed() < Duration::from_millis(750));
        assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
    }

    enum InnerOutcome {
        Success,
        Panic,
    }

    /// The inner scope retires with `outcome`; the outer scope still cancels its slow query.
    fn assert_retired_inner_scope_preserves_outer(outcome: InnerOutcome) {
        let mut fixture = Fixture::new();
        let started = Instant::now();
        let outer_until = started + Duration::from_millis(400);
        let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
            let inner_until = Instant::now() + Duration::from_millis(40);
            match outcome {
                InnerOutcome::Success => {
                    let inner = with_database_deadline(conn, inner_until, |_| Ok(7));
                    assert_eq!(inner.unwrap(), 7);
                }
                InnerOutcome::Panic => {
                    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        with_database_deadline(conn, inner_until, |_| -> Result<(), CafleetError> {
                            std::panic::panic_any(DeadlinePanic(195));
                        })
                    }));
                    let payload = match unwound {
                        Err(payload) => payload,
                        Ok(_) => panic!("the inner operation panic must propagate"),
                    };
                    assert_eq!(
                        payload.downcast_ref::<DeadlinePanic>(),
                        Some(&DeadlinePanic(195))
                    );
                }
            }
            std::thread::sleep(
                inner_until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            assert_eq!(progress_checked_sum(conn)?, 50005000);
            sum(conn, SLOW_SUM)
        });
        assert!(result.is_err());
        assert!(
            started.elapsed() >= Duration::from_millis(350),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(1250));
        assert_eq!(progress_checked_sum(&fixture.conn).unwrap(), 50005000);
    }

    #[test]
    fn runner_database_deadline_review_inner_success_restores_the_live_outer_scope() {
        assert_retired_inner_scope_preserves_outer(InnerOutcome::Success);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::error::CafleetError;
    use rusqlite::Connection;

    /// A recursive sum that runs far past any test deadline unless the scope cancels it.
    pub(crate) const SLOW_SUM: &str = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter";
    /// A recursive sum that crosses several progress-handler checks and totals 50005000.
    pub(crate) const FAST_SUM: &str = "WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<10000) SELECT sum(x) FROM counter";

    pub(crate) fn sum(conn: &Connection, sql: &str) -> Result<i64, CafleetError> {
        conn.query_row(sql, [], |row| row.get(0))
            .map_err(|cause| CafleetError::App(cause.to_string()))
    }
}
