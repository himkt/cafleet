use super::*;
use crate::broker::runtime as storage;
use crate::coding_agent::CodingAgent;
use crate::config::Settings;
use crate::multiplexer::{CommandRunner, MultiplexerContext, resolve_multiplexer};
use crate::runtime::system::SystemRunner;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::rc::Rc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePolicyFile {
    schema_version: u32,
    masks: Vec<CaptureMask>,
}
pub(crate) fn run_native_capture_verification(config: &Path) -> Result<i32> {
    run_native_capture_verification_with_export_hooks(config, Arc::new(InertCaptureExportHooks))
}
pub(crate) fn run_native_capture_verification_with_export_hooks(
    config: &Path,
    hooks: Arc<dyn NativeCaptureExportHooks>,
) -> Result<i32> {
    let limits = NativeCaptureLimits::default();
    let config = read_native_capture_config(config, &limits)?;
    let settings = Settings {
        database_url: config.database_url.clone(),
        max_text_len: 200,
        multiplexer: None,
    };
    let conn = crate::db::connect(&settings.database_url)?;
    let descriptor = load_descriptor(&conn, &config.descriptor_path)?;
    let run = storage::get_run(&conn, descriptor.run_id)?;
    ensure(
        run.run_id == config.expected_run_id && run.launch_nonce == config.expected_launch_nonce,
        "verification configuration run/nonce mismatch",
    )?;
    let catalog = ProfileCatalog::bundled()?;
    let policy: CapturePolicyFile = serde_json::from_slice(&super::files::read_private(
        &config.capture_policy_path,
        limits.max_config_bytes,
    )?)
    .map_err(io_error)?;
    ensure(policy.schema_version == 1, "capture policy schema mismatch")?;
    let capture_policy =
        NativeCapturePolicy::from_launch(&descriptor.spawn, &policy.masks, &catalog)?;
    let (mut port, receiver) = native_capture_channel(&run, capture_policy, limits.clone())?;
    port.service_producer = Some(read_native_capture_producer(
        &config.producer_manifest_path,
        &limits,
    )?);
    let task = CaptureExporterTask::install(&mut port, receiver, config.clone(), limits, hooks);
    let result = run_with_test_services(
        &settings,
        &config.descriptor_path,
        RunnerTestServices {
            catalog,
            clock: Arc::new(RealObservationClock),
            writer_hooks: Arc::new(InertWriterHooks),
            capture: Some(port),
            observation_hooks: None,
        },
    );
    let exported = task.finish(&result);
    match (result, exported) {
        (Ok(status), Ok(())) => Ok(status),
        (Ok(status), Err(cause)) if status != 0 => Err(error(format!(
            "capture service exited with status {status}"
        ))
        .with_cleanup(cause.message())),
        (Ok(_), Err(cause)) => Err(cause),
        (Err(primary), Err(secondary)) => Err(primary.with_cleanup(secondary.message())),
        (Err(cause), Ok(())) => Err(cause),
    }
}

// Committed native fixtures derive from these captures, so their visible paths must not name a home directory.
const DRIVER_NEUTRAL_ROOT: &str = "/tmp/cafleet-194";
// The repository mount reports root-created files as owned by another user, which the private
// ownership checks refuse, so case databases and sockets live on the local filesystem.
const DRIVER_PRIVATE_ROOT: &str = "/tmp/cafleet-194-p";
const DRIVER_ANSWER_WINDOW_MS: u64 = 60_000;
const DRIVER_TOOL_FREE_PROMPT: &str = "Reply with the single word ready and then wait.";

