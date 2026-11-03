use crate::assets::HermesConfigRunner;
use crate::assets::agent_paths;
use crate::assets::install_agent;
use crate::broker::MemberRunRegistry;
use crate::broker::runtime::CleanupState;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::SpawnProbe;
use crate::config::EnvLookup;
use crate::config::Settings;
use crate::db;
use crate::db::SchemaState;
use crate::error::CafleetError;
use crate::runtime::control::MemberState;
use crate::runtime::system::SystemProbe;
use clap::Args;
use rusqlite::Connection;
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
    let env = |name: &str| std::env::var(name).ok();
    let mut hermes = SystemHermes {
        env: &env,
        home: &home,
    };
    install_selected(&mut conn, &args.coding_agent, &env, &home, &mut hermes)
}

struct SystemHermes<'a> {
    env: EnvLookup<'a>,
    home: &'a Path,
}

impl HermesConfigRunner for SystemHermes<'_> {
    fn binary_available(&self) -> bool {
        SystemProbe.resolve_binary("hermes").is_some()
    }

    fn run(&mut self, args: &[String]) -> Result<String, CafleetError> {
        let executable = SystemProbe
            .resolve_binary("hermes")
            .ok_or_else(|| CafleetError::App("binary hermes not found on PATH".into()))?;
        let base = CodingAgent::Hermes.config_dir(self.env, self.home)?;
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        crate::coding_agent::run_bounded(
            &executable,
            &args,
            &[("HERMES_HOME", &base)],
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        )
        .map_err(CafleetError::App)
    }
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
        let schema = crate::db::classify_selected_schema(
            &conn,
            crate::db::head_version(),
            &settings.database_url,
        );
        match schema {
            SchemaState::Missing | SchemaState::Behind { .. } | SchemaState::Head { .. } => {}
            _ => schema.require_current()?,
        }
    }
    let mut conn = crate::db::connect(&settings.database_url)?;
    let head = crate::db::head_version();
    let schema = crate::db::classify_selected_schema(&conn, head, &settings.database_url);
    match schema {
        SchemaState::Missing | SchemaState::Behind { .. } => {
            crate::db::migrate_to_head(&mut conn)?;
            crate::db::classify_schema(&conn, head).require_current()?;
            println!(
                "Applied migrations to head ({head}) at {}.",
                db_file.display()
            );
        }
        SchemaState::Head { .. } => println!("Already at head ({head})."),
        _ => schema.require_current()?,
    }
    Ok(conn)
}

