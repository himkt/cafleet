//! The `member` group (SPEC §6.3 *member group*): the shared resolution
//! helpers, the `member create` spawn orchestration + rollback ladder,
//! delete, show/list, prompt, and ping.

use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::FleetIdArg;
use super::helpers::{connect, emit, require_fleet_id, resolve_mux, resolve_text_body};
use super::system::SystemProbe;
use crate::broker::{self, NewPlacement};
use crate::coding_agent::coding_agent;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{AnyMultiplexer, Multiplexer};
use crate::output::{format_member, format_member_detail, format_member_list};
use crate::spawn_prompt::substitute_spawn_placeholders;

#[derive(Args)]
pub(crate) struct BodyArgs {
    /// Inline spawn prompt (backend-neutral template).
    #[arg(long)]
    text: Option<String>,
    /// UTF-8 file whose contents are the spawn prompt (`-` = stdin).
    #[arg(long = "text-file", value_name = "PATH")]
    text_file: Option<String>,
}

#[derive(Subcommand)]
pub enum MemberCommand {
    /// Register a member and spawn its coding-agent pane.
    Create {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Display name.
        #[arg(long)]
        name: String,
        /// One-sentence purpose.
        #[arg(long)]
        description: String,
        /// Backend binary to spawn / record [default: inherits the Director's backend].
        #[arg(long = "coding-agent", value_parser = ["claude", "codex", "opencode"])]
        coding_agent: Option<String>,
        /// Model passed to the backend binary.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning-effort level (claude, codex only).
        #[arg(long)]
        effort: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
        /// Switch the non-JSON output to the labeled block.
        #[arg(long)]
        full: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Tear down a member's pane (when one exists) and deregister it.
    Delete {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Show one member's detail.
    Show {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Switch the text output to the labeled verbose block.
        #[arg(long)]
        full: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// List every active registry entry of the fleet.
    List {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Keystroke a prompt (or, with --shell, a shell command) into a
    /// member's pane.
    Prompt {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Dispatch `! <text>` via the coding agent's shell shortcut.
        #[arg(long)]
        shell: bool,
        /// Single line of text to dispatch.
        text: String,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Inject an inbox-poll keystroke into a member's pane.
    Ping {
        #[command(flatten)]
        fleet: FleetIdArg,
        /// Member ID (the member in question).
        #[arg(long = "member-id")]
        member_id: i64,
        /// Print only the bare member id.
        #[arg(long)]
        quiet: bool,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
}

fn require_pane(member: &Value, member_id: i64, action: &str) -> Result<String, CafleetError> {
    member["placement"]["mux_pane_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            CafleetError::App(format!(
                "member {member_id} has no pane yet (pending placement) — nothing to {action}."
            ))
        })
}

/// Fetch the member within the fleet, optionally tolerating a missing
/// placement row (SPEC §6.3 *Load-authorized-member*).
fn load_authorized_member(
    conn: &Connection,
    fleet_id: i64,
    member_id: i64,
    tolerate_missing_placement: bool,
) -> Result<Value, CafleetError> {
    let member = broker::get_member(conn, member_id, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    if member["placement"].is_null() && !tolerate_missing_placement {
        return Err(CafleetError::App(format!(
            "member {member_id} has no placement row; it was not spawned via \
             `cafleet member create`."
        )));
    }
    Ok(member)
}

fn deregister_with_warning(conn: &mut Connection, member_id: i64) {
    if let Err(error) = broker::deregister_member(conn, member_id) {
        eprintln!(
            "WARNING: rollback deregister failed for member {member_id}: {}",
            error.message()
        );
    }
}

fn rollback_register(conn: &mut Connection, member_id: i64, reason: String) -> CafleetError {
    deregister_with_warning(conn, member_id);
    CafleetError::App(format!(
        "{reason}. Rolled back registration of {member_id}."
    ))
}

/// Resolve the effective coding agent: an explicit flag wins; otherwise
/// inherit the Director's placement backend.
fn resolve_coding_agent(
    conn: &Connection,
    fleet_id: i64,
    director_id: i64,
    explicit: Option<&str>,
) -> Result<String, CafleetError> {
    if let Some(agent) = explicit {
        return Ok(agent.to_string());
    }
    let unresolved = |detail: String| {
        CafleetError::App(format!(
            "cannot resolve the member's coding agent: {detail} \
             Re-run with an explicit --coding-agent."
        ))
    };
    let director = match broker::get_member(conn, director_id, fleet_id) {
        Ok(Some(director)) => director,
        Ok(None) => {
            return Err(unresolved(format!(
                "Director (member {director_id}) not found."
            )));
        }
        Err(error) => {
            return Err(unresolved(format!(
                "failed to fetch the Director: {}.",
                error.message()
            )));
        }
    };
    match director["placement"]["coding_agent"].as_str() {
        Some(agent) => Ok(agent.to_string()),
        None => Err(unresolved(format!(
            "Director (member {director_id}) has no placement."
        ))),
    }
}

pub fn run(settings: &Settings, command: MemberCommand) -> Result<(), CafleetError> {
    match command {
        MemberCommand::Create {
            fleet,
            name,
            description,
            coding_agent,
            model,
            effort,
            body,
            full,
            json,
        } => create(
            settings,
            fleet.fleet_id,
            &name,
            &description,
            coding_agent.as_deref(),
            model.as_deref(),
            effort.as_deref(),
            &body,
            full,
            json,
        ),
        MemberCommand::Delete {
            fleet,
            member_id,
            json,
        } => delete(settings, fleet.fleet_id, member_id, json),
        MemberCommand::Show {
            fleet,
            member_id,
            full,
            json,
        } => show(settings, fleet.fleet_id, member_id, full, json),
        MemberCommand::List { fleet, json } => list(settings, fleet.fleet_id, json),
        MemberCommand::Prompt {
            fleet,
            member_id,
            shell,
            text,
            json,
        } => prompt(settings, fleet.fleet_id, member_id, shell, &text, json),
        MemberCommand::Ping {
            fleet,
            member_id,
            quiet,
            json,
        } => ping(settings, fleet.fleet_id, member_id, quiet, json),
    }
}

#[allow(clippy::too_many_arguments)]
fn create(
    settings: &Settings,
    fleet_id: Option<i64>,
    name: &str,
    description: &str,
    explicit_agent: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    body: &BodyArgs,
    full: bool,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let mut conn = connect(settings)?;

    // 1. Auto-resolve the Director from the fleet row, first thing.
    let fleet = broker::get_fleet(&conn, fleet_id)?
        .ok_or_else(|| CafleetError::Usage(format!("Fleet '{fleet_id}' not found.")))?;
    if !fleet["deleted_at"].is_null() {
        return Err(CafleetError::App(format!("fleet {fleet_id} is deleted")));
    }
    let Some(director_id) = fleet["director_member_id"].as_i64() else {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id} has no root Director recorded; re-create the fleet \
             with 'cafleet fleet create'."
        )));
    };
    let agent_name = resolve_coding_agent(&conn, fleet_id, director_id, explicit_agent)?;
    let backend = coding_agent(&agent_name)
        .unwrap_or_else(|| panic!("'{agent_name}' is a registry-validated backend"));

