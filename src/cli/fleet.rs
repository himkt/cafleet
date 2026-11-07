use crate::broker;
use crate::broker::DeletedFleet;
use crate::broker::DeletedMember;
use crate::broker::MemberRecord;
use crate::broker::PLACEMENT_COLUMNS;
use crate::broker::Placement;
use crate::broker::members::NewPlacement;
use crate::broker::members::active_member;
use crate::cli::emit;
use crate::coding_agent::CodingAgent;
use crate::config::Settings;
use crate::db::finalize_deletion;
use crate::db::with_database_budget;
use crate::error::CafleetError;
use crate::multiplexer::CommandRunner;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MuxBackend;
use crate::output::format_fleet_create;
use crate::output::format_fleet_delete;
use crate::output::format_fleet_list;
use crate::output::format_fleet_show;
use crate::pane_lock::PaneLock;
use crate::pane_lock::lock_deletion;
use crate::pane_lock::lock_path;
use crate::runtime::system::SystemRunner;
use crate::time::Deadline;
use clap::Subcommand;
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Subcommand)]
pub(crate) enum FleetCommand {
    /// Register the calling Director in a new fleet.
    Create {
        /// Fleet name.
        #[arg(long)]
        name: String,
        /// Director's coding agent.
        #[arg(long = "coding-agent", value_enum)]
        coding_agent: CodingAgent,
        /// The Director's pane [default: the caller's pane from TMUX_PANE or HERDR_PANE_ID].
        #[arg(long, value_name = "PANE_ID")]
        pane: Option<String>,
        /// The Director's tmux socket path or herdr session [default: the caller's server].
        #[arg(long = "mux-server", value_name = "SERVER", requires = "pane")]
        mux_server: Option<String>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// List non-deleted fleets.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one fleet.
    Show {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long)]
        json: bool,
    },
    /// End a fleet's ordinary members, then deregister the fleet.
    Delete {
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        #[arg(long)]
        json: bool,
    },
}

pub(crate) fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: FleetCommand,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    match command {
        FleetCommand::Create {
            name,
            coding_agent,
            pane,
            mux_server,
            json,
        } => create(conn, &name, coding_agent, pane, mux_server, json),
        FleetCommand::List { json } => {
            let fleets = broker::list_fleets(conn)?;
            let payload = Value::Array(
                fleets
                    .iter()
                    .map(crate::output::json::fleet_summary)
                    .collect(),
            );
            emit(json, &payload, || format_fleet_list(&fleets))
        }
        FleetCommand::Show { fleet_id, json } => {
            let fleet = broker::require_fleet(conn, fleet_id)?;
            emit(json, &crate::output::json::fleet(&fleet), || {
                format_fleet_show(&fleet)
            })
        }
        FleetCommand::Delete { fleet_id, json } => {
            let result = crate::cli::fleet::delete_fleet(conn, settings, fleet_id, deadline)?;
            emit(json, &crate::output::json::deleted_fleet(&result), || {
                format_fleet_delete(&result)
            })
        }
    }
}

fn create(
    conn: &mut Connection,
    name: &str,
    agent: CodingAgent,
    pane: Option<String>,
    mux_server: Option<String>,
    json: bool,
) -> Result<(), CafleetError> {
    let env = std::env::vars().collect::<std::collections::HashMap<_, _>>();
    let lookup = |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned();
    let backend = MuxBackend::detect(lookup).map_err(|_| {
        CafleetError::App("cafleet fleet create must be run inside a tmux or herdr session".into())
    })?;
    let pane = match pane {
        Some(pane) => pane,
        None => caller_pane(backend, lookup)?,
    };
    let runner: Rc<dyn CommandRunner> = Rc::new(SystemRunner);
    let server = match mux_server {
        Some(server) => server,
        None => crate::multiplexer::caller_server(backend, lookup, runner.as_ref())?,
    };
    let mux = backend.for_server(&server, runner);
    mux.ensure_available()?;
    let context = mux.pane_context(&pane)?.ok_or_else(|| {
        CafleetError::App(format!(
            "pane {pane} was not found on the {} server {server}; pass --pane and --mux-server for the Director's pane",
            backend.name()
        ))
    })?;
    let fleet = broker::create_fleet(
        conn,
        Some(name),
        &NewPlacement {
            backend: backend.name().into(),
            mux_session: context.session,
            mux_window_id: context.window_id,
            mux_pane_id: Some(context.pane_id),
            coding_agent: agent.name().into(),
            mux_server: server,
        },
    )?;
    emit(json, &crate::output::json::created_fleet(&fleet), || {
        format_fleet_create(&fleet)
    })
}

/// The caller's own pane; a Hermes terminal tool runs in the shared gateway, so its pane
/// variables may name another session's pane.
fn caller_pane(
    backend: MuxBackend,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, CafleetError> {
    if lookup("HERMES_SESSION_ID").is_some() {
        return Err(CafleetError::App(
            "cafleet fleet create cannot identify its pane inside a Hermes session; pass --pane <pane-id>".into(),
        ));
    }
    let (variable, missing) = match backend {
        MuxBackend::Tmux => (
            "TMUX_PANE",
            "cafleet fleet create must be run inside a tmux or herdr session",
        ),
        MuxBackend::Herdr => (
            "HERDR_PANE_ID",
            "cafleet fleet create cannot identify its herdr pane because HERDR_PANE_ID is not set; pass --pane <pane-id>",
        ),
    };
    lookup(variable).ok_or_else(|| CafleetError::App(missing.into()))
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DeletionPlacement {
    member_id: i64,
    placement: Placement,
    path: PathBuf,
    run_identity: Option<serde_json::Value>,
}

pub(crate) fn deletion_placements(
    conn: &Connection,
    settings: &Settings,
    fleet_id: i64,
) -> Result<Vec<DeletionPlacement>, CafleetError> {
    let mut statement = conn.prepare(&format!(
        "SELECT m.member_id, {PLACEMENT_COLUMNS}
             FROM members m JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NOT NULL"
    ))?;
    let rows = statement.query_map([fleet_id], |row| {
        Ok((row.get::<_, i64>(0)?, Placement::from_row(row, 1)?))
    })?;
    let mut placements = Vec::new();
    for row in rows {
        let (member_id, placement) = row?;
        let path = lock_path(&settings.database_url, &placement)
            .map_err(crate::pane_lock::member_pane_input_error(member_id))?;
        let run_identity = crate::broker::runtime::member_runtime(conn, member_id)?
            .map(|runtime| crate::runtime::stable_run_identity(&runtime.run));
        placements.push(DeletionPlacement {
            member_id,
            placement,
            path,
            run_identity,
        });
    }
    placements
        .sort_by(|left, right| (&left.path, left.member_id).cmp(&(&right.path, right.member_id)));
    Ok(placements)
}

pub(crate) const PENDING_PANE_STATUS: &str = "(pending — no pane)";
pub(crate) const NO_PLACEMENT_STATUS: &str = "(no placement)";

pub(crate) trait DeletionHost {
    #[expect(
        clippy::too_many_arguments,
        reason = "deletion forwards the validated caller identity and actual retained guard"
    )]
    fn recover(
        &mut self,
        conn: &mut Connection,
        settings: &Settings,
        member_id: i64,
        expected_run: &crate::broker::runtime::RunRecord,
        expected_placement: &Placement,
        held: &PaneLock,
        deadline: Deadline,
    ) -> Result<crate::runtime::RecoveryCompletion, CafleetError> {
        crate::runtime::recover_member_under_pane_lock(
            conn,
            settings,
            member_id,
            expected_run,
            expected_placement,
            deadline.instant(),
            held,
        )
    }
    fn shutdown(
        &mut self,
        conn: &mut Connection,
        run: &crate::broker::runtime::RunRecord,
        deadline: Deadline,
    ) -> Result<crate::runtime::ShutdownOutcome, CafleetError> {
        crate::runtime::request_shutdown(conn, run, deadline.instant())
    }
    fn settle_hermes(
        &mut self,
        _conn: &Connection,
        run: &crate::broker::runtime::RunRecord,
        _pane_id: &str,
        _deadline: Deadline,
    ) -> Result<(), CafleetError> {
        Err(CafleetError::App(format!(
            "hermes settlement of run {} requires the member's multiplexer",
            run.run_id
        )))
    }
    fn close_pane(&mut self, placement: &Placement, deadline: Deadline)
    -> Result<(), CafleetError>;
    /// Restores the layout after the preceding `close_pane`.
    fn restore_layout(&mut self) -> Result<(), CafleetError>;
}

