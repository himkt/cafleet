use clap::{Args, Subcommand};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use super::helpers::{emit, resolve_body, resolve_command_body, resolve_mux};
use crate::broker::records::MemberRecord;
use crate::broker::{self, NewPlacement};
use crate::capture::{CaptureSnapshot, MemberCapture, write_member_capture};
use crate::coding_agent::{CodingAgent, ProfileCatalog, coding_agent};
use crate::config::Settings;
use crate::error::CafleetError;

use crate::multiplexer::{AnyMultiplexer, Lead};
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
    /// Reconcile a recorded member run under exclusive ownership.
    Recover {
        #[arg(index = 1, value_name = "MEMBER_ID", value_parser = clap::value_parser!(i64).range(1..))]
        member_id: i64,
        #[arg(long, requires_all = ["expected_run_id", "expected_owner_epoch", "evidence_ref"])]
        owner_terminated: bool,
        #[arg(long, requires = "owner_terminated", value_parser = clap::value_parser!(i64).range(1..))]
        expected_run_id: Option<i64>,
        #[arg(long, requires = "owner_terminated", value_parser = clap::value_parser!(i64).range(1..))]
        expected_owner_epoch: Option<i64>,
        #[arg(long, requires = "owner_terminated", value_parser = recovery_reference)]
        evidence_ref: Option<String>,
        #[arg(long)]
        json: bool,
    },
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
        #[arg(long, value_name = "MODE", help = CodingAgent::permission_mode_help())]
        permission_mode: Option<String>,
        /// Persist metadata or explicitly redacted screen evidence.
        #[arg(long, default_value = "metadata", value_parser = ["metadata", "redacted"])]
        event_evidence: String,
        /// Absolute UTF-8 JSON redaction configuration for redacted evidence.
        #[arg(long, value_name = "PATH")]
        event_redaction_file: Option<PathBuf>,
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
    /// Send an ordered vector of explicit native keys to a member pane.
    Press {
        #[arg(index = 1, value_name = "MEMBER_ID", value_parser = clap::value_parser!(i64).range(1..))]
        member_id: i64,
        #[arg(index = 2, value_name = "KEY", num_args = 1..=64, required = true)]
        keys: Vec<String>,
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

fn recovery_reference(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err("nonblank termination evidence reference required".into())
    } else {
        Ok(value.to_owned())
    }
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
        MemberCommand::Recover {
            member_id,
            owner_terminated,
            expected_run_id,
            expected_owner_epoch,
            evidence_ref,
            json,
        } => {
            let attestation =
                owner_terminated.then(|| crate::runtime::runner::OwnerTerminationAttestation {
                    expected_run_id: expected_run_id.expect("complete attestation options"),
                    expected_owner_epoch: expected_owner_epoch
                        .expect("complete attestation options"),
                    evidence_ref: evidence_ref.expect("complete attestation options"),
                });
            let report = crate::runtime::runner::recover_member(
                conn,
                settings,
                member_id,
                attestation.as_ref(),
                deadline.instant(),
            )?;
            emit(json, &presentation::runtime_recovery(&report), || {
                crate::output::format_runtime_recovery(&report)
            });
            Ok(())
        }
        MemberCommand::Create {
            fleet_id,
            name,
            description,
            coding_agent,
            model,
            effort,
            permission_mode,
            event_evidence,
            event_redaction_file,
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
            permission_mode.as_deref(),
            &event_evidence,
            event_redaction_file.as_deref(),
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
        MemberCommand::Press { .. } => unreachable!("press has a dedicated typed dispatch"),
        MemberCommand::Capture {
            member_id,
            lines,
            ansi,
            json,
        } => capture(conn, settings, member_id, lines, ansi, json),
    }
}

