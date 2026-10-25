use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::creation::RegistrationGuard;

use super::helpers::{emit, resolve_body, resolve_command_body, resolve_mux};
use crate::broker::records::MemberRecord;
use crate::broker::{self, NewPlacement};
use crate::capture::{CaptureSnapshot, MemberCapture, write_member_capture};
use crate::coding_agent::{CodingAgent, coding_agent};
use crate::config::Settings;
use crate::error::CafleetError;

use crate::multiplexer::{AnyMultiplexer, Lead, PaneCleanup, PaneOwnership};
use crate::output::{format_member, format_member_detail, format_member_list};
use crate::presentation;
use crate::runtime::system::SystemProbe;
use crate::time::now_utc;

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct PromptArgs {
    /// Inline spawn prompt (backend-neutral template). Exactly one of
    /// PROMPT / --file.
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,
    /// UTF-8 file whose contents are the spawn prompt (`-` = stdin).
    #[arg(long, value_name = "PATH")]
    file: Option<String>,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct ExecBodyArgs {
    /// The command to run. Exactly one of COMMAND / --file.
    #[arg(value_name = "COMMAND")]
    command: Option<String>,
    /// UTF-8 file whose contents are the command (`-` = stdin).
    #[arg(long, value_name = "PATH")]
    file: Option<String>,
}

#[derive(Subcommand)]
pub enum MemberCommand {
    /// Register a member and spawn its coding-agent pane.
    Create {
        /// The fleet the new member joins.
        #[arg(long = "fleet-id", value_name = "INT")]
        fleet_id: i64,
        /// Display name.
        #[arg(long)]
        name: String,
        /// One-sentence purpose.
        #[arg(long)]
        description: String,
        /// Backend binary to spawn / record [default: inherits the Director's backend].
        #[arg(long = "coding-agent", value_enum)]
        coding_agent: Option<CodingAgent>,
        /// Model passed to the backend binary.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning-effort level (claude, codex only).
        #[arg(long)]
        effort: Option<String>,
        #[command(flatten)]
        body: PromptArgs,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Tear down a member's pane (when one exists) and deregister it.
    Delete {
        /// The member to delete.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one member's detail.
    Show {
        /// The member to show.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// List every active registry entry of the fleet.
    List {
        /// The fleet whose roster is listed.
        #[arg(value_name = "FLEET_ID")]
        fleet_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Keystroke a prompt into a member's pane.
    Prompt {
        /// The target member.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Single line of text to dispatch.
        #[arg(value_name = "TEXT")]
        text: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Submit a native shell command to a member's pane, Escape first.
    Exec {
        /// The target member.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        #[command(flatten)]
        body: ExecBodyArgs,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Capture the tail of a member's pane.
    Capture {
        /// The target member.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Number of trailing lines to capture.
        #[arg(long, default_value_t = 20)]
        lines: i64,
        /// Emit the raw capture, ANSI escapes preserved.
        #[arg(long)]
        ansi: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

fn require_pane(
    member: &MemberRecord,
    member_id: i64,
    action: &str,
) -> Result<String, CafleetError> {
    member
        .placement
        .as_ref()
        .and_then(|placement| placement.mux_pane_id.clone())
        .ok_or_else(|| {
            CafleetError::App(format!(
                "member {member_id} has no pane yet (pending placement) — nothing to {action}."
            ))
        })
}

fn load_member(
    conn: &Connection,
    member_id: i64,
    tolerate_missing_placement: bool,
) -> Result<MemberRecord, CafleetError> {
    let fleet_id = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    let member = broker::get_member(conn, member_id, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    if member.placement.is_none() && !tolerate_missing_placement {
        return Err(CafleetError::App(format!(
            "member {member_id} has no placement row; it was not spawned via \
             `cafleet member create`."
        )));
    }
    Ok(member)
}

fn resolve_coding_agent(
    conn: &Connection,
    fleet_id: i64,
    director_id: i64,
    explicit: Option<CodingAgent>,
) -> Result<CodingAgent, CafleetError> {
    if let Some(agent) = explicit {
        return Ok(agent);
    }
    let director = broker::get_member(conn, director_id, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("Director (member {director_id}) not found.")))?;
    let placement = director.placement.ok_or_else(|| {
        CafleetError::App(format!(
            "Director (member {director_id}) has no placement; supply --coding-agent."
        ))
    })?;
    coding_agent(&placement.coding_agent).ok_or(CafleetError::InvalidStoredValue {
        field: "member_placements.coding_agent".into(),
        value: placement.coding_agent,
    })
}

pub(crate) fn prepare_input(command: &mut MemberCommand) -> Result<(), CafleetError> {
    if let MemberCommand::Exec { body, .. } = command {
        body.command = Some(resolve_command_body(
            body.command.as_deref(),
            body.file.as_deref(),
        )?);
        body.file = None;
    }
    Ok(())
}

pub fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: MemberCommand,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    match command {
        MemberCommand::Create {
            fleet_id,
            name,
            description,
            coding_agent,
            model,
            effort,
            body,
            json,
        } => create(
            conn,
            settings,
            fleet_id,
            &name,
            &description,
            coding_agent,
            model.as_deref(),
            effort.as_deref(),
            &body,
            json,
        ),
        MemberCommand::Delete { member_id, json } => {
            delete(conn, settings, member_id, json, deadline)
        }
        MemberCommand::Show { member_id, json } => show(conn, member_id, json),
        MemberCommand::List { fleet_id, json } => list(conn, fleet_id, json),
        MemberCommand::Prompt {
            member_id,
            text,
            json,
        } => prompt(conn, settings, member_id, &text, json, deadline),
        MemberCommand::Exec {
            member_id,
            body,
            json,
        } => exec(conn, settings, member_id, &body, json, deadline),
        MemberCommand::Capture {
            member_id,
            lines,
            ansi,
            json,
        } => capture(conn, settings, member_id, lines, ansi, json),
    }
}

#[allow(clippy::too_many_arguments)]
fn create(
    conn: &mut Connection,
    settings: &Settings,
    fleet_id: i64,
    name: &str,
    description: &str,
    explicit_agent: Option<CodingAgent>,
    model: Option<&str>,
    effort: Option<&str>,
    body: &PromptArgs,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet = broker::fleets::fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?;
    if fleet.deleted_at.is_some() {
        return Err(CafleetError::App(format!("fleet {fleet_id} is deleted")));
    }
    let Some(director_id) = fleet.director_member_id else {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id} has no root Director recorded; re-create the fleet \
             with 'cafleet fleet create'."
        )));
    };
    let backend = resolve_coding_agent(conn, fleet_id, director_id, explicit_agent)?;
    let agent_name = backend.name();

    backend.validate_model(model)?;
    backend.validate_effort(effort)?;

    let prompt_body = resolve_body(body.prompt.as_deref(), body.file.as_deref(), "--file")?;

    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    backend.ensure_available(&SystemProbe)?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let env: Vec<_> = std::env::var("CAFLEET_DATABASE_URL")
        .ok()
        .map(|value| ("CAFLEET_DATABASE_URL".to_string(), value))
        .into_iter()
        .collect();

    let placement = NewPlacement {
        backend: mux.name().to_string(),
        mux_session: context.session.clone(),
        mux_window_id: context.window_id.clone(),
        mux_pane_id: None,
        coding_agent: agent_name.into(),
    };
    let registered = broker::register_member(conn, fleet_id, name, description, Some(&placement))
        .map_err(|error| match error {
        CafleetError::App(_) | CafleetError::Usage(_) => error,
        other => CafleetError::App(format!("register failed: {}", other.message())),
    })?;
    let member_id = registered.member_id;
    let mut registration = RegistrationGuard::new(conn, member_id);
    let prompt = match crate::spawn_prompt::substitute_spawn_placeholders(
        &prompt_body,
        fleet_id,
        member_id,
        director_id,
        agent_name,
    ) {
        Ok(prompt) => prompt,
        Err(original) => return Err(registration.rollback(original)),
    };
    let argv = backend.build_spawn_argv(&prompt, name, model, effort);
    let span =
        crate::signals::DeferTermination::begin().map_err(|error| registration.rollback(error))?;
    let pane_id = match mux.split_window(&context, &env, &argv) {
        Ok(pane_id) => pane_id,
        Err(error) => {
            return Err(registration.rollback(CafleetError::App(format!(
                "tmux split-window failed: {error}"
            ))));
        }
    };
    let placement_view = publish_pane(&mut registration, member_id, &pane_id, &mux)?;
    drop(span);

    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered.registered_at,
        "placement": presentation::placement(&placement_view),
    });
    emit(json, &result, || {
        format_member(&registered, &placement_view)
    });
    Ok(())
}

fn publish_pane(
    registration: &mut RegistrationGuard<'_>,
    member_id: i64,
    pane_id: &str,
    mux: &AnyMultiplexer,
) -> Result<crate::broker::records::Placement, CafleetError> {
    let mut pane = PaneOwnership::new(pane_id.to_owned(), |id| mux.kill_pane(id, true));

    let patched = broker::update_placement_pane_id(registration.connection(), member_id, pane_id);
    let placement_view = match patched {
        Ok(Some(placement)) => placement,
        outcome => {
            let primary = match outcome {
                Err(error) => {
                    CafleetError::App(format!("placement update failed: {}", error.message()))
                }
                _ => CafleetError::App("placement row vanished before pane-id patch".into()),
            };
            let primary = match pane.rollback() {
                PaneCleanup::Attempted {
                    pane_id,
                    error: Some(error),
                } => primary.with_cleanup(format!("cleanup failed for pane {pane_id}: {error}")),
                PaneCleanup::Attempted { error: None, .. } => primary,
                PaneCleanup::Unknown => unreachable!("the CLI owns a concrete pane"),
            };
            return Err(registration.rollback(primary));
        }
    };
    pane.finish();
    registration.finish();
    Ok(placement_view)
}

enum DeleteAction {
    Deregistered(&'static str),
    Close(String),
}

fn lock_member_deletion(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    deadline: crate::input::Deadline,
) -> Result<crate::input::LockedDeletion<MemberRecord, DeleteAction>, CafleetError> {
    crate::input::lock_deletion(
        conn,
        settings,
        deadline,
        |conn| {
            let member = load_member(conn, member_id, true)?;
            let targets = member
                .placement
                .as_ref()
                .filter(|placement| placement.mux_pane_id.is_some())
                .map(|placement| vec![(member_id, placement.clone())])
                .unwrap_or_default();
            Ok((member, targets))
        },
        |conn, snapshot| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::broker::members::db_err)?;
            let current = load_member(&tx, member_id, true)?;
            if current.placement != snapshot.placement {
                return Ok(None);
            }
            if current.kind == crate::broker::records::MemberKind::Director {
                return Err(CafleetError::App(
                    "cannot deregister the root Director; use 'cafleet fleet delete' instead"
                        .into(),
                ));
            }
            let pane = current
                .placement
                .as_ref()
                .and_then(|placement| placement.mux_pane_id.as_deref());
            let action = if let Some(pane_id) = pane {
                DeleteAction::Close(pane_id.to_owned())
            } else {
                let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
                assert!(changed, "the revalidated member remains active");
                DeleteAction::Deregistered(if current.placement.is_none() {
                    "(no placement)"
                } else {
                    "(pending — no pane)"
                })
            };
            tx.commit().map_err(crate::broker::members::db_err)?;
            Ok(Some(action))
        },
        |_| {
            CafleetError::App(format!(
                "member {member_id}'s pane input remained busy for 5 seconds; member was not deleted."
            ))
        },
    )
}

fn delete(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let locked = lock_member_deletion(conn, settings, member_id, deadline)?;
    let pane_status = match &locked.value {
        DeleteAction::Deregistered(status) => status.to_string(),
        DeleteAction::Close(_) => {
            let mux = resolve_mux(settings)
                .map_err(|error| CafleetError::App(error.to_string()))?
                .bounded(deadline);
            mux.ensure_available()
                .map_err(|error| CafleetError::App(error.to_string()))?;
            close_member(conn, member_id, &locked, &mux, deadline)?
        }
    };
    drop(locked);
    emit(
        json,
        &json!({"member_id":member_id, "pane_status":pane_status}),
        || format!("Member deleted.\n  member_id: {member_id}\n  pane_id:   {pane_status}"),
    );
    Ok(())
}

fn close_member(
    conn: &mut Connection,
    member_id: i64,
    locked: &crate::input::LockedDeletion<MemberRecord, DeleteAction>,
    mux: &AnyMultiplexer,
    deadline: crate::input::Deadline,
) -> Result<String, CafleetError> {
    let DeleteAction::Close(pane_id) = &locked.value else {
        unreachable!("pane close requires a concrete input-locked placement");
    };
    let layout = mux.close_pane(pane_id, true).map_err(|error| {
        CafleetError::App(format!(
            "kill_pane failed for pane {pane_id}: {error}. Verify with 'cafleet doctor'."
        ))
    })?;
    crate::input::complete_database_budget(conn, deadline, |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::broker::members::db_err)?;
        let current = load_member(&tx, member_id, true)?;
        if current.placement != locked.snapshot.placement {
            return Err(CafleetError::App(
                "member placement changed after pane close".into(),
            ));
        }
        let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
        assert!(
            changed,
            "the input-locked member stays active until deregistration"
        );
        tx.commit().map_err(crate::broker::members::db_err)
    })
    .map_err(|error| {
        CafleetError::App(format!(
            "pane {pane_id} was closed, but deregister failed: {}",
            error.message()
        ))
    })?;
    if let Err(error) = mux.restore_after_close(layout) {
        eprintln!(
            "Member {member_id} was deregistered; optional layout restoration failed: {error}"
        );
    }
    Ok(format!("{pane_id} (killed)"))
}

