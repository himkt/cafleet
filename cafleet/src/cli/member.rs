//! The `member` group (SPEC §6.3 *member group*): the shared resolution
//! helpers, the `member create` spawn orchestration + rollback ladder,
//! delete, show/list, prompt, ping, and capture.

use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::creation::{PaneGuard, RegistrationGuard};

use super::helpers::{emit, resolve_body, resolve_command_body, resolve_mux};
use crate::broker::records::MemberRecord;
use crate::broker::{self, NewPlacement};
use crate::capture::{CaptureSnapshot, MemberCapture, write_member_capture};
use crate::coding_agent::coding_agent;
use crate::config::Settings;
use crate::delivery::DELIVERY_CAPTURE_LINES;
use crate::error::CafleetError;

use crate::multiplexer::Multiplexer;
use crate::output::{format_member, format_member_detail, format_member_list};
use crate::pane_state::{PaneState, classify};
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
        #[arg(long = "coding-agent", value_parser = ["claude", "codex", "opencode"])]
        coding_agent: Option<String>,
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
    /// Run a command to completion in a member's pane and resume the member.
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
    /// Inject an inbox-poll keystroke into a member's pane.
    Ping {
        /// The target member.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
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

/// Fetch the member by id — the fleet is derived from the member row —
/// optionally tolerating a missing placement row (SPEC §6.3 *Load-member*).
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
    match director.placement.as_ref().map(|p| p.coding_agent.as_str()) {
        Some(agent) => Ok(agent.to_string()),
        None => Err(unresolved(format!(
            "Director (member {director_id}) has no placement."
        ))),
    }
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
            coding_agent.as_deref(),
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
        MemberCommand::Ping { member_id, json } => ping(conn, settings, member_id, json, deadline),
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
    explicit_agent: Option<&str>,
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
    let agent_name = resolve_coding_agent(conn, fleet_id, director_id, explicit_agent)?;
    let backend = coding_agent(&agent_name)
        .unwrap_or_else(|| panic!("'{agent_name}' is a registry-validated backend"));

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
        coding_agent: agent_name.clone(),
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
        &agent_name,
    ) {
        Ok(prompt) => prompt,
        Err(original) => return Err(registration.rollback(original)),
    };
    let argv = backend.build_spawn_argv(&prompt, name, model, effort);
    let pane_id = match mux.split_window(&context, &env, &argv) {
        Ok(pane_id) => pane_id,
        Err(error) => {
            return Err(registration.rollback(CafleetError::App(format!(
                "tmux split-window failed: {error}"
            ))));
        }
    };
    let mut pane = PaneGuard::with_kill(pane_id.clone(), Box::new(|id| mux.kill_pane(id, true)));

    let patched = broker::update_placement_pane_id(registration.connection(), member_id, &pane_id);
    let placement_view = match patched {
        Ok(Some(placement)) => placement,
        outcome => {
            let primary = match outcome {
                Err(error) => {
                    CafleetError::App(format!("placement update failed: {}", error.message()))
                }
                _ => CafleetError::App("placement row vanished before pane-id patch".into()),
            };
            let primary = pane.rollback(primary);
            return Err(registration.rollback(primary));
        }
    };
    pane.finish();
    registration.finish();

    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered.registered_at,
        "placement": presentation::placement(&placement_view),
    });
    emit(json, &result, || format_member(&result));
    Ok(())
}