pub(crate) struct MultiplexerDeletionHost<'a>(pub(crate) &'a dyn Multiplexer);

impl DeletionHost for MultiplexerDeletionHost<'_> {
    fn settle_hermes(
        &mut self,
        conn: &Connection,
        run: &crate::broker::runtime::RunRecord,
        pane_id: &str,
        deadline: Deadline,
    ) -> Result<(), CafleetError> {
        crate::coding_agent::hermes::settle_run(conn, self.0, run, pane_id, deadline)
    }
    fn close_pane(&mut self, placement: &Placement, _: Deadline) -> Result<(), CafleetError> {
        let pane = placement
            .mux_pane_id
            .as_deref()
            .expect("deletion target has a concrete pane");
        Ok(self.0.close_pane(pane, true)?)
    }

    fn restore_layout(&mut self) -> Result<(), CafleetError> {
        Ok(self.0.restore_after_close()?)
    }
}

struct NativeFleetDeletionHost {
    /// Read before deletion removes the placement rows; its error surfaces only when a pane
    /// actually needs the multiplexer.
    director: Result<Placement, CafleetError>,
    mux: Option<Box<dyn Multiplexer>>,
}

impl NativeFleetDeletionHost {
    fn multiplexer(
        &mut self,
        deadline: Deadline,
    ) -> Result<MultiplexerDeletionHost<'_>, CafleetError> {
        if self.mux.is_none() {
            let director = self.director.as_ref().map_err(Clone::clone)?;
            self.mux = Some(crate::runtime::mux_for_placement(director, Some(deadline))?);
        }
        Ok(MultiplexerDeletionHost(
            self.mux
                .as_deref()
                .expect("the multiplexer was resolved above"),
        ))
    }
}

impl DeletionHost for NativeFleetDeletionHost {
    fn settle_hermes(
        &mut self,
        conn: &Connection,
        run: &crate::broker::runtime::RunRecord,
        pane_id: &str,
        deadline: Deadline,
    ) -> Result<(), CafleetError> {
        self.multiplexer(deadline)?
            .settle_hermes(conn, run, pane_id, deadline)
    }
    fn close_pane(
        &mut self,
        placement: &Placement,
        deadline: Deadline,
    ) -> Result<(), CafleetError> {
        self.multiplexer(deadline)?.close_pane(placement, deadline)
    }

    fn restore_layout(&mut self) -> Result<(), CafleetError> {
        let mux = self
            .mux
            .as_deref()
            .expect("layout restoration follows a pane close");
        MultiplexerDeletionHost(mux).restore_layout()
    }
}

pub(crate) fn member_deletion_matches(expected: &MemberRecord, current: &MemberRecord) -> bool {
    expected.member_id == current.member_id
        && expected.fleet_id == current.fleet_id
        && expected.kind == current.kind
        && expected.placement == current.placement
        && expected
            .runtime
            .as_ref()
            .map(|runtime| crate::runtime::stable_run_identity(&runtime.run))
            == current
                .runtime
                .as_ref()
                .map(|runtime| crate::runtime::stable_run_identity(&runtime.run))
}

fn revalidate_member(
    conn: &Connection,
    expected: &MemberRecord,
    stage: &str,
) -> Result<(), CafleetError> {
    if member_deletion_matches(expected, &active_member(conn, expected.member_id)?) {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "member placement or runtime ownership changed {stage}"
        )))
    }
}

/// `scope` revalidates the caller's wider deletion state, such as a fleet's
/// placement set, before each irreversible step up to the pane close.
pub(crate) fn end_member(
    conn: &mut Connection,
    settings: &Settings,
    snapshot: &mut MemberRecord,
    held: &PaneLock,
    host: &mut impl DeletionHost,
    deadline: &mut Deadline,
    scope: &mut dyn FnMut(&Connection) -> Result<(), CafleetError>,
) -> Result<String, CafleetError> {
    use crate::broker::runtime::CleanupState;
    let member_id = snapshot.member_id;
    let placement = snapshot
        .placement
        .clone()
        .expect("an ended member holds its concrete placement");
    let pane_id = placement
        .mux_pane_id
        .clone()
        .expect("an ended member has a concrete pane");
    let hermes_run = with_database_budget(conn, *deadline, |conn| {
        revalidate_member(conn, snapshot, "before cleanup")?;
        scope(conn)?;
        Ok(crate::broker::runtime::member_runtime(conn, member_id)?
            .map(|runtime| runtime.run)
            .filter(|run| run.backend == crate::coding_agent::CodingAgent::Hermes.name()))
    })?;
    if let Some(run) = hermes_run {
        // Settlement has its own budget; the time it takes is added to the deletion deadline.
        let started = std::time::Instant::now();
        host.settle_hermes(
            conn,
            &run,
            &pane_id,
            crate::coding_agent::hermes::settlement_deadline(),
        )?;
        *deadline = deadline.extended_by(started.elapsed());
    }
    let deadline = *deadline;
    with_database_budget(conn, deadline, |conn| {
        revalidate_member(conn, snapshot, "before cleanup")?;
        scope(conn)?;
        if let Some(runtime) = crate::broker::runtime::member_runtime(conn, member_id)?
            && !runtime.run.status.terminal()
        {
            let outcome = host.shutdown(conn, &runtime.run, deadline)?;
            if outcome.cleanup_state != CleanupState::Complete {
                return Err(CafleetError::App(format!(
                    "member {member_id} child cleanup remains unconfirmed: {}; inspect with cafleet doctor",
                    outcome.diagnostics.join("; ")
                )));
            }
        }
        if let Some(expected) = snapshot.runtime.as_ref().map(|runtime| runtime.run.clone()) {
            scope(conn)?;
            let current = crate::broker::runtime::get_run(conn, expected.run_id)?;
            if crate::runtime::stable_run_identity(&current)
                != crate::runtime::stable_run_identity(&expected)
            {
                return Err(CafleetError::App(
                    "member runtime ownership changed during cleanup".into(),
                ));
            }
            deadline.remaining()?;
            let completion = host.recover(
                conn, settings, member_id, &current, &placement, held, deadline,
            )?;
            crate::runtime::validate_recovery_completion(&expected, &completion)?;
            if !matches!(
                completion.report.outcome,
                crate::runtime::RecoveryOutcome::Recovered
            ) {
                return Err(CafleetError::App(
                    "member recovery remains incomplete; inspect with cafleet doctor".into(),
                ));
            }
            snapshot.runtime.as_mut().expect("captured runtime").run = completion.run;
        }
        revalidate_member(conn, snapshot, "before pane close")?;
        scope(conn)
    })?;
    deadline.remaining()?;
    host.close_pane(&placement, deadline).map_err(|error| {
        CafleetError::App(format!(
            "kill_pane failed for pane {pane_id}: {}. Verify with 'cafleet doctor'.",
            error.message()
        ))
    })?;
    finalize_deletion(conn, deadline, snapshot.runtime.is_some(), |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        revalidate_member(&tx, snapshot, "after pane close")?;
        let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
        assert!(
            changed,
            "the input-locked member stays active until deregistration"
        );
        tx.commit().map_err(crate::error::CafleetError::from)
    })
    .map_err(|error| {
        CafleetError::App(format!(
            "pane {pane_id} was closed, but deregister failed: {}",
            error.message()
        ))
    })?;
    if let Err(error) = host.restore_layout() {
        eprintln!(
            "Member {member_id} was deregistered; optional layout restoration failed: {}",
            error.message()
        );
    }
    Ok(format!("{pane_id} (killed)"))
}

pub(crate) fn delete_fleet(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    deadline: Deadline,
) -> Result<DeletedFleet, CafleetError> {
    delete_fleet_with_host(
        conn,
        settings,
        fleet_id,
        deadline,
        &mut NativeFleetDeletionHost {
            director: broker::director_placement(conn, fleet_id),
            mux: None,
        },
    )
}