fn show(conn: &mut Connection, member_id: i64, json: bool) -> Result<(), CafleetError> {
    let member = load_member(conn, member_id, true)?;
    let value = presentation::member(&member);
    emit(json, &value, || format_member_detail(&member));
    Ok(())
}

fn list(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let members = broker::list_members(conn, fleet_id)?;
    let payload = Value::Array(members.iter().map(presentation::member_activity).collect());
    emit(json, &payload, || format_member_list(&members));
    Ok(())
}

fn dispatch_input(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    text: &str,
    label: &str,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let id = member.member_id;
    let Some(locked) = crate::input::lock_member(conn, settings, member, deadline)? else {
        return Err(CafleetError::App(format!(
            "member {id}'s pane input remained busy; {label} was not dispatched."
        )));
    };
    let _span = crate::signals::DeferTermination::begin()?;
    let placement = locked.member.placement.as_ref().expect("locked placement");
    let pane = placement
        .mux_pane_id
        .as_deref()
        .expect("locked concrete pane");
    let prewrite = |cause| {
        CafleetError::App(format!(
            "{label} was not dispatched to member {id}: {cause}"
        ))
    };
    let mux = resolve_mux(settings)
        .map_err(|e| prewrite(e.to_string()))?
        .bounded(deadline);
    mux.ensure_available()
        .map_err(|e| prewrite(e.to_string()))?;
    mux.send_prompt(pane, text, Lead::Escape).map_err(|error| {
        if error.submission_started() { CafleetError::App(format!(
            "{label} delivery to member {id} is uncertain: {error}. Inspect the pane before retrying."
        )) } else { prewrite(error.to_string()) }
    })
}

