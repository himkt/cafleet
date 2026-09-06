//! The `fleet` group (SPEC §6.3 *fleet group*) — the atomic fleet + Director
//! + monitor bootstrap, list, show, and soft-delete.

use rusqlite::Connection;

use clap::Subcommand;
use serde_json::Value;

use super::creation::{FleetCreateOptions, PaneGuard, PreparedSpawn, SpawnPreparation};
use super::helpers::{emit, resolve_body, resolve_mux};
use crate::broker;
use crate::coding_agent::{SpawnProbe, coding_agent};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::MultiplexerError;
use crate::multiplexer::spawn::{
    Deadline, PaneSpawnRequest, SpawnExecution, SpawnMultiplexer, SystemClock,
};
use crate::output::format_fleet_create;
use crate::runtime::system::SystemProbe;
use crate::runtime::system::SystemRunner;
use std::time::Duration;

const MONITOR_NAME: &str = "monitor";
const MONITOR_DESCRIPTION: &str = "Monitor member for this fleet";

#[derive(Subcommand)]
pub enum FleetCommand {
    /// Create a fleet with its root Director and monitor member.
    Create {
        /// Human-readable name for the fleet.
        #[arg(long)]
        name: String,
        /// The backend the Director is actually running on.
        #[arg(long = "coding-agent", value_parser = ["claude", "codex", "opencode"])]
        coding_agent: String,
        /// UTF-8 file whose contents are the monitor's spawn prompt (`-` = stdin).
        #[arg(long = "monitor-file", value_name = "PATH")]
        monitor_file: String,
        /// Model passed to the monitor's backend binary.
        #[arg(long = "monitor-model", value_name = "MODEL")]
        monitor_model: Option<String>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// List non-deleted fleets.
    List {
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one fleet (soft-deleted included).
    Show {
        /// The fleet to show.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Soft-delete a fleet and deregister its members.
    Delete {
        /// The fleet to delete.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

pub fn run(
    slot: &mut Option<Connection>,
    settings: &Settings,
    command: FleetCommand,
) -> Result<(), CafleetError> {
    match command {
        FleetCommand::Create {
            name,
            coding_agent,
            monitor_file,
            monitor_model,
            json,
        } => create(
            slot,
            settings,
            &name,
            &coding_agent,
            &monitor_file,
            monitor_model.as_deref(),
            json,
        ),
        FleetCommand::List { json } => {
            list(slot.as_mut().expect("open invocation connection"), json)
        }
        FleetCommand::Show { fleet_id, json } => show(
            slot.as_mut().expect("open invocation connection"),
            fleet_id,
            json,
        ),
        FleetCommand::Delete { fleet_id, json } => delete(
            slot.as_mut().expect("open invocation connection"),
            fleet_id,
            json,
        ),
    }
}

/// The atomic bootstrap ladder (SPEC §6.3 *fleet group* → create): multiplexer
/// preconditions → monitor prompt resolution → backend checks → the broker's
/// single transaction, whose spawn callback substitutes the identity
/// placeholders and splits the monitor pane. A post-spawn failure kills the
/// recorded pane after DB rollback. Cleanup failures remain in the diagnostic.
fn create(
    slot: &mut Option<Connection>,
    settings: &Settings,
    name: &str,
    agent_name: &str,
    monitor_file: &str,
    monitor_model: Option<&str>,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet = create_with_options(
        slot,
        &FleetCreateOptions {
            name,
            agent_name,
            monitor_file,
            monitor_model,
        },
        || resolve_mux(settings),
        &SystemProbe,
        &SpawnPreparation {
            cwd: &std::env::current_dir,
            env: &|key| std::env::var(key).ok(),
        },
        &SpawnExecution {
            clock: &SystemClock,
            runner: &SystemRunner,
        },
    )?;
    emit(json, &fleet, || format_fleet_create(&fleet));
    Ok(())
}

pub(crate) fn create_with_options<M: SpawnMultiplexer>(
    slot: &mut Option<Connection>,
    options: &FleetCreateOptions<'_>,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe,
    preparation: &SpawnPreparation<'_>,
    execution: &SpawnExecution<'_>,
) -> Result<Value, CafleetError> {
    let FleetCreateOptions {
        name,
        agent_name,
        monitor_file,
        monitor_model,
    } = *options;

    let inside_session = || {
        CafleetError::App(
            "cafleet fleet create must be run inside a tmux or herdr session".to_string(),
        )
    };
    let mux = resolve_mux().map_err(|_| inside_session())?;
    mux.ensure_available().map_err(|_| inside_session())?;
    let context = mux.context_discovery().map_err(|_| inside_session())?;

    let prompt_body = resolve_body(None, Some(monitor_file), "--monitor-file")?;

    let backend = coding_agent(agent_name)
        .unwrap_or_else(|| panic!("'{agent_name}' is a registry-validated backend"));
    backend.validate_model(monitor_model)?;
    backend.ensure_available(probe)?;

    let plan = PreparedSpawn::prepare(
        prompt_body,
        backend,
        MONITOR_NAME,
        monitor_model,
        None,
        mux.name(),
        preparation,
    )?;
    let conn = slot
        .as_mut()
        .ok_or_else(|| CafleetError::App("database connection is closed".into()))?;
    let mut spawned_pane: Option<PaneGuard<'_>> = None;
    let bootstrap = broker::fleets::create_fleet(
        conn,
        Some(name),
        &context.session,
        &context.window_id,
        &context.pane_id,
        agent_name,
        mux.name(),
        MONITOR_NAME,
        MONITOR_DESCRIPTION,
        |fleet_id, director_id, monitor_id| {
            let deadline = Deadline::after(execution.clock, Duration::from_secs(30));
            let argv = plan.render(fleet_id, monitor_id, director_id)?;
            let pane_id = mux
                .split_prepared(
                    &PaneSpawnRequest {
                        reference: &context,
                        env: &plan.env,
                        command: &argv,
                        cwd: plan.cwd.as_deref(),
                    },
                    &deadline,
                    execution,
                )
                .map_err(|error| CafleetError::App(format!("tmux split-window failed: {error}")))?;
            spawned_pane = Some(PaneGuard::with_kill(
                pane_id.clone(),
                Box::new(|id| {
                    mux.kill_pane_with_deadline(
                        id,
                        true,
                        &Deadline::after(execution.clock, Duration::from_secs(5)),
                        execution,
                    )
                }),
            ));
            Ok(pane_id)
        },
    );
    let fleet = match bootstrap {
        Ok(fleet) => fleet,
        Err(error) => {
            // The broker has ended its transaction scope. Close our DB handle
            // before external cleanup, including when explicit rollback failed.
            drop(slot.take());
            return Err(match &mut spawned_pane {
                Some(pane) => pane.rollback(error),
                None => error,
            });
        }
    };
    if let Some(pane) = &mut spawned_pane {
        pane.finish();
    }
    Ok(fleet)
}

fn list(conn: &mut Connection, json: bool) -> Result<(), CafleetError> {
    let fleets = broker::list_fleets(conn)?;
    emit(json, &Value::Array(fleets.clone()), || {
        if fleets.is_empty() {
            return "No fleets found.".to_string();
        }
        let cell = |value: &Value| match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut lines = vec![format!(
            "{:<40}{:<40}{:<20}{:<8}{}",
            "FLEET_ID", "DIRECTOR", "NAME", "MEMBERS", "CREATED_AT"
        )];
        for fleet in &fleets {
            lines.push(format!(
                "{:<40}{:<40}{:<20}{:<8}{}",
                cell(&fleet["fleet_id"]),
                cell(&fleet["director_member_id"]),
                cell(&fleet["name"]),
                cell(&fleet["member_count"]),
                cell(&fleet["created_at"]),
            ));
        }
        lines.join("\n")
    });
    Ok(())
}

fn show(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let fleet = broker::get_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    emit(json, &fleet, || {
        let scalar = |value: &Value| match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut lines = vec![
            format!("fleet_id:   {}", scalar(&fleet["fleet_id"])),
            format!("name:       {}", scalar(&fleet["name"])),
            format!("created_at: {}", scalar(&fleet["created_at"])),
        ];
        if !fleet["deleted_at"].is_null() {
            lines.push(format!("deleted_at: {}", scalar(&fleet["deleted_at"])));
        }
        lines.join("\n")
    });
    Ok(())
}

fn delete(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let result = broker::delete_fleet(conn, fleet_id)?;
    emit(json, &result, || {
        format!(
            "Deleted fleet {fleet_id}. Deregistered {} members.",
            result["deregistered_count"]
        )
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::FleetCommand;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: FleetCommand,
    }

    fn parse(args: &[&str]) -> Result<FleetCommand, clap::Error> {
        Harness::try_parse_from(args).map(|harness| harness.command)
    }

    fn create_monitor_flags(command: FleetCommand) -> (String, Option<String>) {
        let FleetCommand::Create {
            monitor_file,
            monitor_model,
            ..
        } = command
        else {
            panic!("parsed a non-create command");
        };
        (monitor_file, monitor_model)
    }

    #[test]
    fn create_requires_the_monitor_file() {
        let Err(err) = parse(&[
            "cafleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
        ]) else {
            panic!("--monitor-file is required");
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        assert!(
            err.to_string().contains("--monitor-file"),
            "the error names the missing flag, got: {err}"
        );
    }

    #[test]
    fn create_parses_without_a_monitor_model() {
        let command = parse(&[
            "cafleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
            "--monitor-file",
            "prompt.md",
        ])
        .unwrap();
        let (monitor_file, monitor_model) = create_monitor_flags(command);
        assert_eq!(monitor_file, "prompt.md");
        assert_eq!(monitor_model, None);
    }

    #[test]
    fn create_accepts_stdin_as_the_monitor_file_and_a_monitor_model() {
        let command = parse(&[
            "cafleet",
            "create",
            "--name",
            "alpha",
            "--coding-agent",
            "claude",
            "--monitor-file",
            "-",
            "--monitor-model",
            "haiku",
        ])
        .unwrap();
        let (monitor_file, monitor_model) = create_monitor_flags(command);
        assert_eq!(monitor_file, "-", "the stdin sentinel is an ordinary value");
        assert_eq!(monitor_model.as_deref(), Some("haiku"));
    }
}
