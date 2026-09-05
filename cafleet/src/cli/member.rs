//! The `member` group (SPEC §6.3 *member group*): the shared resolution
//! helpers, the `member create` spawn orchestration + rollback ladder,
//! delete, show/list, prompt, ping, and capture.

use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::creation::{CreationHooks, NoopCreationHooks, PaneGuard, RegistrationGuard};
use super::helpers::{emit, resolve_body, resolve_mux};
use crate::broker::records::MemberRecord;
use crate::broker::{self, NewPlacement};
use crate::coding_agent::{SpawnProbe, coding_agent};
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::{Multiplexer, MultiplexerError};
use crate::output::{format_member, format_member_detail, format_member_list, strip_ansi};
use crate::presentation;
use crate::runtime::system::SystemProbe;
use crate::spawn_prompt::substitute_spawn_placeholders;
use crate::time::{format_utc, now_utc};

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
        /// Register the fleet's monitor member (sole accepted value: monitor).
        #[arg(long, value_parser = ["monitor"])]
        role: Option<String>,
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
    /// Keystroke a prompt (or, with --shell, a shell command) into a
    /// member's pane.
    Prompt {
        /// The target member.
        #[arg(value_name = "MEMBER_ID")]
        member_id: i64,
        /// Single line of text to dispatch.
        #[arg(value_name = "TEXT")]
        text: String,
        /// Dispatch `! <text>` via the coding agent's shell shortcut.
        #[arg(long)]
        shell: bool,
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
    let member = broker::get_member_record(conn, member_id, fleet_id)?
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
    let director = match broker::get_member_record(conn, director_id, fleet_id) {
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
) -> Result<(), CafleetError> {
    match command {
        MemberCommand::Create {
            fleet_id,
            name,
            description,
            coding_agent,
            model,
            effort,
            role,
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
            role.is_some(),
            &body,
            json,
        ),
        MemberCommand::Delete { member_id, json } => delete(conn, settings, member_id, json),
        MemberCommand::Show { member_id, json } => show(conn, member_id, json),
        MemberCommand::List { fleet_id, json } => list(conn, fleet_id, json),
        MemberCommand::Prompt {
            member_id,
            text,
            shell,
            json,
        } => prompt(conn, settings, member_id, shell, &text, json),
        MemberCommand::Ping { member_id, json } => ping(conn, settings, member_id, json),
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
    monitor: bool,
    body: &PromptArgs,
    json: bool,
) -> Result<(), CafleetError> {
    let result = create_with_connection(
        conn,
        fleet_id,
        name,
        description,
        explicit_agent,
        model,
        effort,
        monitor,
        body,
        || resolve_mux(settings),
        &SystemProbe,
        &NoopCreationHooks,
    )?;
    emit(json, &result, || format_member(&result));
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn create_with_dependencies<M: Multiplexer>(
    settings: &Settings,
    fleet_id: i64,
    name: &str,
    description: &str,
    explicit_agent: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    monitor: bool,
    body: &PromptArgs,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe,
    hooks: &dyn CreationHooks,
) -> Result<Value, CafleetError> {
    let mut conn = crate::db::connect(&settings.database_url)?;
    create_with_connection(
        &mut conn,
        fleet_id,
        name,
        description,
        explicit_agent,
        model,
        effort,
        monitor,
        body,
        resolve_mux,
        probe,
        hooks,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_with_connection<M: Multiplexer>(
    conn: &mut Connection,
    fleet_id: i64,
    name: &str,
    description: &str,
    explicit_agent: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
    monitor: bool,
    body: &PromptArgs,
    resolve_mux: impl FnOnce() -> Result<M, MultiplexerError>,
    probe: &dyn SpawnProbe,
    hooks: &dyn CreationHooks,
) -> Result<Value, CafleetError> {
    // 1. Auto-resolve the Director from the fleet row, first thing.
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

    // 2. Model and effort validation, before any registration or pane effect.
    backend.validate_model(model)?;
    backend.validate_effort(effort)?;

    // 3. Monitor-role guards, one-per-fleet first, before any registration
    //    or pane effect (the broker also enforces monitor uniqueness).
    let active_monitor = broker::active_monitor_member_id(conn, fleet_id)?;
    if monitor {
        if let Some(existing) = active_monitor {
            return Err(CafleetError::App(format!(
                "fleet {fleet_id} already has an active monitor member (member {existing})"
            )));
        }
    } else if active_monitor.is_none() {
        return Err(CafleetError::App(format!(
            "fleet {fleet_id} has no active monitor member; spawn one with --role monitor first"
        )));
    }

    // 4. Resolve the body before any side effect; substitution is deferred
    //    until the new member id exists.
    let prompt_body = resolve_body(body.prompt.as_deref(), body.file.as_deref(), "--file")?;

    // 5. Preconditions.
    let mux = resolve_mux().map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    backend.ensure_available(probe)?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    // 6. Register the member with a pending placement, threading the
    //    monitor role into its card marker.
    let placement = NewPlacement {
        backend: mux.name().to_string(),
        mux_session: context.session.clone(),
        mux_window_id: context.window_id.clone(),
        mux_pane_id: None,
        coding_agent: agent_name.clone(),
    };
    let registered = broker::register_member_record(
        conn,
        fleet_id,
        name,
        description,
        &[],
        Some(&placement),
        monitor,
    )
    .map_err(|error| match error {
        CafleetError::App(_)
        | CafleetError::Usage(_)
        | CafleetError::ActiveMonitorExists { .. } => error,
        other => CafleetError::App(format!("register failed: {}", other.message())),
    })?;
    let member_id = registered.member_id;
    let mut registration = RegistrationGuard::new(conn, member_id, hooks);

    // 7. Substitute the identity placeholders; on failure deregister and
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
            return Err(registration.rollback(original));
        }
    };

    // 8-9. Build the spawn argv and split the pane, forwarding only
    //      CAFLEET_DATABASE_URL.
    let argv = backend.build_spawn_argv(&rendered, name, model, effort);
    let mut env = Vec::new();
    if let Ok(url) = std::env::var("CAFLEET_DATABASE_URL") {
        env.push(("CAFLEET_DATABASE_URL".to_string(), url));
    }
    let pane_id = match mux.split_window(&context, &env, &argv) {
        Ok(pane_id) => pane_id,
        Err(error) => {
            // Failed splits remain backend-owned: metadata reports attempted
            // cleanup or an unknown ID. No CLI pane guard exists to re-kill.
            return Err(registration.rollback(CafleetError::App(format!(
                "tmux split-window failed: {error}"
            ))));
        }
    };
    let mut pane = PaneGuard::new(&mux, pane_id.clone(), hooks);

    // 10. A failed placement patch compensates the pane before registration.
    let patched = broker::update_placement_record(registration.connection(), member_id, &pane_id);
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

    // 11. Emit with the placement view attached.
    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered.registered_at,
        "placement": presentation::placement(&placement_view),
    });
    Ok(result)
}

fn delete(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    json: bool,
) -> Result<(), CafleetError> {
    // Root-Director guard, before any pane mutation.
    let fleet_id = broker::active_member_fleet(conn, member_id)?
        .ok_or_else(|| CafleetError::App(format!("Member {member_id} not found")))?;
    let fleet = broker::fleets::fetch_fleet(conn, fleet_id)?
        .ok_or_else(|| CafleetError::App(format!("fleet '{fleet_id}' not found.")))?;
    if fleet.director_member_id == Some(member_id) {
        return Err(CafleetError::App(
            "cannot deregister the root Director; use 'cafleet fleet delete' instead".to_string(),
        ));
    }

    let member = load_member(conn, member_id, true)?;
    let pane = member
        .placement
        .as_ref()
        .and_then(|p| p.mux_pane_id.clone());
    let pane_status = if member.placement.is_none() {
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
    broker::deregister_member(conn, member_id)
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
    let members: Vec<Value> = broker::list_member_records(conn, fleet_id)?
        .iter()
        .map(presentation::member_activity)
        .collect();
    emit(json, &Value::Array(members.clone()), || {
        format_member_list(&members)
    });
    Ok(())
}

fn prompt(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    shell: bool,
    text: &str,
    json: bool,
) -> Result<(), CafleetError> {
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

    let member = load_member(conn, member_id, false)?;
    let pane_id = require_pane(&member, member_id, "prompt")?;
    mux.send_prompt(&pane_id, trimmed, shell)
        .map_err(|e| CafleetError::App(format!("send failed: {e}")))?;

    let name = member.name.as_str();
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
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    json: bool,
) -> Result<(), CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    let member = load_member(conn, member_id, false)?;
    let name = member.name.as_str();
    let pane = member
        .placement
        .as_ref()
        .and_then(|p| p.mux_pane_id.as_deref());

    let Some(pane_id) = pane else {
        // The pending-placement skip path: no keystroke, exit 0.
        let result = json!({"member_id": member_id, "pane_id": Value::Null, "skipped": true});
        emit(json, &result, || {
            format!(
                "Member {name} has no pane yet (pending placement) — ping skipped; \
                 it will poll its inbox on spawn."
            )
        });
        return Ok(());
    };

    if !mux.send_poll_trigger(pane_id, member_id) {
        return Err(CafleetError::App(format!(
            "send failed: tmux send-keys did not deliver the poll-trigger keystroke \
             to pane {pane_id}."
        )));
    }
    let result = json!({"member_id": member_id, "pane_id": pane_id, "skipped": false});
    emit(json, &result, || {
        format!("Pinged member {name} ({pane_id}) — poll keystroke dispatched.")
    });
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
    let content = if ansi { raw } else { strip_ansi(&raw) };
    let captured_at = format_utc(now_utc());
    let digest = Sha256::digest(content.as_bytes());
    let content_sha256: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let payload = json!({
        "member_id": member_id,
        "pane_id": pane_id,
        "lines": lines,
        "content": content,
        "captured_at": captured_at,
        "content_sha256": content_sha256,
    });
    if json {
        emit(true, &payload, String::new);
    } else {
        print!("{content}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::MemberCommand;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: MemberCommand,
    }

    fn parse(args: &[&str]) -> Result<MemberCommand, clap::Error> {
        Harness::try_parse_from(args).map(|harness| harness.command)
    }

    fn create_role(command: MemberCommand) -> Option<String> {
        let MemberCommand::Create { role, .. } = command else {
            panic!("parsed a non-create command");
        };
        role
    }

    #[test]
    fn create_accepts_the_sole_role_value_monitor() {
        let command = parse(&[
            "cafleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "monitor",
            "--description",
            "d",
            "--role",
            "monitor",
            "PROMPT",
        ])
        .unwrap();
        assert_eq!(create_role(command).as_deref(), Some("monitor"));
    }

    #[test]
    fn create_parses_without_a_role() {
        let command = parse(&[
            "cafleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "d",
            "PROMPT",
        ])
        .unwrap();
        assert_eq!(create_role(command), None);
    }

    #[test]
    fn create_rejects_any_other_role_value() {
        let Err(err) = parse(&[
            "cafleet",
            "create",
            "--fleet-id",
            "1",
            "--name",
            "worker",
            "--description",
            "d",
            "--role",
            "builder",
            "PROMPT",
        ]) else {
            panic!("monitor is the sole accepted --role value");
        };
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    }
}

#[cfg(test)]
mod creation_regressions {
    use super::*;
    use crate::cli::creation::test_support::{Fixture, SpawnFixture};
    use crate::coding_agent::test_support::FakeProbe;
    use crate::multiplexer::RunError;

    fn create(f: &Fixture, mut spawn: SpawnFixture, prompt: &str) -> Result<Value, CafleetError> {
        let mux = spawn.take_mux();
        create_with_options(
            &mut f.conn(),
            &MemberCreateOptions {
                fleet_id: 1,
                name: "worker",
                description: "fixture",
                explicit_agent: None,
                model: None,
                effort: None,
                monitor: false,
                prompt: Some(prompt),
                file: None,
            },
            || Ok(mux),
            &FakeProbe::with_binary("claude", f.dir.path()),
            &SpawnPreparation {
                cwd: &|| Ok(f.dir.path().to_path_buf()),
                env: &|_| None,
            },
            &spawn.execution(),
            f,
        )
    }

    #[test]
    fn registration_failure_has_no_pane_or_registration_guard_to_compensate() {
        let f = Fixture::new(true);
        f.sql("CREATE TRIGGER fail_register BEFORE INSERT ON members BEGIN SELECT RAISE(ABORT, 'registration failed'); END;");
        let error = create(&f, f.spawn(None, false, false), "prompt").unwrap_err();
        assert_eq!(error.exit_code(), 1);
        assert!(error.to_string().contains("registration failed"));
        assert_eq!(f.timeline(), ["current:ok"]);
        assert_eq!(f.count("members"), 2);
        assert_eq!(f.count("member_placements"), 2);
    }

    #[test]
    fn placeholder_failure_only_deregisters_and_keeps_usage_class_on_cleanup_failure() {
        for fail_cleanup in [false, true] {
            let f = Fixture::new(true);
            if fail_cleanup {
                f.sql("CREATE TRIGGER fail_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'deregister failed'); END;");
            }
            let error = create(&f, f.spawn(None, false, false), "{unknown}").unwrap_err();
            assert!(matches!(error, CafleetError::Usage(_)));
            assert_eq!(error.exit_code(), 2);
            assert!(
                error
                    .to_string()
                    .starts_with("Unknown placeholder 'unknown'")
            );
            assert_eq!(
                f.timeline(),
                [
                    "current:ok",
                    if fail_cleanup {
                        "deregister:error"
                    } else {
                        "deregister:ok"
                    },
                    "disarm:registration"
                ]
            );
            if fail_cleanup {
                assert!(error.to_string().contains("cleanup failed for member 3:"));
                assert_eq!(f.count("member_placements"), 3);
            } else {
                f.assert_deregistered();
            }
        }
    }

    #[test]
    fn backend_run_error_closes_before_deregister_even_when_close_fails() {
        let failure = RunError::Failed {
            stderr: "primary run failure".into(),
        };
        for close_fails in [false, true] {
            let f = Fixture::new(true);
            let error = create(
                &f,
                f.spawn(Some(("run", failure.clone())), close_fails, false),
                "prompt",
            )
            .unwrap_err();
            assert!(matches!(error, CafleetError::App(_)));
            assert_eq!(error.exit_code(), 1);
            assert_eq!(
                f.timeline(),
                [
                    "current:ok",
                    "list:ok",
                    "split:ok",
                    "run:error",
                    "write-lock:true",
                    if close_fails {
                        "close:error"
                    } else {
                        "close:ok"
                    },
                    "deregister:ok",
                    "disarm:registration"
                ]
            );
            assert_eq!(
                error
                    .to_string()
                    .matches("cleanup failed for pane w1:p9:")
                    .count(),
                usize::from(close_fails)
            );
            f.assert_deregistered();
        }
    }

    #[test]
    fn placement_error_or_missing_row_kills_before_deregister_despite_cleanup_errors() {
        for missing in [false, true] {
            for (close_fails, deregister_fails) in [(false, false), (true, false), (true, true)] {
                let f = Fixture::new(true);
                f.sql(if missing {
                    "CREATE TRIGGER fail_patch BEFORE UPDATE OF mux_pane_id ON member_placements BEGIN DELETE FROM member_placements WHERE member_id=NEW.member_id; SELECT RAISE(IGNORE); END;"
                } else {
                    "CREATE TRIGGER fail_patch BEFORE UPDATE OF mux_pane_id ON member_placements BEGIN SELECT RAISE(ABORT, 'primary placement failure'); END;"
                });
                if deregister_fails {
                    f.sql("CREATE TRIGGER fail_deregister BEFORE UPDATE OF status ON members BEGIN SELECT RAISE(ABORT, 'secondary deregister failure'); END;");
                }
                let error = create(&f, f.spawn(None, close_fails, false), "prompt").unwrap_err();
                assert_eq!(error.exit_code(), 1);
                let detail = error.to_string();
                assert!(
                    detail.contains(if missing {
                        "placement row vanished"
                    } else {
                        "primary placement failure"
                    }),
                    "{detail}"
                );
                assert_eq!(
                    f.timeline(),
                    [
                        "current:ok",
                        "list:ok",
                        "split:ok",
                        "run:ok",
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
                        "disarm:pane",
                        if deregister_fails {
                            "deregister:error"
                        } else {
                            "deregister:ok"
                        },
                        "disarm:registration"
                    ]
                );
                if deregister_fails {
                    assert!(
                        detail.find("cleanup failed for pane w1:p9:").unwrap()
                            < detail.find("cleanup failed for member 3:").unwrap()
                    );
                    assert!(!detail.contains("Rolled back"));
                } else {
                    f.assert_deregistered();
                }
            }
        }
    }

    #[test]
    fn split_with_unknown_id_deregisters_without_guessed_kill() {
        let f = Fixture::new(true);
        let error = create(&f, f.spawn(None, false, true), "prompt").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("pane ID unknown; pane cleanup unconfirmed")
        );
        assert_eq!(
            f.timeline(),
            [
                "current:ok",
                "list:ok",
                "split:ok",
                "deregister:ok",
                "disarm:registration"
            ]
        );
        f.assert_deregistered();
    }

    #[test]
    fn successful_creation_disarms_both_guards_before_returning_output_value() {
        let f = Fixture::new(true);
        let value = create(&f, f.spawn(None, false, false), "prompt").unwrap();
        assert_eq!(value["member_id"], 3);
        assert_eq!(value["placement"]["mux_pane_id"], "w1:p9");
        assert_eq!(
            f.timeline(),
            [
                "current:ok",
                "list:ok",
                "split:ok",
                "run:ok",
                "disarm:pane",
                "disarm:registration"
            ]
        );
        assert_eq!(f.count("member_placements"), 3);
    }
}
