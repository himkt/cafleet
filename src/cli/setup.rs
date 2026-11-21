use crate::assets::HermesSetup;
use crate::assets::agent_paths;
use crate::assets::install_agent;
use crate::broker::MemberRunRegistry;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::SpawnProbe;
use crate::coding_agent::hermes::HermesCli;
use crate::coding_agent::hermes::HermesConfig;
use crate::config::EnvLookup;
use crate::config::Settings;
use crate::db;
use crate::db::SchemaState;
use crate::error::CafleetError;
use crate::multiplexer::MultiplexerContext;
use crate::multiplexer::caller;
use crate::multiplexer::caller::Ancestry;
use crate::multiplexer::caller::CallerMux;
use crate::multiplexer::caller::Verification;
use crate::runtime::ProcessTable;
use crate::runtime::SystemProcessTable;
use crate::runtime::control::CheckStatus;
use crate::runtime::control::MemberState;
use crate::runtime::system::SystemProbe;
use crate::runtime::system::SystemRunner;
use clap::Args;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

#[derive(Args)]
pub(crate) struct SetupArgs {
    /// Install selected backends' assets (repeatable; default: every backend).
    #[arg(long = "coding-agent", value_name = "AGENT", num_args = 1.., value_enum)]
    coding_agent: Vec<CodingAgent>,
}

pub(crate) fn run(settings: &Settings, args: SetupArgs) -> Result<(), CafleetError> {
    let mut conn = prepare_database(settings)?;
    let home = crate::config::home_dir()?;
    let hermes = SystemProbe.resolve_binary(CodingAgent::Hermes.name());
    install_selected(
        &mut conn,
        &args.coding_agent,
        &crate::config::process_env,
        &home,
        hermes.as_deref(),
    )
}

fn prepare_database(settings: &Settings) -> Result<Connection, CafleetError> {
    let path = crate::db::database_path(&settings.database_url)?;
    if path.is_empty() {
        return Err(CafleetError::App("database URL has no file path".into()));
    }
    let db_file = Path::new(path);
    if let Some(parent) = db_file.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| {
            CafleetError::App(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    if let Some(conn) = crate::db::open_existing(&settings.database_url, true)? {
        let schema = crate::db::classify_selected_schema(&conn, &settings.database_url);
        match schema {
            SchemaState::Missing | SchemaState::Current => {}
            _ => schema.require_current()?,
        }
    }
    let mut conn = crate::db::connect(&settings.database_url)?;
    let schema = crate::db::classify_selected_schema(&conn, &settings.database_url);
    match schema {
        SchemaState::Missing => {
            crate::db::initialize(&mut conn)?;
            crate::db::classify_selected_schema(&conn, &settings.database_url).require_current()?;
            crate::cli::write_output(&format!("Initialized database at {}.", db_file.display()))?;
        }
        SchemaState::Current => crate::cli::write_output(&format!(
            "Database already initialized at {}.",
            db_file.display()
        ))?,
        _ => schema.require_current()?,
    }
    Ok(conn)
}

fn install_selected(
    conn: &mut Connection,
    selected: &[CodingAgent],
    env: EnvLookup<'_>,
    home: &Path,
    hermes_executable: Option<&Path>,
) -> Result<(), CafleetError> {
    for agent in CodingAgent::all() {
        if selected.is_empty() || selected.contains(agent) {
            let paths = agent_paths(env, home, *agent)?;
            let cli = hermes_executable
                .filter(|_| *agent == CodingAgent::Hermes)
                .map(|executable| HermesCli::new(executable.to_path_buf(), paths.base().into()));
            let hermes = match &cli {
                Some(cli) => HermesSetup::Available(cli),
                None if selected.contains(&CodingAgent::Hermes) => HermesSetup::MissingRequired,
                None => HermesSetup::Absent,
            };
            install_agent(conn, *agent, &paths, hermes, &mut std::io::stdout())?;
        }
    }
    Ok(())
}

#[derive(Args)]
pub(crate) struct DoctorArgs {
    /// Print one row per individual check instead of the per-component summary.
    #[arg(long)]
    verbose: bool,
    /// Output in JSON format.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Check {
    name: String,
    #[serde(serialize_with = "status_text")]
    status: CheckStatus,
    detail: String,
    #[serde(skip)]
    problem: Option<Problem>,
    #[serde(flatten)]
    runtime: Option<RuntimeFields>,
}

/// How a non-`ok` backend check reads on its backend's summary line.
#[derive(Debug)]
struct Problem {
    phrase: String,
    setup_fix: bool,
}

#[derive(Debug, Serialize)]
struct RuntimeFields {
    member_id: Option<i64>,
    #[serde(serialize_with = "state_token")]
    state: MemberState,
}

fn status_text<S: serde::Serializer>(status: &CheckStatus, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(status.as_str())
}

fn state_token<S: serde::Serializer>(state: &MemberState, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(state.token())
}

impl Check {
    fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
            problem: None,
            runtime: None,
        }
    }

    fn problem(mut self, phrase: impl Into<String>, setup_fix: bool) -> Self {
        self.problem = Some(Problem {
            phrase: phrase.into(),
            setup_fix,
        });
        self
    }

    fn runtime(mut self, member_id: Option<i64>, state: MemberState) -> Self {
        self.runtime = Some(RuntimeFields { member_id, state });
        self
    }

    /// A row that rerunning setup for its backend (the row's name prefix) fixes.
    fn fixable(
        name: impl Into<String>,
        status: CheckStatus,
        detail: impl Into<String>,
        phrase: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let backend = name
            .split('.')
            .next()
            .expect("split yields a first segment")
            .to_owned();
        let detail = format!(
            "{}; run 'cafleet setup --coding-agent {backend}'",
            detail.into()
        );
        Self::new(name, status, detail).problem(phrase, true)
    }

    /// An `error` row whose detail is also its summary phrase.
    fn failure(name: impl Into<String>, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self::new(name, CheckStatus::Error, detail.clone()).problem(detail, false)
    }
}

#[derive(Serialize)]
struct Summary {
    component: String,
    #[serde(serialize_with = "status_text")]
    status: CheckStatus,
    detail: String,
    #[serde(flatten)]
    members: Option<MemberGroup>,
}

#[derive(Serialize)]
struct MemberGroup {
    #[serde(serialize_with = "state_token")]
    state: MemberState,
    member_ids: Vec<i64>,
}

impl Summary {
    fn new(component: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status,
            detail: detail.into(),
            members: None,
        }
    }

    fn members(
        status: CheckStatus,
        detail: impl Into<String>,
        state: MemberState,
        member_ids: Vec<i64>,
    ) -> Self {
        Self {
            members: Some(MemberGroup { state, member_ids }),
            ..Self::new("members", status, detail)
        }
    }
}

pub(crate) enum CallerReading {
    /// Row 1 refused before any multiplexer command.
    SharedServer {
        label: String,
        pid: i64,
        message: String,
    },
    Unavailable(CafleetError),
    Resolved {
        name: &'static str,
        server: String,
        pane: Result<PaneReading, CafleetError>,
    },
}

pub(crate) enum PaneReading {
    Verified {
        variable: &'static str,
        context: MultiplexerContext,
    },
    /// The `unverified_message` or `unset_message` body.
    Unverified { detail: String },
}

/// Everything doctor reads from outside the process.
pub(crate) struct DoctorInputs<'a> {
    pub(crate) caller: &'a dyn Fn() -> CallerReading,
    pub(crate) settings: &'a Settings,
    pub(crate) probe: &'a dyn SpawnProbe,
    pub(crate) home: &'a Result<PathBuf, CafleetError>,
    /// The Hermes reader, present whenever `probe` resolves `hermes` and its configuration
    /// resolves.
    pub(crate) hermes: Option<&'a dyn HermesConfig>,
    pub(crate) registry: &'a dyn Fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>,
}

pub(crate) struct Report {
    checks: Vec<Check>,
    summary: Vec<Summary>,
}

impl Report {
    fn has_errors(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == CheckStatus::Error)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Summary,
    Verbose,
    Json,
}

pub(crate) fn doctor(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    doctor_with(settings, args, &SystemProcessTable)
}

pub(crate) fn doctor_with(
    settings: &Settings,
    args: DoctorArgs,
    table: &dyn ProcessTable,
) -> Result<(), CafleetError> {
    let home = crate::config::home_dir();
    let hermes = hermes_reader(&SystemProbe, &home);
    let report = gather(&DoctorInputs {
        caller: &|| detect_caller(table),
        settings,
        probe: &SystemProbe,
        home: &home,
        hermes: hermes.as_ref().map(|cli| cli as &dyn HermesConfig),
        registry: &crate::broker::member_run_registry,
    });
    let mode = match (args.json, args.verbose) {
        (true, _) => Mode::Json,
        (false, true) => Mode::Verbose,
        (false, false) => Mode::Summary,
    };
    crate::cli::write_output(&render(&report, mode))?;
    if report.has_errors() {
        Err(CafleetError::App("doctor found errors".into()))
    } else {
        Ok(())
    }
}

/// The Hermes reader, absent when the configuration or the executable is unusable; the
/// `hermes.configuration` and `hermes.executable` rows report why.
fn hermes_reader(
    probe: &dyn SpawnProbe,
    home: &Result<PathBuf, CafleetError>,
) -> Option<HermesCli> {
    let home = home.as_ref().ok()?;
    let base = CodingAgent::Hermes
        .config_dir(&|name| probe.env_var(name), home)
        .ok()?;
    let executable = probe.resolve_binary(CodingAgent::Hermes.name())?;
    Some(HermesCli::new(executable, base))
}

/// The `fleet create` verification without `--pane`, as doctor reports it.
fn detect_caller(table: &dyn ProcessTable) -> CallerReading {
    let env = match crate::multiplexer::caller_env() {
        Ok(env) => env,
        Err(cause) => return CallerReading::Unavailable(cause),
    };
    let ancestry = caller::walk(table);
    if let Some(shared) = caller::shared_server(&ancestry) {
        return CallerReading::SharedServer {
            message: caller::shared_server_message(&shared),
            label: shared.label,
            pid: shared.pid,
        };
    }
    let mux = match caller::resolve_env_mux(&env, Rc::new(SystemRunner)) {
        Ok(mux) => mux,
        Err(cause) => return CallerReading::Unavailable(cause),
    };
    CallerReading::Resolved {
        name: mux.backend.name(),
        server: mux.server.clone(),
        pane: read_caller_pane(&mux, &env, table, &ancestry),
    }
}

fn read_caller_pane(
    mux: &CallerMux,
    env: &HashMap<String, String>,
    table: &dyn ProcessTable,
    ancestry: &Ancestry,
) -> Result<PaneReading, CafleetError> {
    let Some(pane) = caller::env_pane(mux.backend, env) else {
        return Ok(PaneReading::Unverified {
            detail: caller::unset_message(mux.backend),
        });
    };
    match caller::verify_pane(mux.mux.as_ref(), table, ancestry, &pane.pane)? {
        Verification::Verified => {
            let context = mux.mux.pane_context(&pane.pane)?.ok_or_else(|| {
                CafleetError::App(format!(
                    "pane {} was not found on the {} server {}",
                    pane.pane,
                    mux.backend.name(),
                    mux.server
                ))
            })?;
            Ok(PaneReading::Verified {
                variable: pane.variable,
                context,
            })
        }
        Verification::Refused(refusal) => Ok(PaneReading::Unverified {
            detail: caller::unverified_message(&pane, &refusal, mux),
        }),
    }
}

/// Runs every check: the Hermes batch starts first and overlaps the multiplexer, database,
/// OpenCode and member checks; rows keep their order.
pub(crate) fn gather(inputs: &DoctorInputs<'_>) -> Report {
    let backends = resolve_backends(inputs.probe, inputs.home);
    let hermes = backends
        .iter()
        .find(|backend| backend.agent == CodingAgent::Hermes)
        .expect("the registry includes hermes");
    std::thread::scope(|scope| {
        let runs = hermes.executable.is_some() && hermes.paths.as_ref().is_ok_and(|b| b.exists());
        let batch = runs.then(|| {
            let reader = inputs
                .hermes
                .expect("doctor builds a Hermes reader whenever hermes and its base resolve");
            let deadline = Instant::now() + HERMES_DOCTOR_BUDGET;
            scope.spawn(move || crate::coding_agent::hermes::read_batch(reader, deadline))
        });
        let (mut checks, mut summary) = multiplexer_rows((inputs.caller)());
        let (conn, schema) = database(inputs.settings);
        let (database_check, database_summary) = database_rows(inputs.settings, &schema);
        checks.push(database_check);
        summary.push(database_summary);
        let (member_rows, member_summary) = match (&schema, conn.as_ref()) {
            (SchemaState::Current, Some(conn)) => member_checks(conn, inputs.registry),
            _ => (
                Vec::new(),
                vec![Summary::members(
                    CheckStatus::Info,
                    "not checked (database unavailable)",
                    MemberState::NotChecked,
                    Vec::new(),
                )],
            ),
        };
        let hermes_reading = || batch.map(crate::coding_agent::hermes::joined);
        let backend_checks = agent_checks(conn.as_ref(), &schema, &backends, hermes_reading);
        summary.extend(
            CodingAgent::all()
                .iter()
                .map(|agent| backend_summary(*agent, &backend_checks)),
        );
        checks.extend(backend_checks);
        checks.extend(member_rows);
        summary.extend(member_summary);
        Report { checks, summary }
    })
}