    // 2. Model and effort validation, before any registration or pane effect.
    backend.validate_model(model)?;
    backend.validate_effort(effort)?;

    // 3. Resolve the body before any side effect; substitution is deferred
    //    until the new member id exists.
    let prompt_body = resolve_text_body(body.text.as_deref(), body.text_file.as_deref())?;

    // 4. Preconditions.
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    backend.ensure_available(&SystemProbe)?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    // 5. Register the member with a pending placement.
    let placement = NewPlacement {
        backend: mux.name().to_string(),
        mux_session: context.session.clone(),
        mux_window_id: context.window_id.clone(),
        mux_pane_id: None,
        coding_agent: agent_name.clone(),
    };
    let registered = broker::register_member(
        &mut conn,
        fleet_id,
        name,
        description,
        &[],
        Some(&placement),
    )
    .map_err(|error| match error {
        CafleetError::App(_) | CafleetError::Usage(_) => error,
        other => CafleetError::App(format!("register failed: {}", other.message())),
    })?;
    let member_id = registered["member_id"]
        .as_i64()
        .expect("registration returns the new member id");

    // 6. Substitute the identity placeholders; on failure deregister and
    //    re-raise the original error unwrapped.
    let rendered = match substitute_spawn_placeholders(
        &prompt_body,
        fleet_id,
        member_id,
        director_id,
        &agent_name,
    ) {
        Ok(rendered) => rendered,
        Err(original) => {
            deregister_with_warning(&mut conn, member_id);
            return Err(original);
        }
    };

