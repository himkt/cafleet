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

use crate::config::Settings;
use crate::error::CafleetError;

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
    match args.command {
        Command::Setup(cmd) => setup::run(&settings, cmd),
        Command::Doctor(cmd) => doctor::run(&settings, cmd),
        Command::Server(cmd) => server::run(&settings, cmd),
        Command::Fleet(cmd) => {
            helpers::schema_guard(&settings)?;
            helpers::stale_assets_guard(&settings)?;
            fleet::run(&settings, cmd)
        }
        Command::Member(cmd) => {
            helpers::schema_guard(&settings)?;
            helpers::stale_assets_guard(&settings)?;
            member::run(&settings, cmd)
        }
        Command::Message(cmd) => {
            helpers::schema_guard(&settings)?;
            helpers::stale_assets_guard(&settings)?;
            message::run(&settings, cmd)
        }
        Command::Monitor(cmd) => {
            helpers::schema_guard(&settings)?;
            helpers::stale_assets_guard(&settings)?;
            monitor::run(&settings, cmd)
        }
    }
}