fn launch_evidence(
    mode: &str,
    file: Option<&Path>,
    catalog: &ProfileCatalog,
) -> Result<(broker::runtime::EventEvidence, Value), CafleetError> {
    use broker::runtime::{EventEvidence, evidence::EvidencePolicy};
    match (mode, file) {
        ("metadata", None) => Ok((EventEvidence::Metadata, json!({"mode":"metadata"}))),
        ("redacted", Some(path)) if path.is_absolute() => {
            let config = std::fs::read_to_string(path).map_err(|cause| {
                CafleetError::Usage(format!("cannot read --event-redaction-file: {cause}"))
            })?;
            let config: Value = serde_json::from_str(&config).map_err(|cause| {
                CafleetError::Usage(format!("invalid --event-redaction-file JSON: {cause}"))
            })?;
            let rules = catalog.rule_ids();
            let rule_refs = rules.iter().map(String::as_str).collect::<Vec<_>>();
            EvidencePolicy::redacted(&config, &rule_refs)?;
            Ok((EventEvidence::Redacted, json!({"mode":"redacted","config":config})))
        }
        _ => Err(CafleetError::Usage(
            "--event-evidence metadata requires no redaction file; redacted requires an absolute --event-redaction-file".into(),
        )),
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
    permission_mode: Option<&str>,
    event_evidence: &str,
    event_redaction_file: Option<&Path>,
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
    let policy = backend.resolve_policy(None, permission_mode)?;
    let catalog = ProfileCatalog::bundled()?;
    let (event_evidence, evidence_policy) =
        launch_evidence(event_evidence, event_redaction_file, &catalog)?;

    let prompt_body = resolve_body(body.prompt.as_deref(), body.file.as_deref(), "--file")?;

    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    backend.ensure_available(&SystemProbe)?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    use crate::runtime::runner;
    use std::io::{IsTerminal, Read};
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(10);
    let probe = backend.probe_native_version(
        Path::new(backend.binary_name()),
        deadline.min(Instant::now() + Duration::from_secs(2)),
    );
    let native = crate::coding_agent::native_launch_identity(backend, probe, &catalog);
    if let Some(warning) = &native.warning {
        eprintln!("{warning}");
    }
    crate::spawn_prompt::substitute_spawn_placeholders(
        &prompt_body,
        fleet_id,
        1,
        director_id,
        agent_name,
    )?;
    let identity = broker::runtime::database_identity(conn)?;
    let next_run: i64 = conn
        .query_row(
            "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1",
            [],
            |row| row.get(0),
        )
        .map_err(crate::broker::members::db_err)?;
    runner::RuntimePaths::for_run(&identity, next_run)?;
    let cwd = std::env::current_dir().map_err(|cause| CafleetError::App(cause.to_string()))?;
    let size = if std::io::stdin().is_terminal() {
        runner::terminal_size(&std::io::stdin())?
    } else {
        runner::TerminalSize { rows: 24, cols: 80 }
    };
    let mut random = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|cause| CafleetError::App(format!("launch nonce unavailable: {cause}")))?;
    let launch_nonce = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pending = runner::PendingMemberSpec {
        fleet_id,
        name: name.into(),
        description: description.into(),
        placement: NewPlacement {
            backend: mux.name().into(),
            mux_session: context.session.clone(),
            mux_window_id: context.window_id.clone(),
            mux_pane_id: None,
            coding_agent: agent_name.into(),
        },
    };
    let run_settings = runner::RunSettings {
        database_identity_json: identity,
        creator: runner::ProcessIdentity::current()?,
        launch_nonce,
        requested_mode: policy.requested_mode.clone(),
        resolved_mode: policy.resolved_mode.clone(),
        mode_source: policy.mode_source,
        permission_argv_json: serde_json::to_value(&policy.permission_argv)
            .expect("string argv serializes"),
        event_evidence,
        native_version: native.native_version,
        profile_version: native.profile_version,
        coverage: native.coverage,
    };
    let mut host = runner::NativeLaunchHost::new(&mux, &context);
    let created = runner::create_registered(
        conn,
        &pending,
        &run_settings,
        |conn, pending| {
            let artifacts =
                runner::RunArtifacts::create_with_deadline(conn, pending.run.run_id, deadline)?;
            let prompt = crate::spawn_prompt::substitute_spawn_placeholders(
                &prompt_body,
                fleet_id,
                pending.member.member_id,
                director_id,
                agent_name,
            )?;
            let argv =
                backend.build_spawn_argv_with_policy(&prompt, name, model, effort, &policy)?;
            let spawn = runner::SpawnSpec {
                argv,
                cwd,
                environment: vec![(
                    "CAFLEET_DATABASE_URL".into(),
                    format!(
                        "sqlite:///{}",
                        pending.run.database_identity_json["canonical_path"]
                            .as_str()
                            .expect("canonical database path")
                    ),
                )],
                evidence_policy,
                size,
            };
            Ok((artifacts, spawn))
        },
        &mut host,
        deadline,
    )?;
    let registered = created.member;
    let member_id = created.run.member_id;
    let placement_view = created.placement;
    let runtime = Some(broker::runtime::runtime_snapshot(
        conn,
        created.started.identity.run_id,
    )?);
    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered.registered_at,
        "placement": presentation::placement(&placement_view),
        "runtime": runtime.as_ref().map(presentation::runtime_snapshot),
    });
    emit(json, &result, || {
        let mut text = format_member(&registered, &placement_view);
        if let Some(runtime) = &runtime {
            text.push('\n');
            text.push_str(&crate::output::format_runtime_snapshot(runtime));
        }
        text
    });
    Ok(())
}

