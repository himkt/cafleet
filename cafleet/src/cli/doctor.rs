use std::path::{Path, PathBuf};

use clap::Args;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::coding_agent::{CodingAgent, SpawnProbe};
use crate::config::Settings;
use crate::config_dir::ResolvedDir;
use crate::diagnosis::{self, SchemaState};
use crate::error::CafleetError;
use crate::runtime::system::SystemProbe;

#[derive(Args)]
pub struct DoctorArgs {
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

pub fn run(settings: &Settings, args: DoctorArgs) -> Result<(), CafleetError> {
    let mux = super::helpers::resolve_mux(settings).and_then(|mux| {
        mux.ensure_available()?;
        let context = mux.context_discovery()?;
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
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| CafleetError::App("HOME is not set".into()));
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
        let mut statement = conn
            .prepare("SELECT run_id FROM member_runs ORDER BY run_id")
            .map_err(crate::broker::members::db_err)?;
        let runs = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(crate::broker::members::db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::broker::members::db_err)?;
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
                match crate::runtime::runner::diagnose_runtime(conn, run_id, until) {
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
    let opened = (|| {
        let path = crate::db::database_path(&settings.database_url)?;
        if !Path::new(path).try_exists().map_err(|cause| {
            CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
        })? {
            return Ok(None);
        }
        let conn = crate::db::connect_read_only(&settings.database_url)?;
        Ok(Some(conn))
    })();
    match opened {
        Ok(Some(conn)) => {
            let schema = diagnosis::classify_selected_schema(
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
                path.path.display().to_string(),
            ),
            Err(cause) => Check::new(format!("{name}.configuration"), "error", cause.to_string()),
        });
        let available = probe.resolve_binary(agent.binary_name()).is_some();
        checks.push(Check::new(
            format!("{name}.executable"),
            if available { "ok" } else { "info" },
            if available {
                format!("{} is on PATH", agent.binary_name())
            } else {
                format!("{} is optional and absent from PATH", agent.binary_name())
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
                    &paths.as_ref().expect("configuration succeeded").path,
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
    paths: &Result<ResolvedDir, CafleetError>,
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
                .required_preset(&path.path, mode)
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
    let recorded = conn.query_row(
        "SELECT cafleet_version, installed_at FROM asset_installs WHERE coding_agent=?1 AND path=?2",
        params![agent.name(), path.display().to_string()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional();
    match recorded {
        Ok(Some((version, when))) if version == super::VERSION => {
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
    use super::*;

    struct Probe {
        home: PathBuf,
        executable: bool,
        invalid: Option<&'static str>,
    }
    impl SpawnProbe for Probe {
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
        let probe = Probe {
            home: home.path().into(),
            executable: false,
            invalid: Some("OPENCODE_CONFIG_DIR"),
        };
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
        let mut conn = crate::broker::tests::migrated_conn(&home);
        let probe = Probe {
            home: home.path().into(),
            executable: true,
            invalid: None,
        };
        let schema = SchemaState::Head {
            version: crate::db::head_version(),
        };
        crate::broker::record_asset_install(
            &mut conn,
            "claude",
            &home.path().join(".claude").display().to_string(),
            super::super::VERSION,
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
        assert_eq!(find(&checks, "opencode.installation").status, "info");
        assert!(
            find(&checks, "opencode.installation")
                .detail
                .starts_with("no installation recorded")
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

    fn preset_checks(checks: &[Check]) -> Vec<&Check> {
        checks
            .iter()
            .filter(|check| check.name.starts_with("opencode.preset"))
            .collect()
    }

    #[test]
    fn opencode_preset_checks_follow_catalog_order_with_the_default_name_unsuffixed() {
        let home = tempfile::tempdir().unwrap();
        let probe = Probe {
            home: home.path().into(),
            executable: true,
            invalid: None,
        };
        let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
        let names = checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>();
        let executable = names
            .iter()
            .position(|name| *name == "opencode.executable")
            .unwrap();
        assert_eq!(
            names[executable + 1..executable + 4],
            [
                "opencode.preset",
                "opencode.preset.cafleet-ask",
                "opencode.installation"
            ]
        );
        assert_eq!(preset_checks(&checks).len(), 2);
    }

    #[test]
    fn opencode_preset_checks_report_ok_with_each_installed_preset_path() {
        let home = tempfile::tempdir().unwrap();
        let agents = home.path().join(".opencode/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("cafleet.md"), "default").unwrap();
        std::fs::write(agents.join("cafleet-ask.md"), "ask").unwrap();
        let probe = Probe {
            home: home.path().into(),
            executable: true,
            invalid: None,
        };
        let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
        for (name, file) in [
            ("opencode.preset", "cafleet.md"),
            ("opencode.preset.cafleet-ask", "cafleet-ask.md"),
        ] {
            let check = find(&checks, name);
            assert_eq!(check.status, "ok", "{name}");
            assert_eq!(check.detail, agents.join(file).display().to_string());
        }
    }

    #[test]
    fn missing_cafleet_ask_preset_is_an_error_with_the_setup_remedy() {
        let home = tempfile::tempdir().unwrap();
        let agents = home.path().join(".opencode/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("cafleet.md"), "default").unwrap();
        let probe = Probe {
            home: home.path().into(),
            executable: true,
            invalid: None,
        };
        let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
        assert_eq!(find(&checks, "opencode.preset").status, "ok");
        let check = find(&checks, "opencode.preset.cafleet-ask");
        assert_eq!(check.status, "error");
        let cause = check
            .detail
            .strip_prefix(&format!("{}: ", agents.join("cafleet-ask.md").display()))
            .and_then(|rest| rest.strip_suffix("; run 'cafleet setup --coding-agent opencode'"))
            .unwrap_or_else(|| panic!("preset error detail: {}", check.detail));
        assert!(!cause.is_empty());

        std::fs::remove_file(agents.join("cafleet.md")).unwrap();
        std::fs::write(agents.join("cafleet-ask.md"), "ask").unwrap();
        let checks = agent_checks(None, &SchemaState::Missing, &probe, Ok(home.path()));
        assert_eq!(find(&checks, "opencode.preset").status, "error");
        assert!(
            find(&checks, "opencode.preset")
                .detail
                .starts_with(&format!("{}: ", agents.join("cafleet.md").display()))
        );
        assert_eq!(find(&checks, "opencode.preset.cafleet-ask").status, "ok");
    }

    #[test]
    fn opencode_preset_checks_are_info_when_configuration_fails_or_the_executable_is_absent() {
        let home = tempfile::tempdir().unwrap();
        let unconfigured = Probe {
            home: home.path().into(),
            executable: true,
            invalid: Some("OPENCODE_CONFIG_DIR"),
        };
        let checks = agent_checks(None, &SchemaState::Missing, &unconfigured, Ok(home.path()));
        assert_eq!(preset_checks(&checks).len(), 2);
        for check in preset_checks(&checks) {
            assert_eq!(check.status, "info", "{}", check.name);
            assert!(
                check
                    .detail
                    .starts_with("unavailable: configuration failed: "),
                "{}",
                check.detail
            );
        }
        let absent = Probe {
            home: home.path().into(),
            executable: false,
            invalid: None,
        };
        let checks = agent_checks(None, &SchemaState::Missing, &absent, Ok(home.path()));
        for check in preset_checks(&checks) {
            assert_eq!(check.status, "info", "{}", check.name);
            assert_eq!(check.detail, "unavailable: executable is absent");
        }
        assert_eq!(preset_checks(&checks).len(), 2);
    }
}