fn delete(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let fleet = crate::input::with_database_budget(conn, deadline, |conn| {
        let fleet_id = broker::active_member_fleet(conn, member_id)?
            .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
        let fleet = broker::fleets::fetch_fleet(conn, fleet_id)?
            .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
        Ok(fleet)
    })?;
    if fleet.director_member_id == Some(member_id) {
        return Err(CafleetError::App(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead".to_string(),
        ));
    }

    let (member, _guard) = loop {
        let member = crate::input::with_database_budget(conn, deadline, |conn| {
            load_member(conn, member_id, true)
        })?;
        let concrete = member
            .placement
            .as_ref()
            .filter(|placement| placement.mux_pane_id.is_some());
        let guard = if let Some(placement) = concrete {
            match crate::input::PaneLock::try_acquire(&settings.database_url, placement).map_err(
                |error| {
                    CafleetError::App(format!(
                        "member {member_id} pane input: {}",
                        error.message()
                    ))
                },
            )? {
                Some(guard) => Some(guard),
                None => {
                    if deadline.retry() {
                        continue;
                    }
                    return Err(CafleetError::App(format!(
                        "member {member_id}'s pane input remained busy for 5 seconds; member was not deleted."
                    )));
                }
            }
        } else {
            None
        };
        let current = crate::input::with_database_budget(conn, deadline, |conn| {
            load_member(conn, member_id, true)
        })?;
        if current.placement != member.placement {
            drop(guard);
            deadline.remaining()?;
            continue;
        }
        if current.kind == crate::broker::records::MemberKind::Director {
            return Err(CafleetError::App(
                "cannot deregister the root Director; use 'cafleet fleet delete' instead".into(),
            ));
        }
        break (current, guard);
    };
    let pane = member
        .placement
        .as_ref()
        .and_then(|p| p.mux_pane_id.clone());
    let pane_status = if member.placement.is_none() {
        "(no placement)".to_string()
    } else if let Some(pane_id) = &pane {
        let mux = resolve_mux(settings)
            .map_err(|e| CafleetError::App(e.to_string()))?
            .bounded(deadline);
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
    crate::input::with_database_budget(conn, deadline, |conn| {
        broker::deregister_member(conn, member_id)
    })
    .map_err(|e| CafleetError::App(format!("deregister failed: {}", e.message())))?;

    let result = json!({"member_id": member_id, "pane_status": pane_status});
    emit(json, &result, || {
        format!("Member deleted.\n  member_id: {member_id}\n  pane_id:   {pane_status}")
    });
    Ok(())
}

fn show(conn: &mut Connection, member_id: i64, json: bool) -> Result<(), CafleetError> {
    let member = load_member(conn, member_id, true)?;
    let value = presentation::member(&member);
    emit(json, &value, || format_member_detail(&value));
    Ok(())
}

fn list(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let members: Vec<Value> = broker::list_members(conn, fleet_id)?
        .iter()
        .map(presentation::member_activity)
        .collect();
    emit(json, &Value::Array(members.clone()), || {
        format_member_list(&members)
    });
    Ok(())
}

fn input_state(state: PaneState) -> Option<&'static str> {
    state.hold_reason()
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
    let Some(locked) = crate::input::with_database_budget(conn, deadline, |conn| {
        crate::input::lock_member(conn, settings, member)
    })?
    else {
        return Err(CafleetError::App(format!(
            "member {id}'s pane is receiving another input; retry after it is idle."
        )));
    };
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
    let content = mux
        .capture_pane(pane, DELIVERY_CAPTURE_LINES)
        .map_err(|e| prewrite(e.to_string()))?;
    if let Some(state) = input_state(classify(&placement.coding_agent, &content)) {
        return Err(CafleetError::App(format!(
            "member {id} is {state}; {label} was not dispatched."
        )));
    }
    mux.send_prompt(pane, text).map_err(|error| {
        if error.submission_started() { CafleetError::App(format!(
            "{label} delivery to member {id} is uncertain: {error}. Inspect the pane before retrying."
        )) } else { prewrite(error.to_string()) }
    })
}

fn valid_input(text: &str) -> bool {
    !text.trim().is_empty() && !text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

fn prompt(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    text: &str,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    if !valid_input(text) {
        return Err(CafleetError::Usage(
            "text must be one non-empty line without control characters.".into(),
        ));
    }
    let member = crate::input::with_database_budget(conn, deadline, |conn| {
        load_member(conn, member_id, false)
    })?;
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
    let command = resolve_command_body(body.command.as_deref(), body.file.as_deref())?;
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

fn ping(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let member = crate::input::with_database_budget(conn, deadline, |conn| {
        load_member(conn, member_id, false)
    })?;
    let placement = member.placement.as_ref().expect("placed member");
    let pane = placement.mux_pane_id.as_deref();
    let reason;
    if let Some(pane) = pane {
        let locked = crate::input::with_database_budget(conn, deadline, |conn| {
            crate::input::lock_member(conn, settings, &member)
        })?;
        reason = match locked {
            None => Some("busy"),
            Some(_locked) => {
                let mux = resolve_mux(settings)
                    .map_err(|e| CafleetError::App(e.to_string()))?
                    .bounded(deadline);
                mux.ensure_available()
                    .map_err(|e| CafleetError::App(e.to_string()))?;
                let content = mux
                    .capture_pane(pane, DELIVERY_CAPTURE_LINES)
                    .map_err(|e| CafleetError::App(e.to_string()))?;
                let reason = input_state(classify(&placement.coding_agent, &content));
                if reason.is_none() {
                    mux.send_prompt(pane, &format!("cafleet message poll {member_id} --json"))
                        .map_err(|error| if error.submission_started() { CafleetError::App(format!(
                            "poll-trigger delivery to member {member_id} is uncertain: {error}. Inspect the pane before retrying."
                        )) } else { CafleetError::App(error.to_string()) })?;
                }
                reason
            }
        };
    } else {
        reason = Some("pending_placement");
    }
    emit(
        json,
        &json!({"member_id":member_id,"pane_id":pane,
        "skipped":reason.is_some(),"reason":reason}),
        || match (pane, reason) {
            (None, _) => format!(
                "Member {} has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.",
                member.name
            ),
            (Some(pane), Some(reason)) => format!(
                "Member {} ({pane}) is {reason} — ping skipped.",
                member.name
            ),
            (Some(pane), None) => format!(
                "Pinged member {} ({pane}) — poll keystroke dispatched.",
                member.name
            ),
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
    use super::MemberCommand;
    use clap::Parser;

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
