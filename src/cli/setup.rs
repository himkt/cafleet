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
use crate::runtime::control::CheckStatus;
use crate::runtime::control::MemberState;
use crate::runtime::system::SystemProbe;
use clap::Args;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
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
            install_agent(
                conn,
                *agent,
                &paths,
                crate::cli::VERSION,
                hermes,
                &mut std::io::stdout(),
            )?;
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

/// The detected multiplexer's name and the caller pane's context.
pub(crate) type MuxReading = Result<(&'static str, MultiplexerContext), CafleetError>;

/// Everything doctor reads from outside the process.
pub(crate) struct DoctorInputs<'a> {
    pub(crate) mux: &'a dyn Fn() -> MuxReading,
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
    let home = crate::config::home_dir();
    let hermes = hermes_reader(&SystemProbe, &home);
    let report = gather(&DoctorInputs {
        mux: &detect_mux,
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

fn detect_mux() -> MuxReading {
    let mux = crate::runtime::resolve_mux()?;
    mux.ensure_available()?;
    let context = mux.context_discovery()?;
    Ok((mux.name(), context))
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
        let (mut checks, mut summary) = multiplexer_rows((inputs.mux)());
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

fn multiplexer_rows(reading: MuxReading) -> (Vec<Check>, Vec<Summary>) {
    let (status, detail, short) = match reading {
        Ok((name, context)) => (
            CheckStatus::Ok,
            format!(
                "{name} session={} window={} pane={}",
                context.session, context.window_id, context.pane_id
            ),
            format!(
                "{name} (session {}, pane {})",
                context.session, context.pane_id
            ),
        ),
        Err(cause) => (CheckStatus::Error, cause.to_string(), cause.to_string()),
    };
    (
        vec![Check::new("multiplexer", status, detail)],
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

fn probe_run<'a>(
    source: Option<&str>,
    row: &'a MemberRunRegistry,
    deadline: Instant,
) -> MemberRun<'a> {
    let now = Instant::now();
    let (state, phrases) = if now >= deadline {
        (
            MemberState::NotChecked,
            vec!["runtime diagnosis time budget exhausted".to_owned()],
        )
    } else {
        let until = deadline.min(now + Duration::from_millis(100));
        let diagnosis = source
            .ok_or_else(|| {
                CafleetError::App("runtime diagnosis requires a file-backed registry".into())
            })
            .and_then(|source| crate::runtime::diagnose_runtime_at(source, row.run_id, until));
        match diagnosis {
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
    source: Option<&str>,
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
        conn.path(),
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
fn director_label(directors: &[Option<i64>]) -> String {
    match directors.split_first() {
        Some((Some(first), rest)) if rest.iter().all(|other| *other == Some(*first)) => {
            first.to_string()
        }
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
        Ok(Some((version, when))) if version == crate::cli::VERSION => Check::new(
            name,
            CheckStatus::Ok,
            format!("recorded {version} at {when}"),
        ),
        Ok(Some((version, when))) => Check::fixable(
            name,
            CheckStatus::Warn,
            format!("recorded {version} at {when}"),
            format!(
                "assets from cafleet {version} (this is {})",
                crate::cli::VERSION
            ),
        ),
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

    fn installed_agents(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT coding_agent FROM asset_installs ORDER BY coding_agent")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
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
        crate::db::classify_schema(&conn).require_current().unwrap();
        assert_eq!(installed_agents(&conn), ["claude"]);
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
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM asset_installs", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                1
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
            "opencode.preset",
            "opencode.preset.cafleet-ask",
            "opencode.installation",
            "claude.installation",
            "codex.installation",
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

    #[test]
    fn healthy_database_distinguishes_absent_stale_current_and_failed_install_queries() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let probe = probe(home.path(), true, None);
        let schema = SchemaState::Current;
        crate::broker::record_asset_install(
            &mut conn,
            "claude",
            &home.path().join(".claude").display().to_string(),
            crate::cli::VERSION,
        )
        .unwrap();
        crate::broker::record_asset_install(
            &mut conn,
            "codex",
            &home.path().join(".codex").display().to_string(),
            "old",
        )
        .unwrap();
        let checks = checks_for(Some(&conn), &schema, &probe, home.path());
        assert_eq!(find(&checks, "claude.installation").status, CheckStatus::Ok);
        assert_eq!(
            find(&checks, "codex.installation").status,
            CheckStatus::Warn
        );
        let base = home.path().join(".config/opencode");
        let configuration = find(&checks, "opencode.configuration");
        assert_eq!(configuration.status, CheckStatus::Ok);
        assert_eq!(configuration.detail, base.display().to_string());
        let installation = find(&checks, "opencode.installation");
        assert_eq!(installation.status, CheckStatus::Warn);
        assert_eq!(
            installation.detail,
            format!(
                "no installation recorded at {}; run 'cafleet setup --coding-agent opencode'",
                base.display()
            )
        );
        conn.execute(
            "UPDATE asset_installs SET cafleet_version=X'00' WHERE coding_agent='claude'",
            [],
        )
        .unwrap();
        assert_eq!(
            find(
                &checks_for(Some(&conn), &schema, &probe, home.path()),
                "claude.installation"
            )
            .status,
            CheckStatus::Error
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
                    "opencode.preset",
                    "opencode.preset.cafleet-ask",
                    "opencode.extension",
                    "opencode.installation"
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
            assert!(installed_agents(&conn).contains(&"hermes".to_owned()));
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
            assert_eq!(installed_agents(&conn), ["hermes"]);
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
            assert!(installed_agents(&conn).is_empty());
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
                assert_eq!(installed_agents(&conn), ["pi"]);
                let recorded: String = conn
                    .query_row(
                        "SELECT path FROM asset_installs WHERE coding_agent='pi'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(recorded, base.display().to_string());
            }
        }

        #[test]
        fn plain_setup_installs_pi_assets_under_the_default_base() {
            let home = tempfile::tempdir().unwrap();
            let settings = settings(home.path());
            let mut conn = prepare_database(&settings).unwrap();
            install_selected(&mut conn, &[], &|_| Ok(None), home.path(), None).unwrap();
            assert_installed_pi_assets(&pi_base(home.path()));
            assert!(installed_agents(&conn).contains(&"pi".to_owned()));
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
            assert!(installed_agents(&conn).is_empty());
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
                    "pi.installation"
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
        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::broker::runtime::CleanupState;
        use crate::broker::runtime::RunRecord;
        use crate::broker::runtime::RunStatus;
        use crate::broker::runtime::test_support::assert_key_order;
        use crate::broker::runtime::test_support::register_member_run;
        use crate::cli::setup::tests::*;
        use serde_json::Value;
        use serde_json::json;

        #[test]
        fn a_run_reached_after_the_time_budget_is_not_checked() {
            let dir = crate::runtime::test_support::short_tempdir();
            let mut conn = crate::db::test_support::migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "doctor-budget");
            let run = register_member_run(&mut conn, fleet, director, "unprobed");
            let rows = member_run_registry(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            let deadline = Instant::now();
            let probed = probe_run(conn.path(), &rows[0], deadline);
            assert_eq!(probed.state, MemberState::NotChecked);
            let check = runtime_check(&probed);
            assert_eq!(
                format!("{} {}: {}", check.status.as_str(), check.name, check.detail),
                format!(
                    "info member_runtime.{}: member {1} not checked (time limit): runtime diagnosis time budget exhausted — run 'cafleet member show {1}'",
                    run.run_id, run.member_id
                )
            );
            let summary = state_summary(probed.state, &[&probed]);
            assert_eq!(
                format!(
                    "{} {}: {}",
                    summary.status.as_str(),
                    summary.component,
                    summary.detail
                ),
                format!(
                    "info members: 1 not checked (time limit): {} — run 'cafleet member show ID'",
                    run.member_id
                )
            );
        }

        #[test]
        fn recover_hints_name_the_shared_fleet_director_or_keep_the_placeholder() {
            let row = |member_id, director_member_id| MemberRunRegistry {
                run_id: member_id,
                member_id,
                run_status: RunStatus::Running,
                cleanup_state: CleanupState::Incomplete,
                member_status: "active".into(),
                director_member_id,
            };
            fn lost(row: &MemberRunRegistry) -> MemberRun<'_> {
                MemberRun {
                    row,
                    state: MemberState::LostRuntime,
                    phrases: vec!["owner lock is free".into()],
                }
            }
            let same_fleet = [row(3, Some(1)), row(4, Some(1))];
            let same_fleet = [lost(&same_fleet[0]), lost(&same_fleet[1])];
            assert_eq!(
                runtime_check(&same_fleet[0]).detail,
                "member 3 lost runtime: owner lock is free — run 'cafleet member recover 3 --from-member-id 1' or 'cafleet member delete 3'"
            );
            assert_eq!(
                state_summary(MemberState::LostRuntime, &[&same_fleet[0], &same_fleet[1]]).detail,
                "2 lost runtime: 3, 4 — run 'cafleet member recover ID --from-member-id 1' or 'cafleet member delete ID'"
            );
            for spanning in [
                [row(3, Some(1)), row(9, Some(7))],
                [row(3, Some(1)), row(9, None)],
            ] {
                let runs = [lost(&spanning[0]), lost(&spanning[1])];
                assert_eq!(
                    state_summary(MemberState::LostRuntime, &[&runs[0], &runs[1]]).detail,
                    "2 lost runtime: 3, 9 — run 'cafleet member recover ID --from-member-id <director-id>' or 'cafleet member delete ID'"
                );
            }
        }

        #[test]
        fn id_ranges_collapse_runs_of_three_or_more_consecutive_ids() {
            for (ids, expected) in [
                (&[][..], ""),
                (&[7, 8], "7, 8"),
                (&[1, 2, 3], "1-3"),
                (&[1, 2, 4, 5], "1, 2, 4, 5"),
                (&[1, 2, 3, 7, 9], "1-3, 7, 9"),
            ] {
                assert_eq!(format_id_ranges(ids), expected, "{ids:?}");
            }
        }

        #[test]
        fn id_ranges_require_ascending_de_duplicated_ids() {
            for ids in [&[3, 3, 4][..], &[9, 1]] {
                let panic = std::panic::catch_unwind(|| format_id_ranges(ids))
                    .expect_err("unsorted or duplicated IDs violate the caller contract");
                assert_eq!(
                    panic.downcast_ref::<&str>(),
                    Some(&"callers pass ascending, de-duplicated IDs"),
                    "{ids:?}"
                );
            }
        }

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
        const REGISTRY_FAILURE: &str = "registry fixture failure";
        const PHANTOM_RUN: i64 = 999;
        const PHANTOM_MEMBER: i64 = 42;

        type Registry = fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>;

        fn failing_registry(_: &Connection) -> Result<Vec<MemberRunRegistry>, CafleetError> {
            Err(CafleetError::App(REGISTRY_FAILURE.into()))
        }

        fn phantom_registry(conn: &Connection) -> Result<Vec<MemberRunRegistry>, CafleetError> {
            let mut rows = member_run_registry(conn)?;
            rows.push(MemberRunRegistry {
                run_id: PHANTOM_RUN,
                member_id: PHANTOM_MEMBER,
                run_status: RunStatus::Running,
                cleanup_state: CleanupState::Incomplete,
                member_status: "active".into(),
                director_member_id: None,
            });
            Ok(rows)
        }

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

            fn opencode_extension(&self) -> (PathBuf, PathBuf) {
                install_opencode_extension(&self.path("opencode"))
            }

            fn pi_extension(&self) -> (PathBuf, PathBuf) {
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
                let mux = || -> MuxReading {
                    Ok((
                        "tmux",
                        MultiplexerContext {
                            session: "main".into(),
                            window_id: "@1".into(),
                            pane_id: "%1".into(),
                        },
                    ))
                };
                gather(&DoctorInputs {
                    mux: &mux,
                    settings: &settings,
                    probe: &probe,
                    home: &home,
                    hermes: Some(&self.hermes as &dyn HermesConfig),
                    registry: &self.registry,
                })
            }

            fn summary_and_rows(&self, code: i32) -> (Vec<String>, Vec<String>) {
                let views = self.views();
                views.assert_exit(code);
                (views.text, views.verbose)
            }

            fn views(&self) -> Views {
                let report = self.report();
                let lines = |mode| {
                    render(&report, mode)
                        .lines()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                let views = Views {
                    code: if report.has_errors() { 1 } else { 0 },
                    text: lines(Mode::Summary),
                    verbose: lines(Mode::Verbose),
                    json: serde_json::from_str(&render(&report, Mode::Json)).unwrap(),
                };
                assert_views_agree(&views);
                views
            }
        }

        struct Views {
            code: i32,
            text: Vec<String>,
            verbose: Vec<String>,
            json: Value,
        }

        impl Views {
            fn payload(&self) -> Value {
                self.json.clone()
            }

            fn verbose_row(&self, prefix: &str) -> &str {
                let rows = self
                    .verbose
                    .iter()
                    .filter(|line| line.starts_with(prefix))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 1, "{prefix}: {:?}", self.verbose);
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
            for line in &views.verbose {
                let name = row_name(line);
                assert!(
                    ["multiplexer", "database", "member_runtime"].contains(&name)
                        || name.contains('.'),
                    "--verbose prints only check rows: {line}"
                );
            }
            for line in &views.text {
                let name = row_name(line);
                assert!(
                    ["multiplexer", "database", "members"].contains(&name)
                        || crate::coding_agent::coding_agent(name).is_some(),
                    "default output prints only summary lines: {line}"
                );
            }
            let payload = &views.json;
            assert_key_order(payload, &["summary", "checks"]);
            let summary = payload["summary"].as_array().unwrap();
            for entry in summary {
                if entry["component"] == "members" {
                    assert_key_order(
                        entry,
                        &["component", "status", "detail", "state", "member_ids"],
                    );
                } else {
                    assert_key_order(entry, &["component", "status", "detail"]);
                }
            }
            assert_eq!(
                status_lines(summary, "component"),
                views.text,
                "summary holds exactly the text summary lines"
            );
            let checks = payload["checks"].as_array().unwrap();
            for entry in checks {
                if entry["name"]
                    .as_str()
                    .unwrap()
                    .starts_with("member_runtime")
                {
                    assert_key_order(entry, &["name", "status", "detail", "member_id", "state"]);
                } else {
                    assert_key_order(entry, &["name", "status", "detail"]);
                }
            }
            assert_eq!(
                status_lines(checks, "name"),
                views.verbose,
                "checks hold exactly the verbose rows"
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
            println!("{CHILD_STDOUT_MARKER}");
            match doctor(&settings, args) {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }

        struct Outcome {
            code: Option<i32>,
            stdout: String,
            stderr: String,
        }

        /// Runs the real `cafleet doctor` in a child whose `PATH` holds only `tmux` and the
        /// given stubs, with every backend's configuration under `home`.
        fn doctor_process(
            home: &Path,
            database: &Path,
            stubs: &[(&str, String)],
            args: &[&str],
        ) -> Outcome {
            use crate::runtime::system::test_support::TestChild;
            use std::process::Stdio;

            let bin = tempfile::tempdir().unwrap();
            crate::coding_agent::test_support::executable_stub(
                bin.path(),
                "tmux",
                "case \" $* \" in *' display-message '*) printf 'main|@1|%%1\\n' ;; esac",
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
                    serde_json::json!({"database": database, "args": args}).to_string(),
                )
                .env("PATH", bin.path())
                .env("HOME", home)
                .env("TMUX", "cafleet-test,1,0")
                .env("TMUX_PANE", "%1")
                .env_remove("HERDR_ENV")
                .env("RUST_BACKTRACE", "0");
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

        fn summary_lines(stdout: &str) -> Vec<String> {
            let payload: Value = serde_json::from_str(stdout.trim_end()).unwrap();
            status_lines(payload["summary"].as_array().unwrap(), "component")
        }

        #[test]
        fn doctor_process_reads_a_real_hermes_and_prints_json_for_a_healthy_home() {
            let home = crate::runtime::test_support::short_tempdir();
            let mut conn = crate::db::test_support::migrated_conn(&home);
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
                crate::broker::record_asset_install(
                    &mut conn,
                    agent.name(),
                    &home.path().join(agent.name()).display().to_string(),
                    crate::cli::VERSION,
                )
                .unwrap();
            }
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
            let outcome = doctor_process(home.path(), &database, &stubs, &["doctor", "--json"]);
            assert_eq!(outcome.code, Some(0), "{}", outcome.stderr);
            assert_eq!(outcome.stderr, "");
            assert_eq!(
                summary_lines(&outcome.stdout),
                [
                    "ok multiplexer: tmux (session main, pane %1)".to_owned(),
                    format!("ok database: cafleet {}", crate::cli::VERSION),
                    "ok claude: ready".into(),
                    "ok codex: ready".into(),
                    "ok opencode: ready".into(),
                    "ok hermes: ready".into(),
                    "ok pi: ready".into(),
                    "ok members: none running".into(),
                ]
            );
        }

        #[test]
        fn doctor_process_without_hermes_or_a_database_prints_json_and_fails() {
            let home = crate::runtime::test_support::short_tempdir();
            let database = home.path().join("absent.db");
            let outcome = doctor_process(
                home.path(),
                &database,
                &[],
                &["doctor", "--json", "--verbose"],
            );
            assert_eq!(outcome.code, Some(1));
            assert_eq!(outcome.stderr, "Error: doctor found errors\n");
            let lines = summary_lines(&outcome.stdout);
            assert!(lines[1].starts_with("error database: "), "{lines:?}");
            assert_eq!(
                lines[5],
                "info hermes: not installed (hermes is not on PATH)"
            );
            assert!(!database.exists(), "doctor creates no database");
        }

        fn members_entry<'a>(payload: &'a Value, state: &str) -> &'a Value {
            payload["summary"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["component"] == "members" && entry["state"] == state)
                .unwrap_or_else(|| panic!("members entry {state}: {payload}"))
        }

        fn runtime_entry(payload: &Value, run_id: i64) -> &Value {
            let name = format!("member_runtime.{run_id}");
            payload["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == name.as_str())
                .unwrap_or_else(|| panic!("{name}: {payload}"))
        }

        fn multiplexer_line() -> String {
            "ok multiplexer: tmux (session main, pane %1)".into()
        }

        fn database_line() -> String {
            format!("ok database: cafleet {}", crate::cli::VERSION)
        }

        fn record(conn: &mut Connection, scenario: &Scenario, agent: CodingAgent, version: &str) {
            crate::broker::record_asset_install(
                conn,
                agent.name(),
                &scenario.config(agent),
                version,
            )
            .unwrap();
        }

        fn healthy_backends(scenario: &mut Scenario, conn: &mut Connection) {
            for name in ["claude", "codex", "opencode", "pi"] {
                scenario.executable(name);
            }
            scenario.hermes_ready();
            scenario.opencode_presets();
            scenario.opencode_extension();
            scenario.pi_extension();
            for agent in [
                CodingAgent::Claude,
                CodingAgent::Opencode,
                CodingAgent::Hermes,
                CodingAgent::Pi,
            ] {
                record(conn, scenario, agent, crate::cli::VERSION);
            }
        }

        fn healthy_backend_lines(scenario: &Scenario) -> Vec<String> {
            vec![
                "ok claude: ready".into(),
                format!(
                    "warn codex: assets not installed at {} — run 'cafleet setup --coding-agent codex'",
                    scenario.config(CodingAgent::Codex)
                ),
                "ok opencode: ready".into(),
                "ok hermes: ready".into(),
                "ok pi: ready".into(),
            ]
        }

        #[test]
        fn doctor_with_only_warnings_prints_ready_backends_and_exits_zero() {
            let mut scenario = Scenario::new(member_run_registry);
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            let mut expected = vec![multiplexer_line(), database_line()];
            expected.extend(healthy_backend_lines(&scenario));
            expected.push("ok members: none running".into());
            assert_eq!(views.text, expected);
            views.assert_exit(0);
            assert_eq!(
                named_row(&views.verbose, "opencode.extension"),
                format!(
                    "ok opencode.extension: {}",
                    scenario
                        .path("opencode")
                        .join("plugins/cafleet.ts")
                        .display()
                )
            );
            assert_eq!(
                named_row(&views.verbose, "hermes.extension"),
                format!(
                    "ok hermes.extension: {}",
                    scenario.hermes_plugin_dir().display()
                )
            );
            assert_eq!(
                named_row(&views.verbose, "hermes.plugin_enabled"),
                "ok hermes.plugin_enabled: cafleet enabled with gateway injection"
            );
            assert_eq!(
                views.verbose_row("warn codex.installation: "),
                format!(
                    "warn codex.installation: no installation recorded at {}; run 'cafleet setup --coding-agent codex'",
                    scenario.config(CodingAgent::Codex)
                )
            );
            assert!(
                !views
                    .verbose
                    .iter()
                    .any(|line| line.contains("member_runtime"))
            );
            assert_eq!(
                views.payload()["summary"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap(),
                &serde_json::json!({
                    "component": "members",
                    "status": "ok",
                    "detail": "none running",
                    "state": "running",
                    "member_ids": [],
                })
            );
        }

        struct MemberRuns {
            exited: RunRecord,
            ended: RunRecord,
            leftover: RunRecord,
            interrupted: RunRecord,
            lost: Vec<RunRecord>,
            unresponsive: RunRecord,
        }

        fn member_runs(conn: &mut Connection) -> (MemberRuns, Vec<crate::runtime::RunArtifacts>) {
            let (fleet, director) = create_fleet_fixture(conn, "doctor-members");
            let mut run = |name: &str| register_member_run(conn, fleet, director, name);
            let exited = run("exited");
            let ended = run("ended");
            let leftover = run("leftover");
            let interrupted = run("interrupted");
            let lost_first = run("lost-first");
            let unresponsive = run("unresponsive");
            let lost_rest = [run("lost-a"), run("lost-b"), run("lost-c")];
            let ids = [
                director,
                exited.member_id,
                ended.member_id,
                leftover.member_id,
                interrupted.member_id,
                lost_first.member_id,
                unresponsive.member_id,
                lost_rest[0].member_id,
                lost_rest[1].member_id,
                lost_rest[2].member_id,
            ];
            assert!(ids.windows(2).all(|pair| pair[1] == pair[0] + 1), "{ids:?}");
            for (sql, run_id) in [
                (
                    "UPDATE member_runs SET status='exited', cleanup_state='complete' WHERE run_id=?1",
                    exited.run_id,
                ),
                (
                    "UPDATE member_runs SET status='lost', cleanup_state='complete' WHERE run_id=?1",
                    ended.run_id,
                ),
                (
                    "UPDATE members SET status='deregistered' WHERE member_id=(SELECT member_id FROM member_runs WHERE run_id=?1)",
                    ended.run_id,
                ),
                (
                    "UPDATE members SET status='deregistered' WHERE member_id=(SELECT member_id FROM member_runs WHERE run_id=?1)",
                    leftover.run_id,
                ),
            ] {
                conn.execute(sql, [run_id]).unwrap();
            }
            let mut lost = vec![lost_first];
            lost.extend(lost_rest);
            let mut artifacts = Vec::new();
            for run in lost.iter().chain(std::iter::once(&unresponsive)) {
                artifacts.push(crate::runtime::RunArtifacts::create(conn, run.run_id).unwrap());
            }
            let refresh =
                |run: &RunRecord| crate::broker::runtime::get_run(conn, run.run_id).unwrap();
            let runs = MemberRuns {
                exited: refresh(&exited),
                ended: refresh(&ended),
                leftover: refresh(&leftover),
                interrupted: refresh(&interrupted),
                lost: lost.iter().map(refresh).collect(),
                unresponsive: refresh(&unresponsive),
            };
            (runs, artifacts)
        }

        #[test]
        fn doctor_groups_members_by_state_and_joins_backend_problems() {
            let mut scenario = Scenario::new(member_run_registry);
            let mut conn = scenario.connect();
            for name in ["claude", "codex"] {
                scenario.executable(name);
            }
            install_hermes_plugin(&scenario.hermes_plugin_dir());
            scenario.hermes_ready();
            scenario.hermes.put(
                PLUGINS_KEY,
                json!({"entries": {"cafleet": {"allow_gateway_injection": true}}}),
            );
            record(
                &mut conn,
                &scenario,
                CodingAgent::Claude,
                crate::cli::VERSION,
            );
            record(&mut conn, &scenario, CodingAgent::Codex, "0.30.0");
            let (runs, _artifacts) = member_runs(&mut conn);
            let director = runs.exited.director_member_id;
            let _held = crate::runtime::RunLock::acquire(&conn, &runs.unresponsive.owner())
                .unwrap()
                .expect("the fixture owns the unresponsive run's lock");
            let views = scenario.views();
            let lost_ids = runs
                .lost
                .iter()
                .map(|run| run.member_id.to_string())
                .collect::<Vec<_>>();
            assert_eq!(
                views.text,
                [
                    multiplexer_line(),
                    database_line(),
                    "ok claude: ready".into(),
                    format!(
                        "warn codex: assets from cafleet 0.30.0 (this is {}) — run 'cafleet setup --coding-agent codex'",
                        crate::cli::VERSION
                    ),
                    "info opencode: not installed (opencode is not on PATH)".into(),
                    format!(
                        "error hermes: hermes plugin cafleet is not enabled: plugins.enabled; assets not installed at {} — run 'cafleet setup --coding-agent hermes'",
                        scenario.config(CodingAgent::Hermes)
                    ),
                    "info pi: not installed (pi is not on PATH)".into(),
                    format!(
                        "warn members: 1 not responding: {} — run 'cafleet member recover ID --from-member-id {director}'",
                        runs.unresponsive.member_id
                    ),
                    format!(
                        "warn members: 4 lost runtime: {}, {}-{} — run 'cafleet member recover ID --from-member-id {director}' or 'cafleet member delete ID'",
                        lost_ids[0], lost_ids[1], lost_ids[3]
                    ),
                    format!(
                        "warn members: 1 interrupted launch: {} — run 'cafleet member recover ID --from-member-id {director}'",
                        runs.interrupted.member_id
                    ),
                    format!(
                        "warn members: 1 agent exited: {} — run 'cafleet member delete ID'",
                        runs.exited.member_id
                    ),
                    format!(
                        "warn members: 1 deleted with leftover runtime: {} — run 'cafleet member recover ID --from-member-id {director}'",
                        runs.leftover.member_id
                    ),
                ]
            );
            views.assert_exit(1);
            assert!(
                views
                    .verbose_row("warn codex.installation: ")
                    .starts_with("warn codex.installation: recorded 0.30.0 at "),
            );
            assert_eq!(
                views.verbose_row("warn hermes.installation: "),
                format!(
                    "warn hermes.installation: no installation recorded at {}; run 'cafleet setup --coding-agent hermes'",
                    scenario.config(CodingAgent::Hermes)
                )
            );
            let row = |run: &RunRecord| format!("member_runtime.{}: ", run.run_id);
            let exited = &runs.exited;
            assert_eq!(
                views.verbose_row(&format!("warn {}", row(exited))),
                format!(
                    "warn {}member {} agent exited: run exited, cleanup complete — run 'cafleet member delete {}'",
                    row(exited),
                    exited.member_id,
                    exited.member_id
                )
            );
            let ended = &runs.ended;
            assert_eq!(
                views.verbose_row(&format!("info {}", row(ended))),
                format!(
                    "info {}member {} ended: run lost, cleanup complete",
                    row(ended),
                    ended.member_id
                )
            );
            let leftover = &runs.leftover;
            assert_eq!(
                views.verbose_row(&format!("warn {}", row(leftover))),
                format!(
                    "warn {}member {} deleted with leftover runtime: run starting, cleanup incomplete — run 'cafleet member recover {} --from-member-id {director}'",
                    row(leftover),
                    leftover.member_id,
                    leftover.member_id
                )
            );
            let interrupted = &runs.interrupted;
            let interrupted_row = views.verbose_row(&format!("warn {}", row(interrupted)));
            assert!(
                interrupted_row.starts_with(&format!(
                    "warn {}member {} interrupted launch: runtime files are invalid: ",
                    row(interrupted),
                    interrupted.member_id
                )) && interrupted_row.ends_with(&format!(
                    " — run 'cafleet member recover {} --from-member-id {director}'",
                    interrupted.member_id
                )),
                "{interrupted_row}"
            );
            for lost in &runs.lost {
                let lost_row = views.verbose_row(&format!("warn {}", row(lost)));
                assert!(
                    lost_row.starts_with(&format!(
                        "warn {}member {} lost runtime: owner lock is free; runtime did not answer the health probe: ",
                        row(lost),
                        lost.member_id
                    )) && lost_row.ends_with(&format!(
                        " — run 'cafleet member recover {0} --from-member-id {director}' or 'cafleet member delete {0}'",
                        lost.member_id
                    )),
                    "{lost_row}"
                );
            }
            let unresponsive = &runs.unresponsive;
            let unresponsive_row = views.verbose_row(&format!("warn {}", row(unresponsive)));
            assert!(
                unresponsive_row.starts_with(&format!(
                    "warn {}member {} not responding: owner lock is held; runtime did not answer the health probe: ",
                    row(unresponsive),
                    unresponsive.member_id
                )) && unresponsive_row.ends_with(&format!(
                    " — run 'cafleet member recover {} --from-member-id {director}'",
                    unresponsive.member_id
                )),
                "{unresponsive_row}"
            );
            let runtime_rows = views
                .verbose
                .iter()
                .filter(|line| row_name(line).starts_with("member_runtime."))
                .map(|line| {
                    row_name(line)
                        .trim_start_matches("member_runtime.")
                        .parse::<i64>()
                        .unwrap()
                })
                .collect::<Vec<_>>();
            let mut recorded = [exited, ended, leftover, interrupted, unresponsive]
                .into_iter()
                .chain(&runs.lost)
                .map(|run| run.run_id)
                .collect::<Vec<_>>();
            recorded.sort_unstable();
            assert_eq!(
                runtime_rows, recorded,
                "one runtime row per recorded run, in run order"
            );
            let payload = views.payload();
            let lost_entry = members_entry(&payload, "lost_runtime");
            assert_eq!(lost_entry["status"], "warn");
            assert_eq!(
                lost_entry["member_ids"],
                serde_json::json!(
                    runs.lost
                        .iter()
                        .map(|run| run.member_id)
                        .collect::<Vec<_>>()
                )
            );
            for (state, run) in [
                ("not_responding", unresponsive),
                ("interrupted_launch", interrupted),
                ("agent_exited", exited),
                ("deleted_with_leftover_runtime", leftover),
            ] {
                assert_eq!(
                    members_entry(&payload, state)["member_ids"],
                    serde_json::json!([run.member_id]),
                    "{state}"
                );
            }
            for (run, state, status) in [
                (exited, "agent_exited", "warn"),
                (ended, "ended", "info"),
                (leftover, "deleted_with_leftover_runtime", "warn"),
                (interrupted, "interrupted_launch", "warn"),
                (unresponsive, "not_responding", "warn"),
                (&runs.lost[0], "lost_runtime", "warn"),
            ] {
                let entry = runtime_entry(&payload, run.run_id);
                assert_eq!(entry["member_id"], run.member_id, "{state}");
                assert_eq!(entry["state"], state);
                assert_eq!(entry["status"], status, "{state}");
            }
        }

        #[test]
        fn doctor_reports_configuration_and_preset_errors_and_hides_ended_runs() {
            let mut scenario = Scenario::new(member_run_registry);
            let mut conn = scenario.connect();
            for name in ["claude", "opencode"] {
                scenario.executable(name);
            }
            scenario
                .environment
                .push(("CLAUDE_CONFIG_DIR", "relative".into()));
            scenario.opencode_extension();
            let (fleet, director) = create_fleet_fixture(&mut conn, "doctor-ended");
            let ended = register_member_run(&mut conn, fleet, director, "ended");
            conn.execute(
                "UPDATE member_runs SET status='exited', cleanup_state='complete' WHERE run_id=?1",
                [ended.run_id],
            )
            .unwrap();
            conn.execute(
                "UPDATE members SET status='deregistered' WHERE member_id=?1",
                [ended.member_id],
            )
            .unwrap();
            let views = scenario.views();
            let agents = scenario.path("opencode").join("agents");
            assert_eq!(
                views.text,
                [
                    multiplexer_line(),
                    database_line(),
                    "error claude: CLAUDE_CONFIG_DIR must be an absolute path (got 'relative')"
                        .into(),
                    "info codex: not installed (codex is not on PATH)".into(),
                    format!(
                        "error opencode: preset {} is unreadable: No such file or directory (os error 2); preset {} is unreadable: No such file or directory (os error 2); assets not installed at {} — run 'cafleet setup --coding-agent opencode'",
                        agents.join("cafleet.md").display(),
                        agents.join("cafleet-ask.md").display(),
                        scenario.config(CodingAgent::Opencode)
                    ),
                    "info hermes: not installed (hermes is not on PATH)".into(),
                    "info pi: not installed (pi is not on PATH)".into(),
                    "ok members: none running".into(),
                ]
            );
            views.assert_exit(1);
            assert_eq!(
                views.verbose_row("info member_runtime."),
                format!(
                    "info member_runtime.{}: member {} ended: run exited, cleanup complete",
                    ended.run_id, ended.member_id
                )
            );
            let payload = views.payload();
            let entry = runtime_entry(&payload, ended.run_id);
            assert_eq!(entry["state"], "ended");
            assert_eq!(entry["member_id"], ended.member_id);
        }

        #[test]
        fn doctor_without_a_database_marks_installations_and_members_unchecked() {
            let mut scenario = Scenario::new(member_run_registry);
            scenario.executable("claude");
            let views = scenario.views();
            let lines = &views.text;
            assert_eq!(lines.len(), 8, "{lines:?}");
            assert_eq!(lines[0], multiplexer_line());
            assert!(lines[1].starts_with("error database: "), "{}", lines[1]);
            assert_eq!(
                lines[2..],
                [
                    "info claude: assets unchecked (database unavailable)",
                    "info codex: not installed (codex is not on PATH)",
                    "info opencode: not installed (opencode is not on PATH)",
                    "info hermes: not installed (hermes is not on PATH)",
                    "info pi: not installed (pi is not on PATH)",
                    "info members: not checked (database unavailable)",
                ]
            );
            views.assert_exit(1);
            assert!(!scenario.database.exists(), "doctor creates no database");
            assert_eq!(
                members_entry(&views.payload(), "not_checked"),
                &serde_json::json!({
                    "component": "members",
                    "status": "info",
                    "detail": "not checked (database unavailable)",
                    "state": "not_checked",
                    "member_ids": [],
                })
            );
        }

        #[test]
        fn doctor_reports_a_failed_registry_read_and_still_prints_every_other_check() {
            let mut scenario = Scenario::new(failing_registry);
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            let mut expected = vec![multiplexer_line(), database_line()];
            expected.extend(healthy_backend_lines(&scenario));
            expected.push(format!(
                "error members: could not be checked: {REGISTRY_FAILURE}"
            ));
            assert_eq!(views.text, expected);
            views.assert_exit(1);
            assert_eq!(
                views.verbose.last().unwrap(),
                &format!(
                    "error member_runtime: member runtimes could not be read: {REGISTRY_FAILURE}"
                )
            );
            assert!(
                views
                    .verbose
                    .iter()
                    .any(|line| line.starts_with("ok hermes.plugin_enabled: "))
            );
            let payload = views.payload();
            assert_eq!(
                payload["summary"].as_array().unwrap().last().unwrap(),
                &serde_json::json!({
                    "component": "members",
                    "status": "error",
                    "detail": format!("could not be checked: {REGISTRY_FAILURE}"),
                    "state": "check_failed",
                    "member_ids": [],
                })
            );
            assert_eq!(
                payload["checks"].as_array().unwrap().last().unwrap(),
                &serde_json::json!({
                    "name": "member_runtime",
                    "status": "error",
                    "detail": format!("member runtimes could not be read: {REGISTRY_FAILURE}"),
                    "member_id": null,
                    "state": "check_failed",
                })
            );
        }

        #[test]
        fn doctor_reports_a_run_whose_diagnosis_fails_as_could_not_be_checked() {
            let mut scenario = Scenario::new(phantom_registry);
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            assert_eq!(
                views.text.last().unwrap(),
                &format!(
                    "error members: 1 could not be checked: {PHANTOM_MEMBER} — run 'cafleet doctor --verbose'"
                )
            );
            views.assert_exit(1);
            let prefix = format!(
                "error member_runtime.{PHANTOM_RUN}: member {PHANTOM_MEMBER} could not be checked: "
            );
            let row = views.verbose_row(&prefix);
            let cause = row.strip_prefix(&prefix).unwrap();
            assert!(!cause.is_empty(), "{row}");
            assert!(!row.contains(" — run "), "{row}");
            let payload = views.payload();
            let entry = runtime_entry(&payload, PHANTOM_RUN);
            assert_eq!(entry["member_id"], PHANTOM_MEMBER);
            assert_eq!(entry["state"], "check_failed");
            assert_eq!(entry["status"], "error");
            assert_eq!(
                members_entry(&payload, "check_failed")["member_ids"],
                serde_json::json!([PHANTOM_MEMBER])
            );
        }

        fn healthy_scenario() -> (Scenario, Connection) {
            let mut scenario = Scenario::new(member_run_registry);
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            (scenario, conn)
        }

        #[test]
        fn doctor_reports_an_unreadable_hermes_config_path_on_its_row_and_summary() {
            let (mut scenario, _conn) = healthy_scenario();
            scenario.hermes.reply(
                FakeHermes::path_call(),
                Reply::Fail("no configuration found".into()),
            );
            let (summary, rows) = scenario.summary_and_rows(1);
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
                named_row(&summary, "hermes"),
                format!("error hermes: {detail}"),
                "an unreadable config path has no setup fix"
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
        fn doctor_reports_each_hermes_plugin_enablement_state_on_its_row_and_summary() {
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
                let (summary, rows) = scenario.summary_and_rows(code);
                let row = named_row(&rows, "hermes.plugin_enabled");
                let hermes = named_row(&summary, "hermes");
                match expected {
                    Enablement::Ready => {
                        assert_eq!(
                            row,
                            "ok hermes.plugin_enabled: cafleet enabled with gateway injection"
                        );
                        assert_eq!(hermes, "ok hermes: ready");
                    }
                    Enablement::Disabled => {
                        assert_eq!(
                            row,
                            format!(
                                "error hermes.plugin_enabled: hermes plugins.disabled lists cafleet; remove it, then {HERMES_FIX}"
                            )
                        );
                        assert_eq!(
                            hermes, "error hermes: hermes plugins.disabled lists cafleet",
                            "the operator's disable has no setup fix"
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
                        assert_eq!(
                            hermes,
                            format!("error hermes: {} — {HERMES_FIX}", not_enabled(&keys))
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
                        assert_eq!(hermes, format!("error hermes: {detail}"));
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
            let (summary, rows) = scenario.summary_and_rows(0);
            for name in ["hermes.extension", "hermes.plugin_enabled"] {
                assert_eq!(
                    named_row(&rows, name),
                    format!(
                        "info {name}: unavailable: no Hermes home at {}",
                        base.display()
                    )
                );
            }
            assert_eq!(
                named_row(&summary, "hermes"),
                format!(
                    "info hermes: no Hermes home at {} — {HERMES_FIX}",
                    base.display()
                )
            );
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
        fn the_probe_pass_keeps_registry_order_across_more_runs_than_probe_workers() {
            let dir = crate::runtime::test_support::short_tempdir();
            let mut conn = crate::db::test_support::migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "doctor-probes");
            let runs = (0..PROBE_WORKERS + 4)
                .map(|index| register_member_run(&mut conn, fleet, director, &format!("w{index}")))
                .collect::<Vec<_>>();
            let rows = member_run_registry(&conn).unwrap();
            let rows = rows.iter().collect::<Vec<_>>();
            let probed = probe_runs(conn.path(), &rows, Instant::now() + Duration::from_secs(5));
            assert_eq!(
                probed.iter().map(|run| run.row.run_id).collect::<Vec<_>>(),
                runs.iter().map(|run| run.run_id).collect::<Vec<_>>()
            );
            assert!(
                probed
                    .iter()
                    .all(|run| run.state != MemberState::NotChecked),
                "every run is probed within the pass budget"
            );
            let unbacked = probe_runs(None, &rows[..1], Instant::now() + Duration::from_secs(5));
            assert_eq!(unbacked[0].state, MemberState::CheckFailed);
            assert_eq!(
                unbacked[0].phrases,
                ["runtime diagnosis requires a file-backed registry"]
            );
        }

        #[test]
        fn doctor_folds_every_hermes_and_opencode_extension_problem_into_the_summary() {
            let (mut scenario, _conn) = healthy_scenario();
            let file = install_hermes_plugin(&scenario.hermes_plugin_dir())
                .into_iter()
                .next()
                .unwrap();
            std::fs::write(&file, "tampered").unwrap();
            scenario.hermes.put(
                PLUGINS_KEY,
                json!({"entries": {"cafleet": {"allow_gateway_injection": true}}}),
            );
            let (plugin, _) = scenario.opencode_extension();
            std::fs::write(&plugin, "tampered").unwrap();
            let (summary, _) = scenario.summary_and_rows(1);
            assert_eq!(
                named_row(&summary, "hermes"),
                format!(
                    "error hermes: extension {} is missing or differs from the embedded copy; hermes plugin cafleet is not enabled: plugins.enabled — {HERMES_FIX}",
                    file.display()
                )
            );
            assert_eq!(
                named_row(&summary, "opencode"),
                format!(
                    "error opencode: extension {} is missing or differs from the embedded copy — run 'cafleet setup --coding-agent opencode'",
                    plugin.display()
                )
            );
        }

        #[test]
        fn doctor_folds_a_tampered_pi_extension_into_the_pi_summary() {
            let (scenario, _conn) = healthy_scenario();
            let (extension, _) = scenario.pi_extension();
            std::fs::write(&extension, "tampered").unwrap();
            let (summary, rows) = scenario.summary_and_rows(1);
            assert_eq!(
                named_row(&summary, "pi"),
                format!(
                    "error pi: extension {} is missing or differs from the embedded copy — run 'cafleet setup --coding-agent pi'",
                    extension.display()
                )
            );
            assert_eq!(
                named_row(&rows, "pi.extension"),
                format!(
                    "error pi.extension: {}: differs from the embedded copy; run 'cafleet setup --coding-agent pi'",
                    extension.display()
                )
            );
        }

        #[test]
        fn doctor_folds_an_unusable_pi_record_and_a_missing_installation_into_one_summary() {
            let mut scenario = Scenario::new(member_run_registry);
            let _conn = scenario.connect();
            scenario.executable("pi");
            let (_, record) = scenario.pi_extension();
            std::fs::write(&record, "").unwrap();
            let (summary, _) = scenario.summary_and_rows(1);
            assert_eq!(
                named_row(&summary, "pi"),
                format!(
                    "error pi: extension executable record {} is unusable: the record is empty; assets not installed at {} — run 'cafleet setup --coding-agent pi'",
                    record.display(),
                    scenario.config(CodingAgent::Pi)
                )
            );
        }

        #[test]
        fn doctor_reports_a_healthy_pi_backend_with_its_extension_path() {
            let (scenario, _conn) = healthy_scenario();
            let views = scenario.views();
            views.assert_exit(0);
            assert_eq!(named_row(&views.text, "pi"), "ok pi: ready");
            assert_eq!(
                named_row(&views.verbose, "pi.extension"),
                format!(
                    "ok pi.extension: {}",
                    scenario.path("pi").join("extensions/cafleet.ts").display()
                )
            );
        }
    }
}
