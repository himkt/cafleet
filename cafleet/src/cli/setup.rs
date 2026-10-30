use std::path::Path;

use clap::Args;
use rusqlite::Connection;

use crate::assets::{agent_paths, install_agent};
use crate::coding_agent::CodingAgent;
use crate::config::Settings;
use crate::config_dir::EnvLookup;
use crate::diagnosis::SchemaState;
use crate::error::CafleetError;

#[derive(Args)]
pub(crate) struct SetupArgs {
    /// Install selected backends' assets (repeatable; default: every backend).
    #[arg(long = "coding-agent", value_name = "AGENT", num_args = 1.., value_enum)]
    coding_agent: Vec<CodingAgent>,
}

pub(crate) fn run(settings: &Settings, args: SetupArgs) -> Result<(), CafleetError> {
    let mut conn = prepare_database(settings)?;
    let home = crate::config_dir::home_dir()?;
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
    if db_file.try_exists().map_err(|cause| {
        CafleetError::App(format!("cannot inspect database at '{path}': {cause}"))
    })? {
        let conn = crate::db::connect_read_only(&settings.database_url)?;
        let schema = crate::diagnosis::classify_selected_schema(
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
    let schema = crate::diagnosis::classify_selected_schema(&conn, head, &settings.database_url);
    match schema {
        SchemaState::Missing | SchemaState::Behind { .. } => {
            crate::db::migrate_to_head(&mut conn)?;
            crate::diagnosis::classify_schema(&conn, head).require_current()?;
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
            install_agent(conn, *agent, &paths, super::VERSION, &mut std::io::stdout())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::assert_error;
    use crate::cli::{CliArgs, Command};
    use clap::Parser;
    use clap::error::ErrorKind;

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
    }

    #[test]
    fn setup_records_successes_before_the_first_asset_failure() {
        let home = tempfile::tempdir().unwrap();
        let settings = settings(home.path());
        let mut conn = prepare_database(&settings).unwrap();
        let error = install_selected(
            &mut conn,
            &[],
            &|name| (name == "CODEX_HOME").then(|| "relative".into()),
            home.path(),
        )
        .unwrap_err();
        assert!(error.message().contains("CODEX_HOME"));
        crate::diagnosis::classify_schema(&conn, crate::db::head_version())
            .require_current()
            .unwrap();
        let names = conn
            .prepare("SELECT coding_agent FROM asset_installs ORDER BY coding_agent")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(names, ["claude"]);
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
                &|name| (name != "CLAUDE_CONFIG_DIR").then(|| "relative".into()),
                home.path(),
            )
            .unwrap();
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
}