fn install_selected(
    conn: &mut Connection,
    selected: &[CodingAgent],
    env: EnvLookup<'_>,
    home: &Path,
    hermes: &mut dyn HermesConfigRunner,
) -> Result<(), CafleetError> {
    for agent in CodingAgent::all() {
        if selected.is_empty() || selected.contains(agent) {
            let paths = agent_paths(env, home, *agent)?;
            let runner = if *agent == CodingAgent::Hermes
                && (!selected.is_empty() || hermes.binary_available())
            {
                Some(&mut *hermes)
            } else {
                None
            };
            install_agent(
                conn,
                *agent,
                &paths,
                crate::cli::VERSION,
                runner,
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

#[derive(Debug)]
struct Check {
    name: String,
    status: &'static str,
    detail: String,
    problem: Option<Problem>,
    runtime: Option<RuntimeFields>,
}

/// How a non-`ok` backend check reads on its backend's summary line.
#[derive(Debug)]
struct Problem {
    phrase: String,
    setup_fix: bool,
}

#[derive(Debug)]
struct RuntimeFields {
    member_id: Option<i64>,
    state: MemberState,
}

impl Check {
    fn new(name: impl Into<String>, status: &'static str, detail: impl Into<String>) -> Self {
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
}

struct Summary {
    component: String,
    status: &'static str,
    detail: String,
    members: Option<(MemberState, Vec<i64>)>,
}

impl Summary {
    fn new(component: impl Into<String>, status: &'static str, detail: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status,
            detail: detail.into(),
            members: None,
        }
    }

    fn members(
        status: &'static str,
        detail: impl Into<String>,
        state: MemberState,
        member_ids: Vec<i64>,
    ) -> Self {
        Self {
            members: Some((state, member_ids)),
            ..Self::new("members", status, detail)
        }
    }
}

pub(crate) fn doctor(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    doctor_with(settings, args, crate::broker::member_run_registry)
}

fn doctor_with(
    settings: &Settings,
    args: DoctorArgs,
    registry: impl Fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>,
) -> Result<(), CafleetError> {
    let mux = crate::cli::available_mux(settings, None).and_then(|mux| {
        let context = mux
            .context_discovery()
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        Ok((
            format!(
                "{} session={} window={} pane={}",
                mux.name(),
                context.session,
                context.window_id,
                context.pane_id
            ),
            format!(
                "{} (session {}, pane {})",
                mux.name(),
                context.session,
                context.pane_id
            ),
        ))
    });
    let (mut checks, mut summary) = match mux {
        Ok((detail, short)) => (
            vec![Check::new("multiplexer", "ok", detail)],
            vec![Summary::new("multiplexer", "ok", short)],
        ),
        Err(cause) => (
            vec![Check::new("multiplexer", "error", cause.to_string())],
            vec![Summary::new("multiplexer", "error", cause.to_string())],
        ),
    };
    let (conn, schema) = database(settings);
    match &schema {
        SchemaState::Head { version } => {
            checks.push(Check::new(
                "database",
                "ok",
                format!("compatible schema {version}"),
            ));
            summary.push(Summary::new("database", "ok", format!("schema {version}")));
        }
        schema => {
            let cause = schema
                .require_current()
                .expect_err("a non-head schema is unavailable")
                .to_string();
            checks.push(Check::new("database", "error", cause.clone()));
            summary.push(Summary::new("database", "error", cause));
        }
    }
    let home = crate::config::home_dir();
    let backend_checks = agent_checks(conn.as_ref(), &schema, &SystemProbe, home.as_deref());
    summary.extend(
        CodingAgent::all()
            .iter()
            .map(|agent| backend_summary(*agent, &backend_checks)),
    );
    checks.extend(backend_checks);
    if matches!(schema, SchemaState::Head { .. }) {
        let conn = conn
            .as_ref()
            .expect("compatible schema has an open connection");
        let (member_rows, member_summary) = member_checks(conn, registry);
        checks.extend(member_rows);
        summary.extend(member_summary);
    } else {
        summary.push(Summary::members(
            "info",
            "not checked (database unavailable)",
            MemberState::NotChecked,
            Vec::new(),
        ));
    }
    if args.json {
        println!(
            "{}",
            json!({
                "summary": summary.iter().map(summary_json).collect::<Vec<_>>(),
                "checks": checks.iter().map(check_json).collect::<Vec<_>>(),
            })
        );
    } else if args.verbose {
        for check in &checks {
            println!("{} {}: {}", check.status, check.name, escape(&check.detail));
        }
    } else {
        for line in &summary {
            println!(
                "{} {}: {}",
                line.status,
                line.component,
                escape(&line.detail)
            );
        }
    }
    if checks.iter().any(|check| check.status == "error") {
        Err(CafleetError::App("doctor found errors".into()))
    } else {
        Ok(())
    }
}

fn escape(detail: &str) -> String {
    detail.replace('\n', "\\n").replace('\r', "\\r")
}

fn summary_json(summary: &Summary) -> serde_json::Value {
    let mut entry = json!({
        "component": summary.component,
        "status": summary.status,
        "detail": summary.detail,
    });
    if let Some((state, member_ids)) = &summary.members {
        entry["state"] = state.token().into();
        entry["member_ids"] = json!(member_ids);
    }
    entry
}

fn check_json(check: &Check) -> serde_json::Value {
    let mut entry = json!({
        "name": check.name,
        "status": check.status,
        "detail": check.detail,
    });
    if let Some(runtime) = &check.runtime {
        entry["member_id"] = json!(runtime.member_id);
        entry["state"] = runtime.state.token().into();
    }
    entry
}

fn severity(status: &str) -> u8 {
    match status {
        "error" => 3,
        "warn" => 2,
        "info" => 1,
        "ok" => 0,
        other => unreachable!("unknown doctor status {other}"),
    }
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
    if configuration.status == "error" {
        return Summary::new(name, "error", configuration.detail.clone());
    }
    if find("executable").status != "ok" {
        return Summary::new(
            name,
            "info",
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
    let Some(status) = problems
        .iter()
        .map(|(status, _)| *status)
        .max_by_key(|status| severity(status))
    else {
        return Summary::new(name, "ok", "ready");
    };
    let mut detail = problems
        .iter()
        .map(|(_, problem)| problem.phrase.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    if problems.iter().any(|(_, problem)| problem.setup_fix) {
        detail.push_str(&format!(" — run 'cafleet setup --coding-agent {name}'"));
    }
    Summary::new(name, status, detail)
}

struct MemberRun {
    run_id: i64,
    member_id: i64,
    state: MemberState,
    phrases: Vec<String>,
}

fn classify_registry(row: &MemberRunRegistry) -> Option<MemberRun> {
    let active = row.member_status == "active";
    let finished = row.run_status.terminal() && row.cleanup_state == CleanupState::Complete;
    let state = match (finished, active) {
        (true, false) => MemberState::Ended,
        (true, true) => MemberState::AgentExited,
        (false, false) => MemberState::DeletedWithLeftoverRuntime,
        (false, true) => return None,
    };
    Some(MemberRun {
        run_id: row.run_id,
        member_id: row.member_id,
        state,
        phrases: vec![format!(
            "run {}, cleanup {}",
            row.run_status.as_str(),
            row.cleanup_state.as_str()
        )],
    })
}

fn probe_run(conn: &Connection, row: &MemberRunRegistry, deadline: Instant) -> MemberRun {
    let now = Instant::now();
    let (state, phrases) = if now >= deadline {
        (
            MemberState::NotChecked,
            vec!["runtime diagnosis time budget exhausted".to_owned()],
        )
    } else {
        let until = deadline.min(now + Duration::from_millis(100));
        match crate::runtime::diagnose_runtime(conn, row.run_id, until) {
            Ok(diagnosis) => (diagnosis.state, diagnosis.diagnostics),
            Err(cause) => (MemberState::CheckFailed, vec![cause.to_string()]),
        }
    };
    MemberRun {
        run_id: row.run_id,
        member_id: row.member_id,
        state,
        phrases,
    }
}

fn member_checks(
    conn: &Connection,
    registry: impl Fn(&Connection) -> Result<Vec<MemberRunRegistry>, CafleetError>,
) -> (Vec<Check>, Vec<Summary>) {
    let rows = match registry(conn) {
        Ok(rows) => rows,
        Err(cause) => {
            let mut check = Check::new(
                "member_runtime",
                "error",
                format!("member runtimes could not be read: {cause}"),
            );
            check.runtime = Some(RuntimeFields {
                member_id: None,
                state: MemberState::CheckFailed,
            });
            let summary = Summary::members(
                "error",
                format!("could not be checked: {cause}"),
                MemberState::CheckFailed,
                Vec::new(),
            );
            return (vec![check], vec![summary]);
        }
    };
    let classified = rows.iter().map(classify_registry).collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(5);
    let runs = rows
        .iter()
        .zip(classified)
        .map(|(row, run)| run.unwrap_or_else(|| probe_run(conn, row, deadline)))
        .collect::<Vec<_>>();
    let checks = runs.iter().map(runtime_check).collect();
    let mut summary = MemberState::SUMMARY_ORDER
        .iter()
        .filter_map(|state| {
            let member_ids = runs
                .iter()
                .filter(|run| run.state == *state)
                .map(|run| run.member_id)
                .collect::<Vec<_>>();
            (!member_ids.is_empty()).then(|| state_summary(*state, member_ids))
        })
        .collect::<Vec<_>>();
    if summary.is_empty() {
        summary.push(Summary::members(
            "ok",
            "none running",
            MemberState::Running,
            Vec::new(),
        ));
    }
    (checks, summary)
}

fn runtime_check(run: &MemberRun) -> Check {
    let detail = if run.state == MemberState::CheckFailed {
        format!(
            "member {} {}: {}",
            run.member_id,
            run.state.label(),
            run.phrases.join("; ")
        )
    } else {
        format!(
            "member {} {}: {}{}",
            run.member_id,
            run.state.label(),
            run.phrases.join("; "),
            run_suffix(run.state.next_commands(), &run.member_id.to_string())
        )
    };
    let mut check = Check::new(
        format!("member_runtime.{}", run.run_id),
        run.state.status(),
        detail,
    );
    check.runtime = Some(RuntimeFields {
        member_id: Some(run.member_id),
        state: run.state,
    });
    check
}

fn state_summary(state: MemberState, mut member_ids: Vec<i64>) -> Summary {
    member_ids.sort_unstable();
    member_ids.dedup();
    let detail = format!(
        "{} {}: {}{}",
        member_ids.len(),
        state.label(),
        format_id_ranges(&member_ids),
        run_suffix(state.next_commands(), "ID")
    );
    Summary::members(state.status(), detail, state, member_ids)
}

fn run_suffix(commands: &[&str], member_id: &str) -> String {
    if commands.is_empty() {
        return String::new();
    }
    let quoted = commands
        .iter()
        .map(|command| format!("'{}'", command.replace("ID", member_id)))
        .collect::<Vec<_>>();
    format!(" — run {}", quoted.join(" or "))
}

/// Renders ascending, de-duplicated IDs, collapsing three or more consecutive IDs to `a-b`.
fn format_id_ranges(ids: &[i64]) -> String {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut items = Vec::new();
    let mut start = 0;
    while start < ids.len() {
        let mut end = start;
        while end + 1 < ids.len() && ids[end + 1] == ids[end] + 1 {
            end += 1;
        }
        if end - start >= 2 {
            items.push(format!("{}-{}", ids[start], ids[end]));
        } else {
            items.extend(ids[start..=end].iter().map(i64::to_string));
        }
        start = end + 1;
    }
    items.join(", ")
}

fn database(settings: &Settings) -> (Option<Connection>, SchemaState) {
    match crate::db::open_existing(&settings.database_url, true) {
        Ok(Some(conn)) => {
            let schema = db::classify_selected_schema(
                &conn,
                crate::db::head_version(),
                &settings.database_url,
            );
            (Some(conn), schema)
        }
        Ok(None) => (None, SchemaState::Missing),
        Err(cause) => (None, SchemaState::Unreachable { cause }),
    }
}

fn agent_checks(
    conn: Option<&Connection>,
    schema: &SchemaState,
    probe: &dyn SpawnProbe,
    home: Result<&Path, &CafleetError>,
) -> Vec<Check> {
    let mut checks = Vec::new();
    for agent in CodingAgent::all() {
        let name = agent.name();
        let paths = home
            .map_err(Clone::clone)
            .and_then(|home| agent.config_dir(&|name| probe.env_var(name), home));
        checks.push(match &paths {
            Ok(path) => Check::new(
                format!("{name}.configuration"),
                "ok",
                path.display().to_string(),
            ),
            Err(cause) => Check::new(format!("{name}.configuration"), "error", cause.to_string()),
        });
        let available = probe.resolve_binary(name).is_some();
        checks.push(Check::new(
            format!("{name}.executable"),
            if available { "ok" } else { "info" },
            if available {
                format!("{name} is on PATH")
            } else {
                format!("{name} is optional and absent from PATH")
            },
        ));
        if *agent == CodingAgent::Opencode {
            let default_mode = agent
                .resolve_policy(None, None)
                .expect("registered backend has a valid default policy")
                .resolved_mode;
            for mode in agent.policy_choices(None) {
                checks.push(preset_check(
                    *agent,
                    &mode,
                    mode == default_mode,
                    &paths,
                    available,
                ));
            }
        }
        if *agent == CodingAgent::Hermes {
            checks.push(hermes_passthrough_check(probe, &paths));
        }
        let install_name = format!("{name}.installation");
        checks.push(match (&paths, schema) {
            (Err(cause), _) => Check::new(
                install_name,
                "info",
                format!("unavailable: configuration failed: {cause}"),
            ),
            (_, SchemaState::Head { .. }) => {
                let conn = conn.expect("a head-compatible database has an open connection");
                install_check(
                    conn,
                    *agent,
                    paths.as_ref().expect("configuration succeeded"),
                )
            }
            _ => {
                let cause = schema
                    .require_current()
                    .expect_err("a non-head schema is unavailable");
                Check::new(install_name, "info", format!("unavailable: {cause}"))
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
    match (paths, available) {
        (Err(cause), _) => Check::new(
            check_name,
            "info",
            format!("unavailable: configuration failed: {cause}"),
        ),
        (_, false) => Check::new(check_name, "info", "unavailable: executable is absent"),
        (Ok(path), true) => {
            let preset = agent
                .required_preset(path, mode)
                .expect("OpenCode has a required preset for every launch policy");
            match std::fs::read(&preset) {
                Ok(_) => Check::new(check_name, "ok", preset.display().to_string()),
                Err(cause) => Check::new(
                    check_name,
                    "error",
                    format!(
                        "{}: {cause}; run 'cafleet setup --coding-agent {name}'",
                        preset.display()
                    ),
                )
                .problem(
                    format!("preset {} is unreadable: {cause}", preset.display()),
                    true,
                ),
            }
        }
    }
}

fn hermes_passthrough_check(
    probe: &dyn SpawnProbe,
    paths: &Result<PathBuf, CafleetError>,
) -> Check {
    const NAME: &str = "hermes.client_env_passthrough";
    let base = match paths {
        Ok(base) => base,
        Err(cause) => {
            return Check::new(
                NAME,
                "info",
                format!("unavailable: configuration failed: {cause}"),
            );
        }
    };
    let Some(executable) = probe.resolve_binary(CodingAgent::Hermes.name()) else {
        return Check::new(NAME, "info", "unavailable: executable is absent");
    };
    match crate::coding_agent::hermes_missing_client_env_names(
        probe.hermes_client_env_passthrough(&executable, base),
    ) {
        Ok(missing) if missing.is_empty() => Check::new(NAME, "ok", "holds CAFleet's member names"),
        Ok(missing) => {
            let phrase = crate::coding_agent::hermes_missing_names_phrase(&missing);
            Check::new(
                NAME,
                "error",
                format!("hermes {phrase}; run 'cafleet setup --coding-agent hermes'"),
            )
            .problem(phrase, true)
        }
        Err(cause) => Check::new(NAME, "error", cause.message()).problem(cause.message(), false),
    }
}

fn install_check(conn: &Connection, agent: CodingAgent, path: &Path) -> Check {
    let name = format!("{}.installation", agent.name());
    let recorded =
        crate::broker::recorded_asset_install(conn, agent.name(), &path.display().to_string());
    match recorded {
        Ok(Some((version, when))) if version == crate::cli::VERSION => {
            Check::new(name, "ok", format!("recorded {version} at {when}"))
        }
        Ok(Some((version, when))) => Check::new(
            name,
            "warn",
            format!(
                "recorded {version} at {when}; run 'cafleet setup --coding-agent {}'",
                agent.name()
            ),
        )
        .problem(
            format!(
                "assets from cafleet {version} (this is {})",
                crate::cli::VERSION
            ),
            true,
        ),
        Ok(None) => Check::new(
            name,
            "warn",
            format!(
                "no installation recorded at {}; run 'cafleet setup --coding-agent {}'",
                path.display(),
                agent.name()
            ),
        )
        .problem(format!("assets not installed at {}", path.display()), true),
        Err(cause) => {
            let detail = format!("installation query failed: {cause}");
            Check::new(name, "error", detail.clone()).problem(detail, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::test_support::FakeHermes;
    use crate::cli::CliArgs;
    use crate::cli::Command;
    use crate::cli::setup::*;
    use crate::cli::test_support::assert_error;
    use crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH;
    use clap::Parser;
    use clap::error::ErrorKind;
    use std::cell::RefCell;

    fn complete_hermes() -> FakeHermes {
        FakeHermes::complete()
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
        Settings::for_database(&home.join("registry.db"), None)
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
        assert!(
            CliArgs::try_parse_from(["cafleet", "setup", "--coding-agent", "unknown"]).is_err()
        );
        let Command::Setup(args) =
            CliArgs::try_parse_from(["cafleet", "setup", "--coding-agent", "hermes"])
                .unwrap()
                .command
        else {
            panic!("setup command")
        };
        assert_eq!(args.coding_agent, [CodingAgent::Hermes]);
    }

    #[test]
    fn setup_records_successes_before_the_first_asset_failure() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        let mut conn = prepare_database(&settings).unwrap();
        let mut hermes = complete_hermes();
        let error = install_selected(
            &mut conn,
            &[],
            &|name| (name == "CODEX_HOME").then(|| "relative".into()),
            home.path(),
            &mut hermes,
        )
        .unwrap_err();
        assert!(error.message().contains("CODEX_HOME"));
        crate::db::classify_schema(&conn, crate::db::head_version())
            .require_current()
            .unwrap();
        assert_eq!(installed_agents(&conn), ["claude"]);
        assert!(hermes.calls.is_empty());
        assert!(!home.path().join(".config/opencode").exists());
    }

    #[test]
    fn selected_installation_ignores_unrelated_configuration_and_is_repeatable() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        for _ in 0..2 {
            let mut conn = prepare_database(&settings).unwrap();
            let mut hermes = complete_hermes();
            install_selected(
                &mut conn,
                &[CodingAgent::Claude, CodingAgent::Claude],
                &|name| (name != "CLAUDE_CONFIG_DIR").then(|| "relative".into()),
                home.path(),
                &mut hermes,
            )
            .unwrap();
            assert!(
                hermes.calls.is_empty(),
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
            "PRAGMA application_id=1128351288",
            "UPDATE refinery_schema_history SET version=99 WHERE version=(SELECT MAX(version) FROM refinery_schema_history)",
            "ALTER TABLE messages RENAME COLUMN text TO wrong",
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
            (&["setup", "claude"], ErrorKind::UnknownArgument, "claude"),
            (
                &["setup", "--skip", "claude"],
                ErrorKind::UnknownArgument,
                "--skip",
            ),
            (
                &["setup", "--coding-agent", "python"],
                ErrorKind::InvalidValue,
                "python",
            ),
            (
                &["setup", "--coding-agent", "claude", "python"],
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

    struct Probe {
        home: PathBuf,
        executable: bool,
        invalid: Option<&'static str>,
        passthrough: Result<String, String>,
        passthrough_reads: RefCell<Vec<(PathBuf, PathBuf)>>,
    }
    impl Probe {
        fn new(home: &Path, executable: bool, invalid: Option<&'static str>) -> Self {
            Self {
                home: home.into(),
                executable,
                invalid,
                passthrough: Ok(serde_json::json!(HERMES_CLIENT_ENV_PASSTHROUGH).to_string()),
                passthrough_reads: RefCell::new(Vec::new()),
            }
        }
    }
    impl SpawnProbe for Probe {
        fn hermes_client_env_passthrough(
            &self,
            executable: &Path,
            config_base: &Path,
        ) -> Result<String, CafleetError> {
            self.passthrough_reads
                .borrow_mut()
                .push((executable.into(), config_base.into()));
            self.passthrough.clone().map_err(CafleetError::App)
        }
        fn resolve_binary(&self, name: &str) -> Option<PathBuf> {
            assert!(crate::coding_agent::coding_agent(name).is_some());
            self.executable
                .then(|| PathBuf::from("/fixture/bin").join(name))
        }
        fn home_dir(&self) -> PathBuf {
            self.home.clone()
        }
        fn env_var(&self, name: &str) -> Option<String> {
            assert!(
                CodingAgent::all()
                    .iter()
                    .any(|agent| agent.config_variable() == name)
            );
            (self.invalid == Some(name)).then(|| "relative".into())
        }
    }

    fn find<'a>(checks: &'a [Check], name: &str) -> &'a Check {
        checks
            .iter()
            .find(|check| check.name == name)
            .expect("every diagnostic has its stable name")
    }

    #[test]
    fn independent_failures_keep_executable_discovery_and_mark_dependent_checks_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let probe = Probe::new(home.path(), false, Some("OPENCODE_CONFIG_DIR"));
        let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
        assert_eq!(find(&checks, "opencode.configuration").status, "error");
        assert_eq!(find(&checks, "opencode.executable").status, "info");
        for name in [
            "opencode.preset",
            "opencode.preset.cafleet-ask",
            "opencode.installation",
            "claude.installation",
            "codex.installation",
        ] {
            let check = find(&checks, name);
            assert_eq!(check.status, "info");
            assert!(check.detail.starts_with("unavailable:"));
        }
        assert_eq!(find(&checks, "claude.configuration").status, "ok");
    }

    #[test]
    fn healthy_database_distinguishes_absent_stale_current_and_failed_install_queries() {
        let home = tempfile::tempdir().unwrap();
        let mut conn = crate::db::test_support::migrated_conn(&home);
        let probe = Probe::new(home.path(), true, None);
        let schema = SchemaState::Head {
            version: crate::db::head_version(),
        };
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
        let checks = agent_checks(Some(&conn), &schema, &probe, Ok(home.path()));
        assert_eq!(find(&checks, "claude.installation").status, "ok");
        assert_eq!(find(&checks, "codex.installation").status, "warn");
        let base = home.path().join(".config/opencode");
        let configuration = find(&checks, "opencode.configuration");
        assert_eq!(configuration.status, "ok");
        assert_eq!(configuration.detail, base.display().to_string());
        let installation = find(&checks, "opencode.installation");
        assert_eq!(installation.status, "warn");
        assert_eq!(
            installation.detail,
            format!(
                "no installation recorded at {}; run 'cafleet setup --coding-agent opencode'",
                base.display()
            )
        );
        assert_eq!(find(&checks, "opencode.preset").status, "error");
        assert_eq!(find(&checks, "opencode.preset.cafleet-ask").status, "error");
        conn.execute(
            "UPDATE asset_installs SET cafleet_version=X'00' WHERE coding_agent='claude'",
            [],
        )
        .unwrap();
        assert_eq!(
            find(
                &agent_checks(Some(&conn), &schema, &probe, Ok(home.path())),
                "claude.installation"
            )
            .status,
            "error"
        );
    }

    #[derive(Clone, Copy)]
    enum PresetDetail {
        Path,
        ReadError,
        Prefix(&'static str),
        Exact(&'static str),
    }

    #[test]
    fn opencode_preset_checks_follow_catalog_order_and_report_each_preset_state() {
        use PresetDetail::*;
        const FILES: [&str; 2] = ["cafleet.md", "cafleet-ask.md"];
        let configuration_failed = Prefix("unavailable: configuration failed: ");
        let absent = Exact("unavailable: executable is absent");
        for (valid, executable, installed, expected) in [
            (
                true,
                true,
                [false, false],
                [("error", ReadError), ("error", ReadError)],
            ),
            (
                true,
                true,
                [true, false],
                [("ok", Path), ("error", ReadError)],
            ),
            (
                true,
                true,
                [false, true],
                [("error", ReadError), ("ok", Path)],
            ),
            (true, true, [true, true], [("ok", Path), ("ok", Path)]),
            (
                false,
                true,
                [true, true],
                [
                    ("info", configuration_failed),
                    ("info", configuration_failed),
                ],
            ),
            (
                true,
                false,
                [true, true],
                [("info", absent), ("info", absent)],
            ),
        ] {
            let context = format!("valid={valid} executable={executable} installed={installed:?}");
            let home = tempfile::tempdir().unwrap();
            let agents = home.path().join(".config/opencode/agents");
            std::fs::create_dir_all(&agents).unwrap();
            for (file, present) in FILES.iter().zip(installed) {
                if present {
                    std::fs::write(agents.join(file), "preset").unwrap();
                }
            }
            let probe = Probe::new(
                home.path(),
                executable,
                (!valid).then_some("OPENCODE_CONFIG_DIR"),
            );
            let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
            let names = checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>();
            let position = names
                .iter()
                .position(|name| *name == "opencode.executable")
                .unwrap();
            assert_eq!(
                names[position + 1..position + 4],
                [
                    "opencode.preset",
                    "opencode.preset.cafleet-ask",
                    "opencode.installation"
                ],
                "{context}"
            );
            for ((check, file), (status, detail)) in checks[position + 1..position + 3]
                .iter()
                .zip(FILES)
                .zip(expected)
            {
                assert_eq!(check.status, status, "{context} {}", check.name);
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
                    Prefix(prefix) => {
                        assert!(
                            check.detail.starts_with(prefix),
                            "{context}: {}",
                            check.detail
                        )
                    }
                    Exact(text) => assert_eq!(check.detail, text, "{context}"),
                }
            }
        }
    }

    mod hermes {
        use crate::cli::setup::tests::*;

        #[test]
        fn plain_setup_without_hermes_on_path_installs_hermes_skills_and_skips_the_merge() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let mut hermes = complete_hermes();
            hermes.available = false;
            install_selected(&mut conn, &[], &|_| None, home.path(), &mut hermes).unwrap();
            assert!(hermes.calls.is_empty(), "{:?}", hermes.calls);
            assert!(installed_agents(&conn).contains(&"hermes".to_owned()));
            assert!(
                home.path()
                    .join(".hermes/skills/cafleet/SKILL.md")
                    .is_file()
            );
        }

        #[test]
        fn plain_setup_with_hermes_on_path_merges_only_the_passthrough_names() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let mut hermes = FakeHermes::holding(&[]);
            install_selected(&mut conn, &[], &|_| None, home.path(), &mut hermes).unwrap();
            assert_eq!(hermes.calls[0], FakeHermes::path_call());
            assert_eq!(
                hermes.passthrough,
                serde_json::json!(HERMES_CLIENT_ENV_PASSTHROUGH)
            );
            assert!(installed_agents(&conn).contains(&"hermes".to_owned()));
        }

        #[test]
        fn explicit_hermes_setup_without_hermes_on_path_fails_without_recording_hermes() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let mut hermes = complete_hermes();
            hermes.available = false;
            let failure = install_selected(
                &mut conn,
                &[CodingAgent::Hermes],
                &|_| None,
                home.path(),
                &mut hermes,
            )
            .expect_err("explicit hermes setup requires the binary");
            assert_eq!(failure.exit_code(), 1);
            assert_eq!(
                failure.message(),
                format!(
                    "failed to install hermes terminal.client_env_passthrough names into {}: binary hermes not found on PATH",
                    home.path().join(".hermes").display()
                )
            );
            assert!(installed_agents(&conn).is_empty());
        }

        #[test]
        fn explicit_hermes_setup_uses_the_hermes_home_override() {
            let home = tempfile::tempdir().unwrap();
            let custom = home.path().join("hermes home");
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let mut hermes = complete_hermes();
            install_selected(
                &mut conn,
                &[CodingAgent::Hermes],
                &|name| (name == "HERMES_HOME").then(|| custom.display().to_string()),
                home.path(),
                &mut hermes,
            )
            .unwrap();
            assert!(custom.join("skills/cafleet/SKILL.md").is_file());
            assert!(!home.path().join(".hermes").exists());
            let path: String = conn
                .query_row(
                    "SELECT path FROM asset_installs WHERE coding_agent='hermes'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(path, custom.display().to_string());
        }

        fn hermes_row(probe: &Probe, home: &Path) -> (String, &'static str, String) {
            let checks = agent_checks(None, &SchemaState::Missing, probe, Ok(home));
            let names = checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>();
            let position = names
                .iter()
                .position(|name| *name == "hermes.executable")
                .expect("hermes executable row");
            assert_eq!(
                names[position + 1..position + 3],
                ["hermes.client_env_passthrough", "hermes.installation"]
            );
            assert_eq!(names[position - 1], "hermes.configuration");
            let check = &checks[position + 1];
            (check.name.clone(), check.status, check.detail.clone())
        }

        fn row(status: &'static str, detail: &str) -> (String, &'static str, String) {
            (
                "hermes.client_env_passthrough".into(),
                status,
                detail.into(),
            )
        }

        #[test]
        fn doctor_reports_a_healthy_hermes_passthrough_from_one_read_of_the_resolved_home() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), true, None);
            assert_eq!(
                hermes_row(&probe, home.path()),
                row("ok", "holds CAFleet's member names")
            );
            assert_eq!(
                *probe.passthrough_reads.borrow(),
                [(
                    PathBuf::from("/fixture/bin/hermes"),
                    home.path().join(".hermes"),
                )]
            );
        }

        #[test]
        fn doctor_marks_the_hermes_passthrough_row_unavailable_without_the_executable() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), false, None);
            assert_eq!(
                hermes_row(&probe, home.path()),
                row("info", "unavailable: executable is absent")
            );
            assert!(probe.passthrough_reads.borrow().is_empty());
        }

        #[test]
        fn doctor_marks_the_hermes_passthrough_row_unavailable_for_an_invalid_hermes_home() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), true, Some("HERMES_HOME"));
            let (_, status, detail) = hermes_row(&probe, home.path());
            assert_eq!(status, "info");
            assert!(
                detail.starts_with("unavailable: configuration failed: "),
                "{detail}"
            );
            assert!(probe.passthrough_reads.borrow().is_empty());
        }

        #[test]
        fn doctor_reports_missing_hermes_passthrough_names_in_canonical_order() {
            let home = tempfile::tempdir().unwrap();
            let mut probe = Probe::new(home.path(), true, None);
            probe.passthrough =
                Ok(serde_json::json!(["PATH", "CAFLEET_DATABASE_URL", "TMUX"]).to_string());
            let missing = HERMES_CLIENT_ENV_PASSTHROUGH
                .into_iter()
                .filter(|name| !["CAFLEET_DATABASE_URL", "TMUX"].contains(name))
                .collect::<Vec<_>>();
            assert_eq!(missing[0], "CAFLEET_MEMBER_ID");
            assert_eq!(
                hermes_row(&probe, home.path()),
                row(
                    "error",
                    &format!(
                        "hermes terminal.client_env_passthrough lacks CAFleet's member names: {}; run 'cafleet setup --coding-agent hermes'",
                        missing.join(", ")
                    )
                )
            );
        }

        #[test]
        fn doctor_reports_an_unreadable_hermes_passthrough_with_its_cause() {
            let home = tempfile::tempdir().unwrap();
            let mut probe = Probe::new(home.path(), true, None);
            probe.passthrough = Err("did not finish within its deadline".into());
            assert_eq!(
                hermes_row(&probe, home.path()),
                row(
                    "error",
                    "cannot read hermes configuration through 'hermes config get': did not finish within its deadline"
                )
            );
        }
    }

    mod doctor_summary {
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

        #[test]
        fn a_run_reached_after_the_time_budget_is_not_checked() {
            let dir = crate::runtime::test_support::short_tempdir();
            let mut conn = crate::db::test_support::migrated_conn(&dir);
            let (fleet, director) = create_fleet_fixture(&mut conn, "doctor-budget");
            let run = register_member_run(&mut conn, fleet, director, "unprobed");
            let rows = member_run_registry(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            let deadline = Instant::now();
            let probed = probe_run(&conn, &rows[0], deadline);
            assert_eq!(probed.state, MemberState::NotChecked);
            let check = runtime_check(&probed);
            assert_eq!(
                format!("{} {}: {}", check.status, check.name, check.detail),
                format!(
                    "info member_runtime.{}: member {1} not checked (time limit): runtime diagnosis time budget exhausted — run 'cafleet member show {1}'",
                    run.run_id, run.member_id
                )
            );
            let summary = state_summary(probed.state, vec![probed.member_id]);
            assert_eq!(
                format!(
                    "{} {}: {}",
                    summary.status, summary.component, summary.detail
                ),
                format!(
                    "info members: 1 not checked (time limit): {} — run 'cafleet member show ID'",
                    run.member_id
                )
            );
        }

        #[test]
        fn id_ranges_collapse_runs_of_three_or_more_consecutive_ids() {
            for (ids, expected) in [
                (&[][..], ""),
                (&[7], "7"),
                (&[7, 8], "7, 8"),
                (&[1, 2, 3], "1-3"),
                (&[1, 2, 4, 5], "1, 2, 4, 5"),
                (
                    &[
                        1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                    ],
                    "1-6, 9-22",
                ),
                (&[3, 3, 4, 5, 5], "3-5"),
                (&[9, 1, 2, 3, 7], "1-3, 7, 9"),
                (&[23, 8, 7, 24], "7, 8, 23, 24"),
            ] {
                assert_eq!(format_id_ranges(ids), expected, "{ids:?}");
            }
        }

        const CHILD_CONFIG: &str = "CAFLEET_TEST_DOCTOR";
        const CHILD_STDOUT_MARKER: &str = "cafleet-doctor-child-stdout-begins";
        const REGISTRY_FAILURE: &str = "registry fixture failure";
        const PHANTOM_RUN: i64 = 999;
        const PHANTOM_MEMBER: i64 = 42;

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
            });
            Ok(rows)
        }

        #[test]
        #[ignore = "owned doctor subprocess entry"]
        fn doctor_child() {
            let config: Value =
                serde_json::from_str(&std::env::var(CHILD_CONFIG).expect("explicit doctor child"))
                    .unwrap();
            let settings = Settings::for_database(
                Path::new(config["database"].as_str().unwrap()),
                Some("tmux"),
            );
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
            let result = match config["registry"].as_str().unwrap() {
                "system" => doctor_with(&settings, args, member_run_registry),
                "failing" => doctor_with(&settings, args, failing_registry),
                "phantom" => doctor_with(&settings, args, phantom_registry),
                other => panic!("unknown registry fixture {other}"),
            };
            match result {
                Ok(()) => std::process::exit(0),
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::exit(error.exit_code());
                }
            }
        }

        struct Outcome {
            code: Option<i32>,
            lines: Vec<String>,
            stderr: String,
        }

        struct Scenario {
            home: tempfile::TempDir,
            database: PathBuf,
            registry: &'static str,
            executables: Vec<(&'static str, String)>,
            environment: Vec<(&'static str, String)>,
        }

        impl Scenario {
            fn new(registry: &'static str) -> Self {
                let home = crate::runtime::test_support::short_tempdir();
                let database = home.path().join("absent.db");
                Self {
                    home,
                    database,
                    registry,
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
                let conn = crate::db::test_support::migrated_conn(&self.home);
                self.database = PathBuf::from(conn.path().unwrap());
                conn
            }

            fn executable(&mut self, name: &'static str, body: &str) {
                self.executables.push((name, body.to_owned()));
            }

            fn hermes_passthrough(&mut self, names: &[&str]) {
                let body = format!(
                    "case \"$*\" in\n  'config get --json terminal.client_env_passthrough') printf '%s\\n' '{}' ;;\n  *) exit 3 ;;\nesac",
                    serde_json::json!(names)
                );
                self.executable("hermes", &body);
            }

            fn opencode_presets(&self) {
                let agents = self.path("opencode").join("agents");
                std::fs::create_dir_all(&agents).unwrap();
                for preset in ["cafleet.md", "cafleet-ask.md"] {
                    std::fs::write(agents.join(preset), "preset").unwrap();
                }
            }

            fn run(&self, args: &[&str]) -> Outcome {
                use crate::runtime::system::test_support::TestChild;
                use std::process::Stdio;
                use std::time::Duration;
                use std::time::Instant;

                let bin = tempfile::tempdir().unwrap();
                crate::coding_agent::test_support::executable_stub(
                    bin.path(),
                    "tmux",
                    "case \" $* \" in *' display-message '*) printf 'main|@1|%%1\\n' ;; esac",
                );
                for (name, body) in &self.executables {
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
                        serde_json::json!({
                            "database": self.database,
                            "args": args,
                            "registry": self.registry,
                        })
                        .to_string(),
                    )
                    .env("PATH", bin.path())
                    .env("HOME", self.home.path())
                    .env("TMUX", "cafleet-test,1,0")
                    .env("TMUX_PANE", "%1")
                    .env_remove("HERDR_ENV")
                    .env("RUST_BACKTRACE", "0");
                for agent in CodingAgent::all() {
                    command.env(agent.config_variable(), self.config(*agent));
                }
                for (name, value) in &self.environment {
                    command.env(name, value);
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
                    .unwrap_or_else(|| {
                        panic!("{args:?}: child reached doctor: {stdout:?} {stderr}")
                    });
                Outcome {
                    code: status.code(),
                    lines: output.lines().map(str::to_owned).collect(),
                    stderr,
                }
            }

            fn views(&self) -> Views {
                let views = Views {
                    text: self.run(&["doctor"]),
                    verbose: self.run(&["doctor", "--verbose"]),
                    json: self.run(&["doctor", "--json"]),
                };
                assert_views_agree(&views, &self.run(&["doctor", "--json", "--verbose"]));
                views
            }
        }

        struct Views {
            text: Outcome,
            verbose: Outcome,
            json: Outcome,
        }

        impl Views {
            fn payload(&self) -> Value {
                assert_eq!(self.json.lines.len(), 1, "{:?}", self.json.lines);
                serde_json::from_str(&self.json.lines[0]).unwrap()
            }

            fn verbose_row(&self, prefix: &str) -> &str {
                let rows = self
                    .verbose
                    .lines
                    .iter()
                    .filter(|line| line.starts_with(prefix))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 1, "{prefix}: {:?}", self.verbose.lines);
                rows[0]
            }

            fn assert_exit(&self, code: i32) {
                for outcome in [&self.text, &self.verbose, &self.json] {
                    assert_eq!(outcome.code, Some(code), "{}", outcome.stderr);
                    let expected = if code == 0 {
                        ""
                    } else {
                        "Error: doctor found errors\n"
                    };
                    assert_eq!(outcome.stderr, expected);
                }
            }
        }

        fn row_name(line: &str) -> &str {
            line.split_once(' ')
                .and_then(|(_, rest)| rest.split_once(": "))
                .map(|(name, _)| name)
                .unwrap_or_else(|| panic!("STATUS NAME: DETAIL row: {line}"))
        }

        fn assert_views_agree(views: &Views, json_verbose: &Outcome) {
            for line in &views.verbose.lines {
                let name = row_name(line);
                assert!(
                    ["multiplexer", "database", "member_runtime"].contains(&name)
                        || name.contains('.'),
                    "--verbose prints only check rows: {line}"
                );
            }
            for line in &views.text.lines {
                let name = row_name(line);
                assert!(
                    ["multiplexer", "database", "members"].contains(&name)
                        || crate::coding_agent::coding_agent(name).is_some(),
                    "default output prints only summary lines: {line}"
                );
            }
            assert_eq!(json_verbose.lines, views.json.lines);
            let payload = views.payload();
            assert_key_order(&payload, &["summary", "checks"]);
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
                summary
                    .iter()
                    .map(|entry| format!(
                        "{} {}: {}",
                        entry["status"].as_str().unwrap(),
                        entry["component"].as_str().unwrap(),
                        entry["detail"].as_str().unwrap()
                    ))
                    .collect::<Vec<_>>(),
                views.text.lines,
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
                checks
                    .iter()
                    .map(|entry| format!(
                        "{} {}: {}",
                        entry["status"].as_str().unwrap(),
                        entry["name"].as_str().unwrap(),
                        entry["detail"].as_str().unwrap()
                    ))
                    .collect::<Vec<_>>(),
                views.verbose.lines,
                "checks hold exactly the verbose rows"
            );
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
            format!("ok database: schema {}", crate::db::head_version())
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
            for name in ["claude", "codex", "opencode"] {
                scenario.executable(name, "exit 0");
            }
            scenario.hermes_passthrough(&crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH);
            scenario.opencode_presets();
            for agent in [
                CodingAgent::Claude,
                CodingAgent::Opencode,
                CodingAgent::Hermes,
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
            ]
        }

        #[test]
        fn doctor_with_only_warnings_prints_ready_backends_and_exits_zero() {
            let mut scenario = Scenario::new("system");
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            let mut expected = vec![multiplexer_line(), database_line()];
            expected.extend(healthy_backend_lines(&scenario));
            expected.push("ok members: none running".into());
            assert_eq!(views.text.lines, expected);
            views.assert_exit(0);
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
                    .lines
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
            let mut scenario = Scenario::new("system");
            let mut conn = scenario.connect();
            for name in ["claude", "codex"] {
                scenario.executable(name, "exit 0");
            }
            let mut without_member_id = crate::coding_agent::HERMES_CLIENT_ENV_PASSTHROUGH.to_vec();
            without_member_id.retain(|name| *name != "CAFLEET_MEMBER_ID");
            scenario.hermes_passthrough(&without_member_id);
            record(
                &mut conn,
                &scenario,
                CodingAgent::Claude,
                crate::cli::VERSION,
            );
            record(&mut conn, &scenario, CodingAgent::Codex, "0.30.0");
            let (runs, _artifacts) = member_runs(&mut conn);
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
                views.text.lines,
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
                        "error hermes: terminal.client_env_passthrough lacks CAFleet's member names: CAFLEET_MEMBER_ID; assets not installed at {} — run 'cafleet setup --coding-agent hermes'",
                        scenario.config(CodingAgent::Hermes)
                    ),
                    format!(
                        "warn members: 1 not responding: {} — run 'cafleet member recover ID'",
                        runs.unresponsive.member_id
                    ),
                    format!(
                        "warn members: 4 lost runtime: {}, {}-{} — run 'cafleet member recover ID' or 'cafleet member delete ID'",
                        lost_ids[0], lost_ids[1], lost_ids[3]
                    ),
                    format!(
                        "warn members: 1 interrupted launch: {} — run 'cafleet member recover ID'",
                        runs.interrupted.member_id
                    ),
                    format!(
                        "warn members: 1 agent exited: {} — run 'cafleet member delete ID'",
                        runs.exited.member_id
                    ),
                    format!(
                        "warn members: 1 deleted with leftover runtime: {} — run 'cafleet member recover ID'",
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
                    "warn {}member {} deleted with leftover runtime: run starting, cleanup incomplete — run 'cafleet member recover {}'",
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
                    " — run 'cafleet member recover {}'",
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
                        " — run 'cafleet member recover {0}' or 'cafleet member delete {0}'",
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
                    " — run 'cafleet member recover {}'",
                    unresponsive.member_id
                )),
                "{unresponsive_row}"
            );
            let runtime_rows = views
                .verbose
                .lines
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
            let mut scenario = Scenario::new("system");
            let mut conn = scenario.connect();
            for name in ["claude", "opencode"] {
                scenario.executable(name, "exit 0");
            }
            scenario
                .environment
                .push(("CLAUDE_CONFIG_DIR", "relative".into()));
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
                views.text.lines,
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
            let mut scenario = Scenario::new("system");
            scenario.executable("claude", "exit 0");
            let views = scenario.views();
            let lines = &views.text.lines;
            assert_eq!(lines.len(), 7, "{lines:?}");
            assert_eq!(lines[0], multiplexer_line());
            assert!(lines[1].starts_with("error database: "), "{}", lines[1]);
            assert_eq!(
                lines[2..],
                [
                    "info claude: assets unchecked (database unavailable)",
                    "info codex: not installed (codex is not on PATH)",
                    "info opencode: not installed (opencode is not on PATH)",
                    "info hermes: not installed (hermes is not on PATH)",
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
            let mut scenario = Scenario::new("failing");
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            let mut expected = vec![multiplexer_line(), database_line()];
            expected.extend(healthy_backend_lines(&scenario));
            expected.push(format!(
                "error members: could not be checked: {REGISTRY_FAILURE}"
            ));
            assert_eq!(views.text.lines, expected);
            views.assert_exit(1);
            assert_eq!(
                views.verbose.lines.last().unwrap(),
                &format!(
                    "error member_runtime: member runtimes could not be read: {REGISTRY_FAILURE}"
                )
            );
            assert!(
                views
                    .verbose
                    .lines
                    .iter()
                    .any(|line| line.starts_with("ok hermes.client_env_passthrough: "))
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
            let mut scenario = Scenario::new("phantom");
            let mut conn = scenario.connect();
            healthy_backends(&mut scenario, &mut conn);
            let views = scenario.views();
            assert_eq!(
                views.text.lines.last().unwrap(),
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
    }
}
