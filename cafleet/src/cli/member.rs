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
    /// Submit a native shell command to a member's idle pane.
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

enum DeleteAction {
    Deregistered(&'static str),
    Close(String),
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

    let locked = crate::input::lock_deletion(
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
    )?;
    let member = &locked.snapshot;
    let pane_status = match &locked.value {
        DeleteAction::Deregistered(status) => status.to_string(),
        DeleteAction::Close(pane_id) => {
            let mux = resolve_mux(settings)
                .map_err(|error| CafleetError::App(error.to_string()))?
                .bounded(deadline);
            mux.ensure_available()
                .map_err(|error| CafleetError::App(error.to_string()))?;
            let layout = mux.close_pane(pane_id, true).map_err(|error| {
                CafleetError::App(format!(
                    "kill_pane failed for pane {pane_id}: {error}. The {} server may be \
                     unreachable. Verify with 'cafleet doctor', then re-run the command.",
                    mux.name()
                ))
            })?;
            crate::input::complete_database_budget(conn, deadline, |conn| {
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(crate::broker::members::db_err)?;
                let current = load_member(&tx, member_id, true)?;
                if current.placement != member.placement {
                    return Err(CafleetError::App(
                        "member placement changed after pane close".into(),
                    ));
                }
                let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
                assert!(
                    changed,
                    "the input-locked member remains active until deregistration"
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
            format!("{pane_id} (killed)")
        }
    };

    drop(locked);
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
    if member.kind == crate::broker::records::MemberKind::Director {
        return Err(CafleetError::App(
            "cannot ping the Director's own pane".into(),
        ));
    }
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

    struct DeleteKillRelease(std::path::PathBuf);

    impl Drop for DeleteKillRelease {
        fn drop(&mut self) {
            std::fs::write(&self.0, b"release").unwrap();
        }
    }

    fn pending_delete_in_private_process(home: &std::path::Path) {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let database_url = format!("sqlite:///{}", home.join("broker_test.db").display());
        let mut deletion = crate::db::connect(&database_url).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut gated = false;
        let expire_after_close =
            std::env::var_os("CAFLEET_TEST_DELETE_EXPIRE_AFTER_CLOSE").is_some();
        let expires = Instant::now() + Duration::from_secs(5);
        deletion
            .authorizer(Some(move |context: AuthContext<'_>| {
                if !gated
                    && matches!(
                        context.action,
                        AuthAction::Transaction {
                            operation: TransactionOperation::Begin
                        }
                    )
                {
                    gated = true;
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                }
                if expire_after_close
                    && matches!(
                        context.action,
                        AuthAction::Update {
                            table_name: "members",
                            ..
                        }
                    )
                {
                    std::thread::sleep(
                        expires.saturating_duration_since(Instant::now())
                            + Duration::from_millis(10),
                    );
                }
                Authorization::Allow
            }))
            .unwrap();
        let settings = crate::config::Settings {
            database_url: database_url.clone(),
            max_text_len: 200,
            multiplexer: Some("tmux".into()),
        };
        let _kill_release = DeleteKillRelease(home.join("release-kill"));
        let deleting = std::thread::spawn(move || {
            super::delete(
                &mut deletion,
                &settings,
                2,
                true,
                crate::input::Deadline::until(expires),
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("deletion reaches its first registry transaction");
        let mut creation = crate::db::connect(&database_url).unwrap();
        let patched = crate::broker::update_placement_pane_id(&mut creation, 2, "%9")
            .unwrap()
            .expect("creation patches the pending placement before deletion mutates it");
        assert_eq!(patched.mux_pane_id.as_deref(), Some("%9"));
        release_tx.send(()).unwrap();

        let entered = home.join("kill-entered");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !entered.exists() && !deleting.is_finished() {
            assert!(
                Instant::now() < deadline,
                "deletion reaches concrete pane close"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            entered.exists(),
            "deletion must close the pane published after its pending snapshot"
        );
        assert!(
            crate::input::PaneLock::try_acquire(&database_url, &patched)
                .unwrap()
                .is_none(),
            "the concrete pane close retains exclusive input ownership"
        );
        creation.busy_timeout(Duration::ZERO).unwrap();
        creation
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
            .expect("a blocked multiplexer close leaves the SQLite writer available");
        std::fs::write(home.join("release-kill"), b"release").unwrap();
        deleting.join().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("mux-calls")).unwrap(),
            "kill-pane -t %9\n"
        );
        let (status, deregistered_at): (String, Option<String>) = creation
            .query_row(
                "SELECT status,deregistered_at FROM members WHERE member_id=2",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "deregistered");
        assert!(deregistered_at.is_some());
        let placements: i64 = creation
            .query_row(
                "SELECT count(*) FROM member_placements WHERE member_id=2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(placements, 0);
        let director_status: String = creation
            .query_row("SELECT status FROM members WHERE member_id=1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(director_status, "active");
    }

    #[test]
    fn member_delete_revalidates_a_pending_placement_before_registry_mutation() {
        run_private_delete_regression(false);
    }

    #[test]
    fn successful_pane_close_is_finalized_when_the_operation_budget_expires() {
        run_private_delete_regression(true);
    }

    fn run_private_delete_regression(expire_after_close: bool) {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        if let Some(home) = std::env::var_os("CAFLEET_TEST_PENDING_DELETE_UNIT_HOME") {
            pending_delete_in_private_process(std::path::Path::new(&home));
            return;
        }
        let home = tempfile::TempDir::new().unwrap();
        let mut conn = crate::broker::test_support::migrated_conn(&home);
        let (fleet, _) = crate::broker::test_support::create_fleet(&mut conn, "pending-delete");
        let member = crate::broker::test_support::register(&mut conn, fleet, "worker", None);
        assert_eq!(member, 2);
        drop(conn);
        let bin = home.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tmux = bin.join("tmux");
        std::fs::write(
            &tmux,
            r#"#!/bin/sh
[ "$*" = 'kill-pane -t %9' ] || exit 91
printf '%s\n' "$*" >> "$HOME/mux-calls"
printf 'entered\n' > "$HOME/kill-entered"
loops=0
while [ ! -e "$HOME/release-kill" ] && [ "$loops" -lt 600 ]; do
    /bin/sleep 0.01
    loops=$((loops + 1))
done
[ -e "$HOME/release-kill" ] || exit 92
"#,
        )
        .unwrap();
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
                "--exact",
                "cli::member::tests::member_delete_revalidates_a_pending_placement_before_registry_mutation",
                "--nocapture",
            ])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", &bin)
            .env("TMUX", "/tmp/cafleet-test,1,0")
            .env("TMUX_PANE", "%0")
            .env("CAFLEET_TEST_PENDING_DELETE_UNIT_HOME", home.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if expire_after_close {
            command.env("CAFLEET_TEST_DELETE_EXPIRE_AFTER_CLOSE", "1");
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(9);
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "private deletion regression: stdout={}, stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "private deletion regression exceeded cleanup bound: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
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
