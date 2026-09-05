//! The `cafleet` command tree (SPEC §6.3, §10): clap parsing, the shared
//! option surface, the schema-version and stale-assets guard prologues, and
//! the per-group handlers. Orchestration glue only — it wires broker /
//! multiplexer / output / coding-agent.

mod creation;
mod doctor;
mod fleet;
pub(crate) mod helpers;
mod member;
mod message;
mod monitor;
mod server;
mod setup;

use clap::{Parser, Subcommand};
use rusqlite::Connection;

use crate::config::Settings;
use crate::diagnosis::{self, AssetMode, AssetReport, Diagnosis, SchemaState};
use crate::error::CafleetError;

pub(crate) enum SchemaPoint {
    Guard,
    Doctor,
    SetupBefore,
    SetupAfter,
}
pub(crate) enum InvocationPhase {
    CommandBody,
    SetupDatabase,
    SetupAssets,
}
// Fields are consumed by per-invocation observers; the default observer is a no-op.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum InvocationEvent<'a> {
    SchemaInspected {
        point: SchemaPoint,
        conn: &'a Connection,
        state: &'a SchemaState,
    },
    AssetsInspected {
        conn: Option<&'a Connection>,
        result: &'a Result<AssetReport, CafleetError>,
    },
    Finished {
        phase: InvocationPhase,
        conn: Option<&'a Connection>,
        result: &'a Result<(), CafleetError>,
    },
}
pub(crate) struct InvocationHooks<'a> {
    pub(crate) connect: &'a dyn Fn(&str) -> Result<Connection, CafleetError>,
    pub(crate) observe: &'a dyn for<'event> Fn(InvocationEvent<'event>),
    pub(crate) asset_env: crate::config_dir::EnvLookup<'a>,
}

pub(crate) fn inspect_schema(
    conn: &Connection,
    point: SchemaPoint,
    hooks: &InvocationHooks<'_>,
) -> SchemaState {
    let state = diagnosis::classify_schema(conn, crate::db::head_version());
    (hooks.observe)(InvocationEvent::SchemaInspected {
        point,
        conn,
        state: &state,
    });
    state
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
            observe: &|_| {},
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
            let mut facts = Diagnosis {
                head_version: crate::db::head_version(),
                schema: inspect_schema(&conn, SchemaPoint::Guard, hooks),
                assets: None,
            };
            helpers::schema_guard(&facts.schema)?;
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
                (hooks.observe)(InvocationEvent::AssetsInspected {
                    conn: Some(&conn),
                    result: &assets,
                });
                facts.assets = Some(assets);
                let assets = facts
                    .assets
                    .as_ref()
                    .expect("assets just inspected")
                    .as_ref()
                    .map_err(Clone::clone)?;
                helpers::stale_assets_guard(assets, VERSION)?;
            }
            let mut slot = Some(conn);
            let result = match command {
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
            };
            (hooks.observe)(InvocationEvent::Finished {
                phase: InvocationPhase::CommandBody,
                conn: slot.as_ref(),
                result: &result,
            });
            result
        }
    }
}