    // 7-8. Build the spawn argv and split the pane, forwarding only
    //      CAFLEET_DATABASE_URL.
    let argv = backend.build_spawn_argv(&rendered, name, model, effort);
    let mut env = Vec::new();
    if let Ok(url) = std::env::var("CAFLEET_DATABASE_URL") {
        env.push(("CAFLEET_DATABASE_URL".to_string(), url));
    }
    let pane_id = match mux.split_window(&context, &env, &argv) {
        Ok(pane_id) => pane_id,
        Err(error) => {
            return Err(rollback_register(
                &mut conn,
                member_id,
                format!("tmux split-window failed: {error}"),
            ));
        }
    };

    // 9. Patch the pane id onto the placement.
    let patched = match broker::update_placement_pane_id(&mut conn, member_id, &pane_id) {
        Ok(patched) => patched,
        Err(error) => {
            let _ = mux.send_exit(&pane_id, true);
            return Err(rollback_register(
                &mut conn,
                member_id,
                format!("placement update failed: {}", error.message()),
            ));
        }
    };
    let Some(placement_view) = patched else {
        let _ = mux.send_exit(&pane_id, true);
        return Err(rollback_register(
            &mut conn,
            member_id,
            "placement row vanished before pane-id patch".to_string(),
        ));
    };

    // 10. Emit with the placement view attached.
    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered["registered_at"],
        "placement": placement_view,
    });
    emit(json, &result, || format_member(&result, full));
    Ok(())
}

fn delete(
    settings: &Settings,
    fleet_id: Option<i64>,
    member_id: i64,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let mut conn = connect(settings)?;

    // Root-Director guard, before any pane mutation.
    let fleet = broker::get_fleet(&conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    if fleet["director_member_id"].as_i64() == Some(member_id) {
        return Err(CafleetError::App(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead".to_string(),
        ));
    }

    let member = load_authorized_member(&conn, fleet_id, member_id, true)?;
    let pane = member["placement"]["mux_pane_id"]
        .as_str()
        .map(str::to_string);
    let pane_status = if member["placement"].is_null() {
        "(no placement)".to_string()
    } else if let Some(pane_id) = &pane {
        let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
        mux.ensure_available()
            .map_err(|e| CafleetError::App(e.to_string()))?;
        mux.kill_pane(pane_id, true).map_err(|error| {
            CafleetError::App(format!(
                "kill_pane failed for pane {pane_id}: {error}. The {} server may be \
                 unreachable. Verify with 'cafleet doctor', then re-run the command.",
                mux.name()
            ))
        })?;
        format!("{pane_id} (killed)")
    } else {
        "(pending — no pane)".to_string()
    };
    broker::deregister_member(&mut conn, member_id)
        .map_err(|e| CafleetError::App(format!("deregister failed: {}", e.message())))?;

    let result = json!({"member_id": member_id, "pane_status": pane_status});
    emit(json, &result, || {
        format!("Member deleted.\n  member_id: {member_id}\n  pane_id:   {pane_status}")
    });
    Ok(())
}

fn show(
    settings: &Settings,
    fleet_id: Option<i64>,
    member_id: i64,
    full: bool,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let conn = connect(settings)?;
    let member = load_authorized_member(&conn, fleet_id, member_id, true)?;
    emit(json, &member, || format_member_detail(&member, full));
    Ok(())
}

fn list(settings: &Settings, fleet_id: Option<i64>, json: bool) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let conn = connect(settings)?;
    let members = broker::list_members(&conn, fleet_id)?;
    emit(json, &Value::Array(members.clone()), || {
        format_member_list(&members)
    });
    Ok(())
}

