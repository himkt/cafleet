//! The `fleet` group (SPEC §6.3 *fleet group*) — the atomic fleet + Director
//! + monitor bootstrap, list, show, and soft-delete.

use rusqlite::Connection;

use clap::Subcommand;
use serde_json::Value;

use super::creation::{CreationHooks, NoopCreationHooks, PaneGuard};
use super::helpers::{emit, resolve_body, resolve_mux};
use crate::broker;
use crate::coding_agent::{SpawnProbe, coding_agent};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerError};
use crate::output::format_fleet_create;
use crate::runtime::system::SystemProbe;
use crate::spawn_prompt::substitute_spawn_placeholders;

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
    let fleet = create_with_connection(
        slot,
        name,
        agent_name,
        monitor_file,
        monitor_model,
        || resolve_mux(settings),
        &SystemProbe,
        &NoopCreationHooks,
    )?;
    emit(json, &fleet, || format_fleet_create(&fleet));
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn create_with_dependencies<M: Multiplexer>(
    settings: &Settings,
    name: &str,
    agent_name: &str,
    monitor_file: &str,
    monitor_model: Option<&str>,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe,
    hooks: &dyn CreationHooks,
) -> Result<Value, CafleetError> {
    let mut slot = Some(crate::db::connect(&settings.database_url)?);
    create_with_connection(
        &mut slot,
        name,
        agent_name,
        monitor_file,
        monitor_model,
        resolve_mux,
        probe,
        hooks,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_with_connection<M: Multiplexer>(
    slot: &mut Option<Connection>,
    name: &str,
    agent_name: &str,
    monitor_file: &str,
    monitor_model: Option<&str>,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe,
    hooks: &dyn CreationHooks,
) -> Result<Value, CafleetError> {
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

    let conn = slot
        .as_mut()
        .ok_or_else(|| CafleetError::App("database connection is closed".into()))?;
    let mut spawned_pane: Option<PaneGuard<'_>> = None;
    let bootstrap = broker::fleets::create_fleet_with_hooks(
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
            let rendered = substitute_spawn_placeholders(
                &prompt_body,
                fleet_id,
                monitor_id,
                director_id,
                agent_name,
            )?;
            let argv = backend.build_spawn_argv(&rendered, MONITOR_NAME, monitor_model, None);
            let mut env = Vec::new();
            if let Ok(url) = std::env::var("CAFLEET_DATABASE_URL") {
                env.push(("CAFLEET_DATABASE_URL".to_string(), url));
            }
            let pane_id = mux
                .split_window(&context, &env, &argv)
                .map_err(|error| CafleetError::App(format!("tmux split-window failed: {error}")))?;
            spawned_pane = Some(PaneGuard::new(&mux, pane_id.clone(), hooks));
            Ok(pane_id)
        },
        hooks,
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

#[cfg(test)]
mod creation_regressions {
    use super::*;
    use crate::broker::fleets::BootstrapEvent;
    use crate::cli::creation::test_support::{Event, Fixture};
    use crate::coding_agent::test_support::FakeProbe;
    use crate::multiplexer::{AnyMultiplexer, RunError};

    fn create(f: &Fixture, mux: AnyMultiplexer, prompt: &str) -> Result<Value, CafleetError> {
        let path = f.dir.path().join("monitor.md");
        std::fs::write(&path, prompt).unwrap();
        create_with_dependencies(
            &f.settings,
            "fixture",
            "claude",
            path.to_str().unwrap(),
            None,
            || Ok(mux),
            &FakeProbe::with_binary("claude", f.dir.path()),
            f,
        )
    }

    fn invocation_slot(f: &Fixture) -> Option<Connection> {
        let conn = f.conn();
        conn.execute_batch("CREATE TEMP TABLE fleet_invocation_sentinel(value TEXT); INSERT INTO fleet_invocation_sentinel VALUES ('retained invocation')").unwrap();
        Some(conn)
    }

    fn assert_retained_slot(slot: &Option<Connection>) {
        let conn = slot
            .as_ref()
            .expect("the invocation still owns its connection");
        assert_eq!(
            conn.query_row(
                "SELECT value FROM temp.fleet_invocation_sentinel",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "retained invocation"
        );
        assert!(conn.is_autocommit());
    }

    fn create_in_slot(
        f: &Fixture,
        slot: &mut Option<Connection>,
        mux: AnyMultiplexer,
        prompt: &str,
    ) -> Result<Value, CafleetError> {
        let path = f.dir.path().join("monitor.md");
        std::fs::write(&path, prompt).unwrap();
        create_with_connection(
            slot,
            "fixture",
            "claude",
            path.to_str().unwrap(),
            None,
            || Ok(mux),
            &FakeProbe::with_binary("claude", f.dir.path()),
            f,
        )
    }

    #[test]
    fn precondition_failure_retains_the_invocation_slot_and_temp_state() {
        let f = Fixture::new(false);
        let mut slot = invocation_slot(&f);
        let error = create_in_slot(
            &f,
            &mut slot,
            f.mux(
                Some((
                    "current",
                    RunError::Failed {
                        stderr: "context unavailable".into(),
                    },
                )),
                false,
                false,
            ),
            "prompt",
        )
        .unwrap_err();
        assert_eq!(
            error.message(),
            "cafleet fleet create must be run inside a tmux or herdr session"
        );
        assert_retained_slot(&slot);
        assert_eq!(f.timeline(), ["current:error"]);
        f.assert_empty_bootstrap();
    }

    #[test]
    fn callback_placeholder_failure_rolls_back_and_preserves_usage_with_synthetic_diagnostic() {
        for diagnostic in [false, true] {
            let mut f = Fixture::new(false);
            f.diagnostic = diagnostic;
            let error = create(&f, f.mux(None, false, false), "{unknown}").unwrap_err();
            assert!(matches!(error, CafleetError::Usage(_)));
            assert_eq!(error.exit_code(), 2);
            assert!(
                error
                    .to_string()
                    .starts_with("Unknown placeholder 'unknown'")
            );
            assert_eq!(
                error.to_string().contains(
                    "cleanup failed for fleet 1 transaction: synthetic rollback diagnostic"
                ),
                diagnostic
            );
            assert_eq!(
                f.timeline(),
                ["current:ok", "begin", "rollback:ok", "after-real-rollback"]
            );
            f.assert_empty_bootstrap();
        }
    }

    #[test]
    fn backend_close_precedes_real_rollback_on_callback_run_failure() {
        let failure = RunError::Failed {
            stderr: "primary run failure".into(),
        };
        for close_fails in [false, true] {
            let f = Fixture::new(false);
            let mut slot = invocation_slot(&f);
            let error = create_in_slot(
                &f,
                &mut slot,
                f.mux(Some(("run", failure.clone())), close_fails, false),
                "prompt",
            )
            .unwrap_err();
            assert!(
                slot.is_none(),
                "a broker failure closes the invocation connection"
            );
            assert!(matches!(error, CafleetError::App(_)));
            assert_eq!(error.exit_code(), 1);
            assert_eq!(
                f.timeline(),
                [
                    "current:ok",
                    "begin",
                    "list:ok",
                    "split:ok",
                    "run:error",
                    "get:error",
                    "write-lock:false",
                    if close_fails {
                        "close:error"
                    } else {
                        "close:ok"
                    },
                    "rollback:ok",
                    "after-real-rollback"
                ]
            );
            assert_eq!(
                error
                    .to_string()
                    .matches("cleanup failed for pane w1:p9:")
                    .count(),
                usize::from(close_fails)
            );
            f.assert_empty_bootstrap();
        }
    }

    #[test]
    fn unknown_split_or_transport_failure_rolls_back_without_guessed_close() {
        for failed_split in [false, true] {
            let f = Fixture::new(false);
            let failure = failed_split.then_some((
                "split",
                RunError::Failed {
                    stderr: "split transport failed".into(),
                },
            ));
            let error = create(&f, f.mux(failure, false, true), "prompt").unwrap_err();
            assert_eq!(error.exit_code(), 1);
            assert!(
                error
                    .to_string()
                    .contains("pane ID unknown; pane cleanup unconfirmed")
            );
            assert_eq!(
                f.timeline(),
                [
                    "current:ok",
                    "begin",
                    "list:ok",
                    if failed_split {
                        "split:error"
                    } else {
                        "split:ok"
                    },
                    "rollback:ok",
                    "after-real-rollback"
                ]
            );
            f.assert_empty_bootstrap();
        }
    }

    #[test]
    fn placement_insert_failure_releases_db_lock_before_cli_pane_cleanup() {
        for rollback_in_sqlite in [false, true] {
            for close_fails in [false, true] {
                let f = Fixture::new(false);
                let action = if rollback_in_sqlite {
                    "ROLLBACK"
                } else {
                    "ABORT"
                };
                f.sql(&format!("CREATE TRIGGER fail_insert BEFORE INSERT ON member_placements WHEN NEW.member_id=2 BEGIN SELECT RAISE({action}, 'primary placement failure'); END;"));
                let mut slot = invocation_slot(&f);
                let error =
                    create_in_slot(&f, &mut slot, f.mux(None, close_fails, false), "prompt")
                        .unwrap_err();
                assert!(
                    slot.is_none(),
                    "broker failure takes and closes the invocation connection"
                );
                assert_eq!(error.exit_code(), 1);
                assert!(error.to_string().contains("primary placement failure"));
                assert_eq!(
                    f.timeline(),
                    [
                        "current:ok",
                        "begin",
                        "list:ok",
                        "split:ok",
                        "run:ok",
                        "rollback:ok",
                        "after-real-rollback",
                        "get:error",
                        "write-lock:true",
                        if close_fails {
                            "close:error"
                        } else {
                            "close:ok"
                        },
                        if close_fails {
                            "cli-kill:error"
                        } else {
                            "cli-kill:ok"
                        },
                        "disarm:pane"
                    ]
                );
                f.assert_empty_bootstrap();
            }
        }
    }

    #[test]
    fn commit_failure_rolls_back_before_cli_kill_even_with_synthetic_rollback_diagnostic() {
        for diagnostic in [false, true] {
            for close_fails in [false, true] {
                let mut f = Fixture::new(false);
                f.diagnostic = diagnostic;
                f.sql("CREATE TABLE compensation_parent(id INTEGER PRIMARY KEY);
                    CREATE TABLE compensation_child(id INTEGER REFERENCES compensation_parent(id) DEFERRABLE INITIALLY DEFERRED);
                    CREATE TRIGGER defer_commit_failure AFTER INSERT ON member_placements WHEN NEW.member_id=2
                    BEGIN INSERT INTO compensation_child VALUES (999); END;");
                let mut slot = invocation_slot(&f);
                let error =
                    create_in_slot(&f, &mut slot, f.mux(None, close_fails, false), "prompt")
                        .unwrap_err();
                assert!(
                    slot.is_none(),
                    "broker failure takes and closes the invocation connection"
                );
                assert_eq!(error.exit_code(), 1);
                let detail = error.to_string();
                assert!(
                    detail.starts_with("database error: FOREIGN KEY constraint failed"),
                    "{detail}"
                );
                assert_eq!(
                    f.timeline(),
                    [
                        "current:ok",
                        "begin",
                        "list:ok",
                        "split:ok",
                        "run:ok",
                        "commit:error",
                        "rollback:ok",
                        "after-real-rollback",
                        "get:error",
                        "write-lock:true",
                        if close_fails {
                            "close:error"
                        } else {
                            "close:ok"
                        },
                        if close_fails {
                            "cli-kill:error"
                        } else {
                            "cli-kill:ok"
                        },
                        "disarm:pane"
                    ]
                );
                assert_eq!(
                    detail
                        .matches(
                            "cleanup failed for fleet 1 transaction: synthetic rollback diagnostic"
                        )
                        .count(),
                    usize::from(diagnostic)
                );
                assert_eq!(
                    detail.matches("cleanup failed for pane w1:p9:").count(),
                    usize::from(close_fails)
                );
                if diagnostic && close_fails {
                    assert!(
                        detail.find("synthetic rollback diagnostic").unwrap()
                            < detail.find("cleanup failed for pane w1:p9:").unwrap()
                    );
                }
                assert!(!detail.contains("Rolled back"));
                f.assert_empty_bootstrap();
                assert_eq!(f.count("compensation_child"), 0);
            }
        }
    }

    #[test]
    fn failure_before_fleet_id_allocation_does_not_invent_an_id_in_cleanup_diagnostic() {
        let mut f = Fixture::new(false);
        f.diagnostic = true;
        f.sql("CREATE TRIGGER fail_fleet BEFORE INSERT ON fleets BEGIN SELECT RAISE(ABORT, 'primary fleet insert failure'); END;");
        let error = create(&f, f.mux(None, false, false), "prompt").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert_eq!(
            error.to_string(),
            "database error: primary fleet insert failure\ncleanup failed for fleet unknown transaction: synthetic rollback diagnostic"
        );
        assert_eq!(
            f.timeline(),
            ["current:ok", "begin", "rollback:ok", "after-real-rollback"]
        );
        assert!(
            f.events
                .borrow()
                .contains(&Event::Bootstrap(BootstrapEvent::RollbackFinished {
                    fleet_id: None,
                    error: None,
                    autocommit: true
                }))
        );
        f.assert_empty_bootstrap();
    }

    #[test]
    fn commit_success_disarms_pane_before_the_caller_can_emit_output() {
        let f = Fixture::new(false);
        let mut slot = invocation_slot(&f);
        let value = create_in_slot(&f, &mut slot, f.mux(None, false, false), "prompt").unwrap();
        assert_retained_slot(&slot);
        assert_eq!(
            slot.as_ref()
                .unwrap()
                .query_row("SELECT count(*) FROM fleets", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(value["monitor"]["placement"]["mux_pane_id"], "w1:p9");
        assert_eq!(
            f.timeline(),
            [
                "current:ok",
                "begin",
                "list:ok",
                "split:ok",
                "run:ok",
                "commit:ok",
                "disarm:pane"
            ]
        );
        assert_eq!(f.count("fleets"), 1);
        assert_eq!(f.count("members"), 2);
        assert_eq!(f.count("member_placements"), 2);
    }
}
