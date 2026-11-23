use crate::assets::HermesSetup;
use crate::assets::agent_paths;
use crate::assets::install_agent;
use crate::broker::runtime::RunRecord;
use crate::cli::member::ObservationProbe;
use crate::cli::member::probe_observation;
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
use crate::runtime::system::SystemProbe;
use crate::runtime::system::SystemRunner;
use chrono::DateTime;
use chrono::Utc;
use clap::Args;
use serde::Serialize;
use serde_json::Value;
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
    prepare_database(settings)?;
    let home = crate::config::home_dir()?;
    let hermes = SystemProbe.resolve_binary(CodingAgent::Hermes.name());
    install_selected(
        &args.coding_agent,
        &crate::config::process_env,
        &home,
        hermes.as_deref(),
    )
}

fn prepare_database(settings: &Settings) -> Result<(), CafleetError> {
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
            crate::db::initialize(&mut conn, &settings.database_url)?;
            crate::db::classify_selected_schema(&conn, &settings.database_url).require_current()?;
            crate::cli::write_output(&format!("Initialized database at {}.", db_file.display()))?;
        }
        SchemaState::Current => crate::cli::write_output(&format!(
            "Database already initialized at {}.",
            db_file.display()
        ))?,
        _ => schema.require_current()?,
    }
    Ok(())
}

fn install_selected(
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
            install_agent(*agent, &paths, hermes, &mut std::io::stdout())?;
        }
    }
    Ok(())
}

#[derive(Args)]
pub(crate) struct DoctorArgs {
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
}

fn status_text<S: serde::Serializer>(status: &CheckStatus, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(status.as_str())
}

impl Check {
    fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }

    /// A row that rerunning setup for its backend (the row's name prefix) fixes.
    fn fixable(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
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
        Self::new(name, status, detail)
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
}

pub(crate) struct Report {
    checks: Vec<Check>,
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
    Text,
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
    });
    let mode = if args.json { Mode::Json } else { Mode::Text };
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

/// Runs every check: the Hermes batch starts first and overlaps the multiplexer, database and
/// OpenCode checks, and the member probes overlap the backend checks; rows keep their order.
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
        let mut checks = multiplexer_rows((inputs.caller)());
        let (database_check, runs) = database(inputs.settings);
        checks.push(database_check);
        let probes: Vec<_> = runs
            .into_iter()
            .map(|run| {
                scope.spawn(move || {
                    let probe = probe_observation(&run, &crate::time::SystemClock);
                    (run, probe)
                })
            })
            .collect();
        let hermes_reading = || batch.map(crate::coding_agent::hermes::joined);
        let agents = agent_checks(&backends, hermes_reading);
        let now = Utc::now();
        let database_url = inputs
            .settings
            .database_url_explicit
            .then_some(inputs.settings.database_url.as_str());
        checks.extend(probes.into_iter().map(|probe| {
            let (run, probe) = probe.join().expect("a member probe thread completes");
            member_runtime_check(run.member_id, &run, &probe, now, database_url)
        }));
        checks.extend(agents);
        Report { checks }
    })
}

const HERMES_DOCTOR_BUDGET: Duration = Duration::from_secs(2);

