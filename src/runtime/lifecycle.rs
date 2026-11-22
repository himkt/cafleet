use crate::broker;
use crate::broker::NewPlacement;
use crate::broker::Placement;
use crate::broker::RegisteredMember;
use crate::broker::runtime as storage;
use crate::broker::runtime::CleanupState;
use crate::broker::runtime::RunRecord;
use crate::broker::runtime::*;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::PaneTarget;
use crate::multiplexer::SpawnedPane;
use crate::multiplexer::member_pane_name;
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

/// The shutdown boundaries two, three and four units after `start`, each clipped to `limit`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShutdownDeadlines {
    start: Instant,
    unit: Duration,
    limit: Option<Instant>,
}

impl ShutdownDeadlines {
    pub(crate) fn starting_now(clock: &dyn crate::time::Clock) -> Self {
        Self::starting_at(clock.now(), clock.shutdown_unit())
    }
    pub(crate) fn starting_at(start: Instant, unit: Duration) -> Self {
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
    fn clock(&self) -> &dyn crate::time::Clock;
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
    deadlines: ShutdownDeadlines,
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
        host.drain_and_reap_until(deadlines.child_deadline()),
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
    let dropped_display_bytes = note(
        &mut outcome.diagnostics,
        host.flush_display_until(deadlines.display_deadline()),
    )
    .unwrap_or(0);
    let (pending_event_frames, unsaved_observation_gap) = note(
        &mut outcome.diagnostics,
        host.finish_events_until(deadlines.storage_deadline()),
    )
    .unwrap_or((0, 0));
    note(&mut outcome.diagnostics, host.restore_terminal());
    if host.clock().now() > deadlines.storage_deadline() {
        outcome
            .diagnostics
            .push("shutdown exceeded its original storage deadline".into());
    }
    if dropped_display_bytes > 0 {
        outcome.diagnostics.push(format!(
            "display truncated: {dropped_display_bytes} undelivered bytes"
        ));
    }
    if pending_event_frames > 0 || unsaved_observation_gap > 0 {
        outcome.diagnostics.push(format!(
            "pending events={pending_event_frames} unsaved observation gap={unsaved_observation_gap}"
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
    /// The chosen slot's window name, published with the pane.
    pub(crate) window_name: String,
    /// The fleet's `last_window_ordinal` as read under the layout lock, for a new-window slot.
    pub(crate) advances_window_ordinal_from: Option<i64>,
}
pub(crate) struct RunSettings {
    pub(crate) database_identity_json: Value,
    pub(crate) creator: ProcessIdentity,
    pub(crate) launch_nonce: String,
    pub(crate) requested_mode: Option<String>,
    pub(crate) resolved_mode: String,
    pub(crate) mode_source: ModeSource,
    pub(crate) permission_argv_json: Value,
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
        if let Some(read) = member.advances_window_ordinal_from {
            let changed = tx.execute(
                "UPDATE fleets SET last_window_ordinal=?1 WHERE fleet_id=?2 AND last_window_ordinal=?3",
                params![member.placement.window_ordinal, member.fleet_id, read],
            )?;
            if changed != 1 {
                return Err(error(format!(
                    "fleet {} window ordinal changed under the layout lock",
                    member.fleet_id
                )));
            }
        }
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
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock>;
    fn current_executable(&self) -> Result<PathBuf>;
    fn split_runner(
        &mut self,
        pane_name: &str,
        argv: &[String],
        deadline: Instant,
    ) -> Result<SpawnedPane>;
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
    fn shutdown_owned(
        &mut self,
        run: &RunRecord,
        clock: ShutdownDeadlines,
    ) -> Result<ShutdownOutcome>;
    fn close_pane(&mut self, pane: &SpawnedPane, deadline: Instant) -> Result<()>;
    fn settle_hermes(
        &mut self,
        conn: &Connection,
        run: &RunRecord,
        pane_id: &str,
        deadline: Instant,
    ) -> Result<()>;
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
    deadlines: ShutdownDeadlines,
    diagnostics: &[String],
    clock: &dyn crate::time::Clock,
) -> Result<()> {
    with_database_deadline(conn, deadlines.storage_deadline(), |conn| {
        ensure_same_database(conn, &run.database_identity_json)?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE member_runs SET status='failed',ended_at=COALESCE(ended_at,?1),cleanup_state=?2,cleanup_diagnostics_json=?3 WHERE run_id=?4 AND launch_nonce=?5",params![clock.wall_stamp(),if diagnostics.is_empty(){"complete"}else{"incomplete"},serde_json::to_string(diagnostics).map_err(io_error)?,run.run_id,run.launch_nonce])?;
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
    clock: std::sync::Arc<dyn crate::time::Clock>,
}
impl PendingGuard<'_> {
    fn rollback(
        &mut self,
        primary: CafleetError,
        deadlines: ShutdownDeadlines,
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
        if let Err(cause) = pending_failure(
            self.conn,
            &current,
            deadlines,
            &diagnostics,
            self.clock.as_ref(),
        ) {
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
            let deadlines = ShutdownDeadlines::starting_now(self.clock.as_ref());
            let cause = self.rollback(
                error("pending creation was interrupted"),
                deadlines,
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
        clock: host.clock(),
    };
    let (mut artifacts, spawn) = match prepare(guard.conn, &pending) {
        Ok(value) => value,
        Err(cause) => {
            return Err(guard.rollback(
                cause,
                ShutdownDeadlines::starting_now(&*host.clock()),
                Vec::new(),
            ));
        }
    };
    launch_registered(
        &mut guard,
        pending,
        &member.window_name,
        &mut artifacts,
        &spawn,
        host,
        deadline,
    )
}

fn launch_registered(
    guard: &mut PendingGuard<'_>,
    pending: PendingRegistration,
    window_name: &str,
    artifacts: &mut RunArtifacts,
    spawn: &SpawnSpec,
    host: &mut impl LaunchHost,
    deadline: Instant,
) -> Result<LaunchOutcome> {
    let mut pane = None;
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_deadline(host.clock().now(), deadline)?;
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
        pane = Some(host.split_runner(
            &member_pane_name(pending.member.member_id, &pending.member.name),
            &argv,
            deadline,
        )?);
        check_deadline(host.clock().now(), deadline)?;
        let prepared = host.wait_prepared(&descriptor, deadline)?;
        check_deadline(host.clock().now(), deadline)?;
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
            let changed=tx.execute("UPDATE member_runs SET status='prepared',last_health_at=?1 WHERE run_id=?2 AND owner_epoch=?3 AND status='starting'",params![host.clock().wall_stamp(),run.run_id,current.owner_epoch])?;
            ensure(changed == 1, "prepared publication lost run ownership")?;
            let spawned = pane.as_ref().expect("created pane");
            let changed = tx.execute(
                "UPDATE member_placements SET mux_pane_id=?1, mux_window_id=?2, mux_window_name=?3
                 WHERE member_id=?4 AND mux_pane_id IS NULL",
                params![
                    spawned.pane_id,
                    spawned.window_id,
                    window_name,
                    run.member_id
                ],
            )?;
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
        check_deadline(host.clock().now(), deadline)?;
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
    let deadlines = ShutdownDeadlines::starting_now(&*host.clock());
    let mut notes = Vec::new();
    match storage::get_run(guard.conn, pending.run.run_id) {
        Ok(current) if current.owner_kind == OwnerKind::Runner => {
            if current.backend == crate::coding_agent::CodingAgent::Hermes.name()
                && current.child_pid.is_some()
                && let Some(pane_id) = pane.as_ref().map(|spawned| spawned.pane_id.as_str())
            {
                let until = crate::coding_agent::hermes::settlement_deadline().instant();
                note(
                    &mut notes,
                    host.settle_hermes(guard.conn, &current, pane_id, until),
                );
            }
            let panicked = "owned child cleanup panicked; explicit reconciliation required";
            if let Some(outcome) = guarded(&mut notes, panicked, || {
                host.shutdown_owned(&current, deadlines)
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
            host.close_pane(&pane, deadlines.storage_deadline())
        });
    }
    let failure = guard.rollback(primary, deadlines, notes);
    if let Some(payload) = unwind {
        eprintln!("{failure}");
        std::panic::resume_unwind(payload);
    }
    Err(failure)
}

/// Spawns the runner pane at `target`, starting in `cwd`, named by each `split_runner` call.
pub(crate) struct NativeLaunchHost<'a> {
    mux: &'a dyn Multiplexer,
    target: &'a PaneTarget,
    cwd: &'a str,
    client: Option<ControlClient>,
    clock: std::sync::Arc<dyn crate::time::Clock>,
}

impl<'a> NativeLaunchHost<'a> {
    pub(crate) fn new(
        mux: &'a dyn Multiplexer,
        target: &'a PaneTarget,
        cwd: &'a str,
        clock: std::sync::Arc<dyn crate::time::Clock>,
    ) -> Self {
        Self {
            mux,
            target,
            cwd,
            client: None,
            clock,
        }
    }
}

impl LaunchHost for NativeLaunchHost<'_> {
    fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
        self.clock.clone()
    }
    fn current_executable(&self) -> Result<std::path::PathBuf> {
        std::env::current_exe().map_err(io_error)
    }
    fn split_runner(
        &mut self,
        pane_name: &str,
        argv: &[String],
        deadline: Instant,
    ) -> Result<SpawnedPane> {
        self.mux
            .bounded(crate::time::Deadline::until(deadline))
            .spawn_member_pane(self.target, pane_name, self.cwd, argv)
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
        let mut client =
            ControlClient::connect_starting(&paths.control_socket, deadline, self.clock.as_ref())?;
        let request = LifecycleRequest::Prepare {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            identity: descriptor.identity.clone(),
            remaining_ms: deadline
                .saturating_duration_since(self.clock.now())
                .as_millis()
                .min(10000) as u64,
        };
        let result = client.exchange(&request, deadline, self.clock.as_ref())?;
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
            pane_id: placement
                .mux_pane_id
                .clone()
                .expect("published placement carries the created pane"),
        };
        let value = client.exchange(&request, deadline, self.clock.as_ref())?;
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
    fn shutdown_owned(
        &mut self,
        run: &RunRecord,
        deadlines: ShutdownDeadlines,
    ) -> Result<ShutdownOutcome> {
        if let Ok(value) = request(
            run,
            LifecycleRequest::shutdown(run),
            deadlines.storage_deadline(),
            self.clock.as_ref(),
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
            while runner.matches()? && self.clock.now() < deadlines.storage_deadline() {
                pause(deadlines.storage_deadline(), self.clock.as_ref())?;
            }
        }
        let mut notes=vec!["runner control shutdown unavailable; descendant cleanup requires explicit reconciliation".into()];
        if runner.matches()? {
            notes.push("owned runner termination remains unconfirmed".into());
        }
        Ok(ShutdownOutcome::new(None, notes))
    }
    fn close_pane(&mut self, pane: &SpawnedPane, deadline: Instant) -> Result<()> {
        self.mux
            .bounded(crate::time::Deadline::until(deadline))
            .kill_pane(&pane.pane_id, &pane.window_id)
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
            let target = PaneTarget::Column(crate::multiplexer::ColumnTarget {
                window_id: "@1".into(),
                side: crate::multiplexer::layout::ColumnSide::Right,
                column_panes: Vec::new(),
                other_panes: vec!["%1".into()],
            });
            let result = operation(&mut NativeLaunchHost::new(
                &mux,
                &target,
                "/readiness-fixture",
                std::sync::Arc::new(crate::time::SystemClock),
            ));
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
                pause_until(&crate::time::SystemClock, deadline);
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
                        pause_until(&crate::time::SystemClock, ready_at);
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
    use crate::multiplexer::SpawnedPane;
    use crate::runtime::lifecycle::*;
    use crate::time::Clock;
    use crate::time::test_support::TestClock;
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
        clock: std::sync::Arc<TestClock>,
        delay: Duration,
        calls: Vec<&'static str>,
        deadlines: Vec<(&'static str, Instant)>,
        pane_name: Option<String>,
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
        closed: Vec<SpawnedPane>,
    }

    impl LaunchFixtureHost {
        fn new(fixture: &Fixture) -> Self {
            Self {
                database_url: format!("sqlite:///{}", fixture.conn.path().unwrap()),
                clock: std::sync::Arc::new(TestClock::starting_at(Instant::now())),
                delay: Duration::ZERO,
                calls: Vec::new(),
                deadlines: Vec::new(),
                pane_name: None,
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
                closed: Vec::new(),
            }
        }
        fn phase(&mut self, name: &'static str, deadline: Instant) -> Result<(), CafleetError> {
            self.calls.push(name);
            self.deadlines.push((name, deadline));
            self.clock.advance(self.delay);
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

    const FIXTURE_WINDOW: &str = "@4";

    fn fixture_pane() -> SpawnedPane {
        SpawnedPane {
            pane_id: "%9".into(),
            window_id: FIXTURE_WINDOW.into(),
        }
    }

    impl LaunchHost for LaunchFixtureHost {
        fn clock(&self) -> std::sync::Arc<dyn crate::time::Clock> {
            self.clock.clone()
        }
        fn current_executable(&self) -> Result<PathBuf, CafleetError> {
            Ok("/absolute/cafleet".into())
        }
        fn split_runner(
            &mut self,
            pane_name: &str,
            argv: &[String],
            deadline: Instant,
        ) -> Result<SpawnedPane, CafleetError> {
            self.phase("split", deadline)?;
            self.pane_name = Some(pane_name.to_owned());
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
            Ok(fixture_pane())
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
                let evidence=crate::broker::runtime::metadata_evidence(&json!({"backend":"codex","native_version":"0.163.0","rule_id":"native-tool-permission","screen_seq":1,"operation":null,"choices":null,"excerpt":null,"truncated":false})).unwrap();
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
            clock: ShutdownDeadlines,
        ) -> Result<ShutdownOutcome, CafleetError> {
            self.calls.push("shutdown");
            assert_eq!(
                clock.storage_deadline(),
                self.clock.now() + Duration::from_secs(4)
            );
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
        fn close_pane(
            &mut self,
            pane: &SpawnedPane,
            deadline: Instant,
        ) -> Result<(), CafleetError> {
            self.calls.push("close");
            self.deadlines.push(("close", deadline));
            self.closed.push(pane.clone());
            assert_eq!(pane, &fixture_pane());
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
        let deadline = host.clock.now() + Duration::from_secs(10);
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
                window_name: "main".into(),
                advances_window_ordinal_from: None,
            },
            RunSettings {
                database_identity_json: fixture.run.database_identity_json.clone(),
                creator: ProcessIdentity::current().unwrap(),
                launch_nonce: "c".repeat(64),
                requested_mode: None,
                resolved_mode: "approve-for-me".into(),
                mode_source: ModeSource::Default,
                permission_argv_json: fixture.run.permission_argv_json.clone(),
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

    fn assert_compensated(fixture: &Fixture, run: &RunRecord, name: &str) {
        assert_unlaunched(
            fixture,
            run,
            MemberStatus::Deregistered,
            RunStatus::Failed,
            name,
        );
    }

    /// The member and run rows end in these states and the launch descriptor is gone.
    fn assert_unlaunched(
        fixture: &Fixture,
        run: &RunRecord,
        member: MemberStatus,
        status: RunStatus,
        name: &str,
    ) {
        let recorded = fixture
            .conn
            .query_row(
                "SELECT status FROM members WHERE member_id=?1",
                [run.member_id],
                |row| row.get::<_, MemberStatus>(0),
            )
            .unwrap();
        assert_eq!(recorded, member, "{name}");
        assert_eq!(
            storage::get_run(&fixture.conn, run.run_id).unwrap().status,
            status,
            "{name}"
        );
        let paths = RuntimePaths::for_run(&run.database_identity_json, run.run_id).unwrap();
        assert!(!paths.descriptor.exists(), "{name}");
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

    struct VersionCase {
        name: &'static str,
        native: Option<&'static str>,
        coverage: Coverage,
    }

    const VERSION_CASES: &[VersionCase] = &[
        VersionCase {
            name: "an unprobed native version records unknown coverage",
            native: None,
            coverage: Coverage::Unknown,
        },
        VersionCase {
            name: "a probed tested native version records tested coverage",
            native: Some("0.164.0"),
            coverage: Coverage::Tested,
        },
    ];

    #[test]
    fn runner_creation_records_the_probed_version_and_coverage_on_the_pending_run() {
        for case in VERSION_CASES {
            let name = case.name;
            let mut fixture = Fixture::new();
            let (member, mut settings) = inputs(&fixture);
            settings.native_version = case.native.map(str::to_owned);
            settings.coverage = case.coverage;
            let pending = register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            let reread = storage::get_run(&fixture.conn, pending.run.run_id).unwrap();
            for run in [&pending.run, &reread] {
                assert_eq!(run.native_version.as_deref(), case.native, "{name}");
                assert_eq!(run.coverage, case.coverage, "{name}");
            }
        }
    }

    /// How far preparation gets before it fails or unwinds: the stage names the case.
    const PREPARATION_STAGES: &[&str] =
        &["before-artifacts", "after-artifacts", "after-descriptor"];

    #[test]
    fn runner_creation_guard_covers_preparation_failure_before_artifacts_after_allocation_and_after_descriptor()
     {
        for &stage in PREPARATION_STAGES {
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
            assert!(
                error.message().contains("fixture preparation failure"),
                "{stage}: {error}"
            );
            assert_compensated(&fixture, &pending_run(&fixture), stage);
            assert!(host.calls.is_empty(), "{stage}");
            assert_eq!(host.spawn_count, 0, "{stage}");
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
        let allocated = pending_run(&fixture).member_id;
        assert_eq!(outcome.member.member_id, allocated);
        assert_eq!(outcome.member.name, "pending-worker");
        assert_eq!(
            host.pane_name.as_deref(),
            Some(format!("{allocated} - pending-worker").as_str())
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

    /// One failed launch: the fault the host injects and everything the compensation must leave.
    struct CompensationCase {
        name: &'static str,
        backend: &'static str,
        fail: Option<&'static str>,
        corrupt: Option<&'static str>,
        publication: Option<PublicationFailure>,
        delay: Duration,
        cleanup_failure: bool,
        registration_cleanup_fails: bool,
        hermes_failure: Option<&'static str>,
        calls: &'static [&'static str],
        spawns: usize,
        /// Text the first line of the error carries: the primary failure.
        primary: &'static str,
        /// Each appears exactly once in the complete error.
        diagnostics: &'static [&'static str],
        member: MemberStatus,
        run_status: RunStatus,
        cleanup_state: CleanupState,
    }

    const COMPENSATION: CompensationCase = CompensationCase {
        name: "",
        backend: "codex",
        fail: None,
        corrupt: None,
        publication: None,
        delay: Duration::ZERO,
        cleanup_failure: false,
        registration_cleanup_fails: false,
        hermes_failure: None,
        calls: &[],
        spawns: 0,
        primary: "",
        diagnostics: &[],
        member: MemberStatus::Deregistered,
        run_status: RunStatus::Failed,
        cleanup_state: CleanupState::Complete,
    };

    const SPAWNED: &[&str] = &["split", "prepared", "start", "shutdown", "close"];
    const UNSPAWNED: &[&str] = &["split", "prepared", "shutdown", "close"];

    const COMPENSATION_CASES: &[CompensationCase] = &[
        CompensationCase {
            name: "a split failure compensates the registration without a pane",
            fail: Some("split"),
            calls: &["split"],
            primary: "fixture split failure",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a prepare failure closes the owned pane",
            fail: Some("prepared"),
            calls: &["split", "prepared", "close"],
            primary: "fixture prepared failure",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a spawn failure stops the runner and closes the pane",
            fail: Some("start"),
            calls: SPAWNED,
            primary: "fixture start failure",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a stale prepared identity is rejected before starting a child",
            corrupt: Some("prepared-nonce"),
            calls: UNSPAWNED,
            primary: "identity",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a lost started reply compensates one uncertain spawn",
            fail: Some("started-reply"),
            calls: SPAWNED,
            spawns: 1,
            primary: "fixture started reply lost after spawn",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a started reply with a stale epoch compensates one uncertain spawn",
            corrupt: Some("started-epoch"),
            calls: SPAWNED,
            spawns: 1,
            primary: "identity",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a started reply naming another child compensates one uncertain spawn",
            corrupt: Some("started-child"),
            calls: SPAWNED,
            spawns: 1,
            primary: "started reply lacks a matching completed native spawn",
            ..COMPENSATION
        },
        CompensationCase {
            name: "one handshake deadline expires after start and compensates the spawn",
            delay: Duration::from_secs(4),
            calls: SPAWNED,
            spawns: 1,
            primary: "deadline",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a missing placement at publication compensates the owned pane once",
            publication: Some(PublicationFailure::Missing),
            calls: UNSPAWNED,
            primary: "placement",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a rejected publication compensates the owned pane once",
            publication: Some(PublicationFailure::Rejected),
            calls: UNSPAWNED,
            primary: "fixture publication blocked",
            diagnostics: &["fixture publication blocked"],
            ..COMPENSATION
        },
        CompensationCase {
            name: "a rejected publication keeps every combined cleanup error once",
            publication: Some(PublicationFailure::Rejected),
            cleanup_failure: true,
            registration_cleanup_fails: true,
            calls: UNSPAWNED,
            primary: "fixture publication blocked",
            diagnostics: &[
                "fixture publication blocked",
                "fixture owned-child cleanup failed",
                "fixture pane cleanup failed",
                "fixture database cleanup blocked",
            ],
            member: MemberStatus::Active,
            run_status: RunStatus::Starting,
            cleanup_state: CleanupState::Incomplete,
            ..COMPENSATION
        },
        CompensationCase {
            name: "a lost started reply keeps the primary failure and every owned cleanup diagnostic",
            fail: Some("started-reply"),
            cleanup_failure: true,
            calls: SPAWNED,
            spawns: 1,
            primary: "fixture started reply lost after spawn",
            diagnostics: &[
                "fixture owned-child cleanup failed",
                "fixture pane cleanup failed",
            ],
            cleanup_state: CleanupState::Incomplete,
            ..COMPENSATION
        },
        CompensationCase {
            name: "a hermes run whose child started settles its client before runner shutdown",
            backend: "hermes",
            fail: Some("started-reply"),
            calls: &["split", "prepared", "start", "settle", "shutdown", "close"],
            spawns: 1,
            primary: "fixture started reply lost after spawn",
            ..COMPENSATION
        },
        CompensationCase {
            name: "a failed hermes settlement keeps its diagnostic and still fails the run",
            backend: "hermes",
            fail: Some("started-reply"),
            hermes_failure: Some("fixture hermes settlement failed"),
            calls: &["split", "prepared", "start", "settle", "shutdown", "close"],
            spawns: 1,
            primary: "fixture started reply lost after spawn",
            diagnostics: &["fixture hermes settlement failed"],
            cleanup_state: CleanupState::Incomplete,
            ..COMPENSATION
        },
        CompensationCase {
            name: "a hermes run whose spawn failed has no client to settle",
            backend: "hermes",
            fail: Some("start"),
            calls: SPAWNED,
            primary: "fixture start failure",
            ..COMPENSATION
        },
    ];

    #[test]
    fn runner_creation_failure_compensates_every_owned_effect_once() {
        for case in COMPENSATION_CASES {
            let name = case.name;
            let (mut fixture, mut host, deadline) = launch_case();
            let cleanup_attempts = count_registration_cleanup(&mut fixture.conn);
            if case.registration_cleanup_fails {
                reject_registration_cleanup(&fixture.conn);
            }
            host.fail = case.fail;
            host.corrupt = case.corrupt;
            host.publication_failure = case.publication;
            host.delay = case.delay;
            host.cleanup_failure = case.cleanup_failure;
            host.hermes_failure = case.hermes_failure;
            let error = require_launch_error(launch_backend(
                &mut fixture,
                &mut host,
                deadline,
                case.backend,
            ));
            let text = error.message();
            assert_eq!(error.exit_code(), 1, "{name}: {text}");
            assert!(
                text.lines().next().unwrap().contains(case.primary),
                "{name}: {text}"
            );
            for diagnostic in case.diagnostics {
                assert_eq!(text.matches(diagnostic).count(), 1, "{name}: {text}");
            }
            assert_eq!(host.calls, case.calls, "{name}");
            assert_eq!(host.spawn_count, case.spawns, "{name}");
            assert!(
                host.deadlines
                    .iter()
                    .filter(|(phase, _)| *phase != "close")
                    .all(|(_, recorded)| *recorded == deadline),
                "{name}: {:?}",
                host.deadlines
            );
            assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1, "{name}");
            let run = pending_run(&fixture);
            assert_unlaunched(&fixture, &run, case.member, case.run_status, name);
            assert_eq!(run.cleanup_state, case.cleanup_state, "{name}");
        }
    }

    #[test]
    fn runner_creation_preserves_usage_exit_through_preparation_and_launch_cleanup() {
        for phase in ["preparation", "start"] {
            for cleanup_fails in [false, true] {
                let name = format!("usage failure at {phase}, cleanup fails: {cleanup_fails}");
                let name = name.as_str();
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
                assert_eq!(error.exit_code(), 2, "{name}: {error}");
                assert!(
                    error.message().starts_with(&format!("fixture {phase}")),
                    "{name}: {error}"
                );
                assert_eq!(
                    error
                        .message()
                        .matches("fixture database cleanup blocked")
                        .count(),
                    usize::from(cleanup_fails),
                    "{name}"
                );
                assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1, "{name}");
                assert_eq!(host.spawn_count, 0, "{name}");
                let run = pending_run(&fixture);
                if cleanup_fails {
                    assert!(
                        crate::broker::get_member(&fixture.conn, run.member_id, run.fleet_id)
                            .unwrap()
                            .is_some(),
                        "{name}"
                    );
                } else {
                    assert_compensated(&fixture, &run, name);
                }
                let expected_cleanup: &[&str] = if phase == "start" {
                    &["shutdown", "close"]
                } else {
                    &[]
                };
                let cleanup: Vec<&str> = host
                    .calls
                    .iter()
                    .copied()
                    .filter(|call| ["shutdown", "close"].contains(call))
                    .collect();
                assert_eq!(cleanup, expected_cleanup, "{name}");
            }
        }
    }

    #[test]
    fn runner_creation_unwind_during_preparation_compensates_registration_and_descriptor_once() {
        for &stage in PREPARATION_STAGES {
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
            assert_eq!(cleanup_attempts.load(Ordering::SeqCst), 1, "{stage}");
            assert_compensated(&fixture, &pending_run(&fixture), stage);
            assert!(host.calls.is_empty(), "{stage}");
            assert_eq!(host.spawn_count, 0, "{stage}");
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
        assert_compensated(&fixture, &pending_run(&fixture), phase);
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
        assert_compensated(&fixture, &run, "invalid spawn construction");
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
                ShutdownDeadlines::starting_at(start, crate::time::SHUTDOWN_UNIT)
                    .remaining_storage_at(until),
                Duration::from_secs(4).saturating_sub(Duration::from_secs(offset)),
                "clipped {offset} s after start"
            );
            let clock =
                ShutdownDeadlines::starting_at(start, crate::time::SHUTDOWN_UNIT).clipped(until);
            assert_eq!(
                clock.child_deadline(),
                (start + Duration::from_secs(2)).min(until),
                "clipped {offset} s after start"
            );
            assert_eq!(
                clock.display_deadline(),
                (start + Duration::from_secs(3)).min(until),
                "clipped {offset} s after start"
            );
            assert_eq!(
                clock.storage_deadline(),
                (start + Duration::from_secs(4)).min(until),
                "clipped {offset} s after start"
            );
            assert_eq!(
                clock.remaining_storage_at(until),
                Duration::ZERO,
                "clipped {offset} s after start"
            );
        }
        let past = start.checked_sub(Duration::from_secs(1)).unwrap();
        let clock = ShutdownDeadlines::starting_at(start, crate::time::SHUTDOWN_UNIT).clipped(past);
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
        let clock = ShutdownDeadlines::starting_at(start, crate::time::SHUTDOWN_UNIT)
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
        clock: TestClock,
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
                clock: TestClock::starting_at(now),
                calls: Vec::new(),
                deadlines: Vec::new(),
                fail: BTreeSet::new(),
                native: Some(ChildExit::Code(0)),
                dropped: 0,
                pending: 0,
                gap: 0,
            }
        }
        fn reach(&self, deadline: Instant) {
            if let Some(remaining) = deadline.checked_duration_since(self.clock.now()) {
                self.clock.advance(remaining);
            }
        }
        fn call(&mut self, name: &'static str) -> Result<(), CafleetError> {
            self.calls.push((name, self.clock.now()));
            if self.fail.contains(name) {
                Err(CafleetError::App(format!("fixture {name} failure")))
            } else {
                Ok(())
            }
        }
    }

    impl ShutdownHost for Host {
        fn clock(&self) -> &dyn crate::time::Clock {
            &self.clock
        }
        fn gate_programmatic_input(&mut self) {
            self.calls.push(("gate", self.clock.now()));
        }
        fn term_verified_descendants(&mut self) -> Result<(), CafleetError> {
            self.call("term")
        }
        fn drain_and_reap_until(
            &mut self,
            deadline: Instant,
        ) -> Result<Option<ChildExit>, CafleetError> {
            self.deadlines.push(("drain", deadline));
            self.reach(deadline);
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
            self.reach(deadline);
            self.call("display")?;
            Ok(self.dropped)
        }
        fn finish_events_until(&mut self, deadline: Instant) -> Result<(u64, u64), CafleetError> {
            self.deadlines.push(("events", deadline));
            self.reach(deadline);
            self.call("events")?;
            Ok((self.pending, self.gap))
        }
        fn restore_terminal(&mut self) -> Result<(), CafleetError> {
            self.call("restore")
        }
    }

    /// One shutdown: the host's start and faults, the exit reported before shutdown, and the
    /// calls and outcome the one original schedule must produce.
    struct ShutdownCase {
        name: &'static str,
        host_starts_after: Duration,
        drained: Option<ChildExit>,
        fail: &'static [&'static str],
        dropped: u64,
        pending: u64,
        gap: u64,
        reported: Option<ChildExit>,
        calls: &'static [&'static str],
        native_exit: Option<ChildExit>,
        cleanup_state: CleanupState,
        diagnostics: &'static [&'static str],
    }

    const EVERY_SHUTDOWN_CALL: &[&str] = &[
        "gate", "term", "drain", "kill", "close", "display", "events", "restore",
    ];

    const SHUTDOWN_CASES: &[ShutdownCase] = &[
        ShutdownCase {
            name: "one clock bounds descendants, display, events and restoration",
            host_starts_after: Duration::ZERO,
            drained: None,
            fail: &[],
            dropped: 0,
            pending: 0,
            gap: 0,
            reported: Some(ChildExit::Code(7)),
            calls: EVERY_SHUTDOWN_CALL,
            native_exit: Some(ChildExit::Code(7)),
            cleanup_state: CleanupState::Complete,
            diagnostics: &[],
        },
        ShutdownCase {
            name: "every failure is collected and the terminal is still restored without an extra budget",
            host_starts_after: Duration::ZERO,
            drained: None,
            fail: &[
                "term", "drain", "kill", "close", "display", "events", "restore",
            ],
            dropped: 0,
            pending: 0,
            gap: 0,
            reported: Some(ChildExit::Code(9)),
            calls: EVERY_SHUTDOWN_CALL,
            native_exit: Some(ChildExit::Code(9)),
            cleanup_state: CleanupState::Incomplete,
            diagnostics: &[
                "fixture term failure",
                "fixture drain failure",
                "fixture kill failure",
                "fixture close failure",
                "fixture display failure",
                "fixture events failure",
                "fixture restore failure",
            ],
        },
        ShutdownCase {
            name: "display truncation, pending frames and unsaved gaps stay separate",
            host_starts_after: Duration::ZERO,
            drained: Some(ChildExit::Code(0)),
            fail: &[],
            dropped: 17,
            pending: 2,
            gap: 3,
            reported: None,
            calls: EVERY_SHUTDOWN_CALL,
            native_exit: Some(ChildExit::Code(0)),
            cleanup_state: CleanupState::Incomplete,
            diagnostics: &[
                "display truncated: 17 undelivered bytes",
                "pending events=2 unsaved observation gap=3",
            ],
        },
        ShutdownCase {
            name: "a clock started before the current phase is never rebased",
            host_starts_after: Duration::from_secs(3),
            drained: None,
            fail: &[],
            dropped: 0,
            pending: 0,
            gap: 0,
            reported: Some(ChildExit::Signal(15)),
            calls: EVERY_SHUTDOWN_CALL,
            native_exit: Some(ChildExit::Signal(15)),
            cleanup_state: CleanupState::Complete,
            diagnostics: &[],
        },
    ];

    #[test]
    fn runner_shutdown_follows_one_original_schedule_and_keeps_every_outcome() {
        for case in SHUTDOWN_CASES {
            let name = case.name;
            let start = Instant::now();
            let host_start = start + case.host_starts_after;
            let mut host = Host::new(host_start);
            host.native = case.drained;
            host.fail = case.fail.iter().copied().collect();
            host.dropped = case.dropped;
            host.pending = case.pending;
            host.gap = case.gap;
            let outcome = shutdown(
                &mut host,
                ShutdownDeadlines::starting_at(start, crate::time::SHUTDOWN_UNIT),
                case.reported,
            );
            let calls: Vec<_> = host.calls.iter().map(|(call, _)| *call).collect();
            assert_eq!(calls, case.calls, "{name}");
            assert_eq!(
                host.deadlines,
                [
                    ("drain", start + Duration::from_secs(2)),
                    ("display", start + Duration::from_secs(3)),
                    ("events", start + Duration::from_secs(4))
                ],
                "{name}"
            );
            let child_deadline = (start + Duration::from_secs(2)).max(host_start);
            for (call, time) in &host.calls {
                if ["kill", "close"].contains(call) {
                    assert_eq!(*time, child_deadline, "{name}: {call}");
                }
            }
            assert_eq!(host.clock.now(), start + Duration::from_secs(4), "{name}");
            assert_eq!(outcome.native_exit, case.native_exit, "{name}");
            assert_eq!(outcome.cleanup_state, case.cleanup_state, "{name}");
            assert_eq!(outcome.diagnostics, case.diagnostics, "{name}");
        }
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
            assert_eq!(
                runner_exit_status(native, complete).unwrap(),
                expected,
                "native {native:?}, cleanup complete: {complete}"
            );
        }
        assert!(
            runner_exit_status(None, true).is_err(),
            "complete cleanup without a native exit"
        );
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
            |conn| {
                conn.query_row(SLOW_SUM, [], |row| row.get::<_, i64>(0))
                    .map_err(CafleetError::from)
            },
        );
        let error = result.expect_err("the deadline cancels the long statement");
        assert!(
            error.transient_storage(),
            "a statement cut off by its deadline is transient: {error:?}"
        );
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

    #[derive(Clone, Copy)]
    enum InnerOutcome {
        Success,
        Panic,
    }

    const INNER_OUTCOMES: &[(&str, InnerOutcome)] = &[
        ("an inner scope that succeeds", InnerOutcome::Success),
        ("an inner scope that unwinds", InnerOutcome::Panic),
    ];

    #[test]
    fn runner_database_deadline_review_a_retired_inner_scope_restores_the_live_outer_scope() {
        for &(name, outcome) in INNER_OUTCOMES {
            assert_retired_inner_scope_preserves_outer(name, outcome);
        }
    }

    /// The inner scope retires with `outcome`; the outer scope still cancels its slow query.
    fn assert_retired_inner_scope_preserves_outer(name: &str, outcome: InnerOutcome) {
        let mut fixture = Fixture::new();
        let started = Instant::now();
        let outer_until = started + Duration::from_millis(400);
        let result = with_database_deadline(&mut fixture.conn, outer_until, |conn| {
            let inner_until = Instant::now() + Duration::from_millis(40);
            match outcome {
                InnerOutcome::Success => {
                    let inner = with_database_deadline(conn, inner_until, |_| Ok(7));
                    assert_eq!(inner.unwrap(), 7, "{name}");
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
                        Some(&DeadlinePanic(195)),
                        "{name}"
                    );
                }
            }
            std::thread::sleep(
                inner_until.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
            assert_eq!(progress_checked_sum(conn)?, 50005000);
            sum(conn, SLOW_SUM)
        });
        assert!(result.is_err(), "{name}");
        assert!(
            started.elapsed() >= Duration::from_millis(350),
            "{name}: {result:?}"
        );
        assert!(started.elapsed() < Duration::from_millis(1250), "{name}");
        assert_eq!(
            progress_checked_sum(&fixture.conn).unwrap(),
            50005000,
            "{name}"
        );
    }

    mod window_layout {
        use crate::multiplexer::layout::ColumnSide;
        use crate::runtime::lifecycle::tests::*;

        fn last_window_ordinal(fixture: &Fixture) -> i64 {
            fixture
                .conn
                .query_row(
                    "SELECT last_window_ordinal FROM fleets WHERE fleet_id=?1",
                    [fixture.run.fleet_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        fn count(fixture: &Fixture, table: &str) -> i64 {
            fixture
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        }

        /// A pending member that opens overflow window 1 from a fleet whose last ordinal is 0.
        fn overflow_member(fixture: &Fixture) -> (PendingMemberSpec, RunSettings) {
            let (mut member, settings) = inputs(fixture);
            member.placement.window_ordinal = 1;
            member.placement.column_side = ColumnSide::Left;
            member.window_name = format!("cafleet-{}-1", fixture.run.fleet_id);
            member.advances_window_ordinal_from = Some(0);
            (member, settings)
        }

        fn launch_member(
            fixture: &mut Fixture,
            host: &mut LaunchFixtureHost,
            deadline: Instant,
            member: &PendingMemberSpec,
            settings: &RunSettings,
        ) -> Result<LaunchOutcome, CafleetError> {
            let spec = fixture.spec();
            create_registered(
                &mut fixture.conn,
                member,
                settings,
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

        #[test]
        fn publication_records_the_spawned_pane_and_window_with_the_chosen_window_name() {
            let (mut fixture, mut host, deadline) = launch_case();
            let (member, settings) = overflow_member(&fixture);
            let outcome =
                launch_member(&mut fixture, &mut host, deadline, &member, &settings).unwrap();
            let window_name = format!("cafleet-{}-1", fixture.run.fleet_id);
            let recorded = |placement: &Placement| {
                (
                    placement.mux_pane_id.clone(),
                    placement.mux_window_id.clone(),
                    placement.mux_window_name.clone(),
                    placement.window_ordinal,
                    placement.column_side,
                )
            };
            let expected = (
                Some("%9".to_string()),
                Some(FIXTURE_WINDOW.to_string()),
                Some(window_name),
                1,
                ColumnSide::Left,
            );
            assert_eq!(recorded(&outcome.placement), expected);
            let stored = crate::broker::get_member(
                &fixture.conn,
                outcome.member.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement;
            assert_eq!(recorded(&stored), expected);
        }

        #[test]
        fn a_director_window_member_publishes_the_director_window_name_it_was_given() {
            let (mut fixture, mut host, deadline) = launch_case();
            let (mut member, settings) = inputs(&fixture);
            member.window_name = "agents".into();
            let outcome =
                launch_member(&mut fixture, &mut host, deadline, &member, &settings).unwrap();
            assert_eq!(
                (
                    outcome.placement.mux_window_name.as_deref(),
                    outcome.placement.window_ordinal,
                    outcome.placement.column_side,
                ),
                (Some("agents"), 0, ColumnSide::Right)
            );
        }

        struct OverflowFailure {
            name: &'static str,
            fail: &'static str,
            closed: &'static [&'static str],
        }

        const OVERFLOW_FAILURES: &[OverflowFailure] = &[
            OverflowFailure {
                name: "a prepare failure closes the pane in the window it was spawned in",
                fail: "prepared",
                closed: &["%9"],
            },
            OverflowFailure {
                name: "a split failure spawned no pane to close",
                fail: "split",
                closed: &[],
            },
        ];

        #[test]
        fn an_overflow_launch_failure_closes_only_its_spawned_pane_and_keeps_the_new_ordinal_consumed()
         {
            for case in OVERFLOW_FAILURES {
                let name = case.name;
                let (mut fixture, mut host, deadline) = launch_case();
                host.fail = Some(case.fail);
                let (member, settings) = overflow_member(&fixture);
                assert_eq!(last_window_ordinal(&fixture), 0, "{name}");
                let error = require_launch_error(launch_member(
                    &mut fixture,
                    &mut host,
                    deadline,
                    &member,
                    &settings,
                ));
                assert!(
                    error
                        .message()
                        .contains(&format!("fixture {} failure", case.fail)),
                    "{name}: {error}"
                );
                let closed: Vec<SpawnedPane> = case
                    .closed
                    .iter()
                    .map(|&pane_id| SpawnedPane {
                        pane_id: pane_id.into(),
                        window_id: FIXTURE_WINDOW.into(),
                    })
                    .collect();
                assert_eq!(host.closed, closed, "{name}");
                assert_compensated(&fixture, &pending_run(&fixture), name);
                assert_eq!(last_window_ordinal(&fixture), 1, "{name}");
            }
        }

        #[test]
        fn registration_advances_the_window_ordinal_only_for_a_new_window() {
            let mut fixture = Fixture::new();
            let (member, settings) = inputs(&fixture);
            register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            assert_eq!(last_window_ordinal(&fixture), 0);

            let mut fixture = Fixture::new();
            let (member, settings) = overflow_member(&fixture);
            let pending = register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            assert_eq!(last_window_ordinal(&fixture), 1);
            let placement = crate::broker::get_member(
                &fixture.conn,
                pending.member.member_id,
                fixture.run.fleet_id,
            )
            .unwrap()
            .unwrap()
            .placement;
            assert_eq!(
                (
                    placement.window_ordinal,
                    placement.column_side,
                    placement.mux_window_id,
                    placement.mux_window_name,
                    placement.mux_pane_id,
                ),
                (1, ColumnSide::Left, None, None, None)
            );
        }

        #[test]
        fn a_window_ordinal_changed_under_the_lock_rolls_the_registration_back() {
            let mut fixture = Fixture::new();
            let fleet = fixture.run.fleet_id;
            fixture
                .conn
                .execute(
                    "UPDATE fleets SET last_window_ordinal=2 WHERE fleet_id=?1",
                    [fleet],
                )
                .unwrap();
            let (members, runs) = (count(&fixture, "members"), count(&fixture, "member_runs"));
            let (member, settings) = overflow_member(&fixture);
            let error = register_pending_run(
                &mut fixture.conn,
                &member,
                &settings,
                Instant::now() + Duration::from_secs(10),
            )
            .err()
            .expect("a changed ordinal fails registration");
            assert_eq!(
                error.message(),
                format!("fleet {fleet} window ordinal changed under the layout lock")
            );
            assert_eq!(error.exit_code(), 1);
            assert_eq!(count(&fixture, "members"), members);
            assert_eq!(count(&fixture, "member_runs"), runs);
            assert_eq!(last_window_ordinal(&fixture), 2);
        }
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