const HERMES_DOCTOR_BUDGET: Duration = Duration::from_secs(2);

pub(crate) fn render(report: &Report, mode: Mode) -> String {
    let rows: Vec<(CheckStatus, &str, &str)> = match mode {
        Mode::Json => {
            return json!({"summary": report.summary, "checks": report.checks}).to_string();
        }
        Mode::Verbose => report
            .checks
            .iter()
            .map(|check| (check.status, check.name.as_str(), check.detail.as_str()))
            .collect(),
        Mode::Summary => report
            .summary
            .iter()
            .map(|line| (line.status, line.component.as_str(), line.detail.as_str()))
            .collect(),
    };
    rows.iter()
        .map(|(status, name, detail)| format!("{} {name}: {}", status.as_str(), escape(detail)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `multiplexer` and `multiplexer.caller_pane` checks and their folded summary line.
fn multiplexer_rows(reading: CallerReading) -> (Vec<Check>, Vec<Summary>) {
    let ((mux_status, mux_detail), (pane_status, pane_detail), (status, short)) = match reading {
        CallerReading::SharedServer {
            label,
            pid,
            message: detail,
        } => (
            (
                CheckStatus::Info,
                format!(
                    "not checked: a shared {label} server (pid {pid}) supplies the caller environment"
                ),
            ),
            (CheckStatus::Warn, detail.clone()),
            (CheckStatus::Warn, detail),
        ),
        CallerReading::Unavailable(cause) => (
            (CheckStatus::Error, cause.to_string()),
            (
                CheckStatus::Info,
                "unavailable: the multiplexer is unavailable".to_owned(),
            ),
            (CheckStatus::Error, cause.to_string()),
        ),
        CallerReading::Resolved { name, server, pane } => {
            let mux = (CheckStatus::Ok, format!("{name} server={server}"));
            match pane {
                Ok(PaneReading::Verified { variable, context }) => (
                    mux,
                    (
                        CheckStatus::Ok,
                        format!(
                            "{variable}={} session={} window={}",
                            context.pane_id, context.session, context.window_id
                        ),
                    ),
                    (
                        CheckStatus::Ok,
                        format!(
                            "{name} (session {}, pane {})",
                            context.session, context.pane_id
                        ),
                    ),
                ),
                Ok(PaneReading::Unverified { detail }) => (
                    mux,
                    (CheckStatus::Warn, detail.clone()),
                    (
                        CheckStatus::Warn,
                        format!("{name} (server {server}); {detail}"),
                    ),
                ),
                Err(cause) => (
                    mux,
                    (CheckStatus::Error, cause.to_string()),
                    (CheckStatus::Error, cause.to_string()),
                ),
            }
        }
    };
    (
        vec![
            Check::new("multiplexer", mux_status, mux_detail),
            Check::new("multiplexer.caller_pane", pane_status, pane_detail),
        ],
        vec![Summary::new("multiplexer", status, short)],
    )
}

fn database_rows(settings: &Settings, schema: &SchemaState) -> (Check, Summary) {
    let (status, detail, short) = match schema {
        SchemaState::Current => {
            let path = crate::db::database_path(&settings.database_url)
                .expect("a current schema was opened through a valid database URL");
            let version = format!("cafleet {}", crate::cli::VERSION);
            (CheckStatus::Ok, format!("{version} at {path}"), version)
        }
        schema => {
            let cause = schema
                .require_current()
                .expect_err("a schema that is not current is unavailable")
                .to_string();
            (CheckStatus::Error, cause.clone(), cause)
        }
    };
    (
        Check::new("database", status, detail),
        Summary::new("database", status, short),
    )
}

fn escape(detail: &str) -> String {
    detail.replace('\n', "\\n").replace('\r', "\\r")
}

fn backend_summary(agent: CodingAgent, checks: &[Check]) -> Summary {
    let name = agent.name();
    let own = checks
        .iter()
        .filter(|check| {
            check
                .name
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('.'))
        })
        .collect::<Vec<_>>();
    let find = |suffix: &str| {
        let check_name = format!("{name}.{suffix}");
        own.iter()
            .find(|check| check.name == check_name)
            .unwrap_or_else(|| panic!("every backend reports {check_name}"))
    };
    let configuration = find("configuration");
    if configuration.status == CheckStatus::Error {
        return Summary::new(name, CheckStatus::Error, configuration.detail.clone());
    }
    if find("executable").status != CheckStatus::Ok {
        return Summary::new(
            name,
            CheckStatus::Info,
            format!("not installed ({name} is not on PATH)"),
        );
    }
    let problems = own
        .iter()
        .filter_map(|check| {
            check
                .problem
                .as_ref()
                .map(|problem| (check.status, problem))
        })
        .collect::<Vec<_>>();
    let Some(status) = problems.iter().map(|(status, _)| *status).max() else {
        return Summary::new(name, CheckStatus::Ok, "ready");
    };
    let mut phrases = Vec::<&str>::new();
    for (_, problem) in &problems {
        if !phrases.contains(&problem.phrase.as_str()) {
            phrases.push(&problem.phrase);
        }
    }
    let mut detail = phrases.join("; ");
    if problems.iter().any(|(_, problem)| problem.setup_fix) {
        detail.push_str(&format!(" — run 'cafleet setup --coding-agent {name}'"));
    }
    Summary::new(name, status, detail)
}

struct MemberRun<'a> {
    row: &'a MemberRunRegistry,
    state: MemberState,
    phrases: Vec<String>,
}

fn classify_registry(row: &MemberRunRegistry) -> Option<MemberRun<'_>> {
    let active = row.member_status == "active";
    let state = match (row.is_settled(), active) {
        (true, false) => MemberState::Ended,
        (true, true) => MemberState::AgentExited,
        (false, false) => MemberState::DeletedWithLeftoverRuntime,
        (false, true) => return None,
    };
    Some(MemberRun {
        row,
        state,
        phrases: vec![format!(
            "run {}, cleanup {}",
            row.run_status.as_str(),
            row.cleanup_state.as_str()
        )],
    })
}

fn probe_run<'a>(source: &str, row: &'a MemberRunRegistry, deadline: Instant) -> MemberRun<'a> {
    let now = Instant::now();
    let (state, phrases) = if now >= deadline {
        (
            MemberState::NotChecked,
            vec!["runtime diagnosis time budget exhausted".to_owned()],
        )
    } else {
        let until = deadline.min(now + Duration::from_millis(100));
        match crate::runtime::diagnose_runtime_at(source, row.run_id, until) {
            Ok(diagnosis) => (diagnosis.state, diagnosis.diagnostics),
            Err(cause) => (MemberState::CheckFailed, vec![cause.to_string()]),
        }
    };
    MemberRun {
        row,
        state,
        phrases,
    }
}

/// The most runs probed at once, which bounds the open file descriptors.
const PROBE_WORKERS: usize = 16;

/// Probes `rows` on up to [`PROBE_WORKERS`] threads, each probing a contiguous share through its
/// own read-only connections, and returns the runs in `rows` order.
fn probe_runs<'a>(
    source: &str,
    rows: &[&'a MemberRunRegistry],
    deadline: Instant,
) -> Vec<MemberRun<'a>> {
    let chunk = rows.len().div_ceil(PROBE_WORKERS).max(1);
    std::thread::scope(|scope| {
        let workers = rows
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|row| probe_run(source, row, deadline))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(crate::coding_agent::hermes::joined)
            .collect()
    })
}

fn member_checks(
    conn: &Connection,
    registry: &dyn Fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>,
) -> (Vec<Check>, Vec<Summary>) {
    let rows = match registry(conn) {
        Ok(rows) => rows,
        Err(cause) => {
            let check = Check::new(
                "member_runtime",
                CheckStatus::Error,
                format!("member runtimes could not be read: {cause}"),
            )
            .runtime(None, MemberState::CheckFailed);
            let summary = Summary::members(
                CheckStatus::Error,
                format!("could not be checked: {cause}"),
                MemberState::CheckFailed,
                Vec::new(),
            );
            return (vec![check], vec![summary]);
        }
    };
    let classified = rows.iter().map(classify_registry).collect::<Vec<_>>();
    let unclassified = rows
        .iter()
        .zip(&classified)
        .filter(|(_, run)| run.is_none())
        .map(|(row, _)| row)
        .collect::<Vec<_>>();
    let mut probed = probe_runs(
        conn.path().expect("doctor reads a file-backed database"),
        &unclassified,
        Instant::now() + Duration::from_secs(5),
    )
    .into_iter();
    let runs = classified
        .into_iter()
        .map(|run| run.unwrap_or_else(|| probed.next().expect("one probe per unclassified run")))
        .collect::<Vec<_>>();
    let checks = runs.iter().map(runtime_check).collect();
    let mut summary = MemberState::SUMMARY_ORDER
        .iter()
        .filter_map(|state| {
            let grouped = runs
                .iter()
                .filter(|run| run.state == *state)
                .collect::<Vec<_>>();
            (!grouped.is_empty()).then(|| state_summary(*state, &grouped))
        })
        .collect::<Vec<_>>();
    if summary.is_empty() {
        summary.push(Summary::members(
            CheckStatus::Ok,
            "none running",
            MemberState::Running,
            Vec::new(),
        ));
    }
    (checks, summary)
}

fn runtime_check(run: &MemberRun) -> Check {
    let suffix = match run.state {
        MemberState::CheckFailed => String::new(),
        state => run_suffix(
            state.next_commands(),
            &run.row.member_id.to_string(),
            &director_label(&[run.row.director_member_id]),
        ),
    };
    let detail = format!(
        "member {} {}: {}{suffix}",
        run.row.member_id,
        run.state.label(),
        run.phrases.join("; ")
    );
    Check::new(
        format!("member_runtime.{}", run.row.run_id),
        run.state.status(),
        detail,
    )
    .runtime(Some(run.row.member_id), run.state)
}

fn state_summary(state: MemberState, runs: &[&MemberRun]) -> Summary {
    let mut member_ids = runs.iter().map(|run| run.row.member_id).collect::<Vec<_>>();
    member_ids.sort_unstable();
    member_ids.dedup();
    let directors = runs
        .iter()
        .map(|run| run.row.director_member_id)
        .collect::<Vec<_>>();
    let detail = format!(
        "{} {}: {}{}",
        member_ids.len(),
        state.label(),
        format_id_ranges(&member_ids),
        run_suffix(state.next_commands(), "ID", &director_label(&directors))
    );
    Summary::members(state.status(), detail, state, member_ids)
}

/// The literal Director ID shared by every listed run, or `<director-id>` when the runs span
/// several fleets or a fleet records no Director.
fn director_label(directors: &[i64]) -> String {
    match directors.split_first() {
        Some((first, rest)) if rest.iter().all(|other| other == first) => first.to_string(),
        _ => "<director-id>".to_owned(),
    }
}

fn run_suffix(commands: &[&str], member_id: &str, director_id: &str) -> String {
    if commands.is_empty() {
        return String::new();
    }
    let quoted = commands
        .iter()
        .map(|command| {
            format!(
                "'{}'",
                command
                    .replace("DIRECTOR", director_id)
                    .replace("ID", member_id)
            )
        })
        .collect::<Vec<_>>();
    format!(" — run {}", quoted.join(" or "))
}