#[derive(PartialEq, Eq)]
enum DriverMode {
    NoAnswer,
    AnswerStage,
}
struct ApprovedCase {
    id: &'static str,
    backend: CodingAgent,
    permission_mode: &'static str,
    mode: DriverMode,
    decision: Option<&'static str>,
    prefix_hex: &'static [&'static str],
    stage_hex: &'static [&'static str],
    probe_argument: Option<&'static str>,
    database: &'static str,
    socket: &'static str,
    size: TerminalSize,
}
const DRIVER_FULL_SIZE: TerminalSize = TerminalSize {
    rows: 36,
    cols: 120,
};
const DRIVER_WRAPPED_SIZE: TerminalSize = TerminalSize { rows: 36, cols: 52 };
const DRIVER_BOXED_WRAP_SIZE: TerminalSize = TerminalSize { rows: 36, cols: 89 };
const APPROVED_CASES: [ApprovedCase; 21] = [
    ApprovedCase {
        id: "cells-002",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "nc02.db",
        socket: "tc02",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "ans-allow-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::AnswerStage,
        decision: Some("allow"),
        prefix_hex: &[],
        stage_hex: &["31"],
        probe_argument: Some("allow"),
        database: "na01.db",
        socket: "ta01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "ans-deny-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &[],
        stage_hex: &["34"],
        probe_argument: Some("deny"),
        database: "nd01.db",
        socket: "td01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "idle-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: None,
        database: "ni01.db",
        socket: "ti01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "codex-trust-001",
        backend: CodingAgent::Codex,
        permission_mode: "on-request",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: None,
        database: "xt01.db",
        socket: "xt01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "codex-cells-001",
        backend: CodingAgent::Codex,
        permission_mode: "on-request",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "xc01.db",
        socket: "xc01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "codex-ans-allow-001",
        backend: CodingAgent::Codex,
        permission_mode: "on-request",
        mode: DriverMode::AnswerStage,
        decision: Some("allow"),
        prefix_hex: &[],
        stage_hex: &["79"],
        probe_argument: Some("allow"),
        database: "xa01.db",
        socket: "xa01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "codex-ans-deny-001",
        backend: CodingAgent::Codex,
        permission_mode: "on-request",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &[],
        stage_hex: &["33"],
        probe_argument: Some("deny"),
        database: "xd01.db",
        socket: "xd01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "codex-idle-001",
        backend: CodingAgent::Codex,
        permission_mode: "on-request",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: None,
        database: "xi01.db",
        socket: "xi01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "claude-wrap-cells-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "nwc1.db",
        socket: "twc1",
        size: DRIVER_WRAPPED_SIZE,
    },
    ApprovedCase {
        id: "claude-wrap-ans-allow-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::AnswerStage,
        decision: Some("allow"),
        prefix_hex: &[],
        stage_hex: &["31"],
        probe_argument: Some("allow"),
        database: "nwa1.db",
        socket: "twa1",
        size: DRIVER_WRAPPED_SIZE,
    },
    ApprovedCase {
        id: "claude-wrap-ans-deny-001",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &[],
        stage_hex: &["34"],
        probe_argument: Some("deny"),
        database: "nwd1.db",
        socket: "twd1",
        size: DRIVER_WRAPPED_SIZE,
    },
    ApprovedCase {
        id: "claude-boxed-cells-089-wide-operation-probe-01",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "nb89.db",
        socket: "tb89",
        size: DRIVER_BOXED_WRAP_SIZE,
    },
    ApprovedCase {
        id: "claude-boxed-cells-120-wide-operation-probe-01",
        backend: CodingAgent::Claude,
        permission_mode: "default",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "nb12.db",
        socket: "tb12",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-cells-001",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: Some("allow"),
        database: "oc01.db",
        socket: "oc01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-ans-allow-001",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::AnswerStage,
        decision: Some("allow"),
        prefix_hex: &[],
        stage_hex: &["0d"],
        probe_argument: Some("allow"),
        database: "oa01.db",
        socket: "oa01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-ans-deny-001",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &[],
        stage_hex: &["68", "0d"],
        probe_argument: Some("deny"),
        database: "od01.db",
        socket: "od01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-ans-deny-002",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &["68"],
        stage_hex: &["0d", "0d"],
        probe_argument: Some("deny"),
        database: "od02.db",
        socket: "od02",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-ans-deny-003",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &["68", "0d"],
        stage_hex: &["0d"],
        probe_argument: Some("deny"),
        database: "od03.db",
        socket: "od03",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-focus-always-001",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::AnswerStage,
        decision: Some("deny"),
        prefix_hex: &["6c"],
        stage_hex: &[],
        probe_argument: Some("deny"),
        database: "of01.db",
        socket: "of01",
        size: DRIVER_FULL_SIZE,
    },
    ApprovedCase {
        id: "opencode-idle-001",
        backend: CodingAgent::Opencode,
        permission_mode: "cafleet-ask",
        mode: DriverMode::NoAnswer,
        decision: None,
        prefix_hex: &[],
        stage_hex: &[],
        probe_argument: None,
        database: "oi01.db",
        socket: "oi01",
        size: DRIVER_FULL_SIZE,
    },
];
impl ApprovedCase {
    fn select(case: &DriverCase) -> Result<&'static Self> {
        let approved = APPROVED_CASES
            .iter()
            .find(|approved| approved.id == case.case_id)
            .ok_or_else(|| error("unreviewed native case"))?;
        ensure(
            (approved.mode == DriverMode::NoAnswer)
                == (approved.prefix_hex.is_empty() && approved.stage_hex.is_empty())
                && approved
                    .prefix_hex
                    .iter()
                    .chain(approved.stage_hex)
                    .all(|stage| stage.as_bytes().chunks(2).all(|byte| byte != b"1b")),
            "reviewed case stages differ from its driver mode or contain Escape",
        )?;
        let (mode, window) = match approved.mode {
            DriverMode::NoAnswer => ("no_answer", 0),
            DriverMode::AnswerStage => ("answer_stage", DRIVER_ANSWER_WINDOW_MS),
        };
        ensure(
            case.mode == mode
                && case.answer_window_ms == window
                && case.decision.as_deref() == approved.decision
                && case.prefix_hex == approved.prefix_hex
                && case.stage_hex == approved.stage_hex
                && case.probe_argument.as_deref() == approved.probe_argument,
            "case decision, stage or probe binding differs from its reviewed case",
        )?;
        Ok(approved)
    }
    fn member_name(&self) -> String {
        format!("cafleet-194-{}", self.id)
    }
    fn tmux_geometry(&self) -> String {
        format!("{}|{}", self.size.cols, self.size.rows)
    }
    fn requested_marker(&self) -> Option<String> {
        self.probe_argument
            .map(|argument| format!("{argument}.executed"))
    }
    fn prompt(&self, helper: &Path) -> String {
        match (self.probe_argument, self.backend) {
            (Some(argument), CodingAgent::Codex) => format!(
                "Use the shell tool to run exactly {} {argument} as one command with sandbox_permissions set to require_escalated and the justification \"CAFleet permission probe\". Preserve any native approval request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                helper.display()
            ),
            (Some(argument), CodingAgent::Opencode) => format!(
                "Use the shell tool to run exactly {} {argument} as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                helper.display()
            ),
            (Some(argument), CodingAgent::Claude) => format!(
                "Use Bash to attempt exactly {} {argument} as one command. Preserve any native permission request for the operator. After the native decision, report the actual result and stop. Keep work inside this disposable operation.",
                helper.display()
            ),
            (None, _) => DRIVER_TOOL_FREE_PROMPT.into(),
        }
    }
    fn forbidden_markers(&self) -> Vec<&'static str> {
        let allowed = (self.decision == Some("allow")).then_some("allow.executed");
        ["allow.executed", "deny.executed"]
            .into_iter()
            .filter(|marker| Some(*marker) != allowed)
            .collect()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverReference {
    path: PathBuf,
    sha256: String,
}
impl DriverReference {
    fn verify(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.path).map_err(io_error)?;
        ensure(
            self.path.is_absolute() && metadata.is_file() && metadata.len() <= 512 * 1024 * 1024,
            "bounded absolute evidence file required",
        )?;
        ensure(
            super::files::file_digest(&self.path)? == self.sha256,
            "driver evidence digest changed",
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverTmux {
    socket_path: PathBuf,
    socket_device: u64,
    socket_inode: u64,
    server_pid: i64,
    session: String,
    window_id: String,
    pane_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverCase {
    schema_version: u32,
    case_id: String,
    mode: String,
    answer_window_ms: u64,
    decision: Option<String>,
    prefix_hex: Vec<String>,
    stage_hex: Vec<String>,
    probe_argument: Option<String>,
    case_nonce: String,
    expires_at_mono_ns: u64,
    cleanup_reserve_ms: u64,
    database_url: String,
    config_path: PathBuf,
    cwd: PathBuf,
    output_directory: PathBuf,
    arm_path: PathBuf,
    producer_manifest_path: PathBuf,
    launch_evidence_path: PathBuf,
    driver_ownership_path: PathBuf,
    started_record_path: PathBuf,
    stop_path: PathBuf,
    privacy_confirmation_path: PathBuf,
    result_path: PathBuf,
    descriptor_copy_path: PathBuf,
    native_version: String,
    director_backend: String,
    native_executable: DriverReference,
    helper: DriverReference,
    libtest: DriverReference,
    source_manifest: DriverReference,
    version_evidence: DriverReference,
    environment_evidence: DriverReference,
    event_redaction: DriverReference,
    capture_policy: DriverReference,
    tmux: DriverTmux,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverOwnership {
    schema_version: u32,
    case_manifest_sha256: String,
    case_nonce: String,
    driver: ProcessIdentity,
    server: ProcessIdentity,
    tmux: DriverTmux,
    expires_at_mono_ns: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverStarted {
    schema_version: u32,
    case_manifest_sha256: String,
    driver_ownership_sha256: String,
    identity: ControlIdentity,
    runner: ProcessIdentity,
    child: ProcessIdentity,
    pane_id: String,
    rows: u16,
    cols: u16,
    capture_policy_sha256: String,
    event_redaction_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverStop {
    schema_version: u32,
    case_manifest_sha256: String,
    driver_ownership_sha256: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverPrivacy {
    schema_version: u32,
    case_manifest_sha256: String,
    started_record_sha256: String,
    rows: u16,
    cols: u16,
    capture_policy_sha256: String,
    event_redaction_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriverVersion {
    schema_version: u32,
    executable: PathBuf,
    sha256: String,
    argv: Vec<String>,
    stdout: String,
    exit_code: i32,
}

fn driver_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn driver_read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = super::files::read_private(path, 65536)
        .map_err(|cause| error(format!("{}: {}", path.display(), cause.message())))?;
    serde_json::from_slice(&bytes).map_err(|cause| error(format!("{}: {cause}", path.display())))
}
fn driver_publish(path: &Path, value: &impl Serialize, until: Instant) -> Result<()> {
    check_deadline(Instant::now(), until)?;
    let bytes = serde_json::to_vec(value).map_err(io_error)?;
    ensure(bytes.len() <= 65536, "private driver record exceeds bound")?;
    let temporary = path.with_extension(format!("pending-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&temporary)
        .map_err(io_error)?;
    let result = (|| {
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        check_deadline(Instant::now(), until)?;
        std::fs::hard_link(&temporary, path).map_err(io_error)?;
        File::open(
            path.parent()
                .ok_or_else(|| error("record parent required"))?,
        )
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
    })();
    let cleanup = std::fs::remove_file(&temporary).map_err(io_error);
    match (result, cleanup) {
        (Err(primary), Err(cause)) => Err(primary.with_cleanup(cause.message())),
        (Err(cause), _) | (_, Err(cause)) => Err(cause),
        _ => Ok(()),
    }
}
fn driver_tmux(case: &DriverCase, parts: &[&str], until: Instant) -> Result<String> {
    let mut argv = vec![
        "tmux".into(),
        "-S".into(),
        case.tmux.socket_path.to_string_lossy().into_owned(),
    ];
    argv.extend(parts.iter().map(|s| (*s).to_owned()));
    SystemRunner
        .run_until(&argv, until)
        .map_err(|cause| error(format!("owned tmux command failed: {cause:?}")))
}
fn driver_validate_tmux(
    case: &DriverCase,
    ownership: &DriverOwnership,
    until: Instant,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(&case.tmux.socket_path).map_err(io_error)?;
    ensure(
        metadata.dev() == case.tmux.socket_device && metadata.ino() == case.tmux.socket_inode,
        "dedicated tmux socket identity changed",
    )?;
    ensure(
        ownership.server.matches()?,
        "dedicated tmux server generation changed",
    )?;
    let actual = driver_tmux(
        case,
        &[
            "display-message",
            "-p",
            "-t",
            &case.tmux.pane_id,
            "#{pid}|#{session_name}|#{window_id}|#{pane_id}",
        ],
        until,
    )?;
    ensure(
        actual.trim()
            == format!(
                "{}|{}|{}|{}",
                case.tmux.server_pid, case.tmux.session, case.tmux.window_id, case.tmux.pane_id
            ),
        "dedicated controller identity changed",
    )
}
fn driver_validate_run(conn: &Connection, launched: &LaunchOutcome) -> Result<storage::RunRecord> {
    let current = storage::get_run(conn, launched.run.run_id)?;
    launched.started.identity.validate(&current)?;
    ensure(
        current.owner() == launched.run.owner()
            && current.child_pid == Some(launched.started.child.pid)
            && current.child_start_identity.as_deref()
                == Some(launched.started.child.start_identity.as_str()),
        "owned native generation changed",
    )?;
    let placement = crate::broker::get_member(conn, current.member_id, current.fleet_id)?
        .and_then(|member| member.placement)
        .ok_or_else(|| error("owned placement unavailable"))?;
    ensure(placement == launched.placement, "owned placement changed")?;
    Ok(current)
}
fn driver_validate_pane(
    case: &DriverCase,
    ownership: &DriverOwnership,
    launched: &LaunchOutcome,
    until: Instant,
) -> Result<()> {
    driver_validate_tmux(case, ownership, until)?;
    let pane = launched
        .placement
        .mux_pane_id
        .as_deref()
        .expect("launched pane identity");
    let actual = driver_tmux(
        case,
        &[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_pid}|#{session_name}|#{window_id}|#{pane_id}|#{pane_dead}",
        ],
        until,
    )?;
    let live = format!(
        "{}|{}|{}|{}|0",
        launched.started.runner.pid, case.tmux.session, case.tmux.window_id, pane
    );
    let dead = format!(
        "{}|{}|{}|{}|1",
        launched.started.runner.pid, case.tmux.session, case.tmux.window_id, pane
    );
    ensure(
        actual.trim() == dead || (actual.trim() == live && launched.started.runner.matches()?),
        "runner pane generation changed",
    )
}

fn driver_armed(case: &DriverCase) -> Result<bool> {
    Ok(case.arm_path.try_exists().map_err(io_error)?
        || case
            .arm_path
            .with_extension("claimed")
            .try_exists()
            .map_err(io_error)?)
}
fn driver_validate_privacy(
    case: &DriverCase,
    approved: &ApprovedCase,
    ownership: &DriverOwnership,
) -> Result<()> {
    case.capture_policy.verify()?;
    case.event_redaction.verify()?;
    let privacy: DriverPrivacy = driver_read(&case.privacy_confirmation_path)?;
    ensure(
        privacy.schema_version == 1
            && privacy.case_manifest_sha256 == ownership.case_manifest_sha256
            && privacy.started_record_sha256
                == super::files::file_digest(&case.started_record_path)?
            && privacy.rows == approved.size.rows
            && privacy.cols == approved.size.cols
            && privacy.capture_policy_sha256 == case.capture_policy.sha256
            && privacy.event_redaction_sha256 == case.event_redaction.sha256,
        "privacy confirmation binding differs",
    )
}

fn driver_capture_runner_pane(case: &DriverCase, pane: &str, until: Instant) -> Result<PathBuf> {
    const RETAINED_BYTES: usize = 48 * 1024;
    let mut text = driver_tmux(
        case,
        &["capture-pane", "-p", "-J", "-S", "-", "-t", pane],
        until,
    )?;
    let redaction: Value = driver_read(&case.event_redaction.path)?;
    for pattern in redaction["patterns"]
        .as_array()
        .ok_or_else(|| error("case redaction patterns required"))?
    {
        let pattern = regex::Regex::new(
            pattern
                .as_str()
                .ok_or_else(|| error("case redaction pattern string required"))?,
        )
        .map_err(io_error)?;
        text = pattern.replace_all(&text, "[REDACTED]").into_owned();
    }
    let start = (text.len().saturating_sub(RETAINED_BYTES)..=text.len())
        .find(|index| text.is_char_boundary(*index))
        .expect("the string end is a character boundary");
    let path = case.cwd.join("runner-pane-capture.json");
    driver_publish(
        &path,
        &json!({"schema_version":1,"pane_id":pane,"redaction_sha256":case.event_redaction.sha256,
            "omitted_leading_bytes":start,"text":&text[start..]}),
        until,
    )?;
    Ok(path)
}

fn driver_complete_capture(
    case: &DriverCase,
    approved: &ApprovedCase,
    launched: &LaunchOutcome,
    service: &Value,
) -> Result<Value> {
    ensure(
        service["schema_version"] == 1
            && service["result"] == "ok"
            && service["status"]
                .as_i64()
                .is_some_and(|s| (0..=255).contains(&s))
            && service["producer_sha256"]
                == super::files::file_digest(&case.producer_manifest_path)?
            && service["process"]
                == serde_json::to_value(&launched.started.runner).map_err(io_error)?,
        "capture service/export did not return a bound actual status",
    )?;
    let artifact_name = format!("{}.complete.json", case.case_id);
    let manifest_path = case
        .output_directory
        .join(format!("{}.manifest.json", case.case_id));
    let manifest: Value = driver_read(&manifest_path)?;
    ensure(
        manifest["capture_id"] == case.case_id.as_str()
            && manifest["publication_status"] == "complete"
            && manifest["collection_status"] == "complete"
            && manifest["artifact_ref"] == artifact_name.as_str()
            && manifest["producer_ref"] == case.producer_manifest_path.to_string_lossy().as_ref(),
        "complete capture manifest required",
    )?;
    let artifact = case.output_directory.join(&artifact_name);
    let bytes = super::files::read_private(
        &artifact,
        NativeCaptureLimits::default().max_encoded_result_bytes,
    )?;
    ensure(
        manifest["artifact_sha256"] == driver_digest(&bytes)
            && manifest["artifact_bytes"] == bytes.len(),
        "capture artifact/manifest digest mismatch",
    )?;
    let value: Value = serde_json::from_slice(&bytes).map_err(io_error)?;
    ensure(
        value["status"] == "complete"
            && value["capture_id"] == case.case_id.as_str()
            && value["source"]["run_id"] == launched.run.run_id
            && value["source"]["owner_epoch"] == launched.run.owner_epoch
            && value["source"]["launch_nonce"] == launched.run.launch_nonce
            && value["source"]["runner"]
                == serde_json::to_value(&launched.started.runner).map_err(io_error)?
            && value["source"]["child"]
                == serde_json::to_value(&launched.started.child).map_err(io_error)?
            && value["samples"].as_array().is_some_and(|s| {
                s.len() == 2
                    && s.iter().all(|sample| {
                        sample["rows"] == approved.size.rows && sample["cols"] == approved.size.cols
                    })
            }),
        "actual complete parser pair required",
    )?;
    Ok(
        json!({"manifest_path":manifest_path,"manifest_sha256":super::files::file_digest(&manifest_path)?,
        "artifact_path":artifact,"artifact_sha256":driver_digest(&bytes),"service_status":service["status"]}),
    )
}

struct CaptureVerificationLaunchHost<'a> {
    inner: NativeLaunchHost<'a>,
    case: &'a DriverCase,
    case_until: Instant,
}
impl LaunchHost for CaptureVerificationLaunchHost<'_> {
    fn now(&self) -> Instant {
        self.inner.now()
    }
    fn current_executable(&self) -> Result<PathBuf> {
        self.inner.current_executable()
    }
    fn split_runner(
        &mut self,
        argv: &[String],
        environment: &[(String, String)],
        until: Instant,
    ) -> Result<String> {
        let config =
            read_native_capture_config(&self.case.config_path, &NativeCaptureLimits::default())?;
        ensure(
            argv.len() == 4
                && argv[0] == self.case.libtest.path.to_string_lossy()
                && argv[1] == "__member-run"
                && argv[2] == "--descriptor"
                && Path::new(&argv[3]) == config.descriptor_path
                && environment
                    == [(
                        "CAFLEET_DATABASE_URL".into(),
                        self.case.database_url.clone(),
                    )],
            "generated launch tuple changed",
        )?;
        let descriptor: LaunchDescriptor = driver_read(&config.descriptor_path)?;
        ensure(
            descriptor.run_id == config.expected_run_id
                && descriptor.launch_nonce == config.expected_launch_nonce
                && descriptor.spawn.argv[0] == self.case.native_executable.path.to_string_lossy(),
            "generated descriptor binding changed",
        )?;
        driver_publish(
            &self.case.descriptor_copy_path,
            &descriptor,
            self.case_until,
        )?;
        let child = vec![
            argv[0].clone(),
            "--exact".into(),
            "runtime::runner_tests::native_capture::native_capture_service_child".into(),
            "--ignored".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ];
        let mut environment = environment.to_vec();
        environment.push((
            "CAFLEET_NATIVE_CAPTURE_CONFIG".into(),
            self.case.config_path.to_string_lossy().into_owned(),
        ));
        self.inner
            .split_runner(&child, &environment, until.min(self.case_until))
    }
    fn wait_prepared(&mut self, descriptor: &LaunchDescriptor, until: Instant) -> Result<Prepared> {
        self.inner
            .wait_prepared(descriptor, until.min(self.case_until))
    }
    fn start(
        &mut self,
        prepared: &Prepared,
        placement: &Placement,
        until: Instant,
    ) -> Result<Started> {
        self.inner
            .start(prepared, placement, until.min(self.case_until))
    }
    fn shutdown_owned(
        &mut self,
        run: &storage::RunRecord,
        clock: ShutdownClock,
    ) -> Result<ShutdownOutcome> {
        self.inner
            .shutdown_owned(run, clock.clipped(self.case_until))
    }
    fn close_pane(&mut self, pane: &str, until: Instant) -> Result<()> {
        self.inner.close_pane(pane, until.min(self.case_until))
    }
}

fn driver_prepare(
    conn: &mut Connection,
    pending: &PendingRegistration,
    case: &DriverCase,
    until: Instant,
) -> Result<(RunArtifacts, SpawnSpec)> {
    let approved = ApprovedCase::select(case)?;
    let artifacts = RunArtifacts::create_with_deadline(conn, pending.run.run_id, until)?;
    let policy = approved
        .backend
        .resolve_policy(Some(&case.native_version), Some(approved.permission_mode))?;
    let prompt = approved.prompt(&case.helper.path);
    let mut argv = approved.backend.build_spawn_argv_with_policy(
        &prompt,
        &approved.member_name(),
        None,
        None,
        &policy,
    )?;
    argv[0] = case.native_executable.path.to_string_lossy().into_owned();
    let config: Value = driver_read(&case.event_redaction.path)?;
    let catalog = ProfileCatalog::bundled()?;
    let rules = catalog.rule_ids();
    crate::broker::runtime::evidence::EvidencePolicy::redacted(
        &config,
        &rules.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let capture: CapturePolicyFile = driver_read(&case.capture_policy.path)?;
    ensure(
        capture.schema_version == 1 && !capture.masks.is_empty(),
        "reviewed exact-geometry masks required",
    )?;
    ensure(
        capture
            .masks
            .iter()
            .all(|m| m.rows == approved.size.rows && m.cols == approved.size.cols),
        "case mask geometry differs",
    )?;
    let spawn = SpawnSpec {
        argv,
        cwd: case.cwd.clone(),
        environment: vec![("CAFLEET_DATABASE_URL".into(), case.database_url.clone())],
        evidence_policy: json!({"mode":"redacted","config":config}),
        size: approved.size,
    };
    spawn.validate()?;
    NativeCapturePolicy::from_launch(&spawn, &capture.masks, &catalog)?;
    driver_publish(
        &case.launch_evidence_path,
        &json!({"schema_version":1,"run_id":pending.run.run_id,
        "launch_nonce":pending.run.launch_nonce,"argv":spawn.argv,"cwd":spawn.cwd,"policy":spawn.evidence_policy,
        "native_executable":case.native_executable,"version_evidence":case.version_evidence}),
        until,
    )?;
    let producer = json!({"schema_version":1,"kind":"verification_libtest",
        "source_manifest_ref":case.source_manifest.path,"source_manifest_sha256":case.source_manifest.sha256,
        "libtest_executable_path":case.libtest.path,"libtest_executable_sha256":case.libtest.sha256,
        "parser_version":"0.18.1","collector_version":"1","catalog_origin":"bundled_unknown",
        "native_executable":{"path":case.native_executable.path,"sha256":case.native_executable.sha256,
            "version":case.native_version,"version_evidence_ref":case.version_evidence.path},
        "launch_evidence_ref":case.launch_evidence_path,"environment_evidence_ref":case.environment_evidence.path});
    driver_publish(&case.producer_manifest_path, &producer, until)?;
    let config = NativeCaptureConfig {
        schema_version: 1,
        database_url: case.database_url.clone(),
        descriptor_path: artifacts.paths.descriptor.clone(),
        expected_run_id: pending.run.run_id,
        expected_launch_nonce: pending.run.launch_nonce.clone(),
        arm_path: case.arm_path.clone(),
        output_directory: case.output_directory.clone(),
        producer_manifest_path: case.producer_manifest_path.clone(),
        capture_policy_path: case.capture_policy.path.clone(),
    };
    driver_publish(&case.config_path, &config, until)?;
    Ok((artifacts, spawn))
}

fn driver_output_seq(conn: &Connection, launched: &LaunchOutcome, until: Instant) -> Result<u64> {
    let run = storage::get_run(conn, launched.run.run_id)?;
    Ok(crate::runtime::runner::probe_health(&run, until)?.output_seq)
}

fn driver_post_stage(
    conn: &Connection,
    case: &DriverCase,
    approved: &ApprovedCase,
    launched: &LaunchOutcome,
    armed_output_seq: Option<u64>,
) -> Result<Value> {
    let until = Instant::now() + Duration::from_secs(1);
    let stop_output_seq = driver_output_seq(conn, launched, until);
    let mut statement = conn
        .prepare("SELECT e.event_seq,e.kind,e.observed_at,e.interaction_id,i.classification,i.close_reason,i.closed_at FROM member_runtime_events e LEFT JOIN member_interactions i ON i.interaction_id=e.interaction_id WHERE e.run_id=?1 ORDER BY e.event_seq DESC LIMIT 64")
        .map_err(io_error)?;
    let mut events = statement
        .query_map([launched.run.run_id], |row| {
            Ok(json!({"event_seq":row.get::<_, i64>(0)?,"kind":row.get::<_, String>(1)?,
                "observed_at":row.get::<_, String>(2)?,"interaction_id":row.get::<_, Option<i64>>(3)?,
                "classification":row.get::<_, Option<String>>(4)?,"close_reason":row.get::<_, Option<String>>(5)?,
                "closed_at":row.get::<_, Option<String>>(6)?}))
        })
        .map_err(io_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(io_error)?;
    events.reverse();
    let marker = case.cwd.join(
        approved
            .requested_marker()
            .expect("answer-stage cases name their probe"),
    );
    let marker_sha256 = if marker.try_exists().map_err(io_error)? {
        Some(super::files::file_digest(&marker)?)
    } else {
        None
    };
    Ok(
        json!({"decision":approved.decision,"prefix_hex":approved.prefix_hex,"stage_hex":approved.stage_hex,
        "output_seq_at_arm":armed_output_seq,
        "output_seq_at_stop":stop_output_seq.as_ref().ok(),
        "output_seq_error":stop_output_seq.as_ref().err().map(|e|e.message().into_owned()),
        "manual_input_count":"unavailable: runner health exposes no manual-input counter",
        "events":events,"requested_marker":approved.requested_marker(),
        "requested_marker_sha256":marker_sha256}),
    )
}

fn driver_work(
    conn: &Connection,
    case: &DriverCase,
    ownership: &DriverOwnership,
    launched: &LaunchOutcome,
    work_until: Instant,
) -> Result<Value> {
    driver_validate_pane(case, ownership, launched, work_until)?;
    driver_validate_run(conn, launched)?;
    let pane = launched
        .placement
        .mux_pane_id
        .as_deref()
        .ok_or_else(|| error("owned runner pane required"))?;
    let approved = ApprovedCase::select(case)?;
    let cols = approved.size.cols.to_string();
    let rows = approved.size.rows.to_string();
    driver_tmux(
        case,
        &[
            "resize-window",
            "-t",
            &case.tmux.window_id,
            "-x",
            &cols,
            "-y",
            "80",
        ],
        work_until,
    )?;
    driver_tmux(
        case,
        &[
            "select-layout",
            "-t",
            &case.tmux.window_id,
            "main-horizontal",
        ],
        work_until,
    )?;
    driver_tmux(
        case,
        &["resize-pane", "-t", pane, "-x", &cols, "-y", &rows],
        work_until,
    )?;
    let geometry = driver_tmux(
        case,
        &[
            "display-message",
            "-p",
            "-t",
            pane,
            "#{pane_width}|#{pane_height}",
        ],
        work_until,
    )?;
    ensure(
        geometry.trim() == approved.tmux_geometry(),
        "actual native geometry differs",
    )?;
    let ownership_hash = super::files::file_digest(&case.driver_ownership_path)?;
    let started = DriverStarted {
        schema_version: 1,
        case_manifest_sha256: ownership.case_manifest_sha256.clone(),
        driver_ownership_sha256: ownership_hash.clone(),
        identity: launched.started.identity.clone(),
        runner: launched.started.runner.clone(),
        child: launched.started.child.clone(),
        pane_id: pane.into(),
        rows: approved.size.rows,
        cols: approved.size.cols,
        capture_policy_sha256: case.capture_policy.sha256.clone(),
        event_redaction_sha256: case.event_redaction.sha256.clone(),
    };
    driver_publish(&case.started_record_path, &started, work_until)?;
    let mut armed_validated_at: Option<Instant> = None;
    let mut armed_output_seq: Option<u64> = None;
    loop {
        let stopped = if case.stop_path.try_exists().map_err(io_error)? {
            let stop: DriverStop = driver_read(&case.stop_path)?;
            ensure(
                stop.schema_version == 1
                    && stop.reason == "director_stop"
                    && stop.case_manifest_sha256 == ownership.case_manifest_sha256
                    && stop.driver_ownership_sha256 == ownership_hash,
                "stop binding differs",
            )?;
            true
        } else {
            Instant::now() >= work_until
        };
        if stopped {
            return match approved.mode {
                DriverMode::NoAnswer => Ok(Value::Null),
                DriverMode::AnswerStage => {
                    driver_post_stage(conn, case, approved, launched, armed_output_seq)
                }
            };
        }
        ensure(
            launched.started.runner.matches()? && launched.started.child.matches()?,
            "native or runner ended before the requested capture completed",
        )?;
        let armed_validation_due = match armed_validated_at {
            None => true,
            Some(at) => at.elapsed() >= Duration::from_secs(1),
        };
        if armed_validation_due && driver_armed(case)? {
            armed_validated_at = Some(Instant::now());
            driver_validate_privacy(case, approved, ownership)?;
            driver_validate_tmux(case, ownership, work_until)?;
            driver_validate_run(conn, launched)?;
            let geometry = driver_tmux(
                case,
                &[
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_width}|#{pane_height}",
                ],
                work_until,
            )?;
            ensure(
                geometry.trim() == approved.tmux_geometry(),
                "armed geometry changed",
            )?;
            if armed_output_seq.is_none() {
                armed_output_seq = Some(driver_output_seq(conn, launched, work_until)?);
            }
        }
        for marker in approved.forbidden_markers() {
            ensure(
                !case.cwd.join(marker).try_exists().map_err(io_error)?,
                "a probe outside the reviewed decision executed",
            )?;
        }
        std::thread::sleep(
            Duration::from_millis(20).min(work_until.saturating_duration_since(Instant::now())),
        );
    }
}

fn run_native_capture_prepare_start(config_path: &Path) -> Result<()> {
    let case_path = config_path
        .parent()
        .ok_or_else(|| error("case parent required"))?
        .join("case.json");
    let case_bytes = super::files::read_private(&case_path, 65536)?;
    let case: DriverCase = serde_json::from_slice(&case_bytes).map_err(io_error)?;
    let approved = ApprovedCase::select(&case)?;
    ensure(
        case.schema_version == 1
            && case.cleanup_reserve_ms == 4000
            && case.config_path == config_path
            && case.case_nonce.len() == 64,
        "private case identity differs",
    )?;
    let neutral = Path::new(DRIVER_NEUTRAL_ROOT);
    let private = Path::new(DRIVER_PRIVATE_ROOT);
    ensure(
        case.cwd == neutral.join(approved.id)
            && case.database_url
                == format!("sqlite:///{}", private.join(approved.database).display())
            && case.tmux.socket_path == private.join(approved.socket)
            && case.output_directory == neutral.join(format!("{}-capture", approved.id))
            && case.helper.path == case.cwd.join("permission-probe"),
        "case resources differ from approved isolated paths",
    )?;
    for path in [
        &case.config_path,
        &case.arm_path,
        &case.producer_manifest_path,
        &case.launch_evidence_path,
        &case.driver_ownership_path,
        &case.started_record_path,
        &case.stop_path,
        &case.privacy_confirmation_path,
        &case.result_path,
        &case.descriptor_copy_path,
    ] {
        ensure(
            path.parent() == Some(case.cwd.as_path()),
            "driver output outside owned case directory",
        )?;
    }
    ensure(
        crate::coding_agent::coding_agent(&case.director_backend).is_some(),
        "known authorizing Director backend required",
    )?;
    for directory in [&case.cwd, &case.output_directory] {
        let metadata = std::fs::symlink_metadata(directory).map_err(io_error)?;
        ensure(
            metadata.is_dir()
                && metadata.uid() == nix::unistd::geteuid().as_raw()
                && metadata.mode() & 0o777 == 0o700,
            "private owned case directory required",
        )?;
    }
    validate_private_file(
        Path::new(crate::db::database_path(&case.database_url)?),
        nix::unistd::geteuid().as_raw(),
        0o600,
    )?;
    let sample = RealInputDeadlineClock.sample()?;
    let remaining = case
        .expires_at_mono_ns
        .checked_sub(sample.monotonic_ns)
        .filter(|n| *n > 4_000_000_000 && *n <= 120_000_000_000 + case.answer_window_ms * 1_000_000)
        .ok_or(CafleetError::DeadlineExceeded)?;
    let case_until = sample
        .before
        .checked_add(Duration::from_nanos(remaining))
        .ok_or_else(|| error("case clock range"))?;
    let work_until = case_until
        .checked_sub(Duration::from_secs(4))
        .ok_or_else(|| error("case cleanup reserve"))?;
    for reference in [
        &case.native_executable,
        &case.helper,
        &case.libtest,
        &case.source_manifest,
        &case.version_evidence,
        &case.environment_evidence,
        &case.event_redaction,
        &case.capture_policy,
    ] {
        reference.verify()?;
    }
    let version: DriverVersion = driver_read(&case.version_evidence.path)?;
    ensure(
        version.schema_version == 1
            && version.exit_code == 0
            && version.executable == case.native_executable.path
            && version.sha256 == case.native_executable.sha256
            && version.argv
                == [
                    case.native_executable.path.to_string_lossy().into_owned(),
                    "--version".into(),
                ]
            && version.stdout.lines().count() == 1
            && crate::coding_agent::version_token(&version.stdout)
                == Ok(case.native_version.as_str()),
        "same-file actual version evidence differs",
    )?;
    ensure(
        std::env::current_exe().map_err(io_error)? == case.libtest.path,
        "selected libtest differs",
    )?;
    ensure(
        !case
            .cwd
            .join("allow.executed")
            .try_exists()
            .map_err(io_error)?
            && !case
                .cwd
                .join("deny.executed")
                .try_exists()
                .map_err(io_error)?
            && !case.arm_path.try_exists().map_err(io_error)?,
        "fresh case without probe markers or arm required",
    )?;
    let ownership = DriverOwnership {
        schema_version: 1,
        case_manifest_sha256: driver_digest(&case_bytes),
        case_nonce: case.case_nonce.clone(),
        driver: ProcessIdentity::current()?,
        server: ProcessIdentity::read(case.tmux.server_pid)?
            .ok_or_else(|| error("dedicated server absent"))?,
        tmux: case.tmux.clone(),
        expires_at_mono_ns: case.expires_at_mono_ns,
    };
    driver_validate_tmux(&case, &ownership, work_until)?;
    driver_publish(&case.driver_ownership_path, &ownership, work_until)?;
    let runner: Rc<dyn CommandRunner> = Rc::new(SystemRunner);
    let mux =
        resolve_multiplexer(Some("tmux"), std::env::vars().collect(), runner).map_err(io_error)?;
    let context = MultiplexerContext {
        session: case.tmux.session.clone(),
        window_id: case.tmux.window_id.clone(),
        pane_id: case.tmux.pane_id.clone(),
    };
    let mut host = CaptureVerificationLaunchHost {
        inner: NativeLaunchHost::new(&mux, &context),
        case: &case,
        case_until,
    };
    let mut conn = crate::db::connect(&case.database_url)?;
    with_database_deadline(&mut conn, case_until, |conn| {
        crate::db::migrate_to_head(conn)?;
        let fleet = crate::broker::fleets::create_fleet(
            conn,
            Some(format!("native-{}", approved.id).as_str()),
            &context.session,
            &context.window_id,
            &context.pane_id,
            &case.director_backend,
            "tmux",
        )?;
        let policy = approved
            .backend
            .resolve_policy(Some(&case.native_version), Some(approved.permission_mode))?;
        let capabilities =
            ProfileCatalog::bundled()?.capabilities(approved.backend, Some(&case.native_version));
        let settings = RunSettings {
            database_identity_json: storage::database_identity(conn)?,
            creator: ownership.driver.clone(),
            launch_nonce: case.case_nonce.clone(),
            requested_mode: policy.requested_mode,
            resolved_mode: policy.resolved_mode,
            mode_source: policy.mode_source,
            permission_argv_json: json!(policy.permission_argv),
            event_evidence: storage::EventEvidence::Redacted,
            native_version: Some(case.native_version.clone()),
            profile_version: capabilities.profile_version,
            coverage: capabilities.detection,
        };
        let member = PendingMemberSpec {
            fleet_id: fleet.fleet_id,
            name: approved.member_name(),
            description: format!("private {} capture", case.mode),
            placement: crate::broker::NewPlacement {
                backend: "tmux".into(),
                mux_session: context.session.clone(),
                mux_window_id: context.window_id.clone(),
                mux_pane_id: None,
                coding_agent: approved.backend.name().into(),
            },
        };
        let launch_until = work_until.min(Instant::now() + Duration::from_secs(10));
        let launched = create_registered(
            conn,
            &member,
            &settings,
            |conn, pending| driver_prepare(conn, pending, &case, launch_until),
            &mut host,
            launch_until,
        )?;
        let work = catch_unwind(AssertUnwindSafe(|| {
            driver_work(conn, &case, &ownership, &launched, work_until)
        }));
        let (work_error, post_stage) = match &work {
            Ok(Ok(post_stage)) => (None, post_stage.clone()),
            Ok(Err(cause)) => (
                Some(cause.message().chars().take(2048).collect::<String>()),
                Value::Null,
            ),
            Err(_) => (Some("owned post-start work unwound".into()), Value::Null),
        };
        let cleanup = catch_unwind(AssertUnwindSafe(|| {
            let clock = ShutdownClock::new(Instant::now()).clipped(case_until);
            let shutdown = driver_validate_run(conn, &launched)
                .and_then(|current| host.shutdown_owned(&current, clock));
            let pane = launched
                .placement
                .mux_pane_id
                .as_deref()
                .expect("launched pane");
            let observed = (|| {
                driver_validate_pane(&case, &ownership, &launched, clock.storage_deadline())?;
                while launched.started.runner.matches()?
                    && Instant::now() < clock.storage_deadline()
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let service: Value =
                    driver_read(&case.config_path.with_extension("service-outcome.json"))?;
                ensure(
                    service["config_sha256"] == super::files::file_digest(&case.config_path)?
                        && service["run_id"] == launched.run.run_id
                        && service["launch_nonce"] == launched.run.launch_nonce,
                    "service outcome binding differs",
                )?;
                ensure(
                    driver_armed(&case)?,
                    "not_armed: the capture was never armed",
                )?;
                driver_validate_privacy(&case, approved, &ownership)?;
                let capture = driver_complete_capture(&case, approved, &launched, &service)?;
                let pane_status = driver_tmux(
                    &case,
                    &[
                        "display-message",
                        "-p",
                        "-t",
                        pane,
                        "#{pane_dead}|#{pane_dead_status}",
                    ],
                    clock.storage_deadline(),
                )?;
                ensure(
                    pane_status.trim()
                        == format!(
                            "1|{}",
                            service["status"].as_i64().expect("validated status")
                        ),
                    "actual pane exit differs from service status",
                )?;
                Ok::<_, CafleetError>(
                    json!({"service":service,"pane_status":pane_status,"capture":capture}),
                )
            })();
            let pane_capture =
                driver_validate_pane(&case, &ownership, &launched, clock.storage_deadline())
                    .and_then(|()| {
                        driver_capture_runner_pane(&case, pane, clock.storage_deadline())
                    });
            let close =
                driver_validate_pane(&case, &ownership, &launched, clock.storage_deadline())
                    .and_then(|()| host.close_pane(pane, clock.storage_deadline()));
            let runner_absent = launched.started.runner.matches().map(|exists| !exists);
            let native_absent = launched.started.child.matches().map(|exists| !exists);
            let summary = json!({"schema_version":1,"case_manifest_sha256":ownership.case_manifest_sha256,"work_error":work_error,
                "driver_ownership_sha256":super::files::file_digest(&case.driver_ownership_path)?,
                "shutdown":shutdown.as_ref().ok(),"shutdown_error":shutdown.as_ref().err().map(|e|e.message().into_owned()),
                "observation":observed.as_ref().ok(),"observation_error":observed.as_ref().err().map(|e|e.message().into_owned()),
                "pane_capture":pane_capture.as_ref().ok(),"pane_capture_error":pane_capture.as_ref().err().map(|e|e.message().into_owned()),
                "pane_closed":close.is_ok(),"close_error":close.as_ref().err().map(|e|e.message().into_owned()),
                "runner_absent":runner_absent.as_ref().ok(),"native_absent":native_absent.as_ref().ok(),
                "runner_identity_error":runner_absent.as_ref().err().map(|e|e.message().into_owned()),
                "native_identity_error":native_absent.as_ref().err().map(|e|e.message().into_owned()),
                "case_id":approved.id,"post_stage":post_stage,
                "requested_marker":approved.requested_marker(),
                "requested_marker_present":match approved.requested_marker() {
                    Some(marker) => Some(case.cwd.join(marker).try_exists().map_err(io_error)?),
                    None => None,
                },
                "allow_marker_absent":!case.cwd.join("allow.executed").try_exists().map_err(io_error)?,
                "deny_marker_absent":!case.cwd.join("deny.executed").try_exists().map_err(io_error)?});
            driver_publish(&case.result_path, &summary, case_until)?;
            let shutdown = shutdown?;
            observed?;
            close?;
            let mut forbidden_absent = true;
            for marker in approved.forbidden_markers() {
                forbidden_absent &= !case.cwd.join(marker).try_exists().map_err(io_error)?;
            }
            ensure(
                shutdown.cleanup_state == storage::CleanupState::Complete
                    && runner_absent?
                    && native_absent?
                    && forbidden_absent,
                "native case cleanup incomplete or a probe outside the reviewed decision executed",
            )
        }));
        match work {
            Err(panic) => {
                if !matches!(cleanup, Ok(Ok(()))) {
                    eprintln!("native driver cleanup also failed");
                }
                resume_unwind(panic)
            }
            Ok(Err(primary)) => match cleanup {
                Ok(Err(cause)) => Err(primary.with_cleanup(cause.message())),
                Err(_) => Err(primary.with_cleanup("owned cleanup panicked")),
                _ => Err(primary),
            },
            Ok(Ok(_)) => match cleanup {
                Ok(result) => result,
                Err(panic) => resume_unwind(panic),
            },
        }
    })
}

#[test]
#[ignore = "Director-owned native preparation requires an explicit private case"]
fn native_capture_prepare_start_driver() {
    let config = std::env::var_os("CAFLEET_NATIVE_CAPTURE_CONFIG")
        .expect("private capture configuration required");
    let config = Path::new(&config);
    assert!(
        config.is_absolute(),
        "absolute capture configuration required"
    );
    run_native_capture_prepare_start(config).expect("actual native prepare/start/cleanup outcome");
}