pub(crate) fn delete_fleet_with_host(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    mut deadline: Deadline,
    host: &mut impl DeletionHost,
) -> Result<DeletedFleet, CafleetError> {
    with_database_budget(conn, deadline, |conn| {
        crate::broker::require_fleet(conn, fleet_id)?;
        Ok(())
    })?;
    let changed_error = || {
        CafleetError::App(format!(
            "fleet {fleet_id} placement or runtime ownership changed during cleanup; fleet was not deleted."
        ))
    };
    let mut locked = lock_deletion(
        conn,
        settings,
        deadline,
        |conn| {
            let snapshot = deletion_placements(conn, settings, fleet_id)?;
            let targets = snapshot
                .iter()
                .map(|target| (target.member_id, target.placement.clone()))
                .collect();
            Ok((snapshot, targets))
        },
        |conn, snapshot, _guards| {
            if deletion_placements(conn, settings, fleet_id)? != *snapshot {
                return Ok(None);
            }
            let mut members = Vec::new();
            for target in snapshot.iter() {
                let member = active_member(conn, target.member_id)?;
                if member.kind != crate::broker::MemberKind::Director {
                    members.push(member);
                }
            }
            Ok(Some(members))
        },
        |member| {
            CafleetError::App(format!(
                "fleet {fleet_id} pane input remained busy for 5 seconds at member {member}; fleet was not deleted."
            ))
        },
    )?;
    let mut remaining = locked.snapshot.clone();
    let mut members = std::mem::take(&mut locked.value);
    let mut ended = Vec::new();
    for member in &mut members {
        let member_id = member.member_id;
        let placement = member
            .placement
            .clone()
            .expect("a locked deletion target has a concrete placement");
        let held = locked.pane_lock(&settings.database_url, &placement)?;
        let mut scope_changed = false;
        let mut scope = |conn: &Connection| {
            if same_placements(&deletion_placements(conn, settings, fleet_id)?, &remaining) {
                Ok(())
            } else {
                scope_changed = true;
                Err(changed_error())
            }
        };
        match end_member(
            conn,
            settings,
            member,
            held,
            host,
            &mut deadline,
            &mut scope,
        ) {
            Ok(pane_status) => {
                ended.push(DeletedMember {
                    member_id,
                    pane_status,
                });
                remaining.retain(|target| target.member_id != member_id);
            }
            Err(error) if scope_changed => return Err(error),
            Err(error) => return Err(member_not_ended(fleet_id, member_id, &error, &ended)),
        }
    }
    let (deregistered_count, unplaced) = with_database_budget(conn, deadline, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !same_placements(&deletion_placements(&tx, settings, fleet_id)?, &remaining) {
            return Err(changed_error());
        }
        let unplaced = unplaced_member_outcomes(&tx, fleet_id)?;
        let deregistered_count = crate::broker::fleets::delete_fleet_rows(&tx, fleet_id)?;
        deadline.remaining()?;
        tx.commit()?;
        Ok((deregistered_count, unplaced))
    })?;
    drop(locked);
    let deregistered_count = deregistered_count + ended.len();
    ended.extend(unplaced);
    ended.sort_by_key(|member| member.member_id);
    Ok(DeletedFleet {
        fleet_id,
        deregistered_count,
        members: ended,
    })
}

fn same_placements(current: &[DeletionPlacement], expected: &[DeletionPlacement]) -> bool {
    current.len() == expected.len()
        && current.iter().zip(expected).all(|(current, expected)| {
            current.member_id == expected.member_id && current.placement == expected.placement
        })
}