fn prompt(
    settings: &Settings,
    fleet_id: Option<i64>,
    member_id: i64,
    shell: bool,
    text: &str,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    if text.contains('\n') || text.contains('\r') {
        return Err(CafleetError::Usage(
            "text may not contain newlines.".to_string(),
        ));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(CafleetError::Usage("text may not be empty.".to_string()));
    }
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let conn = connect(settings)?;
    let member = load_authorized_member(&conn, fleet_id, member_id, false)?;
    let pane_id = require_pane(&member, member_id, "prompt")?;
    mux.send_prompt(&pane_id, trimmed, shell)
        .map_err(|e| CafleetError::App(format!("send failed: {e}")))?;

    let name = member["name"].as_str().expect("members carry a name");
    let result = json!({
        "member_id": member_id,
        "pane_id": pane_id,
        "text": trimmed,
        "shell": shell,
    });
    emit(json, &result, || {
        let form = if shell { "shell prompt" } else { "prompt" };
        format!("Sent {form} {trimmed:?} to member {name} ({pane_id}).")
    });
    Ok(())
}

fn ping(
    settings: &Settings,
    fleet_id: Option<i64>,
    member_id: i64,
    quiet: bool,
    json: bool,
) -> Result<(), CafleetError> {
    let fleet_id = require_fleet_id(fleet_id)?;
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let conn = connect(settings)?;
    let member = load_authorized_member(&conn, fleet_id, member_id, false)?;
    let name = member["name"].as_str().expect("members carry a name");
    let pane = member["placement"]["mux_pane_id"].as_str();

    let Some(pane_id) = pane else {
        // The pending-placement skip path: no keystroke, exit 0.
        if quiet && !json {
            println!("{member_id}");
            return Ok(());
        }
        let result = json!({"member_id": member_id, "pane_id": Value::Null, "skipped": true});
        emit(json, &result, || {
            format!(
                "Member {name} has no pane yet (pending placement) — ping skipped; \
                 it will poll its inbox on spawn."
            )
        });
        return Ok(());
    };

    let fleet_of_member = fleet_id;
    if !mux.send_poll_trigger(pane_id, fleet_of_member, member_id) {
        return Err(CafleetError::App(format!(
            "send failed: tmux send-keys did not deliver the poll-trigger keystroke \
             to pane {pane_id}."
        )));
    }
    if quiet && !json {
        println!("{member_id}");
        return Ok(());
    }
    let result = json!({"member_id": member_id, "pane_id": pane_id, "skipped": false});
    emit(json, &result, || {
        format!("Pinged member {name} ({pane_id}) — poll keystroke dispatched.")
    });
    Ok(())
}

/// Shared with `monitor capture`: the member-group loader + pane requirement.
pub(crate) fn load_member_with_pane(
    settings: &Settings,
    fleet_id: i64,
    member_id: i64,
    action: &str,
) -> Result<(AnyMultiplexer, String), CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    let conn = connect(settings)?;
    let member = load_authorized_member(&conn, fleet_id, member_id, false)?;
    let pane_id = require_pane(&member, member_id, action)?;
    Ok((mux, pane_id))
}
