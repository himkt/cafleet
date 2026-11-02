use crate::assets::agent_paths;
use crate::assets::install_agent;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::SpawnProbe;
use crate::config::EnvLookup;
use crate::config::Settings;
use crate::db;
use crate::db::SchemaState;
use crate::error::CafleetError;
use crate::runtime::system::SystemProbe;
use clap::Args;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct SetupArgs {
    /// Install selected backends' assets (repeatable; default: every backend).
    #[arg(long = "coding-agent", value_name = "AGENT", num_args = 1.., value_enum)]
    coding_agent: Vec<CodingAgent>,
}

pub(crate) fn run(settings: &Settings, args: SetupArgs) -> Result<(), CafleetError> {
    let mut conn = prepare_database(settings)?;
    let home = crate::config::home_dir()?;
    install_selected(
        &mut conn,
        &args.coding_agent,
        &|name| std::env::var(name).ok(),
        &home,
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
) -> Result<(), CafleetError> {
    for agent in CodingAgent::all() {
        if selected.is_empty() || selected.contains(agent) {
            let paths = agent_paths(env, home, *agent)?;
            install_agent(
                conn,
                *agent,
                &paths,
                crate::cli::VERSION,
                &mut std::io::stdout(),
            )?;
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

#[derive(Debug)]
struct Check {
    name: String,
    status: &'static str,
    detail: String,
}

impl Check {
    fn new(name: impl Into<String>, status: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

pub(crate) fn doctor(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    let mux = crate::cli::available_mux(settings, None).and_then(|mux| {
        let context = mux
            .context_discovery()
            .map_err(|cause| CafleetError::App(cause.to_string()))?;
        Ok(format!(
            "{} session={} window={} pane={}",
            mux.name(),
            context.session,
            context.window_id,
            context.pane_id
        ))
    });
    let mut checks = vec![match mux {
        Ok(detail) => Check::new("multiplexer", "ok", detail),
        Err(cause) => Check::new("multiplexer", "error", cause.to_string()),
    }];
    let (conn, schema) = database(settings);
    checks.push(match &schema {
        SchemaState::Head { version } => {
            Check::new("database", "ok", format!("compatible schema {version}"))
        }
        schema => Check::new(
            "database",
            "error",
            schema
                .require_current()
                .expect_err("a non-head schema is unavailable")
                .to_string(),
        ),
    });
    let home = crate::config::home_dir();
    checks.extend(agent_checks(
        conn.as_ref(),
        &schema,
        &SystemProbe,
        home.as_deref(),
    ));
    if matches!(schema, SchemaState::Head { .. }) {
        let conn = conn
            .as_ref()
            .expect("compatible schema has an open connection");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let runs = crate::broker::member_run_ids(conn)?;
        for run_id in runs {
            if std::time::Instant::now() >= deadline {
                checks.push(Check::new(
                    "member_runtime",
                    "info",
                    "runtime diagnosis budget exhausted; inspect remaining runs explicitly",
                ));
                break;
            }
            let until =
                deadline.min(std::time::Instant::now() + std::time::Duration::from_millis(100));
            checks.push(
                match crate::runtime::diagnose_runtime(conn, run_id, until) {
                    Ok(diagnosis) => Check::new(
                        format!("member_runtime.{run_id}"),
                        "info",
                        diagnosis.diagnostics.join("; "),
                    ),
                    Err(cause) => Check::new(
                        format!("member_runtime.{run_id}"),
                        "error",
                        cause.to_string(),
                    ),
                },
            );
        }
    }
    if args.json {
        println!(
            "{}",
            json!({"checks": checks.iter().map(|check| json!({
            "name": check.name, "status": check.status, "detail": check.detail,
        })).collect::<Vec<_>>()})
        );
    } else {
        for check in &checks {
            let detail = check.detail.replace('\n', "\\n").replace('\r', "\\r");
            println!("{} {}: {}", check.status, check.name, detail);
        }
    }
    if checks.iter().any(|check| check.status == "error") {
        Err(CafleetError::App("doctor found errors".into()))
    } else {
        Ok(())
    }
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
                ),
            }
        }
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
            "info",
            format!(
                "recorded {version} at {when}; run 'cafleet setup --coding-agent {}'",
                agent.name()
            ),
        ),
        Ok(None) => Check::new(
            name,
            "info",
            format!(
                "no installation recorded at {}; run 'cafleet setup --coding-agent {}'",
                path.display(),
                agent.name()
            ),
        ),
        Err(cause) => Check::new(name, "error", format!("installation query failed: {cause}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::assets::test_support::FakeHermes;
    use crate::cli::CliArgs;
    use crate::cli::Command;
    use crate::cli::setup::*;
    use crate::cli::test_support::assert_error;
    use crate::coding_agent::HERMES_DIRECTOR_ONLY_DENIES;
    use clap::Parser;
    use clap::error::ErrorKind;
    use std::cell::RefCell;

    fn complete_hermes() -> FakeHermes {
        FakeHermes::holding(&HERMES_DIRECTOR_ONLY_DENIES)
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
        approvals: Result<String, String>,
        approval_reads: RefCell<Vec<(PathBuf, PathBuf)>>,
    }
    impl Probe {
        fn new(home: &Path, executable: bool, invalid: Option<&'static str>) -> Self {
            Self {
                home: home.into(),
                executable,
                invalid,
                approvals: Ok(serde_json::json!({
                    "mode": "smart",
                    "deny": HERMES_DIRECTOR_ONLY_DENIES,
                })
                .to_string()),
                approval_reads: RefCell::new(Vec::new()),
            }
        }
    }
    impl SpawnProbe for Probe {
        fn hermes_approvals(
            &self,
            executable: &Path,
            config_base: &Path,
        ) -> Result<String, CafleetError> {
            self.approval_reads
                .borrow_mut()
                .push((executable.into(), config_base.into()));
            self.approvals.clone().map_err(CafleetError::App)
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
        assert_eq!(find(&checks, "codex.installation").status, "info");
        let base = home.path().join(".config/opencode");
        let configuration = find(&checks, "opencode.configuration");
        assert_eq!(configuration.status, "ok");
        assert_eq!(configuration.detail, base.display().to_string());
        let installation = find(&checks, "opencode.installation");
        assert_eq!(installation.status, "info");
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
        fn plain_setup_with_hermes_on_path_merges_the_deny_rules() {
            let home = tempfile::tempdir().unwrap();
            let mut conn = prepare_database(&settings(home.path())).unwrap();
            let mut hermes = FakeHermes::holding(&[]);
            install_selected(&mut conn, &[], &|_| None, home.path(), &mut hermes).unwrap();
            assert_eq!(hermes.calls[0], FakeHermes::path_call());
            assert_eq!(hermes.deny, serde_json::json!(HERMES_DIRECTOR_ONLY_DENIES));
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
                    "failed to install hermes approval rules into {}: binary hermes not found on PATH",
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

        fn hermes_rows(probe: &Probe, home: &Path) -> Vec<(String, &'static str, String)> {
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
                names[position + 1..position + 4],
                [
                    "hermes.approvals.mode",
                    "hermes.approvals.deny",
                    "hermes.installation"
                ]
            );
            assert_eq!(names[position - 1], "hermes.configuration");
            checks[position + 1..position + 3]
                .iter()
                .map(|check| (check.name.clone(), check.status, check.detail.clone()))
                .collect()
        }

        fn rows(
            mode: (&'static str, &str),
            deny: (&'static str, &str),
        ) -> Vec<(String, &'static str, String)> {
            vec![
                ("hermes.approvals.mode".into(), mode.0, mode.1.into()),
                ("hermes.approvals.deny".into(), deny.0, deny.1.into()),
            ]
        }

        #[test]
        fn doctor_reports_a_healthy_hermes_configuration_from_one_read_of_the_resolved_home() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), true, None);
            assert_eq!(
                hermes_rows(&probe, home.path()),
                rows(
                    ("ok", "smart"),
                    ("ok", "holds CAFleet's Director-only rules")
                )
            );
            assert_eq!(
                *probe.approval_reads.borrow(),
                [(
                    PathBuf::from("/fixture/bin/hermes"),
                    home.path().join(".hermes")
                )]
            );
        }

        #[test]
        fn doctor_marks_the_approval_rows_unavailable_without_the_executable() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), false, None);
            let absent = "unavailable: executable is absent";
            assert_eq!(
                hermes_rows(&probe, home.path()),
                rows(("info", absent), ("info", absent))
            );
            assert!(probe.approval_reads.borrow().is_empty());
        }

        #[test]
        fn doctor_marks_the_approval_rows_unavailable_for_an_invalid_hermes_home() {
            let home = tempfile::tempdir().unwrap();
            let probe = Probe::new(home.path(), true, Some("HERMES_HOME"));
            for (_, status, detail) in hermes_rows(&probe, home.path()) {
                assert_eq!(status, "info");
                assert!(
                    detail.starts_with("unavailable: configuration failed: "),
                    "{detail}"
                );
            }
            assert!(probe.approval_reads.borrow().is_empty());
        }

        #[test]
        fn doctor_reports_each_hermes_approval_problem_on_its_own_row() {
            let home = tempfile::tempdir().unwrap();
            let mut probe = Probe::new(home.path(), true, None);
            probe.approvals = Ok(serde_json::json!({
                "mode": "manual",
                "deny": HERMES_DIRECTOR_ONLY_DENIES,
            })
            .to_string());
            assert_eq!(
                hermes_rows(&probe, home.path()),
                rows(
                    (
                        "error",
                        "hermes approvals.mode is 'manual'; CAFleet members require 'smart'"
                    ),
                    ("ok", "holds CAFleet's Director-only rules")
                )
            );
            probe.approvals = Ok(serde_json::json!({
                "mode": "smart",
                "deny": &HERMES_DIRECTOR_ONLY_DENIES[..5],
            })
            .to_string());
            assert_eq!(
                hermes_rows(&probe, home.path()),
                rows(
                    ("ok", "smart"),
                    (
                        "error",
                        "hermes approvals.deny lacks CAFleet's Director-only rules: cafleet input reconcile *; run 'cafleet setup --coding-agent hermes'"
                    )
                )
            );
        }

        #[test]
        fn doctor_reports_an_unreadable_hermes_configuration_on_both_rows() {
            let home = tempfile::tempdir().unwrap();
            let mut probe = Probe::new(home.path(), true, None);
            probe.approvals = Err("did not finish within its deadline".into());
            let unreadable = "cannot read hermes configuration through 'hermes config get': did not finish within its deadline";
            assert_eq!(
                hermes_rows(&probe, home.path()),
                rows(("error", unreadable), ("error", unreadable))
            );
        }
    }
}