pub(crate) fn render(report: &Report, mode: Mode) -> String {
    match mode {
        Mode::Json => serde_json::to_string(&report.checks).expect("doctor checks serialize"),
        Mode::Text => report
            .checks
            .iter()
            .map(|check| {
                format!(
                    "{} {}: {}",
                    check.status.as_str(),
                    check.name,
                    escape(&check.detail)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The `multiplexer` and `multiplexer.caller_pane` checks.
fn multiplexer_rows(reading: CallerReading) -> Vec<Check> {
    let ((mux_status, mux_detail), (pane_status, pane_detail)) = match reading {
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
            (CheckStatus::Warn, detail),
        ),
        CallerReading::Unavailable(cause) => (
            (CheckStatus::Error, cause.to_string()),
            (
                CheckStatus::Info,
                "unavailable: the multiplexer is unavailable".to_owned(),
            ),
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
                ),
                Ok(PaneReading::Unverified { detail }) => (mux, (CheckStatus::Warn, detail)),
                Err(cause) => (mux, (CheckStatus::Error, cause.to_string())),
            }
        }
    };
    vec![
        Check::new("multiplexer", mux_status, mux_detail),
        Check::new("multiplexer.caller_pane", pane_status, pane_detail),
    ]
}

fn database_row(settings: &Settings, schema: &SchemaState) -> Check {
    let (status, detail) = match schema {
        SchemaState::Current => {
            let path = crate::db::database_path(&settings.database_url)
                .expect("a current schema was opened through a valid database URL");
            (
                CheckStatus::Ok,
                format!("cafleet {} at {path}", crate::cli::VERSION),
            )
        }
        schema => (
            CheckStatus::Error,
            schema
                .require_current()
                .expect_err("a schema that is not current is unavailable")
                .to_string(),
        ),
    };
    Check::new("database", status, detail)
}

fn escape(detail: &str) -> String {
    detail.replace('\n', "\\n").replace('\r', "\\r")
}

/// The `database` row, and the runs that get member rows when that row is `ok`.
fn database(settings: &Settings) -> (Check, Vec<RunRecord>) {
    let conn = match crate::db::open_existing(&settings.database_url, true) {
        Ok(Some(conn)) => conn,
        Ok(None) => return (database_row(settings, &SchemaState::Missing), Vec::new()),
        Err(cause) => {
            return (
                database_row(settings, &SchemaState::Unreachable { cause }),
                Vec::new(),
            );
        }
    };
    let check = database_row(
        settings,
        &db::classify_selected_schema(&conn, &settings.database_url),
    );
    if check.status != CheckStatus::Ok {
        return (check, Vec::new());
    }
    match crate::broker::runtime::live_member_runs(&conn) {
        Ok(runs) => (check, runs),
        Err(cause) => (
            Check::new("database", CheckStatus::Error, cause.to_string()),
            Vec::new(),
        ),
    }
}

pub(crate) const DOCTOR_GAP_ERROR_AGE: Duration = Duration::from_secs(30);

pub(crate) fn member_runtime_check(
    member_id: i64,
    run: &RunRecord,
    probe: &ObservationProbe,
    now: DateTime<Utc>,
    database_url: Option<&str>,
) -> Check {
    let run_id = run.run_id;
    let hint = || recovery_hint(member_id, database_url);
    let (status, detail) = match probe {
        ObservationProbe::Skipped => (
            CheckStatus::Info,
            format!("run={run_id} status={}: not probed", run.status.as_str()),
        ),
        ObservationProbe::Failed(cause) => (
            CheckStatus::Error,
            format!("run={run_id} health probe failed: {cause}; {}", hint()),
        ),
        ObservationProbe::Answered(None) => (
            CheckStatus::Info,
            format!("run={run_id} observation=pending"),
        ),
        ObservationProbe::Answered(Some(health)) => {
            let observation = format!(
                "run={run_id} observation={}",
                health.eligibility.rendering()
            );
            if health.degraded {
                let degraded = format!(
                    "{observation} diagnostic={}",
                    health
                        .diagnostic
                        .as_deref()
                        .expect("validated degraded health names its diagnostic")
                );
                match health
                    .gap_open_since
                    .as_deref()
                    .filter(|since| gap_age(since, now) > DOCTOR_GAP_ERROR_AGE)
                {
                    Some(since) => (
                        CheckStatus::Error,
                        format!("{degraded} gap_open_since={since}; {}", hint()),
                    ),
                    None => (CheckStatus::Warn, degraded),
                }
            } else {
                (CheckStatus::Ok, observation)
            }
        }
    };
    Check::new(format!("member.{member_id}.runtime"), status, detail)
}

fn gap_age(since: &str, now: DateTime<Utc>) -> Duration {
    let since =
        crate::time::parse_lenient(since).expect("validated health carries an RFC 3339 gap start");
    (now - since).to_std().unwrap_or(Duration::ZERO)
}

fn recovery_hint(member_id: i64, database_url: Option<&str>) -> String {
    let suffix = database_url.map_or_else(String::new, |url| format!(" --database-url {url}"));
    format!(
        "inspect with 'cafleet member show {member_id} --json{suffix}'; if it does not recover, run 'cafleet member delete {member_id}{suffix}' and create a replacement member"
    )
}

/// One backend's configuration base, skills directory and executable, each resolved once.
struct Backend {
    agent: CodingAgent,
    paths: Result<PathBuf, CafleetError>,
    skills: Result<PathBuf, CafleetError>,
    executable: Option<PathBuf>,
}

fn resolve_backends(probe: &dyn SpawnProbe, home: &Result<PathBuf, CafleetError>) -> Vec<Backend> {
    CodingAgent::all()
        .iter()
        .map(|agent| {
            let paths = home
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|home| agent.config_dir(&|name| probe.env_var(name), home));
            let skills = match (&paths, home) {
                (Ok(base), Ok(home)) => Ok(crate::assets::skills_dir(*agent, base, home)),
                (Err(cause), _) | (_, Err(cause)) => Err(cause.clone()),
            };
            Backend {
                agent: *agent,
                paths,
                skills,
                executable: probe.resolve_binary(agent.name()),
            }
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
    })
}

fn agent_checks(
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
        checks.push(skills_check(name, &backend.skills, available));
    }
    checks
}