enum DeleteAction {
    Deregistered(&'static str),
    Close(String),
}

fn member_deletion_matches(expected: &MemberRecord, current: &MemberRecord) -> bool {
    expected.member_id == current.member_id
        && expected.fleet_id == current.fleet_id
        && expected.kind == current.kind
        && expected.placement == current.placement
        && expected
            .runtime
            .as_ref()
            .map(|runtime| crate::runtime::runner::stable_run_identity(&runtime.run))
            == current
                .runtime
                .as_ref()
                .map(|runtime| crate::runtime::runner::stable_run_identity(&runtime.run))
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
        |conn, snapshot, _guards| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(crate::broker::members::db_err)?;
            let current = load_member(&tx, member_id, true)?;
            if !member_deletion_matches(snapshot, &current) {
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
    let mut locked = lock_member_deletion(conn, settings, member_id, deadline)?;
    let pane_status = match &locked.value {
        DeleteAction::Deregistered(status) => status.to_string(),
        DeleteAction::Close(_) => {
            let mux = resolve_mux(settings)
                .map_err(|error| CafleetError::App(error.to_string()))?
                .bounded(deadline);
            mux.ensure_available()
                .map_err(|error| CafleetError::App(error.to_string()))?;
            close_member(conn, settings, member_id, &mut locked, &mux, deadline)?
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
    settings: &Settings,
    member_id: i64,
    locked: &mut crate::input::LockedDeletion<MemberRecord, DeleteAction>,
    mux: &AnyMultiplexer,
    deadline: crate::input::Deadline,
) -> Result<String, CafleetError> {
    let DeleteAction::Close(pane_id) = &locked.value else {
        unreachable!("pane close requires a concrete input-locked placement");
    };
    crate::runtime::runner::with_database_deadline(conn, deadline.instant(), |conn| {
        let current = load_member(conn, member_id, true)?;
        if !member_deletion_matches(&locked.snapshot, &current) {
            return Err(CafleetError::App(
                "member placement or runtime ownership changed before cleanup".into(),
            ));
        }
        Ok(())
    })?;
    if let Some(runtime) = crate::broker::runtime::member_runtime(conn, member_id)?
        && (!runtime.run.status.terminal()
            || runtime.run.cleanup_state == crate::broker::runtime::CleanupState::Incomplete)
    {
        let outcome =
            crate::runtime::runner::request_shutdown(conn, &runtime.run, deadline.instant())?;
        if outcome.cleanup_state != crate::broker::runtime::CleanupState::Complete {
            return Err(CafleetError::App(format!(
                "member {member_id} child cleanup remains unconfirmed: {}; inspect with cafleet doctor",
                outcome.diagnostics.join("; ")
            )));
        }
    }
    if let Some(expected) = locked
        .snapshot
        .runtime
        .as_ref()
        .map(|runtime| runtime.run.clone())
    {
        let current = crate::broker::runtime::get_run(conn, expected.run_id)?;
        if crate::runtime::runner::stable_run_identity(&current)
            != crate::runtime::runner::stable_run_identity(&expected)
        {
            return Err(CafleetError::App(
                "member runtime ownership changed during cleanup".into(),
            ));
        }
        let placement = locked
            .snapshot
            .placement
            .as_ref()
            .expect("input-locked member placement");
        let completion = crate::runtime::runner::recover_member_under_pane_lock(
            conn,
            settings,
            member_id,
            &current,
            placement,
            deadline.instant(),
            locked.pane_lock(&settings.database_url, placement)?,
        )?;
        crate::runtime::runner::validate_recovery_completion(&expected, &completion)?;
        if !matches!(
            completion.report.outcome,
            crate::runtime::runner::RecoveryOutcome::Recovered
        ) {
            return Err(CafleetError::App(
                "member recovery remains incomplete; inspect with cafleet doctor".into(),
            ));
        }
        locked
            .snapshot
            .runtime
            .as_mut()
            .expect("captured runtime")
            .run = completion.run;
    }
    crate::runtime::runner::with_database_deadline(conn, deadline.instant(), |conn| {
        let current = load_member(conn, member_id, true)?;
        if !member_deletion_matches(&locked.snapshot, &current) {
            return Err(CafleetError::App(
                "member placement or runtime ownership changed before pane close".into(),
            ));
        }
        Ok(())
    })?;
    let layout = mux.close_pane(pane_id, true).map_err(|error| {
        CafleetError::App(format!(
            "kill_pane failed for pane {pane_id}: {error}. Verify with 'cafleet doctor'."
        ))
    })?;
    crate::input::finalize_deletion(conn, deadline, locked.snapshot.runtime.is_some(), |conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(crate::broker::members::db_err)?;
        let current = load_member(&tx, member_id, true)?;
        if !member_deletion_matches(&locked.snapshot, &current) {
            return Err(CafleetError::App(
                "member placement or runtime ownership changed after pane close".into(),
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

pub(super) fn press(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &crate::multiplexer::PressKeys,
    json: bool,
    deadline: crate::input::Deadline,
) -> Result<(), CafleetError> {
    let acknowledgment = crate::runtime::press_member(conn, settings, member_id, keys, deadline)?;
    emit(json, &presentation::member_press(&acknowledgment), || {
        crate::output::format_member_press(&acknowledgment)
    });
    Ok(())
}

fn dispatch_input(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    text: &str,
    kind: crate::broker::runtime::InputKind,
    label: &str,
    deadline: crate::input::Deadline,
) -> Result<Option<i64>, CafleetError> {
    let id = member.member_id;
    if let Some(runtime) = &member.runtime {
        use crate::runtime::runner::{InputFailure, SubmissionStatus, SubmitInput};
        use std::io::Read;

        let run = &runtime.run;
        let placement = member.placement.as_ref().expect("placed input member");
        let mut random = [0; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|cause| {
                CafleetError::App(format!("cannot generate input operation key: {cause}"))
            })?;
        let random: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let request = SubmitInput {
            operation_key: format!("{}:{}:{random}", kind.as_str(), run.run_id),
            input_kind: kind,
            message_id: None,
            literal: text.into(),
        };
        let result = crate::runtime::runner::submit_wrapped_input(
            conn, settings, run, placement, &request, deadline,
        )
        .map_err(CafleetError::from)?;
        if result.status != SubmissionStatus::Accepted {
            let description = match result.status {
                SubmissionStatus::Held => "held",
                SubmissionStatus::Rejected => "rejected",
                SubmissionStatus::Uncertain => "uncertain",
                SubmissionStatus::Accepted => unreachable!("accepted result returned below"),
            };
            let mut reason = format!("{label} delivery to member {id} is {description}");
            if let Some(detail) = result.reason {
                reason.push_str(": ");
                reason.push_str(&detail);
            }
            return Err(InputFailure {
                source: CafleetError::App(reason),
                run_id: Some(run.run_id),
                interaction_id: runtime.current_interaction_id,
                input_id: result.input_id,
                operation_key: Some(request.operation_key),
            }
            .into());
        }
        return Ok(result.input_id);
    }
    let Some(locked) = crate::input::lock_member(conn, settings, member, deadline)? else {
        return Err(CafleetError::App(format!(
            "member {id}'s pane input remained busy; {label} was not dispatched."
        )));
    };
    if locked.member.runtime.is_some() {
        return Err(CafleetError::App(
            "member runtime changed while acquiring pane input".into(),
        ));
    }
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
    let literal = match kind {
        crate::broker::runtime::InputKind::Prompt => text.to_owned(),
        crate::broker::runtime::InputKind::Exec => format!("! {text}"),
        _ => unreachable!("member dispatch selects prompt or exec"),
    };
    mux.send_prompt(pane, &literal, Lead::Escape).map_err(|error| {
        if error.submission_started() { CafleetError::App(format!(
            "{label} delivery to member {id} is uncertain: {error}. Inspect the pane before retrying."
        )) } else { prewrite(error.to_string()) }
    }).map(|()| None)
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
    let input_id = dispatch_input(
        conn,
        settings,
        &member,
        text,
        crate::broker::runtime::InputKind::Prompt,
        "prompt",
        deadline,
    )?;
    emit(
        json,
        &json!({"member_id":member_id,"pane_id":pane,"text":text,"input_id":input_id}),
        || {
            let mut output = format!("Sent prompt {text:?} to member {} ({pane}).", member.name);
            if let Some(id) = input_id {
                output.push_str(&format!("\nInput: {id}"));
            }
            output
        },
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
    let input_id = dispatch_input(
        conn,
        settings,
        &member,
        command,
        crate::broker::runtime::InputKind::Exec,
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
        "command":command,"dispatched":true,"input_id":input_id}),
        || {
            let mut output = format!(
                "Dispatched native command to member {member_id} ({pane}). Inspect the pane for its result."
            );
            if let Some(id) = input_id {
                output.push_str(&format!("\nInput: {id}"));
            }
            output
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
    mod recovery_deletion;

    use super::*;
    use crate::broker::tests as common;
    use crate::input::{Deadline, PaneLock};
    use crate::multiplexer::{CommandRunner, RunError, TmuxMultiplexer};
    use clap::Parser;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    const EXTERNAL_MEMBER_ENV: &str = "CAFLEET_TEST_EXTERNAL_MEMBER_CONFIG";

    fn external_member_monotonic_ns() -> u64 {
        let time = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Known(
            rustix::time::ClockId::Monotonic,
        ))
        .unwrap();
        let seconds = u64::try_from(time.tv_sec).unwrap();
        let nanos = u64::try_from(time.tv_nsec).unwrap();
        assert!(nanos < 1_000_000_000);
        seconds
            .checked_mul(1_000_000_000)
            .unwrap()
            .checked_add(nanos)
            .unwrap()
    }

    fn external_member_pause(until: Instant) {
        assert!(
            Instant::now() < until,
            "original external member fixture deadline"
        );
        std::thread::sleep(
            Duration::from_millis(2).min(until.saturating_duration_since(Instant::now())),
        );
    }

    fn external_member_private_json(path: &std::path::Path, value: &Value) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let bytes = serde_json::to_vec(value).unwrap();
        let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
        let publication = (|| -> std::io::Result<()> {
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            std::fs::hard_link(file.path(), path)
        })();
        let cleanup = file.close();
        assert!(
            publication.is_ok() && cleanup.is_ok(),
            "owned private JSON publication={publication:?}; temporary cleanup={cleanup:?}"
        );
    }

    fn external_member_expiration(until: Instant) -> u64 {
        let mono = external_member_monotonic_ns();
        let sampled = Instant::now();
        mono.checked_add(
            u64::try_from(
                until
                    .checked_duration_since(sampled)
                    .filter(|remaining| !remaining.is_zero())
                    .expect("original member bound remains live")
                    .as_nanos(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn external_member_deadline(expiration: u64) -> Deadline {
        let before = Instant::now();
        let remaining = expiration
            .checked_sub(external_member_monotonic_ns())
            .filter(|value| *value > 0)
            .expect("the original parent expiration remains live");
        Deadline::until(before + Duration::from_nanos(remaining))
    }

    #[test]
    #[ignore = "owned member command subprocess entry"]
    fn external_input_member_child() {
        let path = std::path::PathBuf::from(
            std::env::var_os(EXTERNAL_MEMBER_ENV)
                .expect("explicit private member subprocess configuration"),
        );
        let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let setup_expiration = config["setup_expiration"].as_u64().unwrap();
        let setup_deadline = external_member_deadline(setup_expiration);
        let settings = Settings {
            database_url: config["database_url"].as_str().unwrap().into(),
            max_text_len: 200,
            multiplexer: Some("tmux".into()),
        };
        let mut conn = crate::db::connect(&settings.database_url).unwrap();
        external_member_private_json(&path.with_extension("ready"), &json!({"ready":true}));
        while !path.with_extension("continue").exists() {
            external_member_pause(setup_deadline.instant());
        }
        let released: Value =
            serde_json::from_slice(&std::fs::read(path.with_extension("continue")).unwrap())
                .unwrap();
        let expiration = released["expiration"].as_u64().unwrap();
        assert!(
            expiration <= setup_expiration,
            "command expiration retains the original outer bound"
        );
        let deadline = external_member_deadline(expiration);
        let member_id = config["member_id"].as_i64().unwrap();
        let json = config["json"].as_bool().unwrap();
        let result = match config["action"].as_str().unwrap() {
            "prompt" => run(
                &mut conn,
                &settings,
                MemberCommand::Prompt {
                    member_id,
                    text: config["literal"].as_str().unwrap().into(),
                    json,
                },
                deadline,
            ),
            "exec" => run(
                &mut conn,
                &settings,
                MemberCommand::Exec {
                    member_id,
                    body: ExecBodyArgs {
                        command: Some(config["literal"].as_str().unwrap().into()),
                        file: None,
                    },
                    json,
                },
                deadline,
            ),
            "show" => run(
                &mut conn,
                &settings,
                MemberCommand::Show { member_id, json },
                deadline,
            ),
            "input_show" => crate::cli::runtime::input(
                &mut conn,
                &settings,
                crate::cli::runtime::InputCommand::Show {
                    input_id: config["input_id"].as_i64().unwrap(),
                    json,
                },
                deadline,
            ),
            other => panic!("unknown owned member subprocess action: {other}"),
        };
        match result {
            Ok(()) => std::process::exit(0),
            Err(cause) => {
                eprintln!("Error: {cause}");
                std::process::exit(cause.exit_code());
            }
        }
    }

    struct ExternalMemberChild {
        child: crate::runtime::OwnedChild,
        until: Instant,
    }
    impl Drop for ExternalMemberChild {
        fn drop(&mut self) {
            self.child.terminate_until(self.until);
        }
    }

    fn external_member_invoke(
        fixture: &crate::runtime::InputServiceFixture,
        action: &str,
        literal: &str,
        json: bool,
        lose_stdout: bool,
        input_id: Option<i64>,
        outer: Instant,
    ) -> (std::process::ExitStatus, String, String) {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = fixture.fixture._dir.path().join(format!(
            "external-member-{}.json",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let setup_expiration = external_member_expiration(outer);
        external_member_private_json(
            &path,
            &json!({
                "database_url":format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                "member_id":fixture.fixture.run.member_id,"literal":literal,"action":action,
                "json":json,"input_id":input_id,"setup_expiration":setup_expiration,
            }),
        );
        let mut owned = ExternalMemberChild {
            child: crate::runtime::OwnedChild::new(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "cli::member::tests::external_input_member_child",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                        "--quiet",
                    ])
                    .env(EXTERNAL_MEMBER_ENV, &path)
                    .env("RUST_BACKTRACE", "0")
                    .env("PATH", fixture.fixture._dir.path())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            ),
            until: outer,
        };
        while !path.with_extension("ready").exists() {
            assert!(
                owned.child.try_wait().unwrap().is_none(),
                "owned member reaches its command-ready gate"
            );
            external_member_pause(outer);
        }
        if lose_stdout {
            drop(
                owned
                    .child
                    .stdout
                    .take()
                    .expect("owned stdout consumer closes after child entry"),
            );
        }
        let command_started = Instant::now();
        let command_until = (command_started + Duration::from_secs(5)).min(outer);
        let expiration = external_member_expiration(command_until).min(setup_expiration);
        external_member_private_json(
            &path.with_extension("continue"),
            &json!({"expiration":expiration}),
        );
        let status = owned.child.poll_until(command_until).unwrap();
        let command_elapsed = command_started.elapsed();
        if status.is_none() {
            owned.child.terminate_until(outer);
        }
        let status = status.unwrap_or_else(|| panic!(
            "owned member exceeded its original command deadline: elapsed={command_elapsed:?}; budget={:?}",
            command_until.duration_since(command_started)));
        let stdout = if lose_stdout {
            assert!(owned.child.stdout.is_none());
            String::new()
        } else {
            let mut bytes = Vec::new();
            owned
                .child
                .stdout
                .as_mut()
                .unwrap()
                .take(16385)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 16384);
            String::from_utf8(bytes).unwrap()
        };
        let mut stderr = Vec::new();
        owned
            .child
            .stderr
            .as_mut()
            .unwrap()
            .take(16385)
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(stderr.len() <= 16384);
        (status, stdout, String::from_utf8(stderr).unwrap())
    }

    fn external_member_input_ids(conn: &Connection, run_id: i64) -> Vec<i64> {
        conn.prepare(
            "SELECT input_id FROM member_input_operations WHERE run_id=?1 ORDER BY input_id",
        )
        .unwrap()
        .query_map([run_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    }

    fn external_member_json(stdout: &str) -> Value {
        serde_json::from_str(
            stdout
                .lines()
                .find(|line| line.starts_with('{'))
                .expect("actual member JSON output"),
        )
        .unwrap()
    }

    #[test]
    fn external_input_member_prompt_exec_route_raw_commands_and_print_actual_durable_ids() {
        use crate::broker::runtime as storage;
        use sha2::{Digest, Sha256};
        crate::runtime::with_input_service(|fixture, outer| {
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            let member =
                crate::broker::get_member(&fixture.fixture.conn, run.member_id, run.fleet_id)
                    .unwrap()
                    .unwrap();
            let placement = member.placement.as_ref().unwrap();
            let pane = placement.mux_pane_id.as_deref().unwrap();
            for (action, kind) in [
                ("prompt", storage::InputKind::Prompt),
                ("exec", storage::InputKind::Exec),
            ] {
                for json in [true, false] {
                    crate::runtime::input_service_composer(fixture, outer);
                    let before = external_member_input_ids(&fixture.fixture.conn, run.run_id);
                    let literal = format!("member {action} λ json={json}");
                    let (status, stdout, stderr) =
                        external_member_invoke(fixture, action, &literal, json, false, None, outer);
                    assert!(
                        status.success(),
                        "actual {action} failed: {status}; stdout={stdout:?}; stderr={stderr:?}"
                    );
                    assert!(stderr.is_empty(), "{stderr}");
                    let after = external_member_input_ids(&fixture.fixture.conn, run.run_id);
                    assert_eq!(after.len(), before.len() + 1);
                    assert_eq!(&after[..before.len()], before);
                    let row =
                        storage::get_input(&fixture.fixture.conn, *after.last().unwrap()).unwrap();
                    assert_eq!(row.kind, kind);
                    assert_eq!(row.state, storage::InputState::Accepted);
                    assert_eq!(row.message_id, None);
                    let prefix = format!("{}:{}:", kind.as_str(), run.run_id);
                    let random = row
                        .operation_key
                        .strip_prefix(&prefix)
                        .expect("one explicit invocation key");
                    assert_eq!(random.len(), 64);
                    assert!(
                        random
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    );
                    let payload = json!({"kind":kind.as_str(),"literal":literal,"message_id":null,"run_id":run.run_id});
                    let digest: String = Sha256::digest(serde_json::to_vec(&payload).unwrap())
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    assert_eq!(
                        row.payload_digest, digest,
                        "Exec binds the raw command before the native prefix"
                    );
                    if json {
                        let expected = if action == "prompt" {
                            format!(
                                "{{\"member_id\":{},\"pane_id\":{},\"text\":{},\"input_id\":{}}}",
                                run.member_id,
                                serde_json::to_string(pane).unwrap(),
                                serde_json::to_string(&literal).unwrap(),
                                row.input_id
                            )
                        } else {
                            format!(
                                "{{\"member_id\":{},\"pane_id\":{},\"coding_agent\":{},\"command\":{},\"dispatched\":true,\"input_id\":{}}}",
                                run.member_id,
                                serde_json::to_string(pane).unwrap(),
                                serde_json::to_string(&placement.coding_agent).unwrap(),
                                serde_json::to_string(&literal).unwrap(),
                                row.input_id
                            )
                        };
                        assert_eq!(
                            stdout.lines().find(|line| line.starts_with('{')).unwrap(),
                            expected
                        );
                    } else {
                        let original = if action == "prompt" {
                            format!(
                                "Sent prompt {literal:?} to member {} ({pane}).",
                                member.name
                            )
                        } else {
                            format!(
                                "Dispatched native command to member {} ({pane}). Inspect the pane for its result.",
                                run.member_id
                            )
                        };
                        assert!(
                            stdout.contains(&format!("{original}\nInput: {}\n", row.input_id)),
                            "{stdout:?}"
                        );
                    }
                    let line = if action == "exec" {
                        format!("! {literal}")
                    } else {
                        literal.clone()
                    };
                    crate::runtime::wait_input_service_record(fixture, &line, outer);
                    crate::runtime::input_service_barrier(fixture, outer);
                    assert_eq!(
                        crate::runtime::input_service_records(fixture)
                            .iter()
                            .filter(|record| record["line"] == line)
                            .count(),
                        1
                    );
                    assert_eq!(
                        storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
                        row
                    );
                }
            }
        });
    }

    #[test]
    fn external_input_member_lost_stdout_keeps_discoverable_receipt_and_same_key_never_retypes() {
        use crate::broker::runtime as storage;
        use crate::runtime::runner::{SubmissionStatus, SubmitInput, submit_wrapped_input};
        crate::runtime::with_input_service(|fixture, outer| {
            let run = storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
            let before = external_member_input_ids(&fixture.fixture.conn, run.run_id);
            let literal = "member output loss λ";
            let (status, stdout, stderr) =
                external_member_invoke(fixture, "prompt", literal, true, true, None, outer);
            assert_eq!(status.code(), Some(101), "{status}; {stderr}");
            assert!(stdout.is_empty());
            assert!(
                stderr.contains("stdout") && stderr.contains("Broken pipe"),
                "{stderr}"
            );
            let ids = external_member_input_ids(&fixture.fixture.conn, run.run_id);
            assert_eq!(ids.len(), before.len() + 1);
            let row = storage::get_input(&fixture.fixture.conn, *ids.last().unwrap()).unwrap();
            assert_eq!(row.state, storage::InputState::Accepted);
            let (status, stdout, stderr) =
                external_member_invoke(fixture, "show", "", true, false, None, outer);
            assert!(status.success(), "{status}; {stderr}");
            assert!(
                external_member_json(&stdout)["runtime"]["unresolved_input_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(row.input_id))
            );
            let (status, stdout, stderr) = external_member_invoke(
                fixture,
                "input_show",
                "",
                true,
                false,
                Some(row.input_id),
                outer,
            );
            assert!(status.success(), "{status}; {stderr}");
            let discovered = external_member_json(&stdout);
            assert_eq!(discovered["input"]["input_id"], row.input_id);
            assert_eq!(discovered["input"]["operation_key"], row.operation_key);
            assert_eq!(discovered["input"]["state"], "accepted");
            let member =
                crate::broker::get_member(&fixture.fixture.conn, run.member_id, run.fleet_id)
                    .unwrap()
                    .unwrap();
            let placement = member.placement.unwrap();
            let settings = Settings {
                database_url: format!("sqlite:///{}", fixture.fixture.conn.path().unwrap()),
                max_text_len: 200,
                multiplexer: Some("tmux".into()),
            };
            let request = SubmitInput {
                operation_key: row.operation_key.clone(),
                input_kind: row.kind,
                message_id: row.message_id,
                literal: literal.into(),
            };
            let result = submit_wrapped_input(
                &mut fixture.fixture.conn,
                &settings,
                &run,
                &placement,
                &request,
                Deadline::until(outer),
            )
            .unwrap();
            assert_eq!(result.status, SubmissionStatus::Accepted);
            assert_eq!(result.input_id, Some(row.input_id));
            assert_eq!(
                storage::get_input(&fixture.fixture.conn, row.input_id).unwrap(),
                row
            );
            crate::runtime::wait_input_service_record(fixture, literal, outer);
            crate::runtime::input_service_barrier(fixture, outer);
            assert_eq!(
                crate::runtime::input_service_records(fixture)
                    .iter()
                    .filter(|record| record["line"] == literal)
                    .count(),
                1
            );
        });
    }

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
            let mut locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
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
                close_member(&mut conn, &settings, worker, &mut locked, &mux, deadline).unwrap(),
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
        let mut locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
        let runner = Rc::new(CloseRunner {
            calls: Cell::new(0),
            failure: None,
            check: Box::new(|| {}),
        });
        let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()))
            .bounded(deadline);
        let error = close_member(&mut conn, &settings, worker, &mut locked, &mux, deadline)
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