fn member_not_ended(
    fleet_id: i64,
    member_id: i64,
    cause: &CafleetError,
    ended: &[DeletedMember],
) -> CafleetError {
    let ended = if ended.is_empty() {
        "none".to_owned()
    } else {
        ended
            .iter()
            .map(|member| member.member_id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    CafleetError::App(format!(
        "fleet {fleet_id} was not deleted: member {member_id}: {}. Members already deleted: {ended}.",
        cause.message().trim_end_matches('.')
    ))
}

fn unplaced_member_outcomes(
    conn: &Connection,
    fleet_id: i64,
) -> Result<Vec<DeletedMember>, CafleetError> {
    let mut statement = conn.prepare(
        "SELECT m.member_id, p.member_id IS NOT NULL
             FROM members m LEFT JOIN member_placements p ON p.member_id=m.member_id
             WHERE m.fleet_id=?1 AND m.status='active' AND p.mux_pane_id IS NULL
               AND m.member_id IS NOT (SELECT director_member_id FROM fleets WHERE fleet_id=?1)",
    )?;
    let rows = statement.query_map([fleet_id], |row| {
        Ok(DeletedMember {
            member_id: row.get(0)?,
            pane_status: if row.get(1)? {
                PENDING_PANE_STATUS
            } else {
                NO_PLACEMENT_STATUS
            }
            .to_owned(),
        })
    })?;
    rows.collect::<Result<_, _>>()
        .map_err(crate::error::CafleetError::from)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::time::Duration;
    use std::time::Instant;

    use rusqlite::Connection;

    use crate::broker::DeletedFleet;
    use crate::broker::Placement;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::RunRecord;
    use crate::config::Settings;
    use crate::error::CafleetError;
    use crate::pane_lock::PaneLock;
    use crate::runtime::ChildExit;
    use crate::runtime::RecoveryCompletion;
    use crate::runtime::ShutdownOutcome;
    use crate::runtime::recover_member_with_held_pane_lock;
    use crate::runtime::recovery::test_support::Case;
    use crate::runtime::recovery::test_support::Host as RecoveryHost;
    use crate::runtime::test_support::Fixture;

    #[derive(Clone, Copy)]
    pub(crate) enum Mutation {
        None,
        AfterShutdown,
        BeforeRecovery,
        AfterRecovery,
        AfterClose,
        OtherPlacement(i64),
    }

    /// `Fleet` ends each run itself and keeps production recovery; `Recovery`
    /// leaves the run for the recovery coordinator and replays it through
    /// `recovery` on the deleting connection.
    enum Mode {
        Fleet,
        Recovery {
            connection: usize,
            host: Box<RecoveryHost>,
        },
    }

    pub(crate) struct Host {
        mode: Mode,
        pub(crate) settings: Settings,
        pub(crate) deadline: Instant,
        pub(crate) run_id: i64,
        pub(crate) mutation: Mutation,
        pub(crate) calls: Vec<&'static str>,
        pub(crate) owners: Vec<(i64, i64)>,
        pub(crate) panes: Vec<Placement>,
        pub(crate) failing_pane: Option<String>,
        pub(crate) restored: usize,
        pub(crate) recovered: Option<RunRecord>,
        pub(crate) runner_gone: bool,
        pub(crate) hermes_failure: Option<String>,
        settled: Option<Duration>,
    }

    const SETTLEMENT_TIME: Duration = Duration::from_millis(30);

    impl Host {
        pub(crate) fn fleet(
            settings: Settings,
            run_id: i64,
            deadline: crate::cli::fleet::Deadline,
        ) -> Self {
            Self::with_mode(Mode::Fleet, settings, run_id, deadline)
        }

        pub(crate) fn recovering(case: &Case, deadline: crate::cli::fleet::Deadline) -> Self {
            let mode = Mode::Recovery {
                connection: std::ptr::from_ref(&case.fixture.conn).addr(),
                host: Box::new(RecoveryHost::new(deadline.instant())),
            };
            Self::with_mode(
                mode,
                case.settings.clone(),
                case.fixture.run.run_id,
                deadline,
            )
        }

        pub(crate) fn recovery(&mut self) -> &mut RecoveryHost {
            match &mut self.mode {
                Mode::Recovery { host, .. } => host,
                Mode::Fleet => panic!("fleet deletion hosts use production recovery"),
            }
        }

        fn with_mode(
            mode: Mode,
            settings: Settings,
            run_id: i64,
            deadline: crate::cli::fleet::Deadline,
        ) -> Self {
            Self {
                mode,
                settings,
                deadline: deadline.instant(),
                run_id,
                mutation: Mutation::None,
                calls: Vec::new(),
                owners: Vec::new(),
                panes: Vec::new(),
                failing_pane: None,
                restored: 0,
                recovered: None,
                runner_gone: false,
                hermes_failure: None,
                settled: None,
            }
        }

        /// Every cleanup step shares the deletion deadline, which a settlement extends by the
        /// time it took.
        fn assert_deadline(&mut self, deadline: Instant) {
            if let Some(settlement) = self.settled.take() {
                let extension = deadline.saturating_duration_since(self.deadline);
                assert!(
                    extension >= settlement && extension < settlement + Duration::from_secs(1),
                    "{extension:?}"
                );
                self.deadline = deadline;
            }
            assert_eq!(deadline, self.deadline);
        }

        fn assert_pane_owned(&self, placement: &Placement) {
            assert!(
                PaneLock::try_acquire(&self.settings.database_url, placement)
                    .unwrap()
                    .is_none()
            );
        }

        fn assert_connection(&self, conn: &Connection) {
            if let Mode::Recovery { connection, .. } = self.mode {
                assert_eq!(std::ptr::from_ref(conn).addr(), connection);
            }
        }

        fn bump_epoch(conn: &Connection, run_id: i64) {
            conn.execute(
                "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                [run_id],
            )
            .unwrap();
        }
    }

    impl crate::cli::fleet::DeletionHost for Host {
        fn settle_hermes(
            &mut self,
            conn: &Connection,
            run: &RunRecord,
            pane_id: &str,
            deadline: crate::cli::fleet::Deadline,
        ) -> Result<(), CafleetError> {
            self.assert_connection(conn);
            let budget = deadline.instant().saturating_duration_since(Instant::now());
            assert!(
                budget > Duration::from_secs(19) && budget <= Duration::from_secs(20),
                "each Hermes member gets its own settlement budget: {budget:?}"
            );
            assert!(deadline.instant() > self.deadline);
            let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
            assert_eq!(placement.mux_pane_id.as_deref(), Some(pane_id));
            self.assert_pane_owned(&placement);
            self.calls.push("settle");
            std::thread::sleep(SETTLEMENT_TIME);
            self.settled = Some(SETTLEMENT_TIME);
            match &self.hermes_failure {
                Some(cause) => Err(CafleetError::App(cause.clone())),
                None => Ok(()),
            }
        }

        fn shutdown(
            &mut self,
            conn: &mut Connection,
            run: &RunRecord,
            deadline: crate::cli::fleet::Deadline,
        ) -> Result<ShutdownOutcome, CafleetError> {
            self.assert_connection(conn);
            self.assert_deadline(deadline.instant());
            let placement = crate::broker::get_member(conn, run.member_id, run.fleet_id)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
            self.assert_pane_owned(&placement);
            self.calls.push("shutdown");
            if self.runner_gone {
                return Err(CafleetError::App(
                    "member runtime: ECONNREFUSED: Connection refused".into(),
                ));
            }
            let native_exit = match self.mode {
                Mode::Fleet => {
                    self.owners.push((run.run_id, run.owner_epoch));
                    self.run_id = run.run_id;
                    conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T13:00:00Z',reachability='reachable',last_health_at='2026-10-26T13:00:00Z' WHERE run_id=?1", [run.run_id]).unwrap();
                    Some(ChildExit::Code(0))
                }
                Mode::Recovery { .. } => {
                    conn.execute(
                        "UPDATE member_runs SET cleanup_state='complete' WHERE run_id=?1",
                        [run.run_id],
                    )
                    .unwrap();
                    None
                }
            };
            if matches!(self.mutation, Mutation::AfterShutdown) {
                Self::bump_epoch(conn, run.run_id);
            }
            Ok(ShutdownOutcome::complete(native_exit))
        }

        fn recover(
            &mut self,
            conn: &mut Connection,
            settings: &Settings,
            member_id: i64,
            expected_run: &RunRecord,
            expected_placement: &Placement,
            held: &PaneLock,
            deadline: crate::cli::fleet::Deadline,
        ) -> Result<RecoveryCompletion, CafleetError> {
            if matches!(self.mode, Mode::Fleet) {
                return crate::runtime::recover_member_under_pane_lock(
                    conn,
                    settings,
                    member_id,
                    expected_run,
                    expected_placement,
                    deadline.instant(),
                    held,
                );
            }
            self.assert_connection(conn);
            self.assert_deadline(deadline.instant());
            assert_eq!(expected_run.member_id, member_id);
            self.assert_pane_owned(expected_placement);
            held.validate_binding(&settings.database_url, expected_placement)
                .unwrap();
            self.calls.push("recover");
            if matches!(self.mutation, Mutation::BeforeRecovery) {
                Self::bump_epoch(conn, expected_run.run_id);
            }
            let completion = recover_member_with_held_pane_lock(
                conn,
                settings,
                member_id,
                expected_run,
                expected_placement,
                None,
                deadline.instant(),
                held,
                self.recovery(),
            )?;
            assert_eq!(
                completion.run,
                storage::get_run(conn, expected_run.run_id).unwrap()
            );
            self.recovered = Some(completion.run.clone());
            match self.mutation {
                Mutation::AfterRecovery => Self::bump_epoch(conn, expected_run.run_id),
                Mutation::OtherPlacement(member) => {
                    conn.execute(
                        "UPDATE member_placements SET mux_pane_id='%foreign' WHERE member_id=?1",
                        [member],
                    )
                    .unwrap();
                }
                _ => {}
            }
            Ok(completion)
        }

        fn close_pane(
            &mut self,
            placement: &Placement,
            deadline: crate::cli::fleet::Deadline,
        ) -> Result<(), CafleetError> {
            self.assert_deadline(deadline.instant());
            self.assert_pane_owned(placement);
            self.calls.push("close");
            self.panes.push(placement.clone());
            if self.failing_pane.is_some() && self.failing_pane == placement.mux_pane_id {
                return Err(CafleetError::App("pane vanished".into()));
            }
            if matches!(self.mutation, Mutation::AfterClose) {
                let run_id = match self.mode {
                    Mode::Fleet => self.run_id,
                    Mode::Recovery { .. } => {
                        self.recovered
                            .as_ref()
                            .expect("close follows proven recovery")
                            .run_id
                    }
                };
                Self::bump_epoch(
                    &crate::db::connect(&self.settings.database_url).unwrap(),
                    run_id,
                );
            }
            Ok(())
        }

        fn restore_layout(&mut self) -> Result<(), CafleetError> {
            self.restored += 1;
            Ok(())
        }
    }

    pub(crate) fn fleet_deleted(fixture: &Fixture) -> bool {
        crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
            .unwrap()
            .unwrap()
            .deleted_at
            .is_some()
    }

    pub(crate) fn assert_registry_active(fixture: &Fixture) {
        assert!(!fleet_deleted(fixture));
        assert!(
            crate::broker::get_member(&fixture.conn, fixture.run.member_id, fixture.run.fleet_id)
                .unwrap()
                .is_some()
        );
    }

    pub(crate) struct FixtureDeletionHost;

    impl crate::cli::fleet::DeletionHost for FixtureDeletionHost {
        fn shutdown(
            &mut self,
            _: &mut Connection,
            _: &RunRecord,
            _: crate::cli::fleet::Deadline,
        ) -> Result<ShutdownOutcome, CafleetError> {
            unreachable!("broker fixtures register no member runs")
        }

        fn close_pane(
            &mut self,
            _: &Placement,
            _: crate::cli::fleet::Deadline,
        ) -> Result<(), CafleetError> {
            Ok(())
        }

        fn restore_layout(&mut self) -> Result<(), CafleetError> {
            Ok(())
        }
    }

    pub(crate) fn guarded_delete_fleet(
        conn: &mut Connection,
        fleet: i64,
    ) -> Result<DeletedFleet, CafleetError> {
        let path = conn
            .path()
            .expect("fleet deletion has a file-backed fixture");
        assert!(
            !path.is_empty(),
            "fleet deletion has a concrete database path"
        );
        let settings = crate::config::Settings::for_database(std::path::Path::new(path));
        crate::cli::fleet::delete_fleet_with_host(
            conn,
            &settings,
            fleet,
            crate::cli::fleet::Deadline::new(),
            &mut FixtureDeletionHost,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::result::Result;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use rusqlite::Connection;

    use crate::broker::members::test_support::register;
    use crate::broker::runtime as storage;
    use crate::broker::runtime::*;
    use crate::pane_lock::PaneLock;
    use crate::runtime::recovery::test_support::*;
    use crate::runtime::test_support::*;
    use crate::runtime::*;
    use crate::time::Deadline;

    mod deletion_rows {
        use crate::broker;
        use crate::broker::fleets::get_fleet;
        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::members::test_support::register;
        use crate::cli::fleet::test_support::guarded_delete_fleet;
        use crate::db::test_support::migrated_conn;
        use rusqlite::Connection;

        fn row_count(conn: &Connection, table: &str) -> i64 {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        }

        #[test]
        fn fleet_deletion_is_atomic_soft_idempotent_and_preserves_messages() {
            let trigger = "BEFORE DELETE ON member_placements";
            let dir = tempfile::tempdir().unwrap();
            let mut conn = migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "alpha");
            let worker = register(&mut conn, fleet, "worker", None);
            let message =
                broker::send_message(&mut conn, director, worker, "retained history").unwrap();
            let before = get_fleet(&conn, fleet).unwrap();
            let members =
                [director, worker].map(|id| broker::get_member(&conn, id, fleet).unwrap());
            conn.execute_batch(&format!("CREATE TRIGGER fail_deletion {trigger} BEGIN SELECT RAISE(ABORT, 'deletion failure'); END;")).unwrap();
            assert!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap_err()
                    .message()
                    .contains("deletion failure")
            );
            assert_eq!(get_fleet(&conn, fleet).unwrap(), before);
            for (id, member) in [director, worker].into_iter().zip(members) {
                assert_eq!(broker::get_member(&conn, id, fleet).unwrap(), member);
                assert_eq!(
                    conn.query_row(
                        "SELECT deregistered_at FROM members WHERE member_id=?1",
                        [id],
                        |row| row.get::<_, Option<String>>(0)
                    )
                    .unwrap(),
                    None
                );
            }
            conn.execute_batch("DROP TRIGGER fail_deletion").unwrap();
            assert_eq!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap()
                    .deregistered_count,
                2
            );
            assert_eq!(row_count(&conn, "member_placements"), 0);
            let deleted = get_fleet(&conn, fleet).unwrap();
            assert_eq!(
                guarded_delete_fleet(&mut conn, fleet)
                    .unwrap()
                    .deregistered_count,
                0
            );
            assert_eq!(get_fleet(&conn, fleet).unwrap(), deleted);
            assert_eq!(
                broker::get_message(&conn, message.message_id).unwrap(),
                message
            );
            assert!(guarded_delete_fleet(&mut conn, 999).is_err());
        }
    }

    #[test]
    fn fleet_create_accepts_every_registered_backend_including_hermes() {
        use clap::Parser;
        for &agent in crate::coding_agent::CodingAgent::all() {
            let parsed = crate::cli::CliArgs::try_parse_from([
                "cafleet",
                "fleet",
                "create",
                "--name",
                "fixture",
                "--coding-agent",
                agent.name(),
            ]);
            assert!(parsed.is_ok(), "{agent:?}");
        }
    }

    mod fleet {
        use crate::broker::Placement;
        use crate::cli::fleet::delete_fleet_with_host;
        use crate::cli::fleet::test_support::Host;
        use crate::cli::fleet::test_support::Mutation;
        use crate::cli::fleet::test_support::assert_registry_active;
        use crate::cli::fleet::test_support::fleet_deleted;
        use crate::cli::fleet::tests::*;
        use crate::config::Settings;
        use crate::error::CafleetError;
        use std::time::Duration;
        use std::time::Instant;

        fn settings(fixture: &Fixture) -> Settings {
            Settings::for_database(std::path::Path::new(fixture.conn.path().unwrap()))
        }

        fn while_locks_are_acquired(
            fixture: &mut Fixture,
            change: impl FnOnce(&mut Fixture, &Settings) -> Option<PaneLock>,
        ) -> (Result<crate::broker::DeletedFleet, CafleetError>, Host) {
            let configuration = settings(fixture);
            let mut placements = crate::broker::list_members(&fixture.conn, fixture.run.fleet_id)
                .unwrap()
                .into_iter()
                .filter_map(|activity| activity.member.placement)
                .filter(|placement| placement.mux_pane_id.is_some())
                .collect::<Vec<_>>();
            placements.sort_by_key(|placement| {
                crate::pane_lock::lock_path(&configuration.database_url, placement).unwrap()
            });
            assert!(placements.len() >= 2);
            let first = placements.first().unwrap().clone();
            let last =
                PaneLock::try_acquire(&configuration.database_url, placements.last().unwrap())
                    .unwrap()
                    .unwrap();
            let path = fixture.conn.path().unwrap().to_owned();
            let fleet_id = fixture.run.fleet_id;
            let run_id = fixture.run.run_id;
            let worker_settings = configuration.clone();
            let started = Instant::now();
            let deadline = Deadline::until(started + Duration::from_secs(1));
            std::thread::scope(|scope| {
                let deletion = scope.spawn(move || {
                    let mut conn = Connection::open(path).unwrap();
                    let mut host = Host::fleet(worker_settings.clone(), run_id, deadline);
                    let result = delete_fleet_with_host(
                        &mut conn,
                        &worker_settings,
                        fleet_id,
                        deadline,
                        &mut host,
                    );
                    (result, host)
                });
                while PaneLock::try_acquire(&configuration.database_url, &first)
                    .unwrap()
                    .is_some()
                {
                    assert!(
                        started.elapsed() < Duration::from_millis(700),
                        "deletion did not acquire its first physical lock"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                let changed_lock = change(fixture, &configuration);
                drop(last);
                let result = deletion.join().unwrap();
                drop(changed_lock);
                assert!(started.elapsed() < Duration::from_millis(1750));
                result
            })
        }

        #[test]
        fn runner_review_fleet_changed_placement_is_relocked_before_any_destructive_effect() {
            let mut fixture = Fixture::new();
            let (result, host) = while_locks_are_acquired(&mut fixture, |fixture, settings| {
                fixture
                    .conn
                    .execute(
                        "UPDATE member_placements SET mux_pane_id='%changed' WHERE member_id=?1",
                        [fixture.run.member_id],
                    )
                    .unwrap();
                let placement = crate::broker::get_member(
                    &fixture.conn,
                    fixture.run.member_id,
                    fixture.run.fleet_id,
                )
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
                Some(
                    PaneLock::try_acquire(&settings.database_url, &placement)
                        .unwrap()
                        .unwrap(),
                )
            });
            assert!(result.is_err());
            assert!(
                host.calls.is_empty(),
                "stale deletion effects: {:?}",
                host.calls
            );
            assert_registry_active(&fixture);
        }

        #[test]
        fn runner_review_fleet_changed_run_on_the_same_pane_preserves_the_new_run_identity() {
            let mut fixture = Fixture::new();
            let old_run = fixture.run.clone();
            let mut new_run_id = None;
            let (result, host) = while_locks_are_acquired(&mut fixture, |fixture, _| {
                fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", [old_run.run_id]).unwrap();
                let spec = NewRun {
                    launch_nonce: "b".repeat(64),
                    runtime_path: fixture
                        ._dir
                        .path()
                        .join("replacement-run")
                        .to_str()
                        .unwrap()
                        .into(),
                    ..crate::broker::runtime::test_support::Fixture::spec(
                        &fixture.conn,
                        old_run.fleet_id,
                        old_run.director_member_id,
                        old_run.member_id,
                    )
                };
                let new =
                    crate::broker::runtime::test_support::create_run(&mut fixture.conn, &spec)
                        .unwrap();
                new_run_id = Some(new.run_id);
                None
            });
            result.unwrap();
            assert_eq!(host.owners, [(new_run_id.unwrap(), 1)]);
            assert_ne!(host.owners[0].0, old_run.run_id);
            assert_eq!(host.calls, ["shutdown", "close"]);
        }

        #[test]
        fn runner_review_fleet_owner_change_during_shutdown_stops_before_pane_close() {
            let mut fixture = Fixture::new();
            let settings = settings(&fixture);
            let deadline = Deadline::new();
            let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            host.mutation = Mutation::AfterShutdown;
            assert!(
                delete_fleet_with_host(
                    &mut fixture.conn,
                    &settings,
                    fixture.run.fleet_id,
                    deadline,
                    &mut host
                )
                .is_err()
            );
            assert_eq!(host.calls, ["shutdown"]);
            assert_registry_active(&fixture);
        }

        #[test]
        fn hermes_member_deletion_settles_before_shutdown_and_an_unverified_stop_keeps_the_member()
        {
            for failure in [
                None,
                Some(
                    "hermes work for member 2 could not be verified as stopped: Hermes did not report /stop; member was not deleted.",
                ),
            ] {
                let mut fixture = Fixture::new();
                fixture
                    .conn
                    .execute(
                        "UPDATE member_runs SET backend='hermes' WHERE run_id=?1",
                        [fixture.run.run_id],
                    )
                    .unwrap();
                let settings = settings(&fixture);
                let deadline = Deadline::new();
                let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
                host.hermes_failure = failure.map(str::to_owned);
                let result = delete_fleet_with_host(
                    &mut fixture.conn,
                    &settings,
                    fixture.run.fleet_id,
                    deadline,
                    &mut host,
                );
                match failure {
                    None => {
                        result.unwrap();
                        assert_eq!(host.calls, ["settle", "shutdown", "close"]);
                    }
                    Some(cause) => {
                        let error = result.expect_err(cause);
                        assert_eq!(error.exit_code(), 1);
                        assert!(error.message().contains(cause), "{}", error.message());
                        assert_eq!(
                            host.calls,
                            ["settle"],
                            "no shutdown after an unverified stop"
                        );
                        assert_registry_active(&fixture);
                        assert!(
                            !storage::get_run(&fixture.conn, fixture.run.run_id)
                                .unwrap()
                                .status
                                .terminal(),
                            "the runner keeps running for a retry"
                        );
                    }
                }
            }
        }

        #[test]
        fn runner_review_fleet_final_identity_change_preserves_registry_without_repeating_effects()
        {
            let mut fixture = Fixture::new();
            let settings = settings(&fixture);
            let deadline = Deadline::new();
            let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            host.mutation = Mutation::AfterClose;
            assert!(
                delete_fleet_with_host(
                    &mut fixture.conn,
                    &settings,
                    fixture.run.fleet_id,
                    deadline,
                    &mut host
                )
                .is_err()
            );
            assert_eq!(host.calls, ["shutdown", "close"]);
            assert_registry_active(&fixture);
        }

        #[test]
        fn runner_review_fleet_mutable_shutdown_evidence_keeps_one_deadline_and_run_less_teardown()
        {
            let mut fixture = Fixture::new();
            fixture.conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0 WHERE run_id=?1", [fixture.run.run_id]).unwrap();
            let settings = settings(&fixture);
            let deadline = Deadline::new();
            let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            let deleted = delete_fleet_with_host(
                &mut fixture.conn,
                &settings,
                fixture.run.fleet_id,
                deadline,
                &mut host,
            )
            .unwrap();
            assert_eq!(deleted.deregistered_count, 2);
            assert_eq!(host.calls, ["close"]);
            assert_eq!(host.panes.len(), 1);
            assert_eq!(host.panes[0].mux_pane_id.as_deref(), Some("%2"));
            assert!(fleet_deleted(&fixture));
        }

        fn outcomes(deleted: &crate::broker::DeletedFleet) -> Vec<(i64, String)> {
            deleted
                .members
                .iter()
                .map(|member| (member.member_id, member.pane_status.clone()))
                .collect()
        }

        fn ordinary_placements_in_lock_order(
            fixture: &Fixture,
            settings: &Settings,
        ) -> Vec<(i64, Placement)> {
            let director = crate::broker::get_fleet(&fixture.conn, fixture.run.fleet_id)
                .unwrap()
                .unwrap()
                .director_member_id
                .unwrap();
            let mut placements = crate::broker::list_members(&fixture.conn, fixture.run.fleet_id)
                .unwrap()
                .into_iter()
                .filter(|activity| activity.member.member_id != director)
                .filter_map(|activity| {
                    Some((activity.member.member_id, activity.member.placement?))
                })
                .filter(|(_, placement)| placement.mux_pane_id.is_some())
                .collect::<Vec<_>>();
            placements.sort_by_key(|(_, placement)| {
                crate::pane_lock::lock_path(&settings.database_url, placement).unwrap()
            });
            placements
        }

        #[test]
        fn fleet_deletion_ends_every_ordinary_member_and_keeps_the_director_pane() {
            let mut fixture = Fixture::new();
            let fleet = fixture.run.fleet_id;
            let run_less = register(&mut fixture.conn, fleet, "run-less", Some("%5"));
            let pending = register(&mut fixture.conn, fleet, "pending", None);
            let placementless = crate::broker::members::test_support::register_member(
                &mut fixture.conn,
                fleet,
                "bare",
                "assignment",
                None,
            )
            .unwrap()
            .member_id;
            let settings = settings(&fixture);
            let deadline = Deadline::new();
            let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            let deleted =
                delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut host)
                    .unwrap();
            assert_eq!(deleted.fleet_id, fleet);
            assert_eq!(deleted.deregistered_count, 5);
            let mut expected = vec![
                (fixture.run.member_id, "%2 (killed)".to_owned()),
                (run_less, "%5 (killed)".to_owned()),
                (pending, "(pending — no pane)".to_owned()),
                (placementless, "(no placement)".to_owned()),
            ];
            expected.sort();
            assert_eq!(outcomes(&deleted), expected);
            let mut closed = host
                .panes
                .iter()
                .map(|placement| placement.mux_pane_id.clone().unwrap())
                .collect::<Vec<_>>();
            closed.sort();
            assert_eq!(closed, ["%2", "%5"]);
            assert_eq!(host.restored, 2);
            assert!(fleet_deleted(&fixture));
            assert_eq!(
                fixture
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM members WHERE fleet_id=?1 AND status='active'",
                        [fleet],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }

        #[test]
        fn fleet_deletion_stops_at_the_first_failed_member_and_a_rerun_resumes() {
            let mut fixture = Fixture::new();
            let fleet = fixture.run.fleet_id;
            for (name, pane) in [("first", "%5"), ("second", "%6"), ("third", "%7")] {
                register(&mut fixture.conn, fleet, name, Some(pane));
            }
            let settings = settings(&fixture);
            let order = ordinary_placements_in_lock_order(&fixture, &settings);
            assert_eq!(order.len(), 4);
            let failing = if order[1].0 == fixture.run.member_id {
                2
            } else {
                1
            };
            let (failed, failed_placement) = order[failing].clone();
            let failed_pane = failed_placement.mux_pane_id.clone().unwrap();
            let deadline = Deadline::new();
            let mut host = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            host.failing_pane = Some(failed_pane.clone());
            let error =
                delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut host)
                    .unwrap_err();
            let ended = order[..failing]
                .iter()
                .map(|(member, _)| member.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            assert_eq!(
                error.message(),
                format!(
                    "fleet {fleet} was not deleted: member {failed}: kill_pane failed for pane {failed_pane}: pane vanished. Verify with 'cafleet doctor'. Members already deleted: {ended}."
                )
            );
            assert_eq!(host.panes.len(), failing + 1);
            assert_eq!(host.restored, failing);
            assert!(!fleet_deleted(&fixture));
            for (member, _) in &order[..failing] {
                assert!(
                    crate::broker::get_member(&fixture.conn, *member, fleet)
                        .unwrap()
                        .is_none()
                );
            }
            for (member, placement) in &order[failing..] {
                let current = crate::broker::get_member(&fixture.conn, *member, fleet)
                    .unwrap()
                    .unwrap();
                assert_eq!(current.placement.as_ref(), Some(placement));
                assert!(
                    PaneLock::try_acquire(&settings.database_url, placement)
                        .unwrap()
                        .is_some()
                );
            }

            let deadline = Deadline::new();
            let mut rerun = Host::fleet(settings.clone(), fixture.run.run_id, deadline);
            let deleted =
                delete_fleet_with_host(&mut fixture.conn, &settings, fleet, deadline, &mut rerun)
                    .unwrap();
            let mut expected = order[failing..]
                .iter()
                .map(|(member, placement)| {
                    (
                        *member,
                        format!("{} (killed)", placement.mux_pane_id.as_deref().unwrap()),
                    )
                })
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(outcomes(&deleted), expected);
            assert_eq!(deleted.deregistered_count, order.len() - failing + 1);
            assert_eq!(rerun.panes.len(), order.len() - failing);
            assert!(fleet_deleted(&fixture));
        }
    }

    mod recovery {
        use crate::cli::fleet::delete_fleet_with_host;
        use crate::cli::fleet::test_support::Host as DeletionTestHost;
        use crate::cli::fleet::test_support::Mutation;
        use crate::cli::fleet::test_support::assert_registry_active;
        use crate::cli::fleet::tests::*;
        use crate::runtime::test_support::SLOW_SUM;
        use std::sync::Mutex;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;
        use std::sync::mpsc;

        #[test]
        fn runner_recovery_deletion_uses_real_shared_outbox_open_scan_append_and_compact_under_outer_budget()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let original = case.fixture.run.clone();
            case.release();
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
            let mut host = DeletionTestHost::recovering(&case, deadline);
            case.fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            delete_fleet_with_host(
                &mut case.fixture.conn,
                &case.settings,
                original.fleet_id,
                deadline,
                &mut host,
            )
            .unwrap();
            assert_eq!(host.calls, vec!["shutdown", "recover", "close"]);
            let recovered = host.recovered.as_ref().unwrap();
            assert_eq!(recovered.owner_epoch, original.owner_epoch + 1);
            assert_eq!(recovered.status, RunStatus::Lost);
            let events = events(&case);
            assert_eq!(
                events.iter().map(|row| row.event.kind).collect::<Vec<_>>(),
                vec![
                    RuntimeEventKind::InteractionOpened,
                    RuntimeEventKind::InteractionClosed,
                    RuntimeEventKind::RunnerLost
                ]
            );
            assert!(events.iter().all(|row| row.event.notification_state
                == NotificationState::Inbox
                && row.event.message_id.is_some()));
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
            assert!(
                crate::broker::get_fleet(&case.fixture.conn, original.fleet_id)
                    .unwrap()
                    .unwrap()
                    .deleted_at
                    .is_some()
            );
            let (status, deregistered_at): (String, Option<String>) = case
                .fixture
                .conn
                .query_row(
                    "SELECT status,deregistered_at FROM members WHERE member_id=?1 AND fleet_id=?2",
                    rusqlite::params![original.member_id, original.fleet_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "deregistered");
            assert!(deregistered_at.is_some());
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                137
            );
            for boundary in ["before_replay", "after_commit", "before_compact"] {
                assert!(host.recovery().calls.iter().any(|name| name == boundary));
            }
        }

        #[test]
        fn runner_recovery_deletion_of_an_exited_runner_with_incomplete_cleanup_recovers_without_contacting_it()
         {
            let mut case = Case::new();
            case.fixture
        .conn
        .execute(
            "UPDATE member_runs SET status='exited',cleanup_state='incomplete',exit_code=143,ended_at=?1 WHERE run_id=?2",
            rusqlite::params![NOW, case.fixture.run.run_id],
        )
        .unwrap();
            case.refresh();
            let original = case.fixture.run.clone();
            case.release();
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
            let mut host = DeletionTestHost::recovering(&case, deadline);
            host.runner_gone = true;
            delete_fleet_with_host(
                &mut case.fixture.conn,
                &case.settings,
                original.fleet_id,
                deadline,
                &mut host,
            )
            .unwrap();
            assert_eq!(
                host.calls,
                vec!["recover", "close"],
                "an exited runner is recovered, never asked to shut down"
            );
            let recovered = host.recovered.as_ref().unwrap();
            assert_eq!(recovered.status, RunStatus::Exited);
            assert_eq!(recovered.exit_code, Some(143));
            assert_eq!(recovered.cleanup_state, CleanupState::Complete);
            assert!(events(&case).is_empty());
            assert!(case.spool_bytes().is_none_or(|bytes| bytes.is_empty()));
            let status: String = case
                .fixture
                .conn
                .query_row(
                    "SELECT status FROM members WHERE member_id=?1 AND fleet_id=?2",
                    rusqlite::params![original.member_id, original.fleet_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, "deregistered");
        }

        #[test]
        fn runner_recovery_deletion_rejects_foreign_pre_entry_post_recovery_and_post_close_owner_changes()
         {
            for mutation in [
                Mutation::BeforeRecovery,
                Mutation::AfterRecovery,
                Mutation::AfterClose,
            ] {
                let mut case = Case::new();
                case.seed(&[opening(&case.fixture.run, 1, 1)]);
                let epoch = case.fixture.run.owner_epoch;
                let original_bytes = case.spool_bytes().unwrap();
                case.release();
                let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
                let mut host = DeletionTestHost::recovering(&case, deadline);
                host.mutation = mutation;
                assert!(
                    delete_fleet_with_host(
                        &mut case.fixture.conn,
                        &case.settings,
                        case.fixture.run.fleet_id,
                        deadline,
                        &mut host
                    )
                    .is_err()
                );
                assert_registry_active(&case.fixture);
                let current =
                    storage::get_run(&case.fixture.conn, case.fixture.run.run_id).unwrap();
                match mutation {
                    Mutation::BeforeRecovery => {
                        assert_eq!(host.calls, vec!["shutdown", "recover"]);
                        assert!(events(&case).is_empty());
                        assert_eq!(current.owner_epoch, epoch + 1);
                        assert_eq!(case.spool_bytes().unwrap(), original_bytes);
                    }
                    Mutation::AfterRecovery => {
                        assert_eq!(host.calls, vec!["shutdown", "recover"]);
                        assert_eq!(events(&case).len(), 3);
                        assert_eq!(current.owner_epoch, epoch + 2);
                    }
                    Mutation::AfterClose => {
                        assert_eq!(host.calls, vec!["shutdown", "recover", "close"]);
                        assert_eq!(events(&case).len(), 3);
                        assert_eq!(current.owner_epoch, epoch + 2);
                    }
                    _ => unreachable!(),
                }
            }
        }

        #[test]
        fn runner_recovery_deletion_updates_only_its_proven_target_and_keeps_other_placements_pinned()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let other = register(
                &mut case.fixture.conn,
                case.fixture.run.fleet_id,
                "other-deletion-target",
                None,
            );
            case.release();
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
            let mut host = DeletionTestHost::recovering(&case, deadline);
            host.mutation = Mutation::OtherPlacement(other);
            assert!(
                delete_fleet_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    case.fixture.run.fleet_id,
                    deadline,
                    &mut host
                )
                .is_err()
            );
            assert_registry_active(&case.fixture);
            assert!(host.recovered.is_some());
            assert_eq!(host.calls, vec!["shutdown", "recover"]);
            assert_eq!(
                crate::broker::get_member(&case.fixture.conn, other, case.fixture.run.fleet_id)
                    .unwrap()
                    .unwrap()
                    .placement
                    .unwrap()
                    .mux_pane_id
                    .as_deref(),
                Some("%foreign")
            );
        }

        #[test]
        fn runner_recovery_deletion_exhausted_deadline_retains_pending_namespace_and_active_registry()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            let before = rows(&case.fixture.conn);
            let bytes = case.spool_bytes().unwrap();
            case.release();
            let deadline = Deadline::until(
                Instant::now()
                    .checked_sub(Duration::from_millis(1))
                    .unwrap(),
            );
            let mut host = DeletionTestHost::recovering(&case, deadline);
            assert!(
                delete_fleet_with_host(
                    &mut case.fixture.conn,
                    &case.settings,
                    case.fixture.run.fleet_id,
                    deadline,
                    &mut host
                )
                .is_err()
            );
            assert_eq!(rows(&case.fixture.conn), before);
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            assert_registry_active(&case.fixture);
            assert!(host.calls.is_empty());
        }

        struct EmergencyInterrupt {
            cancellation: Option<mpsc::Sender<()>>,
            worker: Option<std::thread::JoinHandle<()>>,
            fired: Arc<AtomicBool>,
        }
        impl EmergencyInterrupt {
            fn arm(conn: &Connection, deadline: Instant) -> Self {
                let interrupt = conn.get_interrupt_handle();
                let fired = Arc::new(AtomicBool::new(false));
                let observed = fired.clone();
                let (cancellation, cancelled) = mpsc::channel();
                let worker = std::thread::spawn(move || {
                    match cancelled.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    {
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            observed.store(true, Ordering::Release);
                            interrupt.interrupt();
                        }
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
                    }
                });
                Self {
                    cancellation: Some(cancellation),
                    worker: Some(worker),
                    fired,
                }
            }
            fn finish(mut self) -> bool {
                drop(self.cancellation.take());
                self.worker.take().unwrap().join().unwrap();
                self.fired.load(Ordering::Acquire)
            }
        }
        impl Drop for EmergencyInterrupt {
            fn drop(&mut self) {
                drop(self.cancellation.take());
                if let Some(worker) = self.worker.take() {
                    drop(worker.join());
                }
            }
        }

        #[test]
        fn runner_recovery_deletion_unwind_restores_the_still_live_outer_scope() {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            case.release();
            let start = Instant::now();
            let inner = start + Duration::from_millis(600);
            let outer = start + Duration::from_millis(1000);
            let emergency =
                EmergencyInterrupt::arm(&case.fixture.conn, start + Duration::from_secs(2));
            let deadline = Deadline::until(inner);
            let mut host = DeletionTestHost::recovering(&case, deadline);
            host.recovery().action = Some(Box::new(move |boundary, _, _| {
                if boundary == "before_compact" {
                    panic!("owned injected recovery unwind");
                }
                Ok(())
            }));
            case.fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            let fleet = case.fixture.run.fleet_id;
            let result = with_database_deadline(&mut case.fixture.conn, outer, |conn| {
                let deletion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    delete_fleet_with_host(conn, &case.settings, fleet, deadline, &mut host)
                }));
                assert!(deletion.is_err());
                assert!(
                    host.recovery()
                        .calls
                        .iter()
                        .any(|name| name == "before_compact")
                );
                std::thread::sleep(
                    (inner + Duration::from_millis(20)).saturating_duration_since(Instant::now()),
                );
                assert!(Instant::now() > inner && Instant::now() < outer);
                assert_eq!(
                    conn.query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                        .unwrap(),
                    42
                );
                conn.query_row(SLOW_SUM, [], |row| row.get::<_, i64>(0))
                    .map_err(crate::error::CafleetError::from)
            });
            assert!(
                !emergency.finish(),
                "the original outer deadline cancels SQL"
            );
            assert!(result.is_err());
            assert!(
                Instant::now() >= outer,
                "the retired inner deadline cannot cancel outer work"
            );
            assert!(start.elapsed() < Duration::from_millis(1400));
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                137
            );
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                42
            );
        }

        #[test]
        fn runner_recovery_deletion_actual_replay_sql_is_cancelled_within_the_supplied_remaining_budget()
         {
            let mut case = Case::new();
            case.seed(&[opening(&case.fixture.run, 1, 1)]);
            case.fixture.conn.execute_batch(&format!("CREATE TABLE recovery_sql_probe (entered INTEGER NOT NULL); CREATE TRIGGER slow_replay BEFORE INSERT ON member_runtime_events BEGIN INSERT INTO recovery_sql_probe VALUES (1); SELECT ({SLOW_SUM}); END")).unwrap();
            let entered = Arc::new(Mutex::new(None));
            let observed = entered.clone();
            case.fixture
                .conn
                .update_hook(Some(
                    move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                        if table == "recovery_sql_probe" {
                            observed
                                .lock()
                                .expect("owned replay SQL entry observation")
                                .get_or_insert_with(Instant::now);
                        }
                    },
                ))
                .unwrap();
            let bytes = case.spool_bytes().unwrap();
            case.release();
            case.fixture
                .conn
                .busy_timeout(Duration::from_millis(137))
                .unwrap();
            let start = Instant::now();
            let deadline = Deadline::until(start + Duration::from_millis(300));
            let emergency =
                EmergencyInterrupt::arm(&case.fixture.conn, start + Duration::from_secs(1));
            let mut host = DeletionTestHost::recovering(&case, deadline);
            let result = delete_fleet_with_host(
                &mut case.fixture.conn,
                &case.settings,
                case.fixture.run.fleet_id,
                deadline,
                &mut host,
            );
            let returned = Instant::now();
            let emergency_fired = emergency.finish();
            case.fixture
                .conn
                .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
                .unwrap();
            let entered = *entered.lock().expect("owned replay SQL entry observation");
            assert!(
                !emergency_fired,
                "the original replay deadline cancels SQL: entered={:?}, returned={:?}",
                entered.map(|at| at.duration_since(start)),
                returned.duration_since(start)
            );
            let entered =
                entered.expect("the actual replay trigger entered before SQL cancellation");
            assert!(entered >= start && entered < deadline.instant());
            assert!(returned >= deadline.instant());
            assert!(result.is_err());
            assert!(start.elapsed() < Duration::from_millis(900));
            assert!(
                host.recovery()
                    .calls
                    .iter()
                    .any(|name| name == "before_replay")
            );
            assert!(!host.calls.contains(&"close"));
            assert!(events(&case).is_empty());
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("SELECT COUNT(*) FROM recovery_sql_probe", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0,
                "canceled replay rolls back its actual trigger write"
            );
            assert_eq!(case.spool_bytes().unwrap(), bytes);
            assert_registry_active(&case.fixture);
            case.fixture
                .conn
                .execute_batch("DROP TRIGGER slow_replay")
                .unwrap();
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                137
            );
            assert_eq!(
                case.fixture
                    .conn
                    .query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                42
            );
        }
    }

    mod pane_locks {
        use crate::cli::fleet::delete_fleet;
        use crate::cli::fleet::deletion_placements;
        use crate::cli::fleet::tests::*;
        use crate::pane_lock::test_support::LockFixture;
        #[test]
        fn fleet_deletion_contention_preserves_rows_and_releases_preceding_guards() {
            let mut fixture = LockFixture::new();
            let placements =
                deletion_placements(&fixture.conn, &fixture.settings, fixture.member.fleet_id)
                    .unwrap();
            let held = placements.last().unwrap();
            let guard = PaneLock::try_acquire(&fixture.settings.database_url, &held.placement)
                .unwrap()
                .unwrap();
            let before =
                crate::broker::list_members(&fixture.conn, fixture.member.fleet_id).unwrap();
            let started = Instant::now();
            assert!(
                delete_fleet(
                    &mut fixture.conn,
                    &fixture.settings,
                    fixture.member.fleet_id,
                    Deadline::until(started + Duration::from_millis(160))
                )
                .is_err()
            );
            assert!(started.elapsed() < Duration::from_millis(700));
            assert_eq!(
                crate::broker::list_members(&fixture.conn, fixture.member.fleet_id).unwrap(),
                before
            );
            for placement in &placements[..placements.len() - 1] {
                assert!(
                    PaneLock::acquire_released(
                        &fixture.settings.database_url,
                        &placement.placement
                    )
                    .unwrap()
                    .is_some()
                );
            }
            drop(guard);
            assert_eq!(
                crate::cli::fleet::test_support::guarded_delete_fleet(
                    &mut fixture.conn,
                    fixture.member.fleet_id
                )
                .unwrap()
                .deregistered_count,
                before.len()
            );
        }

        #[test]
        fn fleet_deletion_reloads_new_placements_within_the_original_deadline() {
            let mut fixture = LockFixture::new();
            let placements =
                deletion_placements(&fixture.conn, &fixture.settings, fixture.member.fleet_id)
                    .unwrap();
            let first = &placements[0].placement;
            let last =
                PaneLock::try_acquire(&fixture.settings.database_url, &placements[1].placement)
                    .unwrap()
                    .unwrap();
            let path = fixture.conn.path().unwrap().to_owned();
            let settings = fixture.settings.clone();
            let fleet = fixture.member.fleet_id;
            let started = Instant::now();
            let deadline = Deadline::until(started + Duration::from_millis(600));
            let thread = std::thread::spawn(move || {
                let mut conn = Connection::open(path).unwrap();
                delete_fleet(&mut conn, &settings, fleet, deadline)
            });
            loop {
                if PaneLock::try_acquire(&fixture.settings.database_url, first)
                    .unwrap()
                    .is_none()
                {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_millis(400),
                    "deletion owns the first key before waiting on the last"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let added = register(&mut fixture.conn, fleet, "added-during-wait", Some("%99"));
            let added_placement = crate::broker::get_member(&fixture.conn, added, fleet)
                .unwrap()
                .unwrap()
                .placement
                .unwrap();
            let added_guard =
                PaneLock::try_acquire(&fixture.settings.database_url, &added_placement)
                    .unwrap()
                    .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            drop(last);
            let error = thread.join().unwrap().unwrap_err();
            assert!(
                error.message().contains(&format!("at member {added};")),
                "{error}"
            );
            assert!(started.elapsed() < Duration::from_millis(1100));
            assert_eq!(
                crate::broker::list_members(&fixture.conn, fleet)
                    .unwrap()
                    .len(),
                3
            );
            assert!(
                crate::broker::get_fleet(&fixture.conn, fleet)
                    .unwrap()
                    .unwrap()
                    .deleted_at
                    .is_none()
            );
            drop(added_guard);
            for placement in placements {
                assert!(
                    PaneLock::acquire_released(
                        &fixture.settings.database_url,
                        &placement.placement
                    )
                    .unwrap()
                    .is_some()
                );
            }
        }
    }
}