/// Compares every embedded skill file with its installed copy under the backend's skills dir.
fn skills_check(name: &str, skills: &Result<PathBuf, CafleetError>, available: bool) -> Check {
    let check_name = format!("{name}.skills");
    if let Some(row) = unavailable(&check_name, skills, available) {
        return row;
    }
    let dir = skills
        .as_ref()
        .expect("an available backend has a skills dir");
    let files = crate::assets::skill_files(dir);
    let absent = |target: &Path| {
        std::fs::symlink_metadata(target)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    };
    if files.iter().all(|(target, _)| absent(target)) {
        return Check::fixable(
            check_name,
            CheckStatus::Warn,
            format!("skills not installed at {}", dir.display()),
        );
    }
    for (target, embedded) in &files {
        if let Some(cause) = crate::assets::installed_file_problem(target, embedded) {
            return Check::fixable(
                check_name,
                CheckStatus::Error,
                format!("{}: {cause}", target.display()),
            );
        }
    }
    Check::new(check_name, CheckStatus::Ok, dir.display().to_string())
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
        if let Some(cause) = crate::assets::installed_file_problem(target, embedded) {
            return Check::fixable(
                name,
                CheckStatus::Error,
                format!("{}: {cause}", target.display()),
            );
        }
    }
    if let Some(cause) = crate::assets::executable_record_problem(record) {
        return Check::fixable(
            name,
            CheckStatus::Error,
            format!("{}: {cause}", record.display()),
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
        Err(cause) => Check::new(
            NAME,
            CheckStatus::Error,
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
            return Check::new(
                NAME,
                CheckStatus::Error,
                format!("cannot read hermes configuration through 'hermes config get': {cause}"),
            );
        }
    };
    if state.disabled_lists_cafleet {
        return Check::new(
            NAME,
            CheckStatus::Error,
            format!(
                "hermes {PLUGINS_DISABLED_KEY} lists {PLUGIN_NAME}; remove it, then run 'cafleet setup --coding-agent hermes'"
            ),
        );
    }
    let missing = state.missing();
    if missing.is_empty() {
        return Check::new(
            NAME,
            CheckStatus::Ok,
            "cafleet enabled with gateway injection",
        );
    }
    Check::fixable(
        NAME,
        CheckStatus::Error,
        format!(
            "hermes plugin {PLUGIN_NAME} is not enabled: {}",
            missing.join(", ")
        ),
    )
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

    mod member_runtime_rows {
        use crate::broker::runtime::RunRecord;
        use crate::broker::runtime::test_support::Fixture;
        use crate::cli::member::ObservationProbe;
        use crate::cli::setup::tests::*;
        use crate::runtime::observation::GateReason;
        use crate::runtime::observation::InputEligibility;
        use crate::runtime::observation::ObservationHealth;
        use crate::runtime::observation::Visibility;

        const MEMBER: i64 = 7;
        const NOW: &str = "2026-11-21T12:05:00.000000+00:00";

        fn now() -> chrono::DateTime<chrono::Utc> {
            crate::time::parse_lenient(NOW).unwrap()
        }

        fn ago(seconds: i64) -> String {
            crate::time::format_utc(now() - chrono::Duration::seconds(seconds))
        }

        fn run(status: &str) -> RunRecord {
            let fixture = Fixture::new();
            fixture
                .conn
                .execute(
                    "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                    rusqlite::params![status, fixture.run.run_id],
                )
                .unwrap();
            crate::broker::runtime::get_run(&fixture.conn, fixture.run.run_id).unwrap()
        }

        fn health(
            eligibility: InputEligibility,
            degraded: bool,
            gap_open_since: Option<String>,
        ) -> ObservationHealth {
            ObservationHealth {
                visibility: if degraded {
                    Visibility::Unknown
                } else {
                    Visibility::NotVisible
                },
                eligibility,
                pending_count: 0,
                gap_count: u64::from(degraded),
                degraded,
                diagnostic: degraded.then(|| "sink_unavailable".to_owned()),
                writer_diagnostic: None,
                gap_open_since,
            }
        }

        fn hint(database: Option<&str>) -> String {
            let suffix = database.map_or(String::new(), |url| format!(" --database-url {url}"));
            format!(
                "inspect with 'cafleet member show {MEMBER} --json{suffix}'; if it does not recover, run 'cafleet member delete {MEMBER}{suffix}' and create a replacement member"
            )
        }

        #[test]
        fn doctor_gap_error_age_is_30_seconds() {
            assert_eq!(DOCTOR_GAP_ERROR_AGE, Duration::from_secs(30));
        }

        #[test]
        fn member_runtime_check_applies_the_first_matching_situation() {
            const GAP: InputEligibility = InputEligibility::Hold(GateReason::Gap);
            type Expect = fn(i64) -> String;
            let cases: Vec<(&str, &str, ObservationProbe, CheckStatus, Expect)> = vec![
                (
                    "a starting run is not probed",
                    "starting",
                    ObservationProbe::Skipped,
                    CheckStatus::Info,
                    |run| format!("run={run} status=starting: not probed"),
                ),
                (
                    "a prepared run is not probed",
                    "prepared",
                    ObservationProbe::Skipped,
                    CheckStatus::Info,
                    |run| format!("run={run} status=prepared: not probed"),
                ),
                (
                    "a recovering run is not probed",
                    "recovering",
                    ObservationProbe::Skipped,
                    CheckStatus::Info,
                    |run| format!("run={run} status=recovering: not probed"),
                ),
                (
                    "a failed probe",
                    "running",
                    ObservationProbe::Failed("cannot connect to the runner".into()),
                    CheckStatus::Error,
                    |run| {
                        format!(
                            "run={run} health probe failed: cannot connect to the runner; {}",
                            hint(None)
                        )
                    },
                ),
                (
                    "an answer before the child started",
                    "running",
                    ObservationProbe::Answered(None),
                    CheckStatus::Info,
                    |run| format!("run={run} observation=pending"),
                ),
                (
                    "a gap open for more than 30 s",
                    "running",
                    ObservationProbe::Answered(Some(health(GAP, true, Some(ago(31))))),
                    CheckStatus::Error,
                    |run| {
                        format!(
                            "run={run} observation=hold:gap diagnostic=sink_unavailable gap_open_since={}; {}",
                            ago(31),
                            hint(None)
                        )
                    },
                ),
                (
                    "a gap open for exactly 30 s",
                    "degraded",
                    ObservationProbe::Answered(Some(health(GAP, true, Some(ago(30))))),
                    CheckStatus::Warn,
                    |run| format!("run={run} observation=hold:gap diagnostic=sink_unavailable"),
                ),
                (
                    "a younger gap",
                    "running",
                    ObservationProbe::Answered(Some(health(GAP, true, Some(ago(5))))),
                    CheckStatus::Warn,
                    |run| format!("run={run} observation=hold:gap diagnostic=sink_unavailable"),
                ),
                (
                    "a terminal-fenced gap with no gap start",
                    "running",
                    ObservationProbe::Answered(Some(health(GAP, true, None))),
                    CheckStatus::Warn,
                    |run| format!("run={run} observation=hold:gap diagnostic=sink_unavailable"),
                ),
                (
                    "an empty composer",
                    "running",
                    ObservationProbe::Answered(Some(health(
                        InputEligibility::ComposerEmpty,
                        false,
                        None,
                    ))),
                    CheckStatus::Ok,
                    |run| format!("run={run} observation=composer_empty"),
                ),
                (
                    "an undegraded hold",
                    "running",
                    ObservationProbe::Answered(Some(health(
                        InputEligibility::Hold(GateReason::Interaction),
                        false,
                        None,
                    ))),
                    CheckStatus::Ok,
                    |run| format!("run={run} observation=hold:interaction"),
                ),
            ];
            for (name, status, probe, expected_status, detail) in cases {
                let run = run(status);
                let check = member_runtime_check(MEMBER, &run, &probe, now(), None);
                assert_eq!(check.name, format!("member.{MEMBER}.runtime"), "{name}");
                assert_eq!(check.status, expected_status, "{name}");
                assert_eq!(check.detail, detail(run.run_id), "{name}");
            }
        }

        #[test]
        fn member_runtime_check_names_a_non_default_database_in_every_hint_command() {
            const URL: &str = "sqlite:///tmp/cafleet-doctor/registry.db";
            for probe in [
                ObservationProbe::Failed("cannot connect to the runner".into()),
                ObservationProbe::Answered(Some(health(
                    InputEligibility::Hold(GateReason::Gap),
                    true,
                    Some(ago(31)),
                ))),
            ] {
                let run = run("running");
                let check = member_runtime_check(MEMBER, &run, &probe, now(), Some(URL));
                assert_eq!(check.status, CheckStatus::Error);
                assert!(
                    check.detail.ends_with(&format!("; {}", hint(Some(URL)))),
                    "{}",
                    check.detail
                );
            }
        }
    }

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
    fn setup_prepares_the_database_before_the_first_asset_failure() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        prepare_database(&settings).unwrap();
        let error = install_selected(
            &[],
            &|name| Ok((name == "CODEX_HOME").then(|| "relative".into())),
            home.path(),
        )
        .unwrap_err();
        assert!(error.message().contains("CODEX_HOME"));
        let conn = crate::db::connect(&settings.database_url).unwrap();
        crate::db::classify_selected_schema(&conn, "sqlite:///:memory:")
            .require_current()
            .unwrap();
        assert!(!home.path().join(".hermes").exists());
        assert!(!home.path().join(".config/opencode").exists());
    }

    #[test]
    fn selected_installation_ignores_unrelated_configuration_and_is_repeatable() {
        let home = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            install_selected(
                &[CodingAgent::Claude, CodingAgent::Claude],
                &|name| Ok((name != "CLAUDE_CONFIG_DIR").then(|| "relative".into())),
                home.path(),
            )
            .unwrap();
            assert!(
                !home.path().join(".hermes").exists(),
                "an unselected hermes gets no assets"
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
            prepare_database(&settings).unwrap();
            let conn = crate::db::connect(&settings.database_url).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
            conn.execute_batch(mutation).unwrap();
            conn.execute_batch("BEGIN; SELECT * FROM cafleet_database")
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

    fn checks_for(probe: &FakeProbe, home: &Path) -> Vec<Check> {
        let backends = resolve_backends(probe, &Ok(home.to_path_buf()));
        agent_checks(&backends)
    }

    #[test]
    fn doctor_reports_a_modified_installed_skill() {
        let home = tempfile::tempdir().unwrap();
        let probe = probe(home.path(), true, None);
        let skills = home.path().join(".claude/skills");
        let row = || {
            let checks = checks_for(&probe, home.path());
            let row = find(&checks, "claude.skills");
            (row.status, row.detail.clone())
        };
        assert_eq!(
            row(),
            (
                CheckStatus::Warn,
                format!(
                    "skills not installed at {}; run 'cafleet setup --coding-agent claude'",
                    skills.display()
                )
            )
        );
        install_skills(&skills);
        assert_eq!(row(), (CheckStatus::Ok, skills.display().to_string()));
        let (relative, _) = crate::assets::SKILLS[0];
        let modified = skills.join(relative);
        std::fs::write(&modified, "modified").unwrap();
        assert_eq!(
            row(),
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
        let checks = checks_for(&probe, home.path());
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
            let checks = checks_for(&probe, home.path());
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
            let checks = checks_for(&probe, home.path());
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
        let check = find(&checks_for(&absent, home.path()), "opencode.extension")
            .detail
            .clone();
        assert_eq!(check, "unavailable: executable is absent");
        let invalid = probe(home.path(), true, Some("OPENCODE_CONFIG_DIR"));
        let checks = checks_for(&invalid, home.path());
        let check = find(&checks, "opencode.extension");
        assert_eq!(check.status, CheckStatus::Info);
        assert!(
            check
                .detail
                .starts_with("unavailable: configuration failed: "),
            "{}",
            check.detail
        );
    }

    mod hermes {
        use crate::cli::setup::tests::*;

        #[test]
        fn hermes_setup_installs_only_the_skills_into_the_resolved_home_without_hermes_on_path() {
            for (name, selected) in [
                ("plain setup", &[][..]),
                ("explicit hermes setup", &[CodingAgent::Hermes][..]),
            ] {
                let home = tempfile::tempdir().unwrap();
                let custom = home.path().join("hermes home");
                install_selected(
                    selected,
                    &|variable| {
                        Ok((variable == "HERMES_HOME").then(|| custom.display().to_string()))
                    },
                    home.path(),
                )
                .unwrap_or_else(|error| panic!("{name}: {}", error.message()));
                for (relative, bytes) in crate::assets::SKILLS {
                    assert_eq!(
                        std::fs::read(custom.join("skills").join(relative)).unwrap(),
                        *bytes,
                        "{name}"
                    );
                }
                let entries = std::fs::read_dir(&custom)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .collect::<Vec<_>>();
                assert_eq!(entries, ["skills"], "{name}: only the skills directory");
            }
        }

        #[test]
        fn doctor_reports_only_the_configuration_executable_and_skills_rows_for_hermes() {
            let home = tempfile::tempdir().unwrap();
            for executable in [true, false] {
                let checks = checks_for(&probe(home.path(), executable, None), home.path());
                let names = checks
                    .iter()
                    .map(|check| check.name.as_str())
                    .filter(|name| name.starts_with("hermes."))
                    .collect::<Vec<_>>();
                assert_eq!(
                    names,
                    ["hermes.configuration", "hermes.executable", "hermes.skills"],
                    "executable={executable}"
                );
            }
        }
    }

    mod pi {
        use crate::cli::setup::tests::*;

        fn pi_base(home: &Path) -> PathBuf {
            home.join(".pi/agent")
        }

        fn pi_checks(probe: &FakeProbe, home: &Path) -> Vec<Check> {
            checks_for(probe, home)
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
            let base = home.path().join("pi agent");
            for _ in 0..2 {
                install_selected(
                    &[CodingAgent::Pi],
                    &|name| Ok((name == "PI_CODING_AGENT_DIR").then(|| base.display().to_string())),
                    home.path(),
                )
                .unwrap();
                assert_installed_pi_assets(&base);
            }
        }

        #[test]
        fn plain_setup_installs_pi_assets_under_the_default_base() {
            let home = tempfile::tempdir().unwrap();
            install_selected(&[], &|_| Ok(None), home.path()).unwrap();
            assert_installed_pi_assets(&pi_base(home.path()));
        }

        #[test]
        fn pi_setup_rejects_an_invalid_base_before_writing_pi_assets() {
            let home = tempfile::tempdir().unwrap();
            let failure = install_selected(
                &[CodingAgent::Pi],
                &|name| Ok((name == "PI_CODING_AGENT_DIR").then(|| "relative".into())),
                home.path(),
            )
            .unwrap_err();
            assert_eq!(
                failure.message(),
                "PI_CODING_AGENT_DIR must be an absolute path (got 'relative')"
            );
            assert!(!pi_base(home.path()).exists());
        }

        #[test]
        fn pi_rows_follow_hermes_and_order_the_extension_between_executable_and_skills() {
            let home = tempfile::tempdir().unwrap();
            let checks = checks_for(&probe(home.path(), true, None), home.path());
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
        use rusqlite::Connection;
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

        const CHILD_CONFIG: &str = "CAFLEET_TEST_DOCTOR";
        const CHILD_STDOUT_MARKER: &str = "cafleet-doctor-child-stdout-begins";
        const CHILD_ANCESTOR: i64 = 4242;

        struct Scenario {
            home: tempfile::TempDir,
            database: PathBuf,
            caller: fn() -> CallerReading,
            executables: Vec<&'static str>,
            environment: Vec<(&'static str, String)>,
        }

        impl Scenario {
            fn new() -> Self {
                let home = crate::runtime::test_support::short_tempdir();
                let database = home.path().join("absent.db");
                Self {
                    home,
                    database,
                    caller: verified_caller,
                    executables: Vec::new(),
                    environment: Vec::new(),
                }
            }

            fn path(&self, name: &str) -> PathBuf {
                self.home.path().join(name)
            }

            fn config(&self, agent: CodingAgent) -> String {
                self.path(agent.name()).display().to_string()
            }

            fn connect(&mut self) -> Connection {
                let conn = crate::db::test_support::initialized_file_conn(&self.home);
                self.database = PathBuf::from(conn.path().unwrap());
                conn
            }

            fn executable(&mut self, name: &'static str) {
                self.executables.push(name);
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
                    text: render(&report, Mode::Text)
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
                        entry["detail"]
                            .as_str()
                            .unwrap()
                            .replace('\n', "\\n")
                            .replace('\r', "\\r")
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
            let conn = crate::db::test_support::initialized_file_conn(&home);
            let database = PathBuf::from(conn.path().unwrap());
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
            let stubs = ["claude", "codex", "opencode", "hermes", "pi"]
                .map(|name| (name, "exit 0".to_owned()))
                .to_vec();
            (home, database, stubs)
        }

        fn check_lines(stdout: &str) -> Vec<String> {
            let payload: Value = serde_json::from_str(stdout.trim_end()).unwrap();
            status_lines(payload.as_array().unwrap(), "name")
        }

        #[test]
        fn doctor_process_prints_json_for_a_healthy_home() {
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
            for name in ["claude", "codex", "opencode", "hermes", "pi"] {
                scenario.executable(name);
            }
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
                let mut scenario = Scenario::new();
                scenario.connect();
                healthy_backends(&mut scenario);
                scenario.caller = caller;
                scenario.views()
            }

            #[test]
            fn the_caller_reading_decides_the_multiplexer_and_caller_pane_rows_and_exit() {
                let tmux = format!("ok multiplexer: tmux server={TMUX_SOCKET}");
                type CallerCase = (&'static str, fn() -> CallerReading, [String; 2], i32);
                let cases: [CallerCase; 7] = [
                    (
                        "shared server",
                        || CallerReading::SharedServer {
                            label: codex_daemon().label,
                            pid: codex_daemon().pid,
                            message: shared_server_message(&codex_daemon()),
                        },
                        [
                            "info multiplexer: not checked: a shared codex app-server server (pid 407596) supplies the caller environment".into(),
                            format!(
                                "warn multiplexer.caller_pane: {}",
                                shared_server_message(&codex_daemon())
                            ),
                        ],
                        0,
                    ),
                    (
                        "unavailable multiplexer",
                        || CallerReading::Unavailable(CafleetError::App(NO_MULTIPLEXER.into())),
                        [
                            format!("error multiplexer: {NO_MULTIPLEXER}"),
                            "info multiplexer.caller_pane: unavailable: the multiplexer is unavailable".into(),
                        ],
                        1,
                    ),
                    (
                        "unset env pane",
                        || CallerReading::Resolved {
                            name: "tmux",
                            server: TMUX_SOCKET.to_string(),
                            pane: Ok(PaneReading::Unverified {
                                detail: unset_message(MuxBackend::Tmux),
                            }),
                        },
                        [
                            tmux.clone(),
                            format!(
                                "warn multiplexer.caller_pane: {}",
                                unset_message(MuxBackend::Tmux)
                            ),
                        ],
                        0,
                    ),
                    (
                        "refused env pane",
                        || CallerReading::Resolved {
                            name: "tmux",
                            server: TMUX_SOCKET.to_string(),
                            pane: Ok(PaneReading::Unverified {
                                detail: unverified_detail(),
                            }),
                        },
                        [
                            tmux.clone(),
                            format!("warn multiplexer.caller_pane: {}", unverified_detail()),
                        ],
                        0,
                    ),
                    (
                        "verified tmux pane",
                        verified_caller,
                        [
                            tmux.clone(),
                            "ok multiplexer.caller_pane: TMUX_PANE=%1 session=main window=@1".into(),
                        ],
                        0,
                    ),
                    (
                        "propagated pane error",
                        || CallerReading::Resolved {
                            name: "tmux",
                            server: TMUX_SOCKET.to_string(),
                            pane: Err(CafleetError::App(PANE_READ_FAILURE.into())),
                        },
                        [
                            tmux.clone(),
                            format!("error multiplexer.caller_pane: {PANE_READ_FAILURE}"),
                        ],
                        1,
                    ),
                    (
                        "verified herdr pane",
                        || CallerReading::Resolved {
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
                        },
                        [
                            "ok multiplexer: herdr server=default".into(),
                            "ok multiplexer.caller_pane: HERDR_PANE_ID=w4:p3N session=w4 window=w4:t1".into(),
                        ],
                        0,
                    ),
                ];
                for (name, caller, rows, code) in cases {
                    let views = views_with(caller);
                    assert_eq!(views.text[..2], rows, "{name}");
                    views.assert_exit(code);
                }
            }
        }

        /// Registers `name` with a run in `status`; each run gets its own nonce and path.
        fn member_with_run(
            conn: &mut Connection,
            fleet: i64,
            director: i64,
            name: &str,
            nonce: char,
            status: &str,
        ) -> (i64, i64) {
            let member =
                crate::broker::members::test_support::register(conn, fleet, name, Some("%5"));
            let mut spec =
                crate::broker::runtime::test_support::Fixture::spec(conn, fleet, director, member);
            spec.launch_nonce = nonce.to_string().repeat(64);
            spec.runtime_path = spec.runtime_path.replace("/fixture/", &format!("/{name}/"));
            let run = crate::broker::runtime::test_support::create_run(conn, &spec).unwrap();
            conn.execute(
                "UPDATE member_runs SET status=?1 WHERE run_id=?2",
                rusqlite::params![status, run.run_id],
            )
            .unwrap();
            (member, run.run_id)
        }

        #[test]
        fn doctor_adds_member_runtime_rows_after_the_database_row_and_fails_on_an_error_row() {
            let mut scenario = Scenario::new();
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario);
            let (fleet, director) =
                crate::broker::fleets::test_support::create_fleet_fixture(&mut conn, "doctor");
            let (prepared, prepared_run) =
                member_with_run(&mut conn, fleet, director, "prepared", 'a', "prepared");
            let (running, running_run) =
                member_with_run(&mut conn, fleet, director, "running", 'b', "running");
            let (exited, _) = member_with_run(&mut conn, fleet, director, "exited", 'c', "exited");
            let views = scenario.views();
            views.assert_exit(1);
            let database = views
                .text
                .iter()
                .position(|line| row_name(line) == "database")
                .expect("the database row");
            assert_eq!(
                views.text[database + 1],
                format!(
                    "info member.{prepared}.runtime: run={prepared_run} status=prepared: not probed"
                ),
                "{:?}",
                views.text
            );
            let failed = &views.text[database + 2];
            let url = Settings::for_database(&scenario.database).database_url;
            assert!(
                failed.starts_with(&format!(
                    "error member.{running}.runtime: run={running_run} health probe failed: "
                )),
                "{failed}"
            );
            assert!(
                failed.ends_with(&format!(
                    "; inspect with 'cafleet member show {running} --json --database-url {url}'; if it does not recover, run 'cafleet member delete {running} --database-url {url}' and create a replacement member"
                )),
                "{failed}"
            );
            assert!(
                !views
                    .text
                    .iter()
                    .any(|line| row_name(line) == format!("member.{exited}.runtime")),
                "a terminal run has no row: {:?}",
                views.text
            );
            assert!(
                !views
                    .text
                    .iter()
                    .any(|line| row_name(line) == format!("member.{director}.runtime")),
                "the Director has no row: {:?}",
                views.text
            );
        }

        #[test]
        fn doctor_prints_an_ok_member_row_from_a_live_runner_probe() {
            crate::runtime::service::test_support::input_service::with_input_service(
                |fixture, _| {
                    let mut scenario = Scenario::new();
                    healthy_backends(&mut scenario);
                    scenario.database = PathBuf::from(fixture.fixture.conn.path().unwrap());
                    let run = &fixture.fixture.run;
                    let views = scenario.views();
                    let name = format!("member.{}.runtime", run.member_id);
                    assert_eq!(
                        named_row(&views.text, &name),
                        format!("ok {name}: run={} observation=composer_empty", run.run_id)
                    );
                },
            );
        }

        #[test]
        fn doctor_with_only_warnings_prints_ready_backends_and_exits_zero() {
            let mut scenario = Scenario::new();
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
                named_row(&views.text, "hermes.skills"),
                format!(
                    "ok hermes.skills: {}",
                    Path::new(&scenario.config(CodingAgent::Hermes))
                        .join("skills")
                        .display()
                )
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
            let mut scenario = Scenario::new();
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
            let mut scenario = Scenario::new();
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
            let mut scenario = Scenario::new();
            let conn = scenario.connect();
            healthy_backends(&mut scenario);
            (scenario, conn)
        }

        #[test]
        fn doctor_reports_a_tampered_opencode_extension_on_its_row() {
            let (scenario, _conn) = healthy_scenario();
            let (plugin, _) = scenario.seed_opencode_extension();
            std::fs::write(&plugin, "tampered").unwrap();
            let rows = scenario.rows(1);
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
            let mut scenario = Scenario::new();
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
            let mut scenario = Scenario::new();
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
                    "hermes.executable",
                    "ok",
                    "hermes is on PATH".to_owned(),
                    "ok hermes.executable: hermes is on PATH".to_owned(),
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