fn prompt(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    text: &str,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    if !crate::input::valid_input(text) {
        return Err(CafleetError::Usage(
            "text must be one non-empty line without control characters.".into(),
        ));
    }
    let member = crate::input::with_database_budget(conn, deadline, |conn| {
        load_member(conn, member_id, false)
    })?;
    if member.kind == crate::broker::records::MemberKind::Director {
        return Err(CafleetError::App(
            "cannot prompt the Director's own pane".into(),
        ));
    }
    let pane = require_pane(&member, member_id, "prompt")?;
    dispatch_input(conn, settings, &member, text, "prompt", deadline)?;
    emit(
        json,
        &json!({"member_id":member_id,"pane_id":pane,"text":text}),
        || format!("Sent prompt {text:?} to member {} ({pane}).", member.name),
    );
    Ok(())
}

fn exec(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    body: &ExecBodyArgs,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let command = body
        .command
        .as_deref()
        .expect("exec input was resolved before its deadline");
    let member = crate::input::with_database_budget(conn, deadline, |conn| {
        load_member(conn, member_id, false)
    })?;
    if member.kind == crate::broker::records::MemberKind::Director {
        return Err(CafleetError::App(
            "cannot exec in the Director's own pane".into(),
        ));
    }
    let pane = require_pane(&member, member_id, "exec")?;
    dispatch_input(
        conn,
        settings,
        &member,
        &format!("! {command}"),
        "native command",
        deadline,
    )?;
    let agent = &member
        .placement
        .as_ref()
        .expect("placed member")
        .coding_agent;
    emit(
        json,
        &json!({"member_id":member_id,"pane_id":pane,"coding_agent":agent,
        "command":command,"dispatched":true}),
        || {
            format!(
                "Dispatched native command to member {member_id} ({pane}). Inspect the pane for its result."
            )
        },
    );
    Ok(())
}

