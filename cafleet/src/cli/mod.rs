//! The `cafleet` command tree (SPEC §6.3, §10): clap parsing, the shared
//! option surface, the stale-assets guard prologue, and the per-group
//! handlers. Orchestration glue only — it wires broker / multiplexer /
//! output / coding-agent.

mod doctor;
mod fleet;
mod helpers;
mod member;
mod message;
mod monitor;
mod setup;
mod system;

use clap::{Args, Parser, Subcommand};

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
    /// Print the resolved multiplexer backend, the calling pane's
    /// identifiers, and the assets-install report.
    Doctor(doctor::DoctorArgs),
    /// Fleet lifecycle.
    #[command(subcommand)]
    Fleet(fleet::FleetCommand),
    /// Member lifecycle and pane interaction.
    #[command(subcommand)]
    Member(member::MemberCommand),
    /// Message broker.
    #[command(subcommand)]
    Message(message::MessageCommand),
    /// Supervision scheduler.
    #[command(subcommand)]
    Monitor(monitor::MonitorCommand),
}

/// The shared required-`--fleet-id` surface: declared optional at the parser,
/// enforced post-parse by [`helpers::require_fleet_id`].
#[derive(Args)]
pub(crate) struct FleetIdArg {
    /// Fleet ID (integer); required for this subcommand.
    #[arg(long = "fleet-id", value_name = "INT")]
    fleet_id: Option<i64>,
}

/// Parse the argv and run the selected command. clap's own parse errors exit
/// 2 and `--help` / `--version` exit 0 before this returns.
pub fn run() -> Result<(), CafleetError> {
    let args = CliArgs::parse();
    let settings = Settings::from_env()?;
    match args.command {
        Command::Setup(cmd) => setup::run(&settings, cmd),
        Command::Doctor(cmd) => doctor::run(&settings, cmd),
        Command::Fleet(cmd) => {
            helpers::stale_assets_guard(&settings)?;
            fleet::run(&settings, cmd)
        }
        Command::Member(cmd) => {
            helpers::stale_assets_guard(&settings)?;
            member::run(&settings, cmd)
        }
        Command::Message(cmd) => {
            helpers::stale_assets_guard(&settings)?;
            message::run(&settings, cmd)
        }
        Command::Monitor(cmd) => {
            helpers::stale_assets_guard(&settings)?;
            monitor::run(&settings, cmd)
        }
    }
}