/// Renders ascending, de-duplicated IDs, collapsing three or more consecutive IDs to `a-b`.
fn format_id_ranges(ids: &[i64]) -> String {
    assert!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "callers pass ascending, de-duplicated IDs"
    );
    ids.chunk_by(|left, right| left + 1 == *right)
        .flat_map(|run| match run {
            [first, .., last] if run.len() >= 3 => vec![format!("{first}-{last}")],
            _ => run.iter().map(i64::to_string).collect(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn database(settings: &Settings) -> (Option<Connection>, SchemaState) {
    match crate::db::open_existing(&settings.database_url, true) {
        Ok(Some(conn)) => {
            let schema = db::classify_selected_schema(&conn, &settings.database_url);
            (Some(conn), schema)
        }
        Ok(None) => (None, SchemaState::Missing),
        Err(cause) => (None, SchemaState::Unreachable { cause }),
    }
}

/// One backend's configuration base and executable, each resolved once.
struct Backend {
    agent: CodingAgent,
    paths: Result<PathBuf, CafleetError>,
    executable: Option<PathBuf>,
}

fn resolve_backends(probe: &dyn SpawnProbe, home: &Result<PathBuf, CafleetError>) -> Vec<Backend> {
    CodingAgent::all()
        .iter()
        .map(|agent| Backend {
            agent: *agent,
            paths: home
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|home| agent.config_dir(&|name| probe.env_var(name), home)),
            executable: probe.resolve_binary(agent.name()),
        })
        .collect()
}

/// The Hermes batch's `config path` and `plugins` results.
type HermesReading = (Result<String, String>, Result<Option<Value>, String>);

/// The `info` row of a check whose backend configuration failed or whose executable is absent.
fn unavailable(
    name: &str,
    paths: &Result<PathBuf, CafleetError>,
    available: bool,
) -> Option<Check> {
    let detail = match (paths, available) {
        (Err(cause), _) => format!("unavailable: configuration failed: {cause}"),
        (_, false) => "unavailable: executable is absent".to_owned(),
        (Ok(_), true) => return None,
    };
    Some(Check::new(name, CheckStatus::Info, detail))
}

/// A Hermes row without a batch reading: unavailable, or no Hermes home at the base.
fn hermes_row_without_reading(name: &str, hermes: &Backend) -> Check {
    unavailable(name, &hermes.paths, hermes.executable.is_some()).unwrap_or_else(|| {
        let base = hermes
            .paths
            .as_ref()
            .expect("an available hermes has a base");
        Check::new(
            name,
            CheckStatus::Info,
            format!("unavailable: no Hermes home at {}", base.display()),
        )
        .problem(format!("no Hermes home at {}", base.display()), true)
    })
}

fn agent_checks(
    conn: Option<&Connection>,
    schema: &SchemaState,
    backends: &[Backend],
    hermes_reading: impl FnOnce() -> Option<HermesReading>,
) -> Vec<Check> {
    let mut hermes_reading = Some(hermes_reading);
    let mut checks = Vec::new();
    for backend in backends {
        let agent = backend.agent;
        let name = agent.name();
        let paths = &backend.paths;
        let available = backend.executable.is_some();
        let (status, detail) = match paths {
            Ok(path) => (CheckStatus::Ok, path.display().to_string()),
            Err(cause) => (CheckStatus::Error, cause.to_string()),
        };
        checks.push(Check::new(format!("{name}.configuration"), status, detail));
        let (status, detail) = if available {
            (CheckStatus::Ok, format!("{name} is on PATH"))
        } else {
            (
                CheckStatus::Info,
                format!("{name} is optional and absent from PATH"),
            )
        };
        checks.push(Check::new(format!("{name}.executable"), status, detail));
        if agent == CodingAgent::Opencode {
            let default_mode = agent
                .resolve_policy(None)
                .expect("registered backend has a valid default policy")
                .resolved_mode();
            for mode in agent.policy_choices() {
                checks.push(preset_check(
                    agent,
                    mode,
                    *mode == default_mode,
                    paths,
                    available,
                ));
            }
            checks.push(opencode_extension_check(paths, available));
        }
        if agent == CodingAgent::Pi {
            checks.push(pi_extension_check(paths, available));
        }
        if agent == CodingAgent::Hermes {
            match hermes_reading.take().expect("one hermes backend")() {
                Some((config_path, plugins)) => {
                    checks.push(hermes_extension_check(config_path));
                    checks.push(hermes_plugin_enabled_check(plugins));
                }
                None => checks.extend(
                    ["hermes.extension", "hermes.plugin_enabled"]
                        .map(|name| hermes_row_without_reading(name, backend)),
                ),
            }
        }
        let install_name = format!("{name}.installation");
        checks.push(match (paths, schema) {
            (Err(cause), _) => Check::new(
                install_name,
                CheckStatus::Info,
                format!("unavailable: configuration failed: {cause}"),
            ),
            (Ok(path), SchemaState::Current) => install_check(
                conn.expect("a current database has an open connection"),
                agent,
                path,
            ),
            _ => {
                let cause = schema
                    .require_current()
                    .expect_err("a non-current schema is unavailable");
                Check::new(
                    install_name,
                    CheckStatus::Info,
                    format!("unavailable: {cause}"),
                )
                .problem("assets unchecked (database unavailable)", false)
            }
        });
    }
    checks
}

fn preset_check(
    agent: CodingAgent,
    mode: &str,
    is_default: bool,
    paths: &Result<PathBuf, CafleetError>,
    available: bool,
) -> Check {
    let name = agent.name();
    let check_name = if is_default {
        format!("{name}.preset")
    } else {
        format!("{name}.preset.{mode}")
    };
    if let Some(row) = unavailable(&check_name, paths, available) {
        return row;
    }
    let preset = agent
        .required_preset(
            paths.as_ref().expect("an available backend has a base"),
            mode,
        )
        .expect("OpenCode has a required preset for every launch policy");
    match std::fs::read(&preset) {
        Ok(_) => Check::new(check_name, CheckStatus::Ok, preset.display().to_string()),
        Err(cause) => Check::fixable(
            check_name,
            CheckStatus::Error,
            format!("{}: {cause}", preset.display()),
            format!("preset {} is unreadable: {cause}", preset.display()),
        ),
    }
}

/// Compares installed extension files with the embedded copies, then checks the executable record.
fn extension_check(
    name: &str,
    files: &[(PathBuf, &'static [u8])],
    record: &Path,
    ok_detail: String,
) -> Check {
    for (target, embedded) in files {
        if let Some(cause) = crate::assets::extension_file_problem(target, embedded) {
            return Check::fixable(
                name,
                CheckStatus::Error,
                format!("{}: {cause}", target.display()),
                format!(
                    "extension {} is missing or differs from the embedded copy",
                    target.display()
                ),
            );
        }
    }
    if let Some(cause) = crate::assets::executable_record_problem(record) {
        return Check::fixable(
            name,
            CheckStatus::Error,
            format!("{}: {cause}", record.display()),
            format!(
                "extension executable record {} is unusable: {cause}",
                record.display()
            ),
        );
    }
    Check::new(name, CheckStatus::Ok, ok_detail)
}

fn extension_files(root: &Path, prefix: &str) -> Vec<(PathBuf, &'static [u8])> {
    crate::assets::embedded_extension(prefix)
        .into_iter()
        .map(|(relative, bytes)| (root.join(relative), bytes))
        .collect()
}

fn opencode_extension_check(paths: &Result<PathBuf, CafleetError>, available: bool) -> Check {
    file_extension_check(
        "opencode.extension",
        paths,
        available,
        crate::assets::opencode_plugins_dir,
        crate::assets::OPENCODE_EXTENSION_PREFIX,
        crate::assets::OPENCODE_EXECUTABLE_RECORD,
    )
}

fn pi_extension_check(paths: &Result<PathBuf, CafleetError>, available: bool) -> Check {
    file_extension_check(
        "pi.extension",
        paths,
        available,
        crate::assets::pi_extensions_dir,
        crate::assets::PI_EXTENSION_PREFIX,
        crate::assets::PI_EXECUTABLE_RECORD,
    )
}

/// Checks an extension installed as embedded files beside one executable record under its base.
fn file_extension_check(
    name: &str,
    paths: &Result<PathBuf, CafleetError>,
    available: bool,
    extension_dir: fn(&Path) -> PathBuf,
    prefix: &str,
    record: &str,
) -> Check {
    if let Some(row) = unavailable(name, paths, available) {
        return row;
    }
    let dir = extension_dir(paths.as_ref().expect("an available backend has a base"));
    let files = extension_files(&dir, prefix);
    let ok_detail = files
        .iter()
        .map(|(target, _)| target.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    extension_check(name, &files, &dir.join(record), ok_detail)
}

fn hermes_extension_check(config_path: Result<String, String>) -> Check {
    const NAME: &str = "hermes.extension";
    let plugin_dir = config_path.and_then(|path| crate::coding_agent::hermes::plugin_dir(&path));
    match plugin_dir {
        Ok(plugin_dir) => extension_check(
            NAME,
            &extension_files(&plugin_dir, crate::assets::HERMES_EXTENSION_PREFIX),
            &plugin_dir.join(crate::assets::HERMES_EXECUTABLE_RECORD),
            plugin_dir.display().to_string(),
        ),
        Err(cause) => Check::failure(
            NAME,
            format!("cannot read hermes configuration through 'hermes config path': {cause}"),
        ),
    }
}

fn hermes_plugin_enabled_check(plugins: Result<Option<Value>, String>) -> Check {
    use crate::coding_agent::hermes::PLUGIN_NAME;
    use crate::coding_agent::hermes::PLUGINS_DISABLED_KEY;
    const NAME: &str = "hermes.plugin_enabled";
    let state = match plugins.and_then(crate::coding_agent::hermes::plugin_state) {
        Ok(state) => state,
        Err(cause) => {
            return Check::failure(
                NAME,
                format!("cannot read hermes configuration through 'hermes config get': {cause}"),
            );
        }
    };
    if state.disabled_lists_cafleet {
        let phrase = format!("hermes {PLUGINS_DISABLED_KEY} lists {PLUGIN_NAME}");
        return Check::new(
            NAME,
            CheckStatus::Error,
            format!("{phrase}; remove it, then run 'cafleet setup --coding-agent hermes'"),
        )
        .problem(phrase, false);
    }
    let missing = state.missing();
    if missing.is_empty() {
        return Check::new(
            NAME,
            CheckStatus::Ok,
            "cafleet enabled with gateway injection",
        );
    }
    let phrase = format!(
        "hermes plugin {PLUGIN_NAME} is not enabled: {}",
        missing.join(", ")
    );
    Check::fixable(NAME, CheckStatus::Error, phrase.clone(), phrase)
}

fn install_check(conn: &Connection, agent: CodingAgent, path: &Path) -> Check {
    let name = format!("{}.installation", agent.name());
    let recorded =
        crate::broker::recorded_asset_install(conn, agent.name(), &path.display().to_string());
    match recorded {
        Ok(Some(when)) => Check::new(name, CheckStatus::Ok, format!("recorded at {when}")),
        Ok(None) => Check::fixable(
            name,
            CheckStatus::Warn,
            format!("no installation recorded at {}", path.display()),
            format!("assets not installed at {}", path.display()),
        ),
        Err(cause) => Check::failure(name, format!("installation query failed: {cause}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::CliArgs;
    use crate::cli::Command;
    use crate::cli::setup::*;
    use crate::cli::test_support::assert_error;
    use crate::coding_agent::test_support::FakeProbe;
    use clap::Parser;
    use clap::error::ErrorKind;

    fn running_cafleet_record() -> String {
        format!(
            "{}\n",
            std::env::current_exe()
                .unwrap()
                .canonicalize()
                .unwrap()
                .display()
        )
    }

    fn install_embedded_extension(prefix: &str, root: &Path, record: &Path) -> Vec<PathBuf> {
        let installed = crate::assets::embedded_extension(prefix)
            .into_iter()
            .map(|(relative, bytes)| {
                let target = root.join(relative);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(&target, bytes).unwrap();
                target
            })
            .collect();
        std::fs::write(record, running_cafleet_record()).unwrap();
        installed
    }

    /// Returns the plugin file and its executable record.
    fn install_opencode_extension(base: &Path) -> (PathBuf, PathBuf) {
        let plugins = base.join("plugins");
        let record = plugins.join("cafleet.executable");
        let installed =
            install_embedded_extension(crate::assets::OPENCODE_EXTENSION_PREFIX, &plugins, &record);
        let plugin = plugins.join("cafleet.ts");
        assert!(installed.contains(&plugin));
        (plugin, record)
    }

    /// Returns the extension file and its executable record.
    fn install_pi_extension(base: &Path) -> (PathBuf, PathBuf) {
        let extensions = base.join("extensions");
        let record = extensions.join("cafleet.executable");
        let installed = install_embedded_extension("pi/", &extensions, &record);
        let extension = extensions.join("cafleet.ts");
        assert!(installed.contains(&extension));
        (extension, record)
    }

    /// Returns every installed plugin file under `plugin_dir`.
    fn install_hermes_plugin(plugin_dir: &Path) -> Vec<PathBuf> {
        install_embedded_extension(
            crate::assets::HERMES_EXTENSION_PREFIX,
            plugin_dir,
            &plugin_dir.join("executable"),
        )
    }

    fn install_skills(skills_dir: &Path) {
        for (relative, bytes) in crate::assets::SKILLS {
            let target = skills_dir.join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
    }

    fn settings(home: &Path) -> Settings {
        Settings::for_database(&home.join("registry.db"))
    }

    #[test]
    fn parser_uses_enum_choices_and_supports_repeated_multi_value_selection() {
        let Command::Setup(args) = CliArgs::try_parse_from([
            "cafleet",
            "setup",
            "--coding-agent",
            "opencode",
            "claude",
            "--coding-agent",
            "claude",
        ])
        .unwrap()
        .command
        else {
            panic!("setup command")
        };
        assert_eq!(
            args.coding_agent,
            [
                CodingAgent::Opencode,
                CodingAgent::Claude,
                CodingAgent::Claude
            ]
        );
    }

    #[test]
    fn setup_records_successes_before_the_first_asset_failure() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        let mut conn = prepare_database(&settings).unwrap();
        let error = install_selected(
            &mut conn,
            &[],
            &|name| Ok((name == "CODEX_HOME").then(|| "relative".into())),
            home.path(),
            Some(Path::new("/nonexistent/hermes")),
        )
        .unwrap_err();
        assert!(error.message().contains("CODEX_HOME"));
        crate::db::classify_selected_schema(&conn, "sqlite:///:memory:")
            .require_current()
            .unwrap();
        assert!(!home.path().join(".hermes").exists());
        assert!(!home.path().join(".config/opencode").exists());
    }

    #[test]
    fn selected_installation_ignores_unrelated_configuration_and_is_repeatable() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        for _ in 0..2 {
            let mut conn = prepare_database(&settings).unwrap();
            install_selected(
                &mut conn,
                &[CodingAgent::Claude, CodingAgent::Claude],
                &|name| Ok((name != "CLAUDE_CONFIG_DIR").then(|| "relative".into())),
                home.path(),
                Some(Path::new("/nonexistent/hermes")),
            )
            .unwrap();
            assert!(
                !home.path().join(".hermes").exists(),
                "an unselected hermes is not consulted"
            );
        }
    }

    #[test]
    fn refused_wal_databases_and_sidecars_remain_byte_identical_with_a_held_reader() {
        for mutation in [
            "PRAGMA application_id=42",
            "UPDATE cafleet_database SET cafleet_version='0.0.0'",
        ] {
            let home = tempfile::tempdir().unwrap();
            let settings = settings(home.path());
            let conn = prepare_database(&settings).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM refinery_schema_history")
                .unwrap();
            let path = crate::db::database_path(&settings.database_url).unwrap();
            let before = crate::db::test_support::wal_snapshot(Path::new(path));
            assert!(prepare_database(&settings).is_err(), "{mutation}");
            for (path, bytes) in before {
                assert_eq!(std::fs::read(&path).unwrap(), bytes, "{}", path.display());
            }
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
    #[test]
    fn setup_rejects_unknown_options_positionals_and_agent_values() {
        let cases: &[(&[&str], ErrorKind, &str)] = &[
            (&["setup", "extra"], ErrorKind::UnknownArgument, "extra"),
            (
                &["setup", "--unknown"],
                ErrorKind::UnknownArgument,
                "--unknown",
            ),
            (
                &["setup", "--coding-agent", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "extra"],
                ErrorKind::InvalidValue,
                "extra",
            ),
        ];
        for (args, kind, input) in cases {
            assert_error(args, *kind, input);
        }
    }

    /// Every backend on `PATH` or none, with `invalid` (when given) set to a relative path.
    fn probe(home: &Path, executable: bool, invalid: Option<&str>) -> FakeProbe {
        let mut probe = FakeProbe::installed(home);
        if !executable {
            probe.binaries.clear();
        }
        match invalid {
            Some(name) => probe.with_env(name, "relative"),
            None => probe,
        }
    }

    fn find<'a>(checks: &'a [Check], name: &str) -> &'a Check {
        checks
            .iter()
            .find(|check| check.name == name)
            .expect("every diagnostic has its stable name")
    }

    /// The backend rows for `probe`, with no Hermes batch read.
    fn checks_for(
        conn: Option<&Connection>,
        schema: &SchemaState,
        probe: &FakeProbe,
        home: &Path,
    ) -> Vec<Check> {
        let backends = resolve_backends(probe, &Ok(home.to_path_buf()));
        agent_checks(conn, schema, &backends, || None)
    }

    #[test]
    fn doctor_reports_a_modified_installed_skill() {
        let home = tempfile::tempdir().unwrap();
        let conn = crate::db::test_support::migrated_conn(&home);
        let probe = probe(home.path(), true, None);
        let skills = home.path().join(".claude/skills");
        let row = |conn: &Connection| {
            let checks = checks_for(Some(conn), &SchemaState::Current, &probe, home.path());
            let row = find(&checks, "claude.skills");
            (row.status, row.detail.clone())
        };
        assert_eq!(
            row(&conn),
            (
                CheckStatus::Warn,
                format!(
                    "skills not installed at {}; run 'cafleet setup --coding-agent claude'",
                    skills.display()
                )
            )
        );
        install_skills(&skills);
        assert_eq!(row(&conn), (CheckStatus::Ok, skills.display().to_string()));
        let (relative, _) = crate::assets::SKILLS[0];
        let modified = skills.join(relative);
        std::fs::write(&modified, "modified").unwrap();
        assert_eq!(
            row(&conn),
            (
                CheckStatus::Error,
                format!(
                    "{}: differs from the embedded copy; run 'cafleet setup --coding-agent claude'",
                    modified.display()
                )
            )
        );
    }

    #[test]
    fn independent_failures_keep_executable_discovery_and_mark_dependent_checks_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let probe = probe(home.path(), false, Some("OPENCODE_CONFIG_DIR"));
        let checks = checks_for(None, &SchemaState::Missing, &probe, home.path());
        assert_eq!(
            find(&checks, "opencode.configuration").status,
            CheckStatus::Error
        );
        assert_eq!(
            find(&checks, "opencode.executable").status,
            CheckStatus::Info
        );
        for name in [
            "opencode.preset.cafleet",
            "opencode.preset",
            "opencode.skills",
            "claude.skills",
            "codex.skills",
        ] {
            let check = find(&checks, name);
            assert_eq!(check.status, CheckStatus::Info);
            assert!(check.detail.starts_with("unavailable:"));
        }
        assert_eq!(
            find(&checks, "claude.configuration").status,
            CheckStatus::Ok
        );
    }

    #[derive(Clone, Copy)]
    enum PresetDetail {
        Path,
        ReadError,
    }

    #[test]
    fn opencode_preset_checks_follow_catalog_order_and_report_each_preset_state() {
        use PresetDetail::*;
        const FILES: [&str; 2] = ["cafleet.md", "cafleet-ask.md"];
        for (installed, expected) in [
            ([true, false], [("ok", Path), ("error", ReadError)]),
            ([false, true], [("error", ReadError), ("ok", Path)]),
        ] {
            let context = format!("installed={installed:?}");
            let home = tempfile::tempdir().unwrap();
            let agents = home.path().join(".config/opencode/agents");
            std::fs::create_dir_all(&agents).unwrap();
            for (file, present) in FILES.iter().zip(installed) {
                if present {
                    std::fs::write(agents.join(file), "preset").unwrap();
                }
            }
            let probe = probe(home.path(), true, None);
            let checks = checks_for(None, &SchemaState::Missing, &probe, home.path());
            let names = checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>();
            let position = names
                .iter()
                .position(|name| *name == "opencode.executable")
                .unwrap();
            assert_eq!(
                names[position + 1..position + 5],
                [
                    "opencode.preset.cafleet",
                    "opencode.preset",
                    "opencode.extension",
                    "opencode.skills"
                ],
                "{context}"
            );
            for ((check, file), (status, detail)) in checks[position + 1..position + 3]
                .iter()
                .zip(FILES)
                .zip(expected)
            {
                assert_eq!(check.status.as_str(), status, "{context} {}", check.name);
                let path = agents.join(file).display().to_string();
                match detail {
                    Path => assert_eq!(check.detail, path, "{context}"),
                    ReadError => {
                        let cause = check
                            .detail
                            .strip_prefix(&format!("{path}: "))
                            .and_then(|rest| {
                                rest.strip_suffix("; run 'cafleet setup --coding-agent opencode'")
                            })
                            .unwrap_or_else(|| panic!("{context}: {}", check.detail));
                        assert!(!cause.is_empty(), "{context}");
                    }
                }
            }
        }
    }

    enum ExtensionState {
        Installed,
        Plugin(Option<&'static str>),
        Record,
    }

    fn assert_extension_check(
        check: &Check,
        state: &ExtensionState,
        agent: &str,
        plugin: &Path,
        record: &Path,
        context: &str,
    ) {
        let fix = format!("; run 'cafleet setup --coding-agent {agent}'");
        match state {
            ExtensionState::Installed => {
                assert_eq!(check.status, CheckStatus::Ok, "{context}: {}", check.detail);
                assert_eq!(check.detail, plugin.display().to_string(), "{context}");
                assert!(check.problem.is_none(), "{context}");
            }
            ExtensionState::Plugin(expected) => {
                assert_eq!(
                    check.status,
                    CheckStatus::Error,
                    "{context}: {}",
                    check.detail
                );
                let cause = check
                    .detail
                    .strip_prefix(&format!("{}: ", plugin.display()))
                    .and_then(|rest| rest.strip_suffix(&fix))
                    .unwrap_or_else(|| panic!("{context}: {}", check.detail));
                match expected {
                    Some(expected) => assert_eq!(cause, *expected, "{context}"),
                    None => assert!(!cause.is_empty(), "{context}"),
                }
                let problem = check
                    .problem
                    .as_ref()
                    .expect("an extension error has a phrase");
                assert_eq!(
                    problem.phrase,
                    format!(
                        "extension {} is missing or differs from the embedded copy",
                        plugin.display()
                    ),
                    "{context}"
                );
                assert!(problem.setup_fix, "{context}");
            }
            ExtensionState::Record => {
                assert_eq!(
                    check.status,
                    CheckStatus::Error,
                    "{context}: {}",
                    check.detail
                );
                let cause = check
                    .detail
                    .strip_prefix(&format!("{}: ", record.display()))
                    .and_then(|rest| rest.strip_suffix(&fix))
                    .unwrap_or_else(|| panic!("{context}: {}", check.detail));
                assert!(!cause.is_empty(), "{context}");
                let problem = check.problem.as_ref().expect("a record error has a phrase");
                assert_eq!(
                    problem.phrase,
                    format!(
                        "extension executable record {} is unusable: {cause}",
                        record.display()
                    ),
                    "{context}"
                );
                assert!(problem.setup_fix, "{context}");
            }
        }
    }

    #[test]
    fn opencode_extension_check_reports_each_plugin_and_record_state() {
        type Mutation = fn(&Path, &Path);
        let cases: [(&str, Mutation, ExtensionState); 7] = [
            ("installed", |_, _| {}, ExtensionState::Installed),
            (
                "missing plugin",
                |plugin, _| std::fs::remove_file(plugin).unwrap(),
                ExtensionState::Plugin(None),
            ),
            (
                "tampered plugin",
                |plugin, _| std::fs::write(plugin, "tampered").unwrap(),
                ExtensionState::Plugin(Some("differs from the embedded copy")),
            ),
            (
                "missing record",
                |_, record| std::fs::remove_file(record).unwrap(),
                ExtensionState::Record,
            ),
            (
                "record names a missing file",
                |_, record| std::fs::write(record, "/nonexistent/cafleet\n").unwrap(),
                ExtensionState::Record,
            ),
            (
                "record names a file that is not executable",
                |_, record| {
                    let plain = record.with_file_name("not-executable");
                    std::fs::write(&plain, "plain").unwrap();
                    std::fs::write(record, format!("{}\n", plain.display())).unwrap();
                },
                ExtensionState::Record,
            ),
            (
                "empty record",
                |_, record| std::fs::write(record, "").unwrap(),
                ExtensionState::Record,
            ),
        ];
        for (context, mutate, state) in &cases {
            let home = tempfile::tempdir().unwrap();
            let (plugin, record) =
                install_opencode_extension(&home.path().join(".config/opencode"));
            mutate(&plugin, &record);
            let probe = probe(home.path(), true, None);
            let checks = checks_for(None, &SchemaState::Missing, &probe, home.path());
            assert_extension_check(
                find(&checks, "opencode.extension"),
                state,
                "opencode",
                &plugin,
                &record,
                context,
            );
        }
    }

    #[test]
    fn opencode_extension_check_is_unavailable_without_the_executable_or_configuration() {
        let home = tempfile::tempdir().unwrap();
        install_opencode_extension(&home.path().join(".config/opencode"));
        let absent = probe(home.path(), false, None);
        let check = find(
            &checks_for(None, &SchemaState::Missing, &absent, home.path()),
            "opencode.extension",
        )
        .detail
        .clone();
        assert_eq!(check, "unavailable: executable is absent");
        let invalid = probe(home.path(), true, Some("OPENCODE_CONFIG_DIR"));
        let checks = checks_for(None, &SchemaState::Missing, &invalid, home.path());
        let check = find(&checks, "opencode.extension");
        assert_eq!(check.status, CheckStatus::Info);
        assert!(
            check
                .detail
                .starts_with("unavailable: configuration failed: "),
            "{}",
            check.detail
        );
        assert!(check.problem.is_none());
    }

    mod hermes {
        use crate::cli::setup::tests::*;

        /// A `hermes` stub that logs `HERMES_HOME|ARGS` per call, answers `config path` with
        /// `<profile>/config.yaml`, reports `plugins` as *not set* until gateway injection is
        /// written, and accepts every set.
        fn hermes_stub(bin: &Path, log: &Path, profile: &Path) -> PathBuf {
            let state = profile.join("plugins.json");
            crate::coding_agent::test_support::executable_stub(
                bin,
                "hermes",
                &format!(
                    "printf '%s|%s\\n' \"$HERMES_HOME\" \"$*\" >> '{log}'\n\
                     case \"$*\" in\n\
                     'config path') printf '%s\\n' '{config}' ;;\n\
                     'config get --json plugins')\n\
                       if [ -f '{state}' ]; then cat '{state}'; else\n\
                         printf '%s\\n' 'install out of sync' 'Config key not set: plugins' >&2; exit 1; fi ;;\n\
                     'config set plugins.enabled '*) ;;\n\
                     'config set plugins.entries.cafleet.allow_gateway_injection true')\n\
                       printf '%s' '{{\"enabled\":[\"cafleet\"],\"entries\":{{\"cafleet\":{{\"allow_gateway_injection\":true}}}}}}' > '{state}' ;;\n\
                     *) exit 3 ;;\n\
                     esac",
                    log = log.display(),
                    config = profile.join("config.yaml").display(),
                    state = state.display(),
                ),
            )
        }

        #[test]
        fn plain_setup_without_hermes_on_path_installs_hermes_skills_and_skips_the_plugin() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            install_selected(&mut conn, &[], &|_| Ok(None), home.path(), None).unwrap();
            assert!(
                home.path()
                    .join(".hermes/skills/cafleet/SKILL.md")
                    .is_file()
            );
            assert!(!home.path().join(".hermes/plugins").exists());
        }

        #[test]
        fn setup_with_hermes_on_path_runs_it_against_the_resolved_home_and_enables_the_plugin() {
            let home = tempfile::tempdir().unwrap();
            let custom = home.path().join("hermes home");
            let profile = home.path().join("profile");
            std::fs::create_dir_all(&profile).unwrap();
            let bin = tempfile::tempdir().unwrap();
            let log = home.path().join("hermes.log");
            let hermes = hermes_stub(bin.path(), &log, &profile);
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            crate::coding_agent::test_support::run_stub(|| {
                install_selected(
                    &mut conn,
                    &[CodingAgent::Hermes],
                    &|name| Ok((name == "HERMES_HOME").then(|| custom.display().to_string())),
                    home.path(),
                    Some(&hermes),
                )
            })
            .unwrap();
            let calls = std::fs::read_to_string(&log).unwrap();
            let calls = calls.lines().collect::<Vec<_>>();
            let mut batch = calls[..2].to_vec();
            batch.sort_unstable();
            let home_prefix = format!("{}|", custom.display());
            assert_eq!(
                batch,
                [
                    format!("{home_prefix}config get --json plugins"),
                    format!("{home_prefix}config path"),
                ]
            );
            assert_eq!(
                calls[2..],
                [
                    format!("{home_prefix}config set plugins.enabled [\"cafleet\"]"),
                    format!(
                        "{home_prefix}config set plugins.entries.cafleet.allow_gateway_injection true"
                    ),
                    format!("{home_prefix}config get --json plugins"),
                ]
            );
            assert_eq!(
                std::fs::read_to_string(profile.join("plugins/cafleet/executable")).unwrap(),
                running_cafleet_record()
            );
            assert!(custom.join("skills/cafleet/SKILL.md").is_file());
        }

        #[test]
        fn explicit_hermes_setup_without_hermes_on_path_fails_without_recording_hermes() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let failure = install_selected(
                &mut conn,
                &[CodingAgent::Hermes],
                &|_| Ok(None),
                home.path(),
                None,
            )
            .expect_err("explicit hermes setup requires the binary");
            assert_eq!(failure.exit_code(), 1);
            assert_eq!(
                failure.message(),
                format!(
                    "failed to install extension into {}: binary hermes not found on PATH",
                    home.path().join(".hermes").display()
                )
            );
        }

        #[test]
        fn doctor_marks_the_hermes_plugin_rows_unavailable_without_the_executable_or_configuration()
        {
            let home = tempfile::tempdir().unwrap();
            for (executable, invalid, prefix) in [
                (false, None, "unavailable: executable is absent"),
                (
                    true,
                    Some("HERMES_HOME"),
                    "unavailable: configuration failed: ",
                ),
            ] {
                let probe = probe(home.path(), executable, invalid);
                let checks = checks_for(None, &SchemaState::Missing, &probe, home.path());
                for name in ["hermes.extension", "hermes.plugin_enabled"] {
                    let check = find(&checks, name);
                    assert_eq!(check.status, CheckStatus::Info, "{name}");
                    assert!(check.detail.starts_with(prefix), "{name}: {}", check.detail);
                    if !executable {
                        assert_eq!(check.detail, prefix, "{name}");
                    }
                    assert!(check.problem.is_none(), "{name}");
                }
            }
        }
    }

    mod pi {
        use crate::cli::setup::tests::*;

        fn pi_base(home: &Path) -> PathBuf {
            home.join(".pi/agent")
        }

        fn pi_checks(probe: &FakeProbe, home: &Path) -> Vec<Check> {
            checks_for(None, &SchemaState::Missing, probe, home)
                .into_iter()
                .filter(|check| check.name.starts_with("pi."))
                .collect()
        }

        fn assert_installed_pi_assets(base: &Path) {
            for (relative, bytes) in crate::assets::embedded_extension("pi/") {
                let target = base.join("extensions").join(relative);
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    bytes,
                    "{}",
                    target.display()
                );
            }
            assert_eq!(
                std::fs::read_to_string(base.join("extensions/cafleet.executable")).unwrap(),
                running_cafleet_record()
            );
            for (relative, bytes) in crate::assets::SKILLS {
                assert_eq!(
                    std::fs::read(base.join("skills").join(relative)).unwrap(),
                    *bytes
                );
            }
        }

        #[test]
        fn pi_setup_installs_skills_extension_and_record_without_pi_on_path_and_is_repeatable() {
            let home = tempfile::tempdir().unwrap();
            let settings = settings(home.path());
            let base = home.path().join("pi agent");
            for _ in 0..2 {
                let mut conn = prepare_database(&settings).unwrap();
                install_selected(
                    &mut conn,
                    &[CodingAgent::Pi],
                    &|name| Ok((name == "PI_CODING_AGENT_DIR").then(|| base.display().to_string())),
                    home.path(),
                    None,
                )
                .unwrap();
                assert_installed_pi_assets(&base);
            }
        }

        #[test]
        fn plain_setup_installs_pi_assets_under_the_default_base() {
            let home = tempfile::tempdir().unwrap();
            let settings = settings(home.path());
            let mut conn = prepare_database(&settings).unwrap();
            install_selected(&mut conn, &[], &|_| Ok(None), home.path(), None).unwrap();
            assert_installed_pi_assets(&pi_base(home.path()));
        }

        #[test]
        fn pi_setup_rejects_an_invalid_base_before_writing_pi_assets() {
            let home = tempfile::tempdir().unwrap();
            let settings = settings(home.path());
            let mut conn = prepare_database(&settings).unwrap();
            let failure = install_selected(
                &mut conn,
                &[CodingAgent::Pi],
                &|name| Ok((name == "PI_CODING_AGENT_DIR").then(|| "relative".into())),
                home.path(),
                None,
            )
            .unwrap_err();
            assert_eq!(
                failure.message(),
                "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')"
            );
            assert!(!pi_base(home.path()).exists());
        }

        #[test]
        fn pi_rows_follow_hermes_and_order_the_extension_between_executable_and_installation() {
            let home = tempfile::tempdir().unwrap();
            let checks = checks_for(
                None,
                &SchemaState::Missing,
                &probe(home.path(), true, None),
                home.path(),
            );
            let names = checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>();
            let pi = names
                .iter()
                .position(|name| name.starts_with("pi."))
                .unwrap();
            assert!(
                names[..pi].iter().any(|name| name.starts_with("hermes.")),
                "{names:?}"
            );
            assert_eq!(
                names[pi..],
                [
                    "pi.configuration",
                    "pi.executable",
                    "pi.extension",
                    "pi.skills"
                ]
            );
        }

        #[test]
        fn pi_extension_check_reports_each_extension_and_record_state() {
            type Mutation = fn(&Path, &Path);
            let cases: [(&str, Mutation, ExtensionState); 8] = [
                ("installed", |_, _| {}, ExtensionState::Installed),
                (
                    "missing extension",
                    |extension, _| std::fs::remove_file(extension).unwrap(),
                    ExtensionState::Plugin(None),
                ),
                (
                    "unreadable extension",
                    |extension, _| {
                        std::fs::remove_file(extension).unwrap();
                        std::fs::create_dir(extension).unwrap();
                    },
                    ExtensionState::Plugin(None),
                ),
                (
                    "tampered extension",
                    |extension, _| std::fs::write(extension, "tampered").unwrap(),
                    ExtensionState::Plugin(Some("differs from the embedded copy")),
                ),
                (
                    "missing record",
                    |_, record| std::fs::remove_file(record).unwrap(),
                    ExtensionState::Record,
                ),
                (
                    "record names a missing file",
                    |_, record| std::fs::write(record, "/nonexistent/cafleet\n").unwrap(),
                    ExtensionState::Record,
                ),
                (
                    "record names a file that is not executable",
                    |_, record| {
                        let plain = record.with_file_name("not-executable");
                        std::fs::write(&plain, "plain").unwrap();
                        std::fs::write(record, format!("{}\n", plain.display())).unwrap();
                    },
                    ExtensionState::Record,
                ),
                (
                    "empty record",
                    |_, record| std::fs::write(record, "").unwrap(),
                    ExtensionState::Record,
                ),
            ];
            for (context, mutate, state) in &cases {
                let home = tempfile::tempdir().unwrap();
                let (extension, record) = install_pi_extension(&pi_base(home.path()));
                mutate(&extension, &record);
                let checks = pi_checks(&probe(home.path(), true, None), home.path());
                assert_extension_check(
                    find(&checks, "pi.extension"),
                    state,
                    "pi",
                    &extension,
                    &record,
                    context,
                );
            }
        }

        #[test]
        fn pi_extension_check_is_unavailable_without_the_executable_or_configuration() {
            let home = tempfile::tempdir().unwrap();
            install_pi_extension(&pi_base(home.path()));
            let absent = pi_checks(&probe(home.path(), false, None), home.path());
            let check = find(&absent, "pi.extension");
            assert_eq!(check.status, CheckStatus::Info);
            assert_eq!(check.detail, "unavailable: executable is absent");
            assert!(check.problem.is_none());
            let invalid = pi_checks(
                &probe(home.path(), true, Some("PI_CODING_AGENT_DIR")),
                home.path(),
            );
            let configuration = find(&invalid, "pi.configuration");
            assert_eq!(configuration.status, CheckStatus::Error);
            assert_eq!(
                configuration.detail,
                "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')"
            );
            let check = find(&invalid, "pi.extension");
            assert_eq!(check.status, CheckStatus::Info);
            assert_eq!(
                check.detail,
                "unavailable: configuration failed: PI_CODING_AGENT_DIR must be an absolute path (got 'relative')"
            );
            assert!(check.problem.is_none());
        }

        #[test]
        fn pi_extension_check_reads_a_relocated_base() {
            let home = tempfile::tempdir().unwrap();
            let base = home.path().join("relocated pi");
            let (extension, record) = install_pi_extension(&base);
            let probe = probe(home.path(), true, None)
                .with_env("PI_CODING_AGENT_DIR", base.to_str().unwrap());
            let checks = pi_checks(&probe, home.path());
            assert_extension_check(
                find(&checks, "pi.extension"),
                &ExtensionState::Installed,
                "pi",
                &extension,
                &record,
                "relocated",
            );
        }
    }

    mod doctor_summary {
        use crate::assets::test_support::ENABLED_KEY;
        use crate::assets::test_support::FakeHermes;
        use crate::assets::test_support::INJECTION_KEY;
        use crate::assets::test_support::PLUGINS_KEY;
        use crate::assets::test_support::Reply;
        use crate::broker::fleets::MemberRunRegistry;
        use crate::broker::fleets::member_run_registry;
        use crate::broker::runtime::test_support::assert_key_order;
        use crate::cli::setup::tests::*;
        use crate::multiplexer::MuxBackend;
        use crate::multiplexer::caller::CallerMux;
        use crate::multiplexer::caller::CallerPane;
        use crate::multiplexer::caller::Refusal;
        use crate::multiplexer::caller::SharedServer;
        use crate::multiplexer::caller::shared_server_message;
        use crate::multiplexer::caller::unset_message;
        use crate::multiplexer::caller::unverified_message;
        use crate::runtime::test_support::FixtureProcessTable;
        use serde_json::Value;
        use serde_json::json;

        const TMUX_SOCKET: &str = "/tmp/tmux-1000/default";

        fn main_pane() -> MultiplexerContext {
            MultiplexerContext {
                session: "main".into(),
                window_id: "@1".into(),
                pane_id: "%1".into(),
            }
        }

        fn verified_caller() -> CallerReading {
            CallerReading::Resolved {
                name: "tmux",
                server: TMUX_SOCKET.to_string(),
                pane: Ok(PaneReading::Verified {
                    variable: "TMUX_PANE",
                    context: main_pane(),
                }),
            }
        }

        fn codex_daemon() -> SharedServer {
            SharedServer {
                label: "codex app-server".to_string(),
                pid: 407596,
            }
        }

        fn unverified_detail() -> String {
            unverified_message(
                &CallerPane {
                    variable: "TMUX_PANE",
                    pane: "%3".to_string(),
                },
                &Refusal::PaneNotAncestor { pane_pid: 405228 },
                &CallerMux {
                    backend: MuxBackend::Tmux,
                    server: TMUX_SOCKET.to_string(),
                    mux: MuxBackend::Tmux.for_server(
                        TMUX_SOCKET,
                        crate::multiplexer::test_support::FakeRunner::without_binaries(),
                    ),
                },
            )
        }

        const NO_MULTIPLEXER: &str = "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session";
        const PANE_READ_FAILURE: &str = "tmux command failed: server exited unexpectedly";

        fn named_row<'a>(rows: &'a [String], name: &str) -> &'a str {
            let matching = rows
                .iter()
                .filter(|line| row_name(line) == name)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "{name}: {rows:?}");
            matching[0]
        }

        const HERMES_FIX: &str = "run 'cafleet setup --coding-agent hermes'";

        const CHILD_CONFIG: &str = "CAFLEET_TEST_DOCTOR";
        const CHILD_STDOUT_MARKER: &str = "cafleet-doctor-child-stdout-begins";
        const CHILD_ANCESTOR: i64 = 4242;

        type Registry = fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>;

        fn ready_plugins() -> Value {
            serde_json::json!({
                "enabled": ["cafleet"],
                "entries": {"cafleet": {"allow_gateway_injection": true}},
            })
        }

        struct Scenario {
            home: tempfile::TempDir,
            database: PathBuf,
            registry: Registry,
            caller: fn() -> CallerReading,
            executables: Vec<&'static str>,
            environment: Vec<(&'static str, String)>,
            hermes: FakeHermes,
        }

        impl Scenario {
            fn new(registry: Registry) -> Self {
                let home = crate::runtime::test_support::short_tempdir();
                let database = home.path().join("absent.db");
                Self {
                    home,
                    database,
                    registry,
                    caller: verified_caller,
                    executables: Vec::new(),
                    environment: Vec::new(),
                    hermes: FakeHermes::new(),
                }
            }

            fn path(&self, name: &str) -> PathBuf {
                self.home.path().join(name)
            }

            fn config(&self, agent: CodingAgent) -> String {
                self.path(agent.name()).display().to_string()
            }

            fn connect(&mut self) -> Connection {
                let conn = crate::db::test_support::migrated_conn(&self.home);
                self.database = PathBuf::from(conn.path().unwrap());
                conn
            }

            fn executable(&mut self, name: &'static str) {
                self.executables.push(name);
            }

            /// The plugin directory under the active-profile home that `hermes config path`
            /// names, distinct from `HERMES_HOME`.
            fn hermes_plugin_dir(&self) -> PathBuf {
                self.hermes.plugin_dir()
            }

            /// Hermes on `PATH` with an existing home, an installed plugin and a ready
            /// configuration.
            fn hermes_ready(&mut self) {
                self.executable("hermes");
                std::fs::create_dir_all(self.config(CodingAgent::Hermes)).unwrap();
                install_hermes_plugin(&self.hermes_plugin_dir());
                self.hermes.put(PLUGINS_KEY, ready_plugins());
            }

            fn opencode_presets(&self) {
                let agents = self.path("opencode").join("agents");
                std::fs::create_dir_all(&agents).unwrap();
                for preset in ["cafleet.md", "cafleet-ask.md"] {
                    std::fs::write(agents.join(preset), "preset").unwrap();
                }
            }

            fn seed_opencode_extension(&self) -> (PathBuf, PathBuf) {
                install_opencode_extension(&self.path("opencode"))
            }

            fn seed_pi_extension(&self) -> (PathBuf, PathBuf) {
                install_pi_extension(&self.path("pi"))
            }

            fn probe(&self) -> FakeProbe {
                let mut probe = FakeProbe::installed(self.home.path());
                probe.binaries = self
                    .executables
                    .iter()
                    .map(|name| (name.to_string(), self.path("bin").join(name)))
                    .collect();
                for agent in CodingAgent::all() {
                    probe = probe.with_env(agent.config_variable(), &self.config(*agent));
                }
                for (name, value) in &self.environment {
                    probe = probe.with_env(name, value);
                }
                probe
            }

            fn report(&self) -> Report {
                let settings = Settings::for_database(&self.database);
                let probe = self.probe();
                let home = Ok(self.home.path().to_path_buf());
                gather(&DoctorInputs {
                    caller: &self.caller,
                    settings: &settings,
                    probe: &probe,
                    home: &home,
                    hermes: Some(&self.hermes as &dyn HermesConfig),
                    registry: &self.registry,
                })
            }

            fn rows(&self, code: i32) -> Vec<String> {
                let views = self.views();
                views.assert_exit(code);
                views.text
            }

            fn views(&self) -> Views {
                let report = self.report();
                let views = Views {
                    code: if report.has_errors() { 1 } else { 0 },
                    text: render(&report, Mode::Verbose)
                        .lines()
                        .map(str::to_owned)
                        .collect(),
                    json: serde_json::from_str(&render(&report, Mode::Json)).unwrap(),
                };
                assert_views_agree(&views);
                views
            }
        }

        struct Views {
            code: i32,
            text: Vec<String>,
            json: Value,
        }

        impl Views {
            fn row(&self, prefix: &str) -> &str {
                let rows = self
                    .text
                    .iter()
                    .filter(|line| line.starts_with(prefix))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 1, "{prefix}: {:?}", self.text);
                rows[0]
            }

            fn assert_exit(&self, code: i32) {
                assert_eq!(self.code, code, "{:?}", self.text);
            }
        }

        fn row_name(line: &str) -> &str {
            line.split_once(' ')
                .and_then(|(_, rest)| rest.split_once(": "))
                .map(|(name, _)| name)
                .unwrap_or_else(|| panic!("STATUS NAME: DETAIL row: {line}"))
        }

        fn status_lines(entries: &[Value], name_key: &str) -> Vec<String> {
            entries
                .iter()
                .map(|entry| {
                    format!(
                        "{} {}: {}",
                        entry["status"].as_str().unwrap(),
                        entry[name_key].as_str().unwrap(),
                        entry["detail"].as_str().unwrap()
                    )
                })
                .collect()
        }

        fn assert_views_agree(views: &Views) {
            for line in &views.text {
                let name = row_name(line);
                assert!(
                    ["multiplexer", "database"].contains(&name)
                        || (name.contains('.') && !name.starts_with("member_runtime")),
                    "doctor prints only environment check rows: {line}"
                );
            }
            let checks = views.json.as_array().unwrap();
            for entry in checks {
                assert_key_order(entry, &["name", "status", "detail"]);
            }
            assert_eq!(
                status_lines(checks, "name"),
                views.text,
                "the JSON array holds exactly the text rows in check order"
            );
        }

        #[test]
        #[ignore = "owned doctor subprocess entry"]
        fn doctor_child() {
            let config: Value =
                serde_json::from_str(&std::env::var(CHILD_CONFIG).expect("explicit doctor child"))
                    .unwrap();
            let settings = Settings::for_database(Path::new(config["database"].as_str().unwrap()));
            let argv = std::iter::once("cafleet").chain(
                config["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap()),
            );
            let Command::Doctor(args) = CliArgs::try_parse_from(argv).unwrap().command else {
                panic!("doctor command");
            };
            let own = i64::from(std::process::id());
            let table = match config["ancestor"].as_array() {
                Some(ancestor) => {
                    let ancestor: Vec<&str> =
                        ancestor.iter().map(|arg| arg.as_str().unwrap()).collect();
                    FixtureProcessTable::new(own)
                        .process(own, CHILD_ANCESTOR, &["cafleet", "doctor"])
                        .process(CHILD_ANCESTOR, 1, &ancestor)
                }
                None => FixtureProcessTable::new(own).process(own, 1, &["cafleet", "doctor"]),
            };
            println!("{CHILD_STDOUT_MARKER}");
            match doctor_with(&settings, args, &table) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }

        /// The child's `TMUX_PANE` and the pid the tmux stub reports for pane `%1`.
        #[derive(Clone, Copy)]
        enum ChildPane {
            Own,
            Unset,
            Pid(i64),
        }

        struct Outcome {
            code: Option<i32>,
            stdout: String,
            stderr: String,
        }

        /// Runs `doctor_with` in a child whose `PATH` holds only `tmux` and the given stubs,
        /// with every backend's configuration under `home`. The child's process table records
        /// the child itself with parent 1, or with `ancestor`'s argv between the child and pid 1.
        fn doctor_process(
            home: &Path,
            database: &Path,
            stubs: &[(&str, String)],
            args: &[&str],
            ancestor: Option<&[&str]>,
            pane: ChildPane,
        ) -> Outcome {
            use crate::runtime::system::test_support::TestChild;
            use std::process::Stdio;

            let bin = tempfile::tempdir().unwrap();
            // `$PPID` is the doctor child that runs the stub, so the child's own pid is the pane pid.
            let pane_pid = match pane {
                ChildPane::Pid(pid) => pid.to_string(),
                ChildPane::Own | ChildPane::Unset => "\"$PPID\"".to_owned(),
            };
            crate::coding_agent::test_support::executable_stub(
                bin.path(),
                "tmux",
                &format!(
                    "case \" $* \" in *'#{{pane_pid}}'*) printf '%%1|%s\\n' {pane_pid} ;; *' display-message '*) printf 'main|@1|%%1\\n' ;; esac"
                ),
            );
            for (name, body) in stubs {
                crate::coding_agent::test_support::executable_stub(bin.path(), name, body);
            }
            let capture = tempfile::tempdir().unwrap();
            let stdout_path = capture.path().join("stdout");
            let stderr_path = capture.path().join("stderr");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    test_path!(doctor_child),
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                    "--quiet",
                ])
                .env(
                    CHILD_CONFIG,
                    serde_json::json!({"database": database, "args": args, "ancestor": ancestor})
                        .to_string(),
                )
                .env("PATH", bin.path())
                .env("HOME", home)
                .env("TMUX", "cafleet-test,1,0")
                .env_remove("TMUX_PANE")
                .env_remove("HERDR_ENV")
                .env("RUST_BACKTRACE", "0");
            if !matches!(pane, ChildPane::Unset) {
                command.env("TMUX_PANE", "%1");
            }
            for agent in CodingAgent::all() {
                command.env(agent.config_variable(), home.join(agent.name()));
            }
            let started = Instant::now();
            let mut child = TestChild::new(
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::from(std::fs::File::create(&stdout_path).unwrap()))
                    .stderr(Stdio::from(std::fs::File::create(&stderr_path).unwrap()))
                    .spawn()
                    .unwrap(),
            );
            let status = child.poll_until(started + Duration::from_secs(20)).unwrap();
            if status.is_none() {
                child.terminate_until(started + Duration::from_secs(22));
            }
            let stderr = std::fs::read_to_string(&stderr_path).unwrap();
            let status = status
                .unwrap_or_else(|| panic!("{args:?}: doctor exceeded its deadline: {stderr}"));
            let stdout = std::fs::read_to_string(stdout_path).unwrap();
            let (_, output) = stdout
                .split_once(&format!("{CHILD_STDOUT_MARKER}\n"))
                .unwrap_or_else(|| panic!("{args:?}: child reached doctor: {stdout:?} {stderr}"));
            Outcome {
                code: status.code(),
                stdout: output.to_owned(),
                stderr,
            }
        }

        /// A home where every backend is ready, with its database and the backend stubs.
        fn healthy_home() -> (tempfile::TempDir, PathBuf, Vec<(&'static str, String)>) {
            let home = crate::runtime::test_support::short_tempdir();
            let conn = crate::db::test_support::migrated_conn(&home);
            let database = PathBuf::from(conn.path().unwrap());
            let profile = home.path().join("hermes-profile");
            std::fs::create_dir_all(home.path().join("hermes")).unwrap();
            install_hermes_plugin(&profile.join("plugins/cafleet"));
            let opencode = home.path().join("opencode");
            let agents = opencode.join("agents");
            std::fs::create_dir_all(&agents).unwrap();
            for preset in ["cafleet.md", "cafleet-ask.md"] {
                std::fs::write(agents.join(preset), "preset").unwrap();
            }
            install_opencode_extension(&opencode);
            install_pi_extension(&home.path().join("pi"));
            for agent in CodingAgent::all() {
                install_skills(&home.path().join(agent.name()).join("skills"));
            }
            install_skills(&crate::config::opencode_skills_base(home.path()).join("skills"));
            let hermes = format!(
                "case \"$*\" in\n\
                 'config path') printf '%s\\n' '{}' ;;\n\
                 'config get --json plugins') printf '%s\\n' '{}' ;;\n\
                 *) exit 3 ;;\n\
                 esac",
                profile.join("config.yaml").display(),
                ready_plugins()
            );
            let mut stubs = ["claude", "codex", "opencode", "pi"]
                .map(|name| (name, "exit 0".to_owned()))
                .to_vec();
            stubs.push(("hermes", hermes));
            (home, database, stubs)
        }

        fn check_lines(stdout: &str) -> Vec<String> {
            let payload: Value = serde_json::from_str(stdout.trim_end()).unwrap();
            status_lines(payload.as_array().unwrap(), "name")
        }

        #[test]
        fn doctor_process_reads_a_real_hermes_and_prints_json_for_a_healthy_home() {
            let (home, database, stubs) = healthy_home();
            let outcome = doctor_process(
                home.path(),
                &database,
                &stubs,
                &["doctor", "--json"],
                None,
                ChildPane::Own,
            );
            assert_eq!(outcome.code, Some(0), "{}", outcome.stderr);
            assert_eq!(outcome.stderr, "");
            let lines = check_lines(&outcome.stdout);
            assert_eq!(
                lines[..2],
                [
                    "ok multiplexer: tmux server=cafleet-test".to_owned(),
                    "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1".to_owned(),
                ]
            );
            assert!(
                lines.iter().all(|line| line.starts_with("ok ")),
                "{lines:?}"
            );
        }

        #[test]
        fn doctor_process_prints_the_json_array_rows_as_text_in_check_order() {
            let (home, database, stubs) = healthy_home();
            let run = |args: &[&str]| {
                let outcome =
                    doctor_process(home.path(), &database, &stubs, args, None, ChildPane::Own);
                assert_eq!(outcome.code, Some(0), "{}", outcome.stderr);
                outcome.stdout
            };
            let json = run(&["doctor", "--json"]);
            let text = run(&["doctor"]);
            let payload: Value = serde_json::from_str(json.trim_end()).unwrap();
            for entry in payload.as_array().unwrap() {
                assert_key_order(entry, &["name", "status", "detail"]);
            }
            assert_eq!(
                text.lines().map(str::to_owned).collect::<Vec<_>>(),
                check_lines(&json)
            );
        }

        #[test]
        fn doctor_process_under_a_codex_app_server_warns_on_the_caller_pane_and_exits_zero() {
            let (home, database, stubs) = healthy_home();
            let outcome = doctor_process(
                home.path(),
                &database,
                &stubs,
                &["doctor", "--json"],
                Some(&[
                    "/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex",
                    "app-server",
                    "--list",
                ]),
                ChildPane::Own,
            );
            assert_eq!(outcome.code, Some(0), "{}", outcome.stderr);
            assert_eq!(outcome.stderr, "");
            let detail = shared_server_message(&SharedServer {
                label: "codex app-server".to_string(),
                pid: CHILD_ANCESTOR,
            });
            assert_eq!(
                check_lines(&outcome.stdout)[..2],
                [
                    format!(
                        "info multiplexer: not checked: a shared codex app-server server (pid {CHILD_ANCESTOR}) supplies the caller environment"
                    ),
                    format!("warn multiplexer.caller_pane: {detail}"),
                ]
            );
        }

        fn assert_caller_pane_warning(outcome: &Outcome, detail: &str) {
            assert_eq!(outcome.code, Some(0), "{}", outcome.stderr);
            assert_eq!(outcome.stderr, "");
            assert_eq!(
                check_lines(&outcome.stdout)[..2],
                [
                    "ok multiplexer: tmux server=cafleet-test".to_owned(),
                    format!("warn multiplexer.caller_pane: {detail}"),
                ]
            );
        }

        #[test]
        fn doctor_process_with_tmux_pane_unset_warns_on_the_caller_pane_and_exits_zero() {
            let (home, database, stubs) = healthy_home();
            let outcome = doctor_process(
                home.path(),
                &database,
                &stubs,
                &["doctor", "--json"],
                None,
                ChildPane::Unset,
            );
            assert_caller_pane_warning(&outcome, &unset_message(MuxBackend::Tmux));
        }

        #[test]
        fn doctor_process_with_a_non_ancestor_pane_warns_unverified_and_exits_zero() {
            const STRANGER: i64 = 405228;
            let (home, database, stubs) = healthy_home();
            let outcome = doctor_process(
                home.path(),
                &database,
                &stubs,
                &["doctor", "--json"],
                None,
                ChildPane::Pid(STRANGER),
            );
            let detail = unverified_message(
                &CallerPane {
                    variable: "TMUX_PANE",
                    pane: "%1".to_string(),
                },
                &Refusal::PaneNotAncestor { pane_pid: STRANGER },
                &CallerMux {
                    backend: MuxBackend::Tmux,
                    server: "cafleet-test".to_string(),
                    mux: MuxBackend::Tmux.for_server(
                        "cafleet-test",
                        crate::multiplexer::test_support::FakeRunner::without_binaries(),
                    ),
                },
            );
            assert_caller_pane_warning(&outcome, &detail);
        }

        #[test]
        fn doctor_process_without_hermes_or_a_database_prints_json_and_fails() {
            let home = crate::runtime::test_support::short_tempdir();
            let database = home.path().join("absent.db");
            let outcome = doctor_process(
                home.path(),
                &database,
                &[],
                &["doctor", "--json"],
                None,
                ChildPane::Own,
            );
            assert_eq!(outcome.code, Some(1));
            assert_eq!(outcome.stderr, "Error: doctor found errors\n");
            let lines = check_lines(&outcome.stdout);
            assert!(
                named_row(&lines, "database").starts_with("error database: "),
                "{lines:?}"
            );
            assert_eq!(
                named_row(&lines, "hermes.executable"),
                "info hermes.executable: hermes is optional and absent from PATH"
            );
            assert!(!database.exists(), "doctor creates no database");
        }

        fn healthy_backends(scenario: &mut Scenario) {
            for name in ["claude", "codex", "opencode", "pi"] {
                scenario.executable(name);
            }
            scenario.hermes_ready();
            scenario.opencode_presets();
            scenario.seed_opencode_extension();
            scenario.seed_pi_extension();
            for agent in [CodingAgent::Claude, CodingAgent::Hermes, CodingAgent::Pi] {
                install_skills(&Path::new(&scenario.config(agent)).join("skills"));
            }
            install_skills(
                &crate::config::opencode_skills_base(scenario.home.path()).join("skills"),
            );
        }

        mod caller_pane_rows {
            use crate::cli::setup::tests::doctor_summary::*;

            /// The healthy scenario's views with `caller` as the caller reading.
            fn views_with(caller: fn() -> CallerReading) -> Views {
                let mut scenario = Scenario::new(member_run_registry);
                scenario.connect();
                healthy_backends(&mut scenario);
                scenario.caller = caller;
                scenario.views()
            }

            fn assert_multiplexer(views: &Views, rows: [&str; 2], code: i32) {
                assert_eq!(views.text[..2], rows);
                views.assert_exit(code);
            }

            #[test]
            fn a_shared_server_reports_info_multiplexer_and_warn_caller_pane() {
                let detail = shared_server_message(&codex_daemon());
                assert_multiplexer(
                    &views_with(|| CallerReading::SharedServer {
                        label: codex_daemon().label,
                        pid: codex_daemon().pid,
                        message: shared_server_message(&codex_daemon()),
                    }),
                    [
                        "info multiplexer: not checked: a shared codex app-server server (pid 407596) supplies the caller environment",
                        &format!("warn multiplexer.caller_pane: {detail}"),
                    ],
                    0,
                );
            }

            #[test]
            fn an_unavailable_multiplexer_reports_error_and_an_unavailable_caller_pane() {
                assert_multiplexer(
                    &views_with(|| {
                        CallerReading::Unavailable(CafleetError::App(NO_MULTIPLEXER.into()))
                    }),
                    [
                        &format!("error multiplexer: {NO_MULTIPLEXER}"),
                        "info multiplexer.caller_pane: unavailable: the multiplexer is unavailable",
                    ],
                    1,
                );
            }

            #[test]
            fn an_unset_env_pane_warns_with_the_unset_message() {
                let detail = unset_message(MuxBackend::Tmux);
                assert_multiplexer(
                    &views_with(|| CallerReading::Resolved {
                        name: "tmux",
                        server: TMUX_SOCKET.to_string(),
                        pane: Ok(PaneReading::Unverified {
                            detail: unset_message(MuxBackend::Tmux),
                        }),
                    }),
                    [
                        &format!("ok multiplexer: tmux server={TMUX_SOCKET}"),
                        &format!("warn multiplexer.caller_pane: {detail}"),
                    ],
                    0,
                );
            }

            #[test]
            fn a_refused_env_pane_warns_with_the_unverified_message() {
                let detail = unverified_detail();
                assert_multiplexer(
                    &views_with(|| CallerReading::Resolved {
                        name: "tmux",
                        server: TMUX_SOCKET.to_string(),
                        pane: Ok(PaneReading::Unverified {
                            detail: unverified_detail(),
                        }),
                    }),
                    [
                        &format!("ok multiplexer: tmux server={TMUX_SOCKET}"),
                        &format!("warn multiplexer.caller_pane: {detail}"),
                    ],
                    0,
                );
            }

            #[test]
            fn a_verified_pane_reports_its_context() {
                assert_multiplexer(
                    &views_with(verified_caller),
                    [
                        &format!("ok multiplexer: tmux server={TMUX_SOCKET}"),
                        "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1",
                    ],
                    0,
                );
            }

            #[test]
            fn a_propagated_pane_error_reports_error_caller_pane_and_fails() {
                assert_multiplexer(
                    &views_with(|| CallerReading::Resolved {
                        name: "tmux",
                        server: TMUX_SOCKET.to_string(),
                        pane: Err(CafleetError::App(PANE_READ_FAILURE.into())),
                    }),
                    [
                        &format!("ok multiplexer: tmux server={TMUX_SOCKET}"),
                        &format!("error multiplexer.caller_pane: {PANE_READ_FAILURE}"),
                    ],
                    1,
                );
            }

            #[test]
            fn a_herdr_verified_pane_names_herdr_pane_id() {
                let views = views_with(|| CallerReading::Resolved {
                    name: "herdr",
                    server: "default".to_string(),
                    pane: Ok(PaneReading::Verified {
                        variable: "HERDR_PANE_ID",
                        context: MultiplexerContext {
                            session: "w4".into(),
                            window_id: "w4:t1".into(),
                            pane_id: "w4:p3N".into(),
                        },
                    }),
                });
                assert_multiplexer(
                    &views,
                    [
                        "ok multiplexer: herdr server=default",
                        "ok multiplexer.caller_pane: HERDR_PANE_ID=w4:p3N session=w4 window=w4:t1",
                    ],
                    0,
                );
            }
        }

        #[test]
        fn doctor_with_only_warnings_prints_ready_backends_and_exits_zero() {
            let mut scenario = Scenario::new(member_run_registry);
            scenario.connect();
            healthy_backends(&mut scenario);
            let views = scenario.views();
            views.assert_exit(0);
            assert_eq!(
                named_row(&views.text, "opencode.extension"),
                format!(
                    "ok opencode.extension: {}",
                    scenario
                        .path("opencode")
                        .join("plugins/cafleet.ts")
                        .display()
                )
            );
            assert_eq!(
                named_row(&views.text, "hermes.extension"),
                format!(
                    "ok hermes.extension: {}",
                    scenario.hermes_plugin_dir().display()
                )
            );
            assert_eq!(
                named_row(&views.text, "hermes.plugin_enabled"),
                "ok hermes.plugin_enabled: cafleet enabled with gateway injection"
            );
            assert_eq!(
                views.row("warn codex.skills: "),
                format!(
                    "warn codex.skills: skills not installed at {}; run 'cafleet setup --coding-agent codex'",
                    Path::new(&scenario.config(CodingAgent::Codex))
                        .join("skills")
                        .display()
                )
            );
            assert!(
                views
                    .text
                    .iter()
                    .all(|line| !line.starts_with("warn ") || line.starts_with("warn codex.")),
                "{:?}",
                views.text
            );
        }

        #[test]
        fn doctor_reports_configuration_and_preset_errors_on_their_rows() {
            let mut scenario = Scenario::new(member_run_registry);
            let _conn = scenario.connect();
            for name in ["claude", "opencode"] {
                scenario.executable(name);
            }
            scenario
                .environment
                .push(("CLAUDE_CONFIG_DIR", "relative".into()));
            scenario.seed_opencode_extension();
            let views = scenario.views();
            views.assert_exit(1);
            assert_eq!(
                named_row(&views.text, "claude.configuration"),
                "error claude.configuration: CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')"
            );
            let agents = scenario.path("opencode").join("agents");
            for (name, preset) in [
                ("opencode.preset.cafleet", "cafleet.md"),
                ("opencode.preset", "cafleet-ask.md"),
            ] {
                assert_eq!(
                    named_row(&views.text, name),
                    format!(
                        "error {name}: {}: No such file or directory (os error 2); run 'cafleet setup --coding-agent opencode'",
                        agents.join(preset).display()
                    )
                );
            }
        }

        #[test]
        fn doctor_without_a_database_still_compares_the_installed_skills() {
            let mut scenario = Scenario::new(member_run_registry);
            scenario.executable("claude");
            let skills = Path::new(&scenario.config(CodingAgent::Claude)).join("skills");
            install_skills(&skills);
            let views = scenario.views();
            views.assert_exit(1);
            assert!(
                named_row(&views.text, "database").starts_with("error database: "),
                "{:?}",
                views.text
            );
            assert_eq!(
                named_row(&views.text, "claude.skills"),
                format!("ok claude.skills: {}", skills.display())
            );
            assert!(!scenario.database.exists(), "doctor creates no database");
        }

        fn healthy_scenario() -> (Scenario, Connection) {
            let mut scenario = Scenario::new(member_run_registry);
            let conn = scenario.connect();
            healthy_backends(&mut scenario);
            (scenario, conn)
        }

        #[test]
        fn doctor_reports_an_unreadable_hermes_config_path_on_its_row() {
            let (mut scenario, _conn) = healthy_scenario();
            scenario.hermes.reply(
                FakeHermes::path_call(),
                Reply::Fail("no configuration found".into()),
            );
            let rows = scenario.rows(1);
            let row = named_row(&rows, "hermes.extension");
            let detail = row
                .strip_prefix("error hermes.extension: ")
                .unwrap_or_else(|| panic!("{row}"));
            assert!(
                detail
                    .starts_with("cannot read hermes configuration through 'hermes config path': "),
                "{row}"
            );
            assert_eq!(
                named_row(&rows, "hermes.plugin_enabled"),
                "ok hermes.plugin_enabled: cafleet enabled with gateway injection"
            );
        }

        enum Enablement {
            Ready,
            Disabled,
            Missing(String),
            Unreadable,
        }

        #[test]
        fn doctor_reports_each_hermes_plugin_enablement_state_on_its_row() {
            let not_enabled = |keys: &str| format!("hermes plugin cafleet is not enabled: {keys}");
            let both = format!("{ENABLED_KEY}, {INJECTION_KEY}");
            let cases = [
                (
                    Reply::Value(Some(json!({
                        "disabled": ["other"],
                        "enabled": ["cafleet"],
                        "entries": {"cafleet": {"allow_gateway_injection": true}},
                    }))),
                    Enablement::Ready,
                ),
                (
                    Reply::Value(Some(json!({"disabled": ["other", "cafleet"]}))),
                    Enablement::Disabled,
                ),
                (
                    Reply::Value(Some(
                        json!({"entries": {"cafleet": {"allow_gateway_injection": true}}}),
                    )),
                    Enablement::Missing(ENABLED_KEY.into()),
                ),
                (
                    Reply::Value(Some(json!({
                        "enabled": ["other"],
                        "entries": {"cafleet": {"allow_gateway_injection": "true"}},
                    }))),
                    Enablement::Missing(both.clone()),
                ),
                (
                    Reply::Value(Some(json!({"enabled": "cafleet"}))),
                    Enablement::Unreadable,
                ),
            ];
            for (reply, expected) in cases {
                let (mut scenario, _conn) = healthy_scenario();
                scenario
                    .hermes
                    .reply(FakeHermes::get_call(PLUGINS_KEY), reply);
                let code = if matches!(expected, Enablement::Ready) {
                    0
                } else {
                    1
                };
                let rows = scenario.rows(code);
                let row = named_row(&rows, "hermes.plugin_enabled");
                match expected {
                    Enablement::Ready => {
                        assert_eq!(
                            row,
                            "ok hermes.plugin_enabled: cafleet enabled with gateway injection"
                        );
                    }
                    Enablement::Disabled => {
                        assert_eq!(
                            row,
                            format!(
                                "error hermes.plugin_enabled: hermes plugins.disabled lists cafleet; remove it, then {HERMES_FIX}"
                            )
                        );
                    }
                    Enablement::Missing(keys) => {
                        assert_eq!(
                            row,
                            format!(
                                "error hermes.plugin_enabled: {}; {HERMES_FIX}",
                                not_enabled(&keys)
                            )
                        );
                    }
                    Enablement::Unreadable => {
                        let detail = row
                            .strip_prefix("error hermes.plugin_enabled: ")
                            .unwrap_or_else(|| panic!("{row}"));
                        assert!(
                            detail.starts_with(
                                "cannot read hermes configuration through 'hermes config get': "
                            ),
                            "{row}"
                        );
                    }
                }
                assert!(named_row(&rows, "hermes.extension").starts_with("ok "));
            }
        }

        #[test]
        fn doctor_without_a_hermes_home_reports_both_hermes_rows_without_running_hermes() {
            let (scenario, _conn) = healthy_scenario();
            let base = scenario.path("hermes");
            std::fs::remove_dir_all(&base).unwrap();
            let rows = scenario.rows(0);
            for name in ["hermes.extension", "hermes.plugin_enabled"] {
                assert_eq!(
                    named_row(&rows, name),
                    format!(
                        "info {name}: unavailable: no Hermes home at {}",
                        base.display()
                    )
                );
            }
            assert!(
                scenario.hermes.calls().is_empty(),
                "{:?}",
                scenario.hermes.calls()
            );
            assert!(!base.exists(), "doctor creates no Hermes home");
        }

        #[test]
        fn doctor_overlaps_its_hermes_batch_with_the_other_checks_under_one_two_second_deadline() {
            let (mut scenario, _conn) = healthy_scenario();
            scenario.hermes = FakeHermes::new().rendezvous();
            install_hermes_plugin(&scenario.hermes_plugin_dir());
            scenario.hermes.put(PLUGINS_KEY, ready_plugins());
            let started = Instant::now();
            let views = scenario.views();
            let finished = Instant::now();
            views.assert_exit(0);
            assert!(
                finished - started < Duration::from_secs(2),
                "a serial batch waits out the rendezvous: {:?}",
                finished - started
            );
            let mut calls = scenario.hermes.calls();
            calls.sort();
            let mut expected = vec![FakeHermes::path_call(), FakeHermes::get_call(PLUGINS_KEY)];
            expected.sort();
            assert_eq!(calls, expected, "doctor makes exactly the two batch reads");
            let deadlines = scenario.hermes.deadlines();
            assert_eq!(deadlines[0], deadlines[1], "one deadline covers the batch");
            assert!(
                deadlines[0] >= started + Duration::from_secs(2)
                    && deadlines[0] <= finished + Duration::from_secs(2),
                "the batch gets two seconds"
            );
        }

        #[test]
        fn doctor_reports_each_tampered_hermes_and_opencode_extension_on_its_row() {
            let (scenario, _conn) = healthy_scenario();
            let file = install_hermes_plugin(&scenario.hermes_plugin_dir())
                .into_iter()
                .next()
                .unwrap();
            std::fs::write(&file, "tampered").unwrap();
            let (plugin, _) = scenario.seed_opencode_extension();
            std::fs::write(&plugin, "tampered").unwrap();
            let rows = scenario.rows(1);
            assert_eq!(
                named_row(&rows, "hermes.extension"),
                format!(
                    "error hermes.extension: {}: differs from the embedded copy; {HERMES_FIX}",
                    file.display()
                )
            );
            assert_eq!(
                named_row(&rows, "opencode.extension"),
                format!(
                    "error opencode.extension: {}: differs from the embedded copy; run 'cafleet setup --coding-agent opencode'",
                    plugin.display()
                )
            );
        }

        #[test]
        fn doctor_reports_a_tampered_pi_extension_on_its_row() {
            let (scenario, _conn) = healthy_scenario();
            let (extension, _) = scenario.seed_pi_extension();
            std::fs::write(&extension, "tampered").unwrap();
            let rows = scenario.rows(1);
            assert_eq!(
                named_row(&rows, "pi.extension"),
                format!(
                    "error pi.extension: {}: differs from the embedded copy; run 'cafleet setup --coding-agent pi'",
                    extension.display()
                )
            );
        }

        #[test]
        fn doctor_reports_an_unusable_pi_record_and_a_missing_installation_on_their_rows() {
            let mut scenario = Scenario::new(member_run_registry);
            let _conn = scenario.connect();
            scenario.executable("pi");
            let (_, record) = scenario.seed_pi_extension();
            std::fs::write(&record, "").unwrap();
            let rows = scenario.rows(1);
            assert_eq!(
                named_row(&rows, "pi.extension"),
                format!(
                    "error pi.extension: {}: the record is empty; run 'cafleet setup --coding-agent pi'",
                    record.display()
                )
            );
            assert_eq!(
                named_row(&rows, "pi.skills"),
                format!(
                    "warn pi.skills: skills not installed at {}; run 'cafleet setup --coding-agent pi'",
                    Path::new(&scenario.config(CodingAgent::Pi))
                        .join("skills")
                        .display()
                )
            );
        }

        #[test]
        fn doctor_prints_each_check_as_one_escaped_row_and_one_json_object() {
            let mut scenario = Scenario::new(member_run_registry);
            scenario.connect();
            healthy_backends(&mut scenario);
            scenario
                .environment
                .push(("CLAUDE_CONFIG_DIR", "rel\nat\rive".into()));
            let views = scenario.views();
            views.assert_exit(1);
            let checks = views.json.as_array().unwrap();
            for (name, status, detail, row) in [
                (
                    "multiplexer",
                    "ok",
                    format!("tmux server={TMUX_SOCKET}"),
                    format!("ok multiplexer: tmux server={TMUX_SOCKET}"),
                ),
                (
                    "multiplexer.caller_pane",
                    "ok",
                    "TMUX_PANE=%1 session=main window=@1".to_owned(),
                    "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1".to_owned(),
                ),
                (
                    "claude.configuration",
                    "error",
                    "CLAUDE_CONFIG_DIR must be an absolute path (got 'rel\nat\rive')".to_owned(),
                    r"error claude.configuration: CLAUDE_CONFIG_DIR must be an absolute path (got 'rel\nat\rive')".to_owned(),
                ),
                (
                    "hermes.plugin_enabled",
                    "ok",
                    "cafleet enabled with gateway injection".to_owned(),
                    "ok hermes.plugin_enabled: cafleet enabled with gateway injection".to_owned(),
                ),
            ] {
                assert_eq!(named_row(&views.text, name), row, "{name}");
                let entries = checks
                    .iter()
                    .filter(|entry| entry["name"] == name)
                    .collect::<Vec<_>>();
                assert_eq!(
                    entries,
                    [&json!({"name": name, "status": status, "detail": detail})],
                    "{name}"
                );
            }
        }

        #[test]
        fn doctor_reports_a_healthy_pi_backend_with_its_extension_path() {
            let (scenario, _conn) = healthy_scenario();
            let views = scenario.views();
            views.assert_exit(0);
            assert_eq!(
                named_row(&views.text, "pi.extension"),
                format!(
                    "ok pi.extension: {}",
                    scenario.path("pi").join("extensions/cafleet.ts").display()
                )
            );
        }
    }
}
