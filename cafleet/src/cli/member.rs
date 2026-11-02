use crate::broker;
use crate::broker::MemberRecord;
use crate::broker::NewPlacement;
use crate::broker::runtime::DeliveryProjection;
use crate::cli::available_mux;
use crate::cli::emit;
use crate::cli::nonblank;
use crate::cli::resolve_body;
use crate::cli::resolve_command_body;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::ProfileCatalog;
use crate::coding_agent::SpawnProbe;
use crate::coding_agent::coding_agent;
use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::AnyMultiplexer;
use crate::output;
use crate::output::format_member;
use crate::output::format_member_delete;
use crate::output::format_member_detail;
use crate::output::format_member_exec;
use crate::output::format_member_list;
use crate::output::format_member_prompt;
use crate::runtime::system::SystemProbe;
use crate::time;
use chrono::DateTime;
use chrono::Utc;
use clap::Args;
use clap::Subcommand;
use rusqlite::Connection;
use serde_json::Value;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

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

#[derive(Args)]
pub(crate) struct CreateArgs {
    /// The fleet the new member joins.
    #[arg(long = "fleet-id", value_name = "INT")]
    pub(super) fleet_id: i64,
    /// Display name.
    #[arg(long)]
    pub(super) name: String,
    /// One-sentence purpose.
    #[arg(long)]
    pub(super) description: String,
    /// Backend binary to spawn / record [default: inherits the Director's backend].
    #[arg(long = "coding-agent", value_enum)]
    pub(super) coding_agent: Option<CodingAgent>,
    /// Model passed to the backend binary.
    #[arg(long)]
    pub(super) model: Option<String>,
    /// Reasoning-effort level (claude, codex, hermes).
    #[arg(long)]
    pub(super) effort: Option<String>,
    #[arg(long, value_name = "MODE", help = CodingAgent::permission_mode_help())]
    pub(super) permission_mode: Option<String>,
    /// Persist metadata or explicitly redacted screen evidence.
    #[arg(long, default_value = "metadata", value_parser = ["metadata", "redacted"])]
    pub(super) event_evidence: String,
    /// Absolute UTF-8 JSON redaction configuration for redacted evidence.
    #[arg(long, value_name = "PATH")]
    pub(super) event_redaction_file: Option<PathBuf>,
    #[command(flatten)]
    pub(super) body: PromptArgs,
    /// Output in JSON format.
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Subcommand)]
pub(crate) enum MemberCommand {
    /// Reconcile a recorded member run under exclusive ownership.
    Recover {
        /// The member whose recorded run is reconciled.
        #[arg(index = 1, value_name = "MEMBER_ID", value_parser = clap::value_parser!(i64).range(1..))]
        member_id: i64,
        /// Attest that the inspected owner and startup processes have terminated.
        #[arg(long, requires_all = ["expected_run_id", "expected_owner_epoch", "evidence_ref"])]
        owner_terminated: bool,
        /// Run ID from the inspected registry snapshot the attestation binds to.
        #[arg(long, requires = "owner_terminated", value_parser = clap::value_parser!(i64).range(1..))]
        expected_run_id: Option<i64>,
        /// Owner epoch from the inspected registry snapshot the attestation binds to.
        #[arg(long, requires = "owner_terminated", value_parser = clap::value_parser!(i64).range(1..))]
        expected_owner_epoch: Option<i64>,
        /// Reference to the operator's recorded termination evidence.
        #[arg(long, requires = "owner_terminated", value_parser = nonblank("nonblank termination evidence reference required"))]
        evidence_ref: Option<String>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
    },
    /// Register a member and spawn its coding-agent pane.
    Create(CreateArgs),
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
    /// Submit a native shell command through the member's runner, which uses its measured
    /// submission plan, delivery fallback or queue.
    #[command(override_usage = "cafleet member exec [OPTIONS] <MEMBER_ID> <COMMAND|--file <PATH>>")]
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
        /// The target member.
        #[arg(index = 1, value_name = "MEMBER_ID", value_parser = clap::value_parser!(i64).range(1..))]
        member_id: i64,
        /// One to 64 keys sent in order, with no implicit Escape or Enter.
        #[arg(index = 2, value_name = "KEY", num_args = 1..=64, required = true)]
        keys: Vec<String>,
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
    let member = crate::broker::members::active_member(conn, member_id)?;
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

pub(crate) fn run(
    conn: &mut Connection,
    settings: &Settings,
    command: MemberCommand,
    deadline: crate::time::Deadline,
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
                owner_terminated.then(|| crate::runtime::OwnerTerminationAttestation {
                    expected_run_id: expected_run_id.expect("complete attestation options"),
                    expected_owner_epoch: expected_owner_epoch
                        .expect("complete attestation options"),
                    evidence_ref: evidence_ref.expect("complete attestation options"),
                });
            let report = crate::runtime::recover_member(
                conn,
                settings,
                member_id,
                attestation.as_ref(),
                deadline.instant(),
            )?;
            emit(
                json,
                &crate::output::json::runtime_recovery(&report),
                || crate::output::format_runtime_recovery(&report),
            );
            Ok(())
        }
        MemberCommand::Create(args) => create(conn, settings, &args),
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
    use broker::runtime::EventEvidence;
    use broker::runtime::evidence::EvidencePolicy;
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

fn create(
    conn: &mut Connection,
    settings: &Settings,
    args: &CreateArgs,
) -> Result<(), CafleetError> {
    let fleet_id = args.fleet_id;
    let name = args.name.as_str();
    let description = args.description.as_str();
    let explicit_agent = args.coding_agent;
    let model = args.model.as_deref();
    let effort = args.effort.as_deref();
    let permission_mode = args.permission_mode.as_deref();
    let event_evidence = args.event_evidence.as_str();
    let event_redaction_file = args.event_redaction_file.as_deref();
    let body = &args.body;
    let json = args.json;

    let fleet = broker::get_fleet(conn, fleet_id)?
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

    let mux = available_mux(settings, None)?;
    let executable = backend.ensure_available(&SystemProbe, &policy.resolved_mode)?;
    let context = mux
        .context_discovery()
        .map_err(|e| CafleetError::App(e.to_string()))?;

    use crate::runtime as runner;
    use std::io::IsTerminal;
    use std::time::Duration;
    use std::time::Instant;
    let deadline = Instant::now() + Duration::from_secs(10);
    let hermes_home = (backend == crate::coding_agent::CodingAgent::Hermes)
        .then(|| {
            let base =
                backend.config_dir(&|name| SystemProbe.env_var(name), &SystemProbe.home_dir())?;
            crate::coding_agent::hermes_settlement::effective_home(
                &executable,
                &base,
                deadline.min(Instant::now() + Duration::from_secs(2)),
            )
        })
        .transpose()?;
    let probe = backend.probe_native_version(
        &executable,
        deadline.min(Instant::now() + Duration::from_secs(2)),
    );
    let native = crate::coding_agent::native_launch_identity(backend, probe, &catalog)?;
    if let Some(warning) = &native.warning {
        eprintln!("{warning}");
    }
    substitute_spawn_placeholders(&prompt_body, fleet_id, 1, director_id, agent_name)?;
    let identity = broker::runtime::database_identity(conn)?;
    let environment = backend.spawn_environment(
        &format!(
            "sqlite:///{}",
            identity["canonical_path"]
                .as_str()
                .expect("canonical database path")
        ),
        &|name| SystemProbe.env_var(name),
    )?;
    let next_run: i64 = conn.query_row(
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1",
        [],
        |row| row.get(0),
    )?;
    runner::RuntimePaths::for_run(&identity, next_run)?;
    let cwd = std::env::current_dir().map_err(|cause| CafleetError::App(cause.to_string()))?;
    let size = if std::io::stdin().is_terminal() {
        runner::terminal_size(&std::io::stdin())?
    } else {
        runner::TerminalSize { rows: 24, cols: 80 }
    };
    let launch_nonce = crate::runtime::system::random_hex32()
        .map_err(|cause| CafleetError::App(format!("launch nonce unavailable: {cause}")))?;
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
            if let Some(hermes_home) = &hermes_home {
                broker::runtime::set_native_session(
                    conn,
                    pending.run.run_id,
                    &crate::coding_agent::hermes_settlement::NativeSession {
                        hermes_home: hermes_home.clone(),
                        session_id: None,
                    },
                )?;
            }
            let prompt = substitute_spawn_placeholders(
                &prompt_body,
                fleet_id,
                pending.member.member_id,
                director_id,
                agent_name,
            )?;
            let argv =
                backend.launch_argv(&executable, &prompt, name, &cwd, model, effort, &policy)?;
            let spawn = runner::SpawnSpec {
                argv,
                cwd,
                environment: environment.clone(),
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
    let runtime = broker::runtime::runtime_snapshot(conn, created.started.identity.run_id)?;
    let result = json!({
        "member_id": member_id,
        "name": name,
        "registered_at": registered.registered_at,
        "placement": crate::output::json::placement(&placement_view),
        "runtime": crate::output::json::runtime_snapshot(&runtime),
    });
    emit(json, &result, || {
        format!(
            "{}\n{}",
            format_member(&registered, &placement_view),
            crate::output::format_runtime_snapshot(&runtime)
        )
    });
    Ok(())
}

enum DeleteAction {
    Deregistered(&'static str),
    Close,
}

fn lock_member_deletion(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    deadline: crate::time::Deadline,
) -> Result<crate::pane_lock::LockedDeletion<MemberRecord, DeleteAction>, CafleetError> {
    crate::pane_lock::lock_deletion(
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
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let current = load_member(&tx, member_id, true)?;
            if !crate::cli::fleet::member_deletion_matches(snapshot, &current) {
                return Ok(None);
            }
            if current.kind == crate::broker::MemberKind::Director {
                return Err(CafleetError::App(
                    "cannot deregister the root Director; use 'cafleet fleet delete' instead"
                        .into(),
                ));
            }
            let has_pane = current
                .placement
                .as_ref()
                .is_some_and(|placement| placement.mux_pane_id.is_some());
            let action = if has_pane {
                DeleteAction::Close
            } else {
                let changed = crate::broker::members::deregister_member_rows(&tx, member_id)?;
                assert!(changed, "the revalidated member remains active");
                DeleteAction::Deregistered(if current.placement.is_none() {
                    crate::cli::fleet::NO_PLACEMENT_STATUS
                } else {
                    crate::cli::fleet::PENDING_PANE_STATUS
                })
            };
            tx.commit()?;
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
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    let deadline = match broker::runtime::member_runtime(conn, member_id)? {
        Some(runtime) if runtime.run.backend == crate::coding_agent::CodingAgent::Hermes.name() => {
            crate::coding_agent::hermes_settlement::deletion_deadline(&runtime.run.backend)
        }
        _ => deadline,
    };
    let mut locked = lock_member_deletion(conn, settings, member_id, deadline)?;
    let pane_status = match &locked.value {
        DeleteAction::Deregistered(status) => status.to_string(),
        DeleteAction::Close => {
            let mux = available_mux(settings, Some(deadline))?;
            close_member(conn, settings, member_id, &mut locked, &mux, deadline)?
        }
    };
    drop(locked);
    emit(
        json,
        &json!({"member_id":member_id, "pane_status":pane_status}),
        || format_member_delete(member_id, &pane_status),
    );
    Ok(())
}

fn close_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    locked: &mut crate::pane_lock::LockedDeletion<MemberRecord, DeleteAction>,
    mux: &AnyMultiplexer,
    deadline: crate::time::Deadline,
) -> Result<String, CafleetError> {
    assert!(
        matches!(locked.value, DeleteAction::Close),
        "pane close requires a concrete input-locked placement"
    );
    assert_eq!(locked.snapshot.member_id, member_id);
    let placement = locked
        .snapshot
        .placement
        .clone()
        .expect("input-locked member placement");
    let (snapshot, held) = locked.snapshot_and_pane_lock(&settings.database_url, &placement)?;
    crate::cli::fleet::end_member(
        conn,
        settings,
        snapshot,
        held,
        &mut crate::cli::fleet::MultiplexerDeletionHost(mux),
        deadline,
        &mut |_| Ok(()),
    )
}

fn show(conn: &mut Connection, member_id: i64, json: bool) -> Result<(), CafleetError> {
    let member = load_member(conn, member_id, true)?;
    let value = crate::output::json::member(&member);
    emit(json, &value, || format_member_detail(&member));
    Ok(())
}

fn list(conn: &mut Connection, fleet_id: i64, json: bool) -> Result<(), CafleetError> {
    let members = broker::list_members(conn, fleet_id)?;
    let payload = Value::Array(
        members
            .iter()
            .map(crate::output::json::member_activity)
            .collect(),
    );
    emit(json, &payload, || format_member_list(&members));
    Ok(())
}

pub(super) fn press(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    keys: &crate::multiplexer::PressKeys,
    json: bool,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    let acknowledgment = crate::runtime::press_member(conn, settings, member_id, keys, deadline)?;
    emit(
        json,
        &crate::output::json::member_press(&acknowledgment),
        || crate::output::format_member_press(&acknowledgment),
    );
    Ok(())
}

fn wrapped_delivery_refusal(
    label: &str,
    member_id: i64,
    result: &crate::runtime::SubmissionResult,
) -> String {
    use crate::runtime::SubmissionStatus;
    let description = match result.status {
        SubmissionStatus::Held => "held",
        SubmissionStatus::Rejected => "rejected",
        SubmissionStatus::Uncertain => "uncertain",
        SubmissionStatus::Accepted | SubmissionStatus::Queued => {
            unreachable!("submitted and queued results are not refusals")
        }
    };
    let mut reason = format!("{label} delivery to member {member_id} is {description}");
    if let Some(detail) = &result.reason {
        reason.push_str(": ");
        reason.push_str(detail);
    }
    if let Some(method) = result
        .delivery
        .as_ref()
        .and_then(|delivery| delivery.method)
    {
        reason.push_str(&format!(" (method={})", method.as_str()));
    }
    reason
}

fn dispatch_input(
    conn: &mut Connection,
    settings: &Settings,
    member: &MemberRecord,
    text: &str,
    kind: crate::broker::runtime::InputKind,
    label: &str,
    deadline: crate::time::Deadline,
) -> Result<(Option<i64>, DeliveryProjection), CafleetError> {
    let id = member.member_id;
    let runtime = broker::members::require_runtime(member)?;
    use crate::runtime::InputFailure;
    use crate::runtime::SubmissionStatus;
    use crate::runtime::SubmitInput;

    let run = &runtime.run;
    let placement = member.placement.as_ref().expect("placed input member");
    let random = crate::runtime::system::random_hex32().map_err(|cause| {
        CafleetError::App(format!("cannot generate input operation key: {cause}"))
    })?;
    let request = SubmitInput {
        operation_key: format!("{}:{}:{random}", kind.as_str(), run.run_id),
        input_kind: kind,
        message_id: None,
        literal: text.into(),
    };
    let result =
        crate::runtime::submit_wrapped_input(conn, settings, run, placement, &request, deadline)
            .map_err(CafleetError::from)?;
    if !matches!(
        result.status,
        SubmissionStatus::Accepted | SubmissionStatus::Queued
    ) {
        return Err(InputFailure {
            source: CafleetError::App(wrapped_delivery_refusal(label, id, &result)),
            run_id: Some(run.run_id),
            interaction_id: runtime.current_interaction_id,
            input_id: result.input_id,
            operation_key: Some(request.operation_key),
        }
        .into());
    }
    let delivery = result
        .delivery
        .expect("validated submission replies carry their delivery");
    Ok((result.input_id, delivery))
}

struct Dispatch {
    kind: crate::broker::runtime::InputKind,
    command: &'static str,
    label: &'static str,
    director_refusal: &'static str,
}

fn dispatch_to_member(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    text: &str,
    dispatch: Dispatch,
    deadline: crate::time::Deadline,
) -> Result<(MemberRecord, String, Option<i64>, DeliveryProjection), CafleetError> {
    let member = crate::db::with_database_budget(conn, deadline, |conn| {
        load_member(conn, member_id, false)
    })?;
    if member.kind == crate::broker::MemberKind::Director {
        return Err(CafleetError::App(dispatch.director_refusal.into()));
    }
    let pane = require_pane(&member, member_id, dispatch.command)?;
    let (input_id, delivery) = dispatch_input(
        conn,
        settings,
        &member,
        text,
        dispatch.kind,
        dispatch.label,
        deadline,
    )?;
    Ok((member, pane, input_id, delivery))
}

fn prompt(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    text: &str,
    json: bool,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    if !crate::pane_lock::valid_input(text) {
        return Err(CafleetError::Usage(
            "text must be one non-empty line without control characters.".into(),
        ));
    }
    let (member, pane, input_id, delivery) = dispatch_to_member(
        conn,
        settings,
        member_id,
        text,
        Dispatch {
            kind: crate::broker::runtime::InputKind::Prompt,
            command: "prompt",
            label: "prompt",
            director_refusal: "cannot prompt the Director's own pane",
        },
        deadline,
    )?;
    emit(
        json,
        &crate::output::json::member_prompt(member_id, &pane, text, input_id, &delivery),
        || format_member_prompt(member_id, text, &member.name, &pane, input_id, &delivery),
    );
    Ok(())
}

fn exec(
    conn: &mut Connection,
    settings: &Settings,
    member_id: i64,
    body: &ExecBodyArgs,
    json: bool,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    let command = body
        .command
        .as_deref()
        .expect("exec input was resolved before its deadline");
    let (member, pane, input_id, delivery) = dispatch_to_member(
        conn,
        settings,
        member_id,
        command,
        Dispatch {
            kind: crate::broker::runtime::InputKind::Exec,
            command: "exec",
            label: "native command",
            director_refusal: "cannot exec in the Director's own pane",
        },
        deadline,
    )?;
    let agent = &member
        .placement
        .as_ref()
        .expect("placed member")
        .coding_agent;
    emit(
        json,
        &crate::output::json::member_exec(member_id, &pane, agent, command, input_id, &delivery),
        || format_member_exec(member_id, &pane, input_id, &delivery),
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
    let mux = available_mux(settings, None)?;

    let member = load_member(conn, member_id, false)?;
    let pane_id = require_pane(&member, member_id, "capture")?;
    let raw = mux
        .capture_pane(&pane_id, lines)
        .map_err(|e| CafleetError::App(format!("capture failed: {e}")))?;
    let capture = MemberCapture {
        member_id,
        pane_id,
        lines,
        snapshot: CaptureSnapshot::from_raw(&raw, ansi, chrono::Utc::now()),
    };
    write_member_capture(&mut std::io::stdout(), &capture, json)
}

const BRACE_HINT: &str = "Double literal braces ({{, }}) to keep them as text.";

/// The spawn-placeholder mini-formatter used by `member create`:
/// Python brace grammar — `{{` / `}}` escapes, the four placeholders, and the
/// unknown-vs-malformed usage-error taxonomy (both exit 2).
pub(crate) fn substitute_spawn_placeholders(
    body: &str,
    fleet_id: i64,
    member_id: i64,
    director_member_id: i64,
    coding_agent: &str,
) -> Result<String, CafleetError> {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    continue;
                }
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') => {
                            return Err(malformed("unexpected '{' in placeholder name"));
                        }
                        Some(k) => key.push(k),
                        None => return Err(malformed("expected '}' before end of string")),
                    }
                }
                match key.as_str() {
                    "fleet_id" => out.push_str(&fleet_id.to_string()),
                    "member_id" => out.push_str(&member_id.to_string()),
                    "director_member_id" => out.push_str(&director_member_id.to_string()),
                    "coding_agent" => out.push_str(coding_agent),
                    _ => {
                        return Err(CafleetError::Usage(format!(
                            "Unknown placeholder '{key}' in custom prompt. Supported \
                             placeholders: {{fleet_id}}, {{member_id}}, \
                             {{director_member_id}}, {{coding_agent}}. {BRACE_HINT}"
                        )));
                    }
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push('}');
                } else {
                    return Err(malformed("single '}' encountered outside a placeholder"));
                }
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

fn malformed(detail: &str) -> CafleetError {
    CafleetError::Usage(format!("Malformed custom prompt: {detail}. {BRACE_HINT}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureSnapshot {
    pub(crate) content: String,
    pub(crate) captured_at: String,
    pub(crate) content_sha256: String,
}
impl CaptureSnapshot {
    pub(crate) fn from_raw(raw: &str, ansi: bool, now: DateTime<Utc>) -> Self {
        let content = if ansi {
            raw.to_owned()
        } else {
            output::strip_ansi(raw)
        };
        let content_sha256 = crate::digest::sha256_hex(content.as_bytes());
        Self {
            content,
            captured_at: time::format_utc(now),
            content_sha256,
        }
    }
}

pub(crate) struct MemberCapture {
    pub(crate) member_id: i64,
    pub(crate) pane_id: String,
    pub(crate) lines: i64,
    pub(crate) snapshot: CaptureSnapshot,
}
pub(crate) fn write_member_capture(
    out: &mut dyn Write,
    capture: &MemberCapture,
    json: bool,
) -> Result<(), CafleetError> {
    let result = if json {
        writeln!(
            out,
            "{}",
            output::format_json(&crate::output::json::member_capture(capture))
        )
    } else {
        out.write_all(capture.snapshot.content.as_bytes())
    };
    result.map_err(|e| CafleetError::App(format!("stdout write failed: {e}")))
}

#[cfg(test)]
mod tests {
    mod recovery_deletion {
        use crate::broker::runtime as storage;
        use crate::broker::runtime::*;
        use crate::cli::member::tests::*;
        use crate::runtime::ProcessIdentity;
        use crate::runtime::RunArtifacts;
        use crate::runtime::RunLock;
        use crate::runtime::RuntimePaths;
        use crate::runtime::system::test_support::TestChild;
        use std::process::Command;
        use std::process::Stdio;

        #[test]
        fn runner_recovery_member_deletion_terminal_no_work_finishes_under_the_original_guard() {
            run_case("terminal");
        }

        #[test]
        fn runner_recovery_member_deletion_foreign_epoch_after_close_preserves_registration() {
            run_case("epoch");
        }

        #[test]
        fn runner_recovery_member_deletion_foreign_placement_after_close_preserves_registration() {
            run_case("placement");
        }

        fn run_case(scenario: &str) {
            let dir = crate::runtime::test_support::short_tempdir();
            let log = dir.path().join("child.log");
            let file = std::fs::File::create(&log).unwrap();
            let started = Instant::now();
            let cleanup = started + Duration::from_secs(6);
            let mut child = TestChild::new(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        runner_recovery_member_deletion_child_path(),
                        "--ignored",
                        "--nocapture",
                    ])
                    .env("CAFLEET_TEST_MEMBER_RECOVERY_CASE", scenario)
                    .stdin(Stdio::null())
                    .stdout(Stdio::from(file.try_clone().unwrap()))
                    .stderr(Stdio::from(file))
                    .spawn()
                    .unwrap(),
            );
            let status = child.poll_until(started + Duration::from_secs(5)).unwrap();
            if status.is_none() {
                child.terminate_until(cleanup);
            }
            let diagnostics = std::fs::read_to_string(&log).unwrap();
            assert!(
                status.is_some_and(|status| status.success()),
                "{scenario}: {status:?}\n{diagnostics}"
            );
        }

        pub(crate) fn runner_recovery_member_deletion_child_path() -> &'static str {
            test_path!(runner_recovery_member_deletion_child)
        }

        #[test]
        #[ignore = "owned isolated member-deletion helper"]
        fn runner_recovery_member_deletion_child() {
            let scenario = std::env::var("CAFLEET_TEST_MEMBER_RECOVERY_CASE").unwrap();
            assert!(matches!(
                scenario.as_str(),
                "terminal" | "epoch" | "placement"
            ));
            let _dir = crate::runtime::test_support::short_tempdir();
            let mut conn = migrated_conn(&_dir);
            let settings =
                Settings::for_database(std::path::Path::new(conn.path().unwrap()), Some("tmux"));
            let (fleet, director) = create_fleet_fixture(&mut conn, "member-recovery");
            let worker = register(&mut conn, fleet, "worker", Some("%9"));
            crate::broker::members::test_support::update_placement_pane_id(&mut conn, worker, "%9")
                .unwrap();
            let identity = storage::database_identity(&conn).unwrap();
            let process = ProcessIdentity::current().unwrap();
            let run = storage::create_run(
                &mut conn,
                &NewRun {
                    member_id: worker,
                    fleet_id: fleet,
                    director_member_id: director,
                    backend: "claude".into(),
                    database_identity_json: identity.clone(),
                    launch_nonce: "a".repeat(64),
                    runtime_path: RuntimePaths::for_run(&identity, 1)
                        .unwrap()
                        .run_dir
                        .to_str()
                        .unwrap()
                        .into(),
                    requested_mode: None,
                    resolved_mode: "default".into(),
                    mode_source: ModeSource::Default,
                    permission_argv_json: json!(["--permission-mode", "default"]),
                    event_evidence: EventEvidence::Metadata,
                    native_version: None,
                    coverage: Coverage::Unknown,
                    owner_pid: process.pid,
                    owner_start_identity: process.start_identity.clone(),
                },
            )
            .unwrap();
            let _artifacts = RunArtifacts::create(&mut conn, run.run_id).unwrap();
            let run = storage::get_run(&conn, run.run_id).unwrap();
            let runtime_lock = RunLock::acquire(&conn, &run.owner()).unwrap().unwrap();
            let owner = runtime_lock
                .transfer_to_runner(&mut conn, &run.owner(), process)
                .unwrap();
            conn.execute("UPDATE member_runs SET status='exited',cleanup_state='complete',exit_code=0,ended_at='2026-10-26T20:00:00Z' WHERE run_id=?1",[run.run_id]).unwrap();
            drop(runtime_lock);
            let deadline = Deadline::until(Instant::now() + Duration::from_secs(2));
            let mut locked = lock_member_deletion(&mut conn, &settings, worker, deadline).unwrap();
            let placement = locked.snapshot.placement.clone().unwrap();
            let database_url = settings.database_url.clone();
            let run_id = run.run_id;
            let expected_epoch = owner.owner_epoch;
            let mutation = scenario.clone();
            let runner = Rc::new(CloseRunner {
                calls: Cell::new(0),
                failure: None,
                check: Box::new(move || {
                    assert!(
                        PaneLock::try_acquire(&database_url, &placement)
                            .unwrap()
                            .is_none()
                    );
                    let check = crate::db::connect(&database_url).unwrap();
                    let current = storage::get_run(&check, run_id).unwrap();
                    assert_eq!(
                        current.owner_epoch, expected_epoch,
                        "a terminal no-work observation advances no owner"
                    );
                    assert_eq!(
                        check
                            .query_row(
                                "SELECT COUNT(*) FROM member_runtime_events WHERE run_id=?1",
                                [run_id],
                                |row| row.get::<_, i64>(0)
                            )
                            .unwrap(),
                        0
                    );
                    match mutation.as_str() {
                        "terminal" => {}
                        "epoch" => {
                            check
                                .execute(
                                    "UPDATE member_runs SET owner_epoch=owner_epoch+1 WHERE run_id=?1",
                                    [run_id],
                                )
                                .unwrap();
                        }
                        "placement" => {
                            check.execute("UPDATE member_placements SET mux_pane_id='%foreign' WHERE member_id=?1",[worker]).unwrap();
                        }
                        _ => unreachable!(),
                    }
                }),
            });
            let mux =
                AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), Default::default()))
                    .bounded(deadline);
            let result = close_member(&mut conn, &settings, worker, &mut locked, &mux, deadline);
            assert_eq!(runner.calls.get(), 1);
            if scenario == "terminal" {
                assert_eq!(result.unwrap(), "%9 (killed)");
                assert!(broker::get_member(&conn, worker, fleet).unwrap().is_none());
            } else {
                let error = match result {
                    Ok(_) => {
                        panic!(
                            "foreign {scenario} change preserves registration after the fake pane close"
                        )
                    }
                    Err(error) => error,
                };
                assert!(error.message().contains("closed"), "{error}");
                assert!(broker::get_member(&conn, worker, fleet).unwrap().is_some());
            }
            assert!(
                broker::get_member(&conn, director, fleet)
                    .unwrap()
                    .is_some()
            );
        }
    }

    use crate::broker::fleets::test_support::create_fleet_fixture;
    use crate::broker::members::test_support::register;
    use crate::broker::runtime::DeliveryMethod;
    use crate::broker::runtime::DeliveryStatus;
    use crate::cli::member::*;
    use crate::db::test_support::migrated_conn;
    use crate::multiplexer::CommandRunner;
    use crate::multiplexer::RunError;
    use crate::multiplexer::TmuxMultiplexer;
    use crate::pane_lock::PaneLock;
    use crate::time::Deadline;
    use clap::Parser;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;
    use std::time::Instant;

    fn assert_run_less_input_failure(action: &str) {
        let (_dir, settings, conn, member_id) = recovery_entry::run_less_registry();
        let id = member_id.to_string();
        let (output, pane_calls) = crate::cli::test_support::isolated_cli(
            &settings,
            &["member", action, &id, "literal input", "--json"],
        );
        assert!(pane_calls.is_empty(), "{action}: {pane_calls}");
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("Error: member {member_id} has no recorded run\n")
        );
        assert!(
            broker::active_member_fleet(&conn, member_id)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM member_input_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
    }

    #[test]
    fn member_prompt_run_less_member_fails_before_pane_io() {
        assert_run_less_input_failure("prompt");
    }

    #[test]
    fn member_exec_run_less_member_fails_before_pane_io() {
        assert_run_less_input_failure("exec");
    }

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

    pub(crate) fn external_input_member_child_path() -> &'static str {
        test_path!(external_input_member_child)
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
        let database = crate::db::database_path(config["database_url"].as_str().unwrap()).unwrap();
        let settings = Settings::for_database(std::path::Path::new(database), Some("tmux"));
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
        child: crate::runtime::system::test_support::TestChild,
        until: Instant,
    }
    impl Drop for ExternalMemberChild {
        fn drop(&mut self) {
            self.child.terminate_until(self.until);
        }
    }

    fn external_member_invoke(
        fixture: &crate::runtime::service::test_support::ServiceFixture,
        action: &str,
        literal: &str,
        json: bool,
        lose_stdout: bool,
        input_id: Option<i64>,
        outer: Instant,
    ) -> (std::process::ExitStatus, String, String) {
        use std::io::Read;
        use std::process::Command;
        use std::process::Stdio;
        use std::sync::atomic::AtomicU64;
        use std::sync::atomic::Ordering;
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
            child: crate::runtime::system::test_support::TestChild::new(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        external_input_member_child_path(),
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
    fn wrapped_uncertain_prompt_and_exec_errors_name_the_recorded_cause_and_method() {
        use crate::runtime::SubmissionResult;
        use crate::runtime::SubmissionStage;
        use crate::runtime::SubmissionStatus;
        let uncertain = |cause: Option<&str>, method| SubmissionResult {
            status: SubmissionStatus::Uncertain,
            input_id: Some(9),
            stage: SubmissionStage::Partial,
            output_seq: Some(4),
            reason: cause.map(str::to_owned),
            delivery: Some(DeliveryProjection {
                status: DeliveryStatus::Uncertain,
                method,
                reason: None,
            }),
        };
        for label in ["prompt", "native command"] {
            assert_eq!(
                wrapped_delivery_refusal(
                    label,
                    110,
                    &uncertain(Some("manual_input"), Some(DeliveryMethod::Profile))
                ),
                format!(
                    "{label} delivery to member 110 is uncertain: manual_input (method=profile)"
                )
            );
        }
        assert_eq!(
            wrapped_delivery_refusal(
                "native command",
                110,
                &uncertain(Some("write_failed"), Some(DeliveryMethod::Fallback))
            ),
            "native command delivery to member 110 is uncertain: write_failed (method=fallback)"
        );
        assert_eq!(
            wrapped_delivery_refusal("prompt", 110, &uncertain(None, None)),
            "prompt delivery to member 110 is uncertain",
            "a cause-less receipt without a recorded method adds neither"
        );
    }

    #[test]
    fn external_input_member_prompt_exec_route_raw_commands_and_print_actual_durable_ids() {
        use crate::broker::runtime as storage;
        crate::runtime::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
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
                        crate::runtime::service::test_support::input_service::input_service_composer(fixture, outer);
                        let before = external_member_input_ids(&fixture.fixture.conn, run.run_id);
                        let literal = format!("member {action} λ json={json}");
                        let (status, stdout, stderr) = external_member_invoke(
                            fixture, action, &literal, json, false, None, outer,
                        );
                        assert!(
                            status.success(),
                            "actual {action} failed: {status}; stdout={stdout:?}; stderr={stderr:?}"
                        );
                        assert!(stderr.is_empty(), "{stderr}");
                        let after = external_member_input_ids(&fixture.fixture.conn, run.run_id);
                        assert_eq!(after.len(), before.len() + 1);
                        assert_eq!(&after[..before.len()], before);
                        let row = storage::get_input(&fixture.fixture.conn, *after.last().unwrap())
                            .unwrap();
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
                        let digest =
                            crate::digest::sha256_hex(&serde_json::to_vec(&payload).unwrap());
                        assert_eq!(
                            row.payload_digest, digest,
                            "Exec binds the raw command before the native prefix"
                        );
                        if json {
                            let expected = if action == "prompt" {
                                format!(
                                    "{{\"member_id\":{},\"pane_id\":{},\"text\":{},\"input_id\":{},\"delivery\":{{\"status\":\"submitted\",\"method\":\"profile\",\"reason\":null}}}}",
                                    run.member_id,
                                    serde_json::to_string(pane).unwrap(),
                                    serde_json::to_string(&literal).unwrap(),
                                    row.input_id
                                )
                            } else {
                                format!(
                                    "{{\"member_id\":{},\"pane_id\":{},\"coding_agent\":{},\"command\":{},\"dispatched\":true,\"input_id\":{},\"delivery\":{{\"status\":\"submitted\",\"method\":\"profile\",\"reason\":null}}}}",
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
                                stdout.contains(&format!(
                                    "{original}\nDelivery: status=submitted method=profile reason=-\nInput: {}\n",
                                    row.input_id
                                )),
                                "{stdout:?}"
                            );
                        }
                        let line = if action == "exec" {
                            format!("! {literal}")
                        } else {
                            literal.clone()
                        };
                        crate::runtime::service::test_support::input_service::wait_input_service_record(fixture, &line, outer);
                        crate::runtime::service::test_support::input_service::input_service_barrier(
                            fixture, outer,
                        );
                        assert_eq!(
                        crate::runtime::service::test_support::input_service::input_service_records(fixture)
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
            },
        );
    }

    #[test]
    fn external_input_member_lost_stdout_keeps_discoverable_receipt_and_same_key_never_retypes() {
        use crate::broker::runtime as storage;
        use crate::runtime::SubmissionStatus;
        use crate::runtime::SubmitInput;
        use crate::runtime::submit_wrapped_input;
        crate::runtime::service::test_support::input_service::with_input_service(
            |fixture, outer| {
                let run =
                    storage::get_run(&fixture.fixture.conn, fixture.fixture.run.run_id).unwrap();
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
                let settings = Settings::for_database(
                    std::path::Path::new(fixture.fixture.conn.path().unwrap()),
                    Some("tmux"),
                );
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
                crate::runtime::service::test_support::input_service::wait_input_service_record(
                    fixture, literal, outer,
                );
                crate::runtime::service::test_support::input_service::input_service_barrier(
                    fixture, outer,
                );
                assert_eq!(
                    crate::runtime::service::test_support::input_service::input_service_records(
                        fixture
                    )
                    .iter()
                    .filter(|record| record["line"] == literal)
                    .count(),
                    1
                );
            },
        );
    }

    fn fixture() -> (tempfile::TempDir, Connection, Settings, i64, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = migrated_conn(&dir);
        let settings =
            Settings::for_database(std::path::Path::new(conn.path().unwrap()), Some("tmux"));
        let (fleet, director) = create_fleet_fixture(&mut conn, "deletion");
        let worker = register(&mut conn, fleet, "worker", None);
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
    }

    #[test]
    fn deletion_reloads_a_published_pending_pane_and_retains_ownership_through_completion() {
        use rusqlite::hooks::AuthAction;
        use rusqlite::hooks::AuthContext;
        use rusqlite::hooks::Authorization;
        use rusqlite::hooks::TransactionOperation;
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
                PaneLock::acquire_released(&settings.database_url, &placement)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn irreversible_close_preserves_the_registry_error_and_releases_ownership_on_return() {
        let (_dir, mut conn, settings, fleet, _, worker) = fixture();
        crate::broker::members::test_support::update_placement_pane_id(&mut conn, worker, "%9")
            .unwrap();
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

    mod launch_policy {
        use crate::cli::CliArgs;
        use crate::cli::Command;
        use crate::cli::member;
        use crate::cli::member::tests::*;
        use crate::coding_agent::CodingAgent;
        use crate::db::test_support::migrated_conn;
        use crate::db::test_support::table_snapshot;

        fn invalid_creation(backend: CodingAgent, extra: &[String]) -> CafleetError {
            let dir = crate::runtime::test_support::short_tempdir();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = crate::broker::fleets::test_support::create_fleet_fixture(
                &mut conn,
                "invalid-launch-fixture",
            );
            let mut arguments = create_arguments()
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>();
            arguments[4] = fleet.to_string();
            arguments.extend(["--coding-agent".into(), backend.name().into()]);
            arguments.extend_from_slice(extra);
            arguments.push("literal assignment".into());
            let Command::Member(command) = CliArgs::try_parse_from(arguments).unwrap().command
            else {
                panic!("member command expected")
            };
            let before = table_snapshot(&conn, |_| true);
            let identity = crate::broker::runtime::database_identity(&conn).unwrap();
            let paths = crate::runtime::RuntimePaths::for_run(&identity, 1).unwrap();
            let settings = Settings::for_database(
                std::path::Path::new(conn.path().unwrap()),
                Some("unavailable-test-multiplexer"),
            );
            let failure = member::run(&mut conn, &settings, command, crate::time::Deadline::new())
                .expect_err("invalid launch must fail");
            assert!(
                matches!(failure, CafleetError::Usage(_)),
                "{}",
                failure.message()
            );
            assert_eq!(table_snapshot(&conn, |_| true), before);
            assert!(!paths.run_dir.exists());
            failure
        }

        fn create_arguments() -> Vec<&'static str> {
            vec![
                "cafleet",
                "member",
                "create",
                "--fleet-id",
                "1",
                "--name",
                "worker",
                "--description",
                "policy fixture",
            ]
        }

        #[test]
        fn runtime_policy_parser_accepts_the_backend_qualified_launch_selection() {
            for (backend, mode) in [
                ("claude", "default"),
                ("claude", "manual"),
                ("codex", "on-request"),
                ("opencode", "cafleet"),
                ("opencode", "cafleet-ask"),
                ("hermes", "smart"),
                ("hermes", "manual"),
            ] {
                let mut arguments = create_arguments();
                arguments.extend([
                    "--coding-agent",
                    backend,
                    "--permission-mode",
                    mode,
                    "literal assignment",
                ]);
                assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
            }
        }

        #[test]
        fn runtime_policy_parser_accepts_explicit_metadata_evidence() {
            let mut arguments = create_arguments();
            arguments.extend(["--event-evidence", "metadata", "literal assignment"]);
            assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
        }

        #[test]
        fn runtime_policy_parser_accepts_redacted_evidence_with_its_absolute_file() {
            let mut arguments = create_arguments();
            arguments.extend([
                "--event-evidence",
                "redacted",
                "--event-redaction-file",
                "/tmp/synthetic-redaction.json",
                "literal assignment",
            ]);
            assert!(CliArgs::try_parse_from(&arguments).is_ok(), "{arguments:?}");
        }

        #[test]
        fn runtime_policy_parser_keeps_omitted_selection_and_model_effort_boundaries() {
            let mut arguments = create_arguments();
            arguments.extend([
                "--coding-agent",
                "codex",
                "--model",
                "fixture-model",
                "--effort",
                "high",
                "literal '$x' `text` assignment",
            ]);
            let Command::Member(member::MemberCommand::Create(member::CreateArgs {
                coding_agent,
                model,
                effort,
                ..
            })) = CliArgs::try_parse_from(arguments).unwrap().command
            else {
                panic!("member create expected")
            };
            assert_eq!(coding_agent, Some(crate::coding_agent::CodingAgent::Codex));
            assert_eq!(model.as_deref(), Some("fixture-model"));
            assert_eq!(effort.as_deref(), Some("high"));
        }

        #[test]
        fn runtime_policy_invalid_launch_selection_has_usage_diagnostics_before_creation_effects() {
            for &backend in CodingAgent::all() {
                let failure = invalid_creation(
                    backend,
                    &[
                        "--permission-mode".into(),
                        "unsupported-fixture-mode".into(),
                    ],
                );
                assert!(failure.message().contains(backend.name()));
                for choice in backend.policy_choices(None) {
                    assert!(failure.message().contains(&choice));
                }
            }
        }

        #[test]
        fn hermes_invalid_effort_and_policy_fail_with_usage_errors_before_creation_effects() {
            assert_eq!(
                invalid_creation(CodingAgent::Hermes, &["--effort".into(), "turbo".into()])
                    .message(),
                "--effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got 'turbo')."
            );
            assert_eq!(
                invalid_creation(
                    CodingAgent::Hermes,
                    &["--permission-mode".into(), "off".into()]
                )
                .message(),
                "--permission-mode for hermes must be one of smart, manual (got 'off')"
            );
        }

        #[test]
        fn runtime_policy_invalid_redaction_configuration_fails_before_creation_effects() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("redaction.json");
            for bytes in [
                b"not json".to_vec(),
                vec![0xff],
                br#"{"schema_version":1,"patterns":[],"sensitive_rules":[]}"#.to_vec(),
                br#"{"schema_version":1,"patterns":["["],"sensitive_rules":[]}"#.to_vec(),
                br#"{"schema_version":1,"patterns":["secret"],"sensitive_rules":["missing-rule"]}"#
                    .to_vec(),
            ] {
                std::fs::write(&path, bytes).unwrap();
                invalid_creation(
                    CodingAgent::Codex,
                    &[
                        "--event-evidence".into(),
                        "redacted".into(),
                        "--event-redaction-file".into(),
                        path.to_str().unwrap().into(),
                    ],
                );
            }
        }

        #[test]
        fn runtime_policy_redaction_options_form_a_validated_launch_only_group() {
            for extra in [
                vec!["--event-evidence".into(), "redacted".into()],
                vec![
                    "--event-evidence".into(),
                    "metadata".into(),
                    "--event-redaction-file".into(),
                    "/tmp/unused-redaction.json".into(),
                ],
                vec![
                    "--event-evidence".into(),
                    "redacted".into(),
                    "--event-redaction-file".into(),
                    "relative-redaction.json".into(),
                ],
            ] {
                invalid_creation(CodingAgent::Codex, &extra);
            }
        }
    }

    mod press {
        use crate::cli::CliArgs;
        use crate::cli::Command;
        use crate::cli::dispatch;
        use crate::cli::member;
        use crate::cli::member::tests::*;

        #[test]
        fn press_cli_preserves_keys_after_the_ordinary_option_separator() {
            assert!(CliArgs::try_parse_from(["cafleet", "member", "show", "42"]).is_ok());
            let result = CliArgs::try_parse_from([
                "cafleet",
                "member",
                "press",
                "--json",
                "42",
                "--",
                "-",
                "+",
                ";",
                "A",
                "Ctrl+A",
                "Shift+Tab",
            ]);
            match result
                .unwrap_or_else(|cause| panic!("press parse: {cause}"))
                .command
            {
                Command::Member(member::MemberCommand::Press {
                    member_id,
                    keys,
                    json,
                }) => {
                    assert_eq!(member_id, 42);
                    assert!(json);
                    assert_eq!(keys, ["-", "+", ";", "A", "Ctrl+A", "Shift+Tab"]);
                }
                _ => panic!("press has its dedicated command arm"),
            }
            for arguments in [
                vec!["cafleet", "member", "press", "0", "Enter"],
                vec!["cafleet", "member", "press", "42"],
            ] {
                match CliArgs::try_parse_from(arguments) {
                    Ok(_) => panic!("press requires a positive member and keys"),
                    Err(cause) => assert_eq!(cause.exit_code(), 2),
                }
            }
        }

        #[test]
        fn press_cli_invalid_tail_is_usage_before_missing_database_validation() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("absent.db");
            let settings = Settings::for_database(&path, Some("tmux"));
            let command = CliArgs::try_parse_from([
                "cafleet", "member", "press", "42", "Enter", "Ctrl+1", "--json",
            ])
            .unwrap_or_else(|cause| panic!("raw key arguments reach typed validation: {cause}"))
            .command;
            let failure = dispatch(&settings, command)
                .expect_err("the complete key vector is validated first");
            assert!(matches!(&failure, CafleetError::Usage(message)
            if message == "invalid press key at position 2: \"Ctrl+1\""));
            assert_eq!(failure.exit_code(), 2);
            assert!(!path.exists());
        }

        #[test]
        fn press_presentation_is_canonical_acknowledgment_without_effect_counts() {
            let keys =
                crate::multiplexer::PressKeys::parse(&["cTrL+A".into(), ";".into()]).unwrap();
            let value = crate::runtime::KeyAcknowledgment {
                member_id: 42,
                pane_id: "%5".into(),
                keys: keys.canonical(),
            };
            assert_eq!(
                serde_json::to_string(&crate::output::json::member_press(&value)).unwrap(),
                "{\"member_id\":42,\"pane_id\":\"%5\",\"keys\":[\"Ctrl+a\",\";\"],\"status\":\"acknowledged\"}",
            );
            assert_eq!(
                crate::output::format_member_press(&value),
                "Acknowledged keys [\"Ctrl+a\",\";\"] for member 42 (%5).",
            );
        }
    }
    mod recovery_entry {
        use clap::CommandFactory;
        use clap::Parser;
        use clap::error::ErrorKind;
        use rusqlite::Connection;
        use tempfile::TempDir;

        use crate::broker::fleets::test_support::create_fleet_fixture;
        use crate::cli::CliArgs;
        use crate::cli::dispatch;
        use crate::cli::test_support::usage_error;
        use crate::config::Settings;
        use crate::db::test_support::migrated_conn;

        fn recognized_usage_error(args: &[&str]) {
            let error = usage_error(args);
            assert_eq!(error.exit_code(), 2, "{args:?}: {error}");
            assert!(
                !matches!(
                    error.kind(),
                    ErrorKind::InvalidSubcommand | ErrorKind::UnknownArgument
                ),
                "the recovery parser must validate these arguments: {args:?}: {error}"
            );
        }

        fn exceptional_args() -> Vec<&'static str> {
            vec![
                "member",
                "recover",
                "7",
                "--owner-terminated",
                "--expected-run-id",
                "11",
                "--expected-owner-epoch",
                "3",
                "--evidence-ref",
                "audit:owner-termination",
            ]
        }

        fn argument_id(command: &clap::Command, long: &str) -> String {
            command
                .get_arguments()
                .find(|argument| argument.get_long() == Some(long))
                .unwrap_or_else(|| panic!("recovery defines --{long}"))
                .get_id()
                .to_string()
        }

        #[test]
        fn runner_recovery_entry_ordinary_parser_accepts_positive_member_ids_and_json_selection() {
            for member_id in ["1", "481", "9223372036854775807"] {
                for json in [false, true] {
                    let mut args = vec!["cafleet", "member", "recover", member_id];
                    if json {
                        args.push("--json");
                    }
                    assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                }
            }
        }

        #[test]
        fn runner_recovery_entry_member_ids_require_positive_assigned_integers() {
            recognized_usage_error(&["member", "recover", "--json"]);
            for member_id in ["", "0", "-1", "unknown", "1.0", "9223372036854775808"] {
                recognized_usage_error(&["member", "recover", "--json", "--", member_id]);
            }
        }

        #[test]
        fn runner_recovery_entry_full_attestation_preserves_snapshot_reference_and_json_values() {
            for (member_id, run_id, epoch) in [
                ("7", "11", "3"),
                (
                    "9223372036854775807",
                    "9223372036854775807",
                    "9223372036854775807",
                ),
            ] {
                for reference in [
                    "audit:owner-termination",
                    "  audit:owner 世界 '$x' `literal`  ",
                ] {
                    for json in [false, true] {
                        let mut args = vec![
                            "cafleet",
                            "member",
                            "recover",
                            member_id,
                            "--owner-terminated",
                            "--expected-run-id",
                            run_id,
                            "--expected-owner-epoch",
                            epoch,
                            "--evidence-ref",
                            reference,
                        ];
                        if json {
                            args.push("--json");
                        }
                        assert!(CliArgs::try_parse_from(&args).is_ok(), "{args:?}");
                        let command = CliArgs::command();
                        let recovery = command
                            .find_subcommand("member")
                            .unwrap()
                            .find_subcommand("recover")
                            .unwrap();
                        let member_key = recovery
                            .get_arguments()
                            .find(|argument| argument.get_index() == Some(1))
                            .expect("recover has its member positional argument")
                            .get_id()
                            .to_string();
                        let run_key = argument_id(recovery, "expected-run-id");
                        let epoch_key = argument_id(recovery, "expected-owner-epoch");
                        let reference_key = argument_id(recovery, "evidence-ref");
                        let terminated_key = argument_id(recovery, "owner-terminated");
                        let json_key = argument_id(recovery, "json");
                        let matches = command.try_get_matches_from(args).unwrap();
                        let recovery = matches
                            .subcommand_matches("member")
                            .unwrap()
                            .subcommand_matches("recover")
                            .unwrap();
                        assert_eq!(
                            recovery.get_one::<i64>(&member_key),
                            Some(&member_id.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<i64>(&run_key),
                            Some(&run_id.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<i64>(&epoch_key),
                            Some(&epoch.parse::<i64>().unwrap())
                        );
                        assert_eq!(
                            recovery.get_one::<String>(&reference_key).unwrap(),
                            reference
                        );
                        assert!(recovery.get_flag(&terminated_key));
                        assert_eq!(recovery.get_flag(&json_key), json);
                    }
                }
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_options_require_the_complete_group() {
            let options: [&[&str]; 4] = [
                &["--owner-terminated"],
                &["--expected-run-id", "11"],
                &["--expected-owner-epoch", "3"],
                &["--evidence-ref", "audit:owner-termination"],
            ];
            for selected in 1_u8..15 {
                let mut args = vec!["member", "recover", "7", "--json"];
                for (index, option) in options.iter().enumerate() {
                    if selected & (1 << index) != 0 {
                        args.extend_from_slice(option);
                    }
                }
                recognized_usage_error(&args);
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_snapshot_ids_require_positive_integers() {
            for option in ["--expected-run-id", "--expected-owner-epoch"] {
                for invalid in ["", "0", "-1", "unknown", "1.0", "9223372036854775808"] {
                    let mut args = exceptional_args();
                    let index = args.iter().position(|value| *value == option).unwrap();
                    args.drain(index..index + 2);
                    let literal = format!("{option}={invalid}");
                    args.push(&literal);
                    recognized_usage_error(&args);
                }
            }
        }

        #[test]
        fn runner_recovery_entry_attestation_rejects_blank_references_and_missing_option_values() {
            for reference in ["", " ", "\t\n"] {
                let mut args = exceptional_args();
                *args.last_mut().unwrap() = reference;
                recognized_usage_error(&args);
            }
            for option in [
                "--expected-run-id",
                "--expected-owner-epoch",
                "--evidence-ref",
            ] {
                let mut args = exceptional_args();
                let index = args.iter().position(|value| *value == option).unwrap();
                args.drain(index..index + 2);
                args.push(option);
                recognized_usage_error(&args);
            }
        }

        pub(super) fn run_less_registry() -> (TempDir, Settings, Connection, i64) {
            let dir = crate::runtime::test_support::short_tempdir();
            let mut conn = migrated_conn(&dir);
            let (fleet, _) = create_fleet_fixture(&mut conn, "recovery-entry");
            let member_id = crate::broker::members::test_support::register(
                &mut conn,
                fleet,
                "run-less",
                Some("%2"),
            );
            let settings = Settings::for_database(std::path::Path::new(conn.path().unwrap()), None);
            (dir, settings, conn, member_id)
        }

        #[test]
        fn member_recover_run_less_member_fails_explicitly_before_registry_or_artifact_mutation() {
            let (dir, settings, conn, member_id) = run_less_registry();
            let path = std::path::PathBuf::from(conn.path().unwrap());
            drop(conn);
            let before = std::fs::read(&path).unwrap();
            let mut names = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            names.sort();
            let id = member_id.to_string();
            let args =
                CliArgs::try_parse_from(["cafleet", "member", "recover", &id, "--json"]).unwrap();
            let result = dispatch(&settings, args.command);
            let error = match result {
                Ok(()) => panic!("a run-less member has no recorded run to recover"),
                Err(error) => error,
            };
            let message = error.message();
            assert_eq!(message, format!("member {member_id} has no recorded run"));
            assert_eq!(error.exit_code(), 1);
            assert_eq!(std::fs::read(&path).unwrap(), before);
            let mut after_names = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            after_names.sort();
            assert_eq!(after_names, names);
        }
    }

    fn assert_malformed_usage_error(result: Result<String, CafleetError>) {
        let err = result.expect_err("a malformed brace expression must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.exit_code(), 2);
        let msg = err.message().to_string();
        assert!(msg.starts_with("Malformed custom prompt: "), "got: {msg}");
        assert!(
            msg.ends_with("Double literal braces ({{, }}) to keep them as text."),
            "got: {msg}"
        );
    }

    #[test]
    fn substitutes_all_four_placeholders() {
        let body = "FLEET ID: {fleet_id}\nYOUR MEMBER ID: {member_id}\n\
                    DIRECTOR MEMBER ID: {director_member_id}\nCODING AGENT: {coding_agent}";
        let out = substitute_spawn_placeholders(body, 3, 14, 11, "claude").unwrap();
        assert_eq!(
            out,
            "FLEET ID: 3\nYOUR MEMBER ID: 14\nDIRECTOR MEMBER ID: 11\nCODING AGENT: claude"
        );
    }

    #[test]
    fn substitutes_repeated_placeholders() {
        let out = substitute_spawn_placeholders("{fleet_id}-{fleet_id}", 7, 1, 2, "codex").unwrap();
        assert_eq!(out, "7-7");
    }

    #[test]
    fn body_without_placeholders_is_returned_verbatim() {
        let out =
            substitute_spawn_placeholders("plain prompt, no braces", 1, 2, 3, "claude").unwrap();
        assert_eq!(out, "plain prompt, no braces");
    }

    #[test]
    fn doubled_braces_render_literal_braces() {
        let out =
            substitute_spawn_placeholders("keep {{fleet_id}} literal", 1, 2, 3, "claude").unwrap();
        assert_eq!(out, "keep {fleet_id} literal");
        let out = substitute_spawn_placeholders("a{{b}}c", 1, 2, 3, "claude").unwrap();
        assert_eq!(out, "a{b}c");
    }

    #[test]
    fn doubled_braces_compose_with_substitution() {
        let out =
            substitute_spawn_placeholders("{{x}} and {fleet_id}", 42, 2, 3, "opencode").unwrap();
        assert_eq!(out, "{x} and 42");
    }

    #[test]
    fn unknown_placeholder_is_a_usage_error_with_the_pinned_message() {
        let err = substitute_spawn_placeholders("hello {foo}", 1, 2, 3, "claude")
            .expect_err("an unknown placeholder must error");
        assert!(matches!(err, CafleetError::Usage(_)));
        assert_eq!(err.exit_code(), 2);
        assert_eq!(
            err.message(),
            "Unknown placeholder 'foo' in custom prompt. \
             Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, \
             {coding_agent}. Double literal braces ({{, }}) to keep them as text."
        );
    }

    #[test]
    fn stray_open_brace_is_a_malformed_usage_error() {
        assert_malformed_usage_error(substitute_spawn_placeholders("oops {", 1, 2, 3, "claude"));
    }

    #[test]
    fn stray_close_brace_is_a_malformed_usage_error() {
        assert_malformed_usage_error(substitute_spawn_placeholders("oops }", 1, 2, 3, "claude"));
    }

    #[test]
    fn unclosed_placeholder_is_a_malformed_usage_error() {
        assert_malformed_usage_error(substitute_spawn_placeholders(
            "start {fleet_id and never close",
            1,
            2,
            3,
            "claude",
        ));
    }
    use std::io;
    const WHEN: &str = "2026-09-06T01:02:03.000007+00:00";
    const PLAIN_HASH: &str = "526a35d455e79594452b17d2378d09d811ce0fb0ece884ac1198c3fc619c10f4";
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(WHEN)
            .unwrap()
            .with_timezone(&Utc)
    }
    fn capture() -> MemberCapture {
        MemberCapture {
            member_id: 7,
            pane_id: "%9".into(),
            lines: 12,
            snapshot: CaptureSnapshot::from_raw("雪\n緑", false, now()),
        }
    }
    #[test]
    fn snapshot_strips_csi_and_cr_rewrites_before_hashing_utf8() {
        let snapshot = CaptureSnapshot::from_raw("old\r\x1b[31m雪\x1b[0m\nold\r緑", false, now());
        assert_eq!(
            snapshot,
            CaptureSnapshot {
                content: "雪\n緑".into(),
                captured_at: WHEN.into(),
                content_sha256: PLAIN_HASH.into()
            }
        );
    }
    #[test]
    fn snapshot_treats_crlf_row_terminators_as_line_endings() {
        let snapshot = CaptureSnapshot::from_raw("old\r\x1b[31m雪\x1b[0m\r\n緑\r", false, now());
        assert_eq!(
            snapshot,
            CaptureSnapshot {
                content: "雪\n緑".into(),
                captured_at: WHEN.into(),
                content_sha256: PLAIN_HASH.into()
            }
        );
    }
    #[test]
    fn snapshot_ansi_preserves_every_raw_byte_and_hashes_that_content() {
        let raw = "old\r\x1b[31m雪\x1b[0m\nold\r緑";
        let snapshot = CaptureSnapshot::from_raw(raw, true, now());
        assert_eq!(snapshot.content, raw);
        assert_eq!(snapshot.captured_at, WHEN);
        assert_eq!(
            snapshot.content_sha256,
            "b9bd2b04f3d96774f4916e31055b70707a34e21a07fb96125c9214df2b1ad191"
        );
    }
    #[test]
    fn output_failures_propagate_from_member_capture() {
        struct Fail;
        impl io::Write for Fail {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("output unavailable"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for json in [false, true] {
            assert!(
                write_member_capture(&mut Fail, &capture(), json)
                    .unwrap_err()
                    .to_string()
                    .contains("output unavailable")
            );
        }
    }
}