fn capture(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    lines: i64,
    ansi: bool,
    json: bool,
) -> Result<(), CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let member = load_member(conn, member_id, false)?;
    let pane_id = require_pane(&member, member_id, "capture")?;
    let raw = mux
        .capture_pane(&pane_id, lines)
        .map_err(|e| CafleetError::App(format!("capture failed: {e}")))?;
    let capture = MemberCapture {
        member_id,
        pane_id,
        lines,
        snapshot: CaptureSnapshot::from_raw(&raw, ansi, now_utc()),
    };
    write_member_capture(&mut std::io::stdout(), &capture, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::tests as common;
    use crate::input::{Deadline, PaneLock};
    use crate::multiplexer::{CommandRunner, RunError, TmuxMultiplexer};
    use clap::Parser;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn fixture() -> (tempfile::TempDir, Connection, Settings, i64, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = common::migrated_conn(&dir);
        let settings = Settings {
            database_url: format!("sqlite:///{}", conn.path().unwrap()),
            max_text_len: 200,
            multiplexer: Some("tmux".into()),
        };
        let (fleet, director) = common::create_fleet(&mut conn, "deletion");
        let worker = common::register(&mut conn, fleet, "worker", None);
        (dir, conn, settings, fleet, director, worker)
    }

    struct CreateProbe {
        agent: CodingAgent,
        home: std::path::PathBuf,
    }

    impl crate::coding_agent::SpawnProbe for CreateProbe {
        fn binary_on_path(&self, name: &str) -> bool {
            assert_eq!(name, self.agent.binary_name());
            true
        }

        fn home_dir(&self) -> std::path::PathBuf {
            self.home.clone()
        }

        fn env_var(&self, name: &str) -> Option<String> {
            assert_eq!(name, self.agent.config_variable());
            Some(self.home.join("config").display().to_string())
        }
    }

    fn create_runtime(
        dir: &std::path::Path,
        agent: CodingAgent,
        spawn: bool,
    ) -> (
        Rc<crate::multiplexer::recording::FakeRunner>,
        AnyMultiplexer,
        CreateProbe,
    ) {
        use crate::multiplexer::recording::{FakeRunner, env};

        let config = dir.join("config");
        if let Some(preset) = agent.required_preset(&config) {
            std::fs::create_dir_all(preset.parent().unwrap()).unwrap();
            std::fs::write(preset, "test preset").unwrap();
        }
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("main|@1|%0\n".into()));
        if spawn {
            runner.respond(Ok("%9\n".into()));
            runner.succeed(1);
        }
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
            runner.clone(),
            env(&[("TMUX", "test-session"), ("TMUX_PANE", "%0")]),
        ));
        let probe = CreateProbe {
            agent,
            home: dir.to_owned(),
        };
        (runner, mux, probe)
    }

    fn assignment_body(dir: &std::path::Path, text: &str, from_file: bool) -> PromptArgs {
        if from_file {
            let path = dir.join("assignment.txt");
            std::fs::write(&path, text).unwrap();
            PromptArgs {
                prompt: None,
                file: Some(path.display().to_string()),
            }
        } else {
            PromptArgs {
                prompt: Some(text.into()),
                file: None,
            }
        }
    }

    fn assert_routing_preamble(preamble: &str, fleet: i64, member: i64, director: i64) {
        assert!(preamble.starts_with("MANDATORY CAFLEET COMMAND ROUTING\n"));
        for required in [
            "Execute task shell commands using the backend's supported automatic-approval mode.",
            "Route an observed permission denial to the Director with the exact command, working directory, required environment and actual refusal reason.",
            "Use authorized cafleet message communication commands for broker coordination.",
            "Use native file tools within your assigned role scope.",
            "Read prerequisites with native file tools or permitted shell commands before sending ready.",
            "Preserve the host permission and sandbox settings.",
            "Automatic approval review preserves native guards, explicit deny rules and managed policies.",
            "Surface an unavailable automatic mode or an unresolved approval to the Director.",
            "An ordinary nonzero command result follows normal error recovery.",
            "Director review follows existing host and operator authority; a routed request does not override a native or managed denial.",
        ] {
            assert!(
                preamble.contains(required),
                "missing routing rule: {required}"
            );
        }
        for identity in [
            format!("FLEET ID: {fleet}\n"),
            format!("YOUR MEMBER ID: {member}\n"),
            format!("DIRECTOR MEMBER ID: {director}\n"),
            format!("cafleet message send --from-member-id {member} --to-member-id {director} "),
        ] {
            assert!(
                preamble.contains(&identity),
                "missing broker identity: {identity}"
            );
        }
    }

    #[test]
    fn member_create_routes_inline_and_file_assignments_for_every_registered_backend() {
        let mut shared_preamble = None;
        for &agent in CodingAgent::all() {
            for (from_file, explicit) in [
                (false, Some(agent)),
                (true, Some(agent)),
                (false, None),
                (true, None),
            ] {
                let (dir, mut conn, _, _, _, _) = fixture();
                let (fleet, director) = common::create_fleet(&mut conn, "routing-identities");
                if explicit.is_none() {
                    conn.execute(
                        "UPDATE member_placements SET coding_agent=?1 WHERE member_id=?2",
                        rusqlite::params![agent.name(), director],
                    )
                    .unwrap();
                }
                let (runner, mux, probe) = create_runtime(dir.path(), agent, true);
                let body = assignment_body(
                    dir.path(),
                    "  Ad-hoc assignment\r\n界🙂\t\"quoted\" '$HOME; !'\n\
                     {{\"literal\": \"{{fleet_id}}\"}}\n\
                     fleet={fleet_id}; member={member_id}; director={director_member_id}; backend={coding_agent}\r\n",
                    from_file,
                );
                create_with_runtime(
                    &mut conn,
                    &mux,
                    &probe,
                    fleet,
                    "routing-worker",
                    "test assignment",
                    explicit,
                    None,
                    None,
                    &body,
                    true,
                )
                .unwrap();
                let members = broker::list_members(&conn, fleet).unwrap();
                let member = members
                    .iter()
                    .find(|member| member.member.name == "routing-worker")
                    .expect("created member is published");
                let member_id = member.member.member_id;
                let commands = runner.run_argvs();
                let split = commands
                    .iter()
                    .find(|argv| argv.get(1).map(String::as_str) == Some("split-window"))
                    .expect("member creation spawns through the multiplexer");
                let prompt = split
                    .last()
                    .expect("spawn argv ends with the literal prompt");
                let expected_body = format!(
                    "  Ad-hoc assignment\r\n界🙂\t\"quoted\" '$HOME; !'\n\
                     {{\"literal\": \"{{fleet_id}}\"}}\n\
                     fleet={fleet}; member={member_id}; director={director}; backend={}\r\n",
                    agent.name()
                );
                let preamble = prompt
                    .strip_suffix(&expected_body)
                    .expect("assignment bytes survive after exactly one placeholder substitution");
                if let Some(expected) = &shared_preamble {
                    assert_eq!(preamble, expected, "policy for {agent:?}, file={from_file}");
                } else {
                    assert_routing_preamble(preamble, fleet, member_id, director);
                    shared_preamble = Some(preamble.to_owned());
                }
                let binary = split
                    .iter()
                    .position(|arg| arg == agent.binary_name())
                    .expect("selected backend is the literal child executable");
                assert_eq!(
                    &split[binary..],
                    agent.build_spawn_argv(prompt, "routing-worker", None, None)
                );
                let published = broker::get_member(&conn, member_id, fleet)
                    .unwrap()
                    .unwrap();
                let placement = published.placement.unwrap();
                assert_eq!(placement.mux_pane_id.as_deref(), Some("%9"));
                assert_eq!(placement.coding_agent, agent.name());
                assert!(runner.responses.borrow().is_empty());
            }
        }
        assert!(
            shared_preamble.is_some(),
            "registered backends are exercised"
        );
    }

    #[test]
    fn member_create_rolls_back_invalid_assignments_before_spawning_for_every_backend() {
        for &agent in CodingAgent::all() {
            for from_file in [false, true] {
                for text in ["unknown {not_a_spawn_placeholder}", "unclosed {", "stray }"] {
                    let (dir, mut conn, _, fleet, director, worker) = fixture();
                    let (runner, mux, probe) = create_runtime(dir.path(), agent, false);
                    let body = assignment_body(dir.path(), text, from_file);
                    let expected = crate::spawn_prompt::substitute_spawn_placeholders(
                        text,
                        fleet,
                        0,
                        director,
                        agent.name(),
                    )
                    .unwrap_err();
                    let error = create_with_runtime(
                        &mut conn,
                        &mux,
                        &probe,
                        fleet,
                        "invalid-routing-worker",
                        "invalid assignment",
                        Some(agent),
                        None,
                        None,
                        &body,
                        true,
                    )
                    .unwrap_err();
                    assert_eq!(error.exit_code(), 2);
                    assert_eq!(error.message(), expected.message());
                    let ids: Vec<_> = broker::list_members(&conn, fleet)
                        .unwrap()
                        .iter()
                        .map(|member| member.member.member_id)
                        .collect();
                    assert_eq!(ids.len(), 2);
                    assert!(ids.contains(&director) && ids.contains(&worker));
                    assert_eq!(
                        conn.query_row("SELECT COUNT(*) FROM member_placements", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap(),
                        2
                    );
                    assert!(runner.run_argvs().iter().all(|argv| {
                        argv.get(1).map(String::as_str) == Some("display-message")
                    }));
                }
            }
        }
    }

    struct CloseRunner {
        check: Box<dyn Fn()>,
        calls: Cell<usize>,
        failure: Option<&'static str>,
    }

    impl CommandRunner for CloseRunner {
        fn binary_exists(&self, name: &str) -> bool {
            assert_eq!(name, "tmux");
            true
        }

        fn run(&self, argv: &[String], _: Option<u64>) -> Result<String, RunError> {
            assert_eq!(argv, &["tmux", "kill-pane", "-t", "%9"]);
            self.calls.set(self.calls.get() + 1);
            (self.check)();
            match self.failure {
                Some(cause) => Err(RunError::Failed {
                    stderr: cause.into(),
                }),
                None => Ok(String::new()),
            }
        }

        fn sleep(&self, _: f64) {
            panic!("pane deletion has no input settle");
        }
    }

    #[test]
    fn publication_transfers_owned_panes_or_compensates_each_failure_once() {
        for (failure, kill_fails, cleanup_fails) in [
            ("none", false, false),
            ("missing", false, false),
            ("update", false, false),
            ("update", true, false),
            ("update", true, true),
        ] {
            let (_dir, mut conn, _, fleet, _, worker) = fixture();
            if failure == "missing" {
                conn.execute("DELETE FROM member_placements WHERE member_id=?1", [worker])
                    .unwrap();
            } else if failure == "update" {
                conn.execute_batch(
                    "CREATE TRIGGER reject_publication BEFORE UPDATE ON member_placements
                     BEGIN SELECT RAISE(ABORT, 'publication blocked'); END;",
                )
                .unwrap();
            }
            if cleanup_fails {
                conn.execute_batch(
                    "CREATE TRIGGER reject_cleanup BEFORE UPDATE OF status ON members
                     BEGIN SELECT RAISE(ABORT, 'cleanup blocked'); END;",
                )
                .unwrap();
            }
            let runner = Rc::new(CloseRunner {
                calls: Cell::new(0),
                check: Box::new(|| {}),
                failure: kill_fails.then_some("close blocked"),
            });
            let mux =
                AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()));
            let mut registration = RegistrationGuard::new(&mut conn, worker);
            let result = publish_pane(&mut registration, worker, "%9", &mux);
            drop(registration);
            if failure == "none" {
                assert_eq!(result.unwrap().mux_pane_id.as_deref(), Some("%9"));
                assert_eq!(runner.calls.get(), 0);
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.exit_code(), 1);
                let text = error.to_string();
                assert!(text.contains("placement"), "{text}");
                assert_eq!(text.contains("close blocked"), kill_fails);
                assert_eq!(text.contains("cleanup blocked"), cleanup_fails);
                assert_eq!(runner.calls.get(), 1);
            }
            assert_eq!(
                broker::get_member(&conn, worker, fleet).unwrap().is_some(),
                failure == "none" || cleanup_fails
            );
        }
    }

    #[test]
    fn deletion_reloads_a_published_pending_pane_and_retains_ownership_through_completion() {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        for expire_after_close in [false, true] {
            let (_dir, mut conn, settings, fleet, director, worker) = fixture();
            let path = conn.path().unwrap().to_owned();
            let mut published = false;
            conn.authorizer(Some(move |context: AuthContext<'_>| {
                if !published
                    && matches!(
                        context.action,
                        AuthAction::Transaction {
                            operation: TransactionOperation::Begin
                        }
                    )
                {
                    published = true;
                    Connection::open(&path)
                        .unwrap()
                        .execute(
                            "UPDATE member_placements SET mux_pane_id='%9' WHERE member_id=?1",
                            [worker],
                        )
                        .unwrap();
                }
                Authorization::Allow
            }))
            .unwrap();
            let deadline = Deadline::until(Instant::now() + Duration::from_millis(100));
            let locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
            let placement = locked.snapshot.placement.clone().unwrap();
            assert_eq!(placement.mux_pane_id.as_deref(), Some("%9"));
            let url = settings.database_url.clone();
            let runner = Rc::new(CloseRunner {
                calls: Cell::new(0),
                failure: None,
                check: Box::new(move || {
                    assert!(PaneLock::try_acquire(&url, &placement).unwrap().is_none());
                    crate::db::connect(&url)
                        .unwrap()
                        .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
                        .expect("pane close holds no SQLite writer transaction");
                    if expire_after_close {
                        std::thread::sleep(Duration::from_millis(110));
                    }
                }),
            });
            let mux =
                AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()))
                    .bounded(deadline);
            assert_eq!(
                close_member(&mut conn, worker, &locked, &mux, deadline).unwrap(),
                "%9 (killed)"
            );
            assert_eq!(runner.calls.get(), 1);
            assert!(broker::get_member(&conn, worker, fleet).unwrap().is_none());
            assert!(
                broker::get_member(&conn, director, fleet)
                    .unwrap()
                    .is_some()
            );
            let placement = locked.snapshot.placement.as_ref().unwrap().clone();
            assert!(
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .is_none()
            );
            drop(locked);
            assert!(
                PaneLock::try_acquire(&settings.database_url, &placement)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn irreversible_close_preserves_the_registry_error_and_releases_ownership_on_return() {
        let (_dir, mut conn, settings, fleet, _, worker) = fixture();
        broker::update_placement_pane_id(&mut conn, worker, "%9").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_deregister BEFORE UPDATE OF status ON members
             BEGIN SELECT RAISE(ABORT, 'deregister blocked'); END;",
        )
        .unwrap();
        let deadline = Deadline::new();
        let locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
        let runner = Rc::new(CloseRunner {
            calls: Cell::new(0),
            failure: None,
            check: Box::new(|| {}),
        });
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()))
            .bounded(deadline);
        let error = close_member(&mut conn, worker, &locked, &mux, deadline)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("was closed") && error.contains("deregister blocked"),
            "{error}"
        );
        assert_eq!(runner.calls.get(), 1);
        assert!(broker::get_member(&conn, worker, fleet).unwrap().is_some());
    }

    #[test]
    fn root_targets_and_invalid_native_input_fail_before_multiplexer_effects() {
        let (_dir, mut conn, settings, _, director, worker) = fixture();
        assert!(
            prompt(
                &mut conn,
                &settings,
                director,
                "body",
                false,
                Deadline::new()
            )
            .unwrap_err()
            .to_string()
            .contains("Director's own pane")
        );
        let body = ExecBodyArgs {
            command: Some("echo literal".into()),
            file: None,
        };
        assert!(
            exec(
                &mut conn,
                &settings,
                director,
                &body,
                false,
                Deadline::new()
            )
            .unwrap_err()
            .to_string()
            .contains("Director's own pane")
        );
        assert!(lock_member_deletion(&mut conn, &settings, director, Deadline::new()).is_err());
        assert!(
            prompt(
                &mut conn,
                &settings,
                worker,
                "two\nlines",
                false,
                Deadline::new()
            )
            .is_err()
        );
        let pending = load_member(&conn, worker, false).unwrap();
        assert!(require_pane(&pending, worker, "capture").is_err());
    }

    #[test]
    fn backend_inheritance_and_explicit_selection_preserve_required_stored_values() {
        let (_dir, conn, _, fleet, director, _) = fixture();
        assert_eq!(
            resolve_coding_agent(&conn, fleet, director, None).unwrap(),
            CodingAgent::Claude
        );
        assert_eq!(
            resolve_coding_agent(&conn, fleet, director, Some(CodingAgent::Codex)).unwrap(),
            CodingAgent::Codex
        );
        conn.execute(
            "UPDATE member_placements SET coding_agent='broken' WHERE member_id=?1",
            [director],
        )
        .unwrap();
        assert!(resolve_coding_agent(&conn, fleet, director, None).is_err());
    }

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: MemberCommand,
    }

    fn parse(args: &[&str]) -> Result<MemberCommand, clap::Error> {
        Harness::try_parse_from(args).map(|harness| harness.command)
    }

    #[test]
    fn create_parses_the_supplied_assignment_and_inline_prompt() {
        let parsed = parse(&[
            "cafleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "reviewer",
            "--description",
            "assignment",
            "PROMPT",
        ])
        .unwrap();
        let MemberCommand::Create {
            name, description, ..
        } = parsed
        else {
            panic!("expected member creation");
        };
        assert_eq!(name, "reviewer");
        assert_eq!(description, "assignment");
    }

    #[test]
    fn create_parses_file_input_and_backend_model_selection() {
        parse(&[
            "cafleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "assignment",
            "--coding-agent",
            "codex",
            "--model",
            "gpt-6.1-sol",
            "--effort",
            "high",
            "--file",
            "prompt.md",
            "--json",
        ])
        .unwrap();
    }
}
