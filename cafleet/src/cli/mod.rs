//! The `cafleet` command tree (SPEC §6.3, §10): clap parsing, the shared
//! option surface, the schema-version and stale-assets guard prologues, and
//! the per-group handlers. Orchestration glue only — it wires broker /
//! multiplexer / output / coding-agent.

pub(crate) mod creation;
#[cfg(test)]
mod doc_contract;
mod doctor;
pub(crate) mod fleet;
pub(crate) mod helpers;
pub(crate) mod member;
mod message;
pub(crate) mod monitor;
#[cfg(test)]
mod runtime_docs;
mod server;
mod setup;

use clap::{Parser, Subcommand};
use rusqlite::Connection;

use crate::config::Settings;
use crate::diagnosis::{self, AssetMode};
use crate::error::CafleetError;

pub(crate) struct InvocationHooks<'a> {
    pub(crate) connect: &'a dyn Fn(&str) -> Result<Connection, CafleetError>,
    pub(crate) asset_env: crate::config_dir::EnvLookup<'a>,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "cafleet",
    version,
    about = "CAFleet — CLI for the message broker and member registry."
)]
struct CliArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Migrate the database schema and install the coding-agent assets
    /// (skills and presets).
    Setup(setup::SetupArgs),
    /// Print the three-section environment diagnosis (multiplexer, database,
    /// coding agents).
    Doctor(doctor::DoctorArgs),
    /// Start the admin WebUI server.
    Server(server::ServerArgs),
    /// Fleet lifecycle.
    #[command(subcommand)]
    Fleet(fleet::FleetCommand),
    /// Member lifecycle and pane interaction.
    #[command(subcommand)]
    Member(member::MemberCommand),
    /// Message broker.
    #[command(subcommand)]
    Message(message::MessageCommand),
    /// Run the per-fleet scheduler loop in-process.
    Monitor(monitor::MonitorArgs),
}

/// Parse the argv and run the selected command. clap's own parse errors exit
/// 2 and `--help` / `--version` exit 0 before this returns.
pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    dispatch(
        &settings,
        args,
        &InvocationHooks {
            connect: &crate::db::connect,
            asset_env: &|name| std::env::var(name).ok(),
        },
    )
}

#[cfg(test)]
pub(crate) fn run_with_hooks(
    settings: &Settings,
    argv: &[&str],
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    let args = CliArgs::try_parse_from(argv).map_err(|e| CafleetError::Usage(e.to_string()))?;
    dispatch(settings, args, hooks)
}

fn dispatch(
    settings: &Settings,
    args: CliArgs,
    hooks: &InvocationHooks<'_>,
) -> Result<(), CafleetError> {
    match args.command {
        Command::Setup(cmd) => setup::run(settings, cmd, hooks),
        Command::Doctor(cmd) => doctor::run(settings, cmd, hooks),
        command => {
            let conn = (hooks.connect)(&settings.database_url)?;
            let schema = diagnosis::classify_schema(&conn, crate::db::head_version());
            helpers::schema_guard(&schema)?;
            if !matches!(command, Command::Server(_)) {
                let home = std::path::PathBuf::from(
                    std::env::var("HOME")
                        .map_err(|_| CafleetError::App("HOME is not set".into()))?,
                );
                let assets = diagnosis::diagnose_assets(
                    Some(&conn),
                    hooks.asset_env,
                    &home,
                    VERSION,
                    AssetMode::Guard,
                );
                helpers::stale_assets_guard(&assets?, VERSION)?;
            }
            let mut slot = Some(conn);
            match command {
                Command::Fleet(cmd) => fleet::run(&mut slot, settings, cmd),
                Command::Member(cmd) => member::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Message(cmd) => message::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Monitor(cmd) => monitor::run(
                    slot.as_mut().expect("open invocation connection"),
                    settings,
                    cmd,
                ),
                Command::Server(cmd) => server::run(settings, cmd),
                Command::Setup(_) | Command::Doctor(_) => unreachable!("handled above"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{self, test_support as common};
    use std::cell::Cell;

    #[test]
    fn ack_reuses_the_guard_connection() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::from_lookup(|_| None).unwrap();
        let opens = Cell::new(0);
        let connect = |_: &str| {
            opens.set(opens.get() + 1);
            let mut conn = Connection::open_in_memory().unwrap();
            crate::db::migrate_to_head(&mut conn).unwrap();
            let (_, director) = common::create_fleet(&mut conn, "invocation");
            broker::record_asset_install(
                &mut conn,
                "claude",
                dir.path().to_str().unwrap(),
                VERSION,
            )
            .unwrap();
            let message = broker::send_message(
                &mut conn,
                &common::FakeNotifier::succeeding(),
                200,
                director,
                &director.to_string(),
                "ack",
            )
            .unwrap();
            assert_eq!(message.message.message_id, 1);
            Ok(conn)
        };
        run_with_hooks(
            &settings,
            &["cafleet", "message", "ack", "1", "--json"],
            &InvocationHooks {
                connect: &connect,
                asset_env: &|_| Some(dir.path().display().to_string()),
            },
        )
        .unwrap();
        assert_eq!(opens.get(), 1);
    }

    #[test]
    fn setup_reuses_open_connections_and_retries_failed_opens() {
        for initial in ["missing", "head", "ahead", "open failure"] {
            let dir = tempfile::tempdir().unwrap();
            let url = format!("sqlite:///{}", dir.path().join("database.db").display());
            let settings =
                Settings::from_lookup(|name| (name == "CAFLEET_DATABASE_URL").then(|| url.clone()))
                    .unwrap();
            let opens = Cell::new(0);
            let connect = |url: &str| {
                opens.set(opens.get() + 1);
                if initial == "open failure" && opens.get() == 1 {
                    return Err(CafleetError::App("first open failed".into()));
                }
                let mut conn = crate::db::connect(url).unwrap();
                if initial != "missing" {
                    crate::db::migrate_to_head(&mut conn).unwrap();
                }
                if initial == "ahead" {
                    conn.execute(
                        "UPDATE refinery_schema_history SET version=version+1 WHERE version=?1",
                        [crate::db::head_version()],
                    )
                    .unwrap();
                }
                Ok(conn)
            };
            let result = run_with_hooks(
                &settings,
                &["cafleet", "setup", "--coding-agent", "claude"],
                &InvocationHooks {
                    connect: &connect,
                    asset_env: &|name| {
                        assert_eq!(name, "CLAUDE_CONFIG_DIR");
                        Some(dir.path().display().to_string())
                    },
                },
            );
            assert_eq!(
                result.is_ok(),
                matches!(initial, "missing" | "head"),
                "{initial}"
            );
            assert_eq!(opens.get(), if initial == "open failure" { 2 } else { 1 });
            let conn = crate::db::connect(&url).unwrap();
            let rows = broker::list_asset_installs(&conn).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["coding_agent"], "claude");
            assert_eq!(rows[0]["cafleet_version"], VERSION);
            assert!(dir.path().join("skills/cafleet/SKILL.md").is_file());
        }
    }
}
