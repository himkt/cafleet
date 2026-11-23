use crate::error::CafleetError;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

pub(crate) const PLUGINS_KEY: &str = "plugins";
pub(crate) const PLUGINS_DISABLED_KEY: &str = "plugins.disabled";
pub(crate) const PLUGINS_ENABLED_KEY: &str = "plugins.enabled";
pub(crate) const GATEWAY_INJECTION_KEY: &str = "plugins.entries.cafleet.allow_gateway_injection";
pub(crate) const PLUGIN_NAME: &str = "cafleet";

/// Runs `hermes` with `HERMES_HOME=<base>`.
pub(crate) struct HermesCli {
    executable: PathBuf,
    base: PathBuf,
}

/// Each call runs until `deadline`; calls of one concurrent batch pass the same deadline.
pub(crate) trait HermesConfig: Sync {
    /// The trimmed stdout of `hermes config path`.
    fn config_path(&self, deadline: Instant) -> Result<String, String>;
    /// `hermes config get --json KEY`; `None` is the *not set* answer.
    fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String>;
    /// `hermes config set KEY VALUE`.
    fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String>;
}

/// A completed `hermes` process: its exit code, stdout and stderr.
type HermesOutput = (i32, String, String);

/// The stdout of a command that exited 0, or its stderr text as the cause.
fn succeeded((exit_code, stdout, stderr): HermesOutput) -> Result<String, String> {
    match (exit_code, stderr.trim()) {
        (0, _) => Ok(stdout),
        (code, "") => Err(format!("exited with code {code}")),
        (_, stderr) => Err(stderr.to_owned()),
    }
}

impl HermesCli {
    pub(crate) fn new(executable: PathBuf, base: PathBuf) -> Self {
        Self { executable, base }
    }

    fn run(&self, args: &[&str], deadline: Instant) -> Result<HermesOutput, String> {
        let output = crate::coding_agent::run_bounded_output(
            &self.executable,
            args,
            &[("HERMES_HOME", &self.base)],
            deadline,
        )?;
        let exit_code = output
            .status
            .code()
            .ok_or_else(|| format!("hermes {} ended by {}", args.join(" "), output.status))?;
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| format!("hermes {} printed non-UTF-8 output", args.join(" ")))?;
        Ok((
            exit_code,
            stdout,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

impl HermesConfig for HermesCli {
    fn config_path(&self, deadline: Instant) -> Result<String, String> {
        succeeded(self.run(&["config", "path"], deadline)?).map(|stdout| stdout.trim().to_owned())
    }

    fn get(&self, key: &str, deadline: Instant) -> Result<Option<Value>, String> {
        let output = self.run(&["config", "get", "--json", key], deadline)?;
        let not_set = format!("Config key not set: {key}");
        if output.0 == 1 && output.2.lines().any(|line| line.trim() == not_set) {
            return Ok(None);
        }
        let stdout = succeeded(output)?;
        serde_json::from_str(&stdout)
            .map(Some)
            .map_err(|cause| format!("{key} printed no JSON value: {cause}"))
    }

    fn set(&self, key: &str, value: &str, deadline: Instant) -> Result<(), String> {
        succeeded(self.run(&["config", "set", key, value], deadline)?).map(drop)
    }
}

/// The plugin directory under the Hermes home that owns `config_path`.
pub(crate) fn plugin_dir(config_path: &str) -> Result<PathBuf, String> {
    home_of_config_path(config_path).map(|home| home.join(crate::assets::HERMES_PLUGIN_DIR))
}

/// The CAFleet-relevant members of the resolved `plugins` subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginState {
    pub(crate) disabled_lists_cafleet: bool,
    pub(crate) enabled: Vec<String>,
    pub(crate) injection_allowed: bool,
}

impl PluginState {
    /// The keys whose values still keep the plugin from loading with gateway injection.
    pub(crate) fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.enabled.iter().any(|name| name == PLUGIN_NAME) {
            missing.push(PLUGINS_ENABLED_KEY);
        }
        if !self.injection_allowed {
            missing.push(GATEWAY_INJECTION_KEY);
        }
        missing
    }
}

fn read_plugin_state(hermes: &dyn HermesConfig, deadline: Instant) -> Result<PluginState, String> {
    plugin_state(hermes.get(PLUGINS_KEY, deadline)?)
}

/// Interprets a `plugins` reading: a missing member is that key's *not set* state, and a
/// *not set* `plugins` key leaves all three unset.
pub(crate) fn plugin_state(plugins: Option<Value>) -> Result<PluginState, String> {
    let plugins = plugins.unwrap_or_else(|| Value::Object(Default::default()));
    if !plugins.is_object() {
        return Err(format!("{PLUGINS_KEY} is not a JSON object"));
    }
    let disabled = plugin_list(PLUGINS_DISABLED_KEY, plugins.get("disabled"))?;
    Ok(PluginState {
        disabled_lists_cafleet: disabled.iter().any(|name| name == PLUGIN_NAME),
        enabled: plugin_list(PLUGINS_ENABLED_KEY, plugins.get("enabled"))?,
        injection_allowed: plugins.pointer("/entries/cafleet/allow_gateway_injection")
            == Some(&Value::Bool(true)),
    })
}

/// A set plugin list, which must be a JSON array of strings.
fn plugin_list(key: &str, value: Option<&Value>) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|cause| format!("{key} is not a JSON array of strings: {cause}")),
    }
}

/// Runs `config path` and the `plugins` read as one concurrent batch under `deadline`.
pub(crate) fn read_batch(
    hermes: &dyn HermesConfig,
    deadline: Instant,
) -> (Result<String, String>, Result<Option<Value>, String>) {
    std::thread::scope(|scope| {
        let path = scope.spawn(|| hermes.config_path(deadline));
        let plugins = scope.spawn(|| hermes.get(PLUGINS_KEY, deadline));
        (joined(path), joined(plugins))
    })
}

pub(crate) fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Writes the plugin keys `state` lacks, serially because each set rewrites `config.yaml`, then
/// re-reads `plugins` once when something was written; returns whether it wrote.
pub(crate) fn enable_plugin(
    hermes: &dyn HermesConfig,
    state: &PluginState,
    budget: std::time::Duration,
) -> Result<bool, String> {
    if state.disabled_lists_cafleet {
        return Err(format!(
            "{PLUGINS_DISABLED_KEY} lists {PLUGIN_NAME}; remove it from {PLUGINS_DISABLED_KEY}, then rerun 'cafleet setup --coding-agent hermes'"
        ));
    }
    let missing = state.missing();
    if missing.contains(&PLUGINS_ENABLED_KEY) {
        let mut enabled = state.enabled.clone();
        enabled.push(PLUGIN_NAME.to_owned());
        let value = serde_json::to_string(&enabled).expect("a list of strings serializes as JSON");
        hermes.set(PLUGINS_ENABLED_KEY, &value, Instant::now() + budget)?;
    }
    if missing.contains(&GATEWAY_INJECTION_KEY) {
        hermes.set(GATEWAY_INJECTION_KEY, "true", Instant::now() + budget)?;
    }
    if missing.is_empty() {
        return Ok(false);
    }
    match read_plugin_state(hermes, Instant::now() + budget)?
        .missing()
        .first()
    {
        None => Ok(true),
        Some(&PLUGINS_ENABLED_KEY) => Err(format!(
            "{PLUGINS_ENABLED_KEY} still lacks {PLUGIN_NAME} after the write"
        )),
        Some(key) => Err(format!("{key} is still not true after the write")),
    }
}

/// The member run's Hermes session identity, stored in `member_runs.native_session_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeSession {
    /// The effective Hermes home, the directory of `hermes config path` at launch, which holds
    /// `processes.json`.
    pub(crate) hermes_home: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
}

/// The effects settlement needs: native input and screen reads on the member's pane, the run's
/// child state, the session record and the Hermes process registry.
trait HermesSettlementHost {
    fn client_exited(&mut self) -> Result<bool, CafleetError>;
    fn send_keys(&mut self, keys: &[&str]) -> Result<(), CafleetError>;
    /// The native capture, preserving SGR for placeholder detection.
    fn capture(&mut self) -> Result<String, CafleetError>;
    fn record_session_id(&mut self, session_id: &str) -> Result<(), CafleetError>;
    fn processes_json(&mut self, home: &Path) -> Result<Option<String>, CafleetError>;
    /// Whether the registered process still runs: `pid` alive with `host_start_time`, Hermes's
    /// `/proc/<pid>/stat` start field. A platform that cannot compare start times reports a live
    /// `pid` as running.
    fn registered_process_running(
        &mut self,
        pid: u32,
        host_start_time: u64,
    ) -> Result<bool, CafleetError>;
    fn pause(&mut self);
    fn now(&self) -> Instant;
}

const INTERRUPT_KEYS: [&str; 1] = ["Ctrl+C"];
const STOP_COMMAND: [&str; 5] = ["/", "s", "t", "o", "p"];
const EXIT_COMMAND: [&str; 5] = ["/", "e", "x", "i", "t"];
// Ctrl+C on an empty composer ends the client without stopping its session. Backspace also
// clears the multiline drafts that Hermes's line-kill shortcuts leave behind.
const CLEAR_FIRST_BATCH: [&str; 64] = {
    let mut keys = ["Backspace"; 64];
    keys[0] = "Escape";
    keys
};
const CLEAR_BACKSPACE_BATCH: [&str; 64] = ["Backspace"; 64];
const CLEAR_BATCHES_PER_ROUND: usize = 16;
const MENU_CLOSE_KEYS: [&str; 1] = ["Escape"];
const SUBMIT_KEYS: [&str; 1] = ["Enter"];
// A typed slash command opens a completion menu once it has rendered, so the menu is closed
// before the first Enter and each Enter waits for its effect before the next one.
const MAX_SUBMITS: usize = 3;
const SUBMIT_WINDOW: std::time::Duration = std::time::Duration::from_millis(1000);

const BUSY_MARKERS: [&str; 2] = ["type to redirect this turn", "↑/↓ to select"];
const BUSY_TAIL_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Settlement {
    /// The session's work stopped and none of its registered processes runs.
    Verified,
    /// The client had exited before settlement could stop its session.
    ClientAlreadyExited,
}

/// Stops the member's Hermes session work and verifies that no process it registered still runs.
fn settle(
    member_id: i64,
    session: &NativeSession,
    host: &mut dyn HermesSettlementHost,
    until: Instant,
) -> Result<Settlement, CafleetError> {
    settle_session(session, host, until).map_err(|cause| {
        CafleetError::App(format!(
            "hermes work for member {member_id} could not be verified as stopped: {}; member was not deleted.",
            cause.message()
        ))
    })
}

fn settle_session(
    session: &NativeSession,
    host: &mut dyn HermesSettlementHost,
    until: Instant,
) -> Result<Settlement, CafleetError> {
    let exited = host.client_exited()?;
    let session_id = match (&session.session_id, exited) {
        (Some(recorded), true) => recorded.clone(),
        (None, true) => return Ok(Settlement::ClientAlreadyExited),
        (_, false) => {
            let session_id = stop_client(host, until)?;
            host.record_session_id(&session_id)?;
            session_id
        }
    };
    verify_processes(host, &session.hermes_home, &session_id)?;
    Ok(Settlement::Verified)
}

fn already_exited_warning(member_id: i64) -> String {
    format!(
        "warning: hermes client for member {member_id} had already exited; its session work may still be running in the Hermes gateway"
    )
}

/// The budget of one Hermes member's settlement, which takes several native round trips.
pub(crate) fn settlement_deadline() -> crate::time::Deadline {
    crate::time::Deadline::until(Instant::now() + SETTLEMENT_BUDGET)
}

const SETTLEMENT_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

fn stop_client(
    host: &mut dyn HermesSettlementHost,
    until: Instant,
) -> Result<String, CafleetError> {
    if busy(&host.capture()?) {
        host.send_keys(&INTERRUPT_KEYS)?;
        wait_for(
            host,
            until,
            "the Hermes turn did not stop after Ctrl-C",
            |screen| (!busy(screen)).then_some(()),
        )?;
    }
    let reports = stop_reports(&host.capture()?);
    submit_command(
        host,
        until,
        &STOP_COMMAND,
        "Hermes did not report /stop",
        |screen| (stop_reports(screen) > reports).then_some(()),
    )?;
    let session_id = submit_command(
        host,
        until,
        &EXIT_COMMAND,
        "Hermes printed no exit summary after /exit",
        exit_summary_session,
    )?;
    poll(host, until, |host| Ok(host.client_exited()?.then_some(())))?
        .ok_or_else(|| CafleetError::App("the Hermes client did not exit after /exit".into()))?;
    Ok(session_id)
}

/// Asks `done` until it yields a value or `until` passes, pausing between attempts; `None` once
/// `until` passes.
fn poll<T>(
    host: &mut dyn HermesSettlementHost,
    until: Instant,
    mut done: impl FnMut(&mut dyn HermesSettlementHost) -> Result<Option<T>, CafleetError>,
) -> Result<Option<T>, CafleetError> {
    loop {
        if let Some(value) = done(host)? {
            return Ok(Some(value));
        }
        if host.now() >= until {
            return Ok(None);
        }
        host.pause();
    }
}

/// Clears the composer, types `command`, closes its completion menu, then presses Enter up to
/// `MAX_SUBMITS` times, each after the previous one showed no effect within `SUBMIT_WINDOW`, and
/// keeps waiting for `done` until `until`.
fn submit_command<T>(
    host: &mut dyn HermesSettlementHost,
    until: Instant,
    command: &[&str],
    timeout: &str,
    mut done: impl FnMut(&str) -> Option<T>,
) -> Result<T, CafleetError> {
    clear_composer(host, until)?;
    host.send_keys(command)?;
    host.pause();
    host.send_keys(&MENU_CLOSE_KEYS)?;
    for _ in 0..MAX_SUBMITS {
        host.pause();
        if host.client_exited()? {
            break;
        }
        host.send_keys(&SUBMIT_KEYS)?;
        let window = (host.now() + SUBMIT_WINDOW).min(until);
        if let Some(value) = poll(host, window, |host| Ok(done(&host.capture()?)))? {
            return Ok(value);
        }
    }
    wait_for(host, until, timeout, done)
}

fn clear_composer(host: &mut dyn HermesSettlementHost, until: Instant) -> Result<(), CafleetError> {
    let failed = || CafleetError::App("the Hermes composer still holds text after clearing".into());
    let mut screen = host.capture()?;
    loop {
        if host.now() >= until {
            return Err(failed());
        }
        if composer_empty(&screen) {
            return Ok(());
        }
        let previous = composer_region(&screen).ok_or_else(failed)?;
        let previous = crate::output::strip_ansi(&previous.join("\n"));
        for batch in 0..CLEAR_BATCHES_PER_ROUND {
            if host.now() >= until {
                return Err(failed());
            }
            host.send_keys(if batch == 0 {
                &CLEAR_FIRST_BATCH
            } else {
                &CLEAR_BACKSPACE_BATCH
            })?;
        }
        host.pause();
        screen = host.capture()?;
        let current = composer_region(&screen).ok_or_else(failed)?;
        if crate::output::strip_ansi(&current.join("\n")) == previous {
            return Err(failed());
        }
    }
}

fn wait_for<T>(
    host: &mut dyn HermesSettlementHost,
    until: Instant,
    timeout: &str,
    mut done: impl FnMut(&str) -> Option<T>,
) -> Result<T, CafleetError> {
    poll(host, until, |host| Ok(done(&host.capture()?)))?
        .ok_or_else(|| CafleetError::App(timeout.into()))
}

fn busy(screen: &str) -> bool {
    let screen = crate::output::strip_ansi(screen);
    let lines = screen
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    lines[lines.len().saturating_sub(BUSY_TAIL_LINES)..]
        .iter()
        .any(|line| BUSY_MARKERS.iter().any(|marker| line.contains(marker)))
}

fn stop_reports(screen: &str) -> usize {
    static REPORT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"Stopped \d+ process\(es\)\.|No running background processes\.")
            .expect("the /stop report pattern is valid")
    });
    REPORT.find_iter(&crate::output::strip_ansi(screen)).count()
}

fn exit_summary_session(screen: &str) -> Option<String> {
    static SUMMARY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^Session:\s+(local-[0-9a-f]+)\s*$")
            .expect("the exit summary pattern is valid")
    });
    SUMMARY
        .captures_iter(&crate::output::strip_ansi(screen))
        .last()
        .map(|captures| captures[1].to_owned())
}

#[derive(Deserialize)]
struct RegisteredProcess {
    pid: u32,
    host_start_time: u64,
    owner_task_id: String,
}

fn verify_processes(
    host: &mut dyn HermesSettlementHost,
    home: &Path,
    session_id: &str,
) -> Result<(), CafleetError> {
    let Some(text) = host.processes_json(home)? else {
        return Ok(());
    };
    let registry = home.join("processes.json");
    let processes = serde_json::from_str::<Vec<RegisteredProcess>>(&text).map_err(|cause| {
        CafleetError::App(format!(
            "{} is not a Hermes process registry: {cause}",
            registry.display()
        ))
    })?;
    let mut running = Vec::new();
    for process in processes
        .iter()
        .filter(|process| process.owner_task_id == session_id)
    {
        if host.registered_process_running(process.pid, process.host_start_time)? {
            running.push(process.pid.to_string());
        }
    }
    if running.is_empty() {
        Ok(())
    } else {
        Err(CafleetError::App(format!(
            "background processes of session {session_id} are still running: {}",
            running.join(", ")
        )))
    }
}

fn composer_region(screen: &str) -> Option<Vec<&str>> {
    let rows = screen.lines().collect::<Vec<_>>();
    let stripped = rows
        .iter()
        .map(|row| crate::output::strip_ansi(row))
        .collect::<Vec<_>>();
    let bottom = stripped
        .iter()
        .rposition(|row| !row.is_empty() && row.chars().all(|character| character == '─'))?;
    let width = stripped[bottom].chars().count();
    let top = stripped[..bottom].iter().rposition(|row| {
        row.starts_with('─') && row.ends_with('─') && row.chars().count() == width
    })?;
    Some(rows[top + 1..bottom].to_vec())
}

fn composer_empty(screen: &str) -> bool {
    let Some(region) = composer_region(screen) else {
        return false;
    };
    match region.as_slice() {
        [row] => prompt_row_empty(row),
        _ => false,
    }
}

fn prompt_row_empty(row: &str) -> bool {
    static TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\x1b\[([0-9;]*)m|\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|([^\x1b])",
        )
        .expect("the capture token pattern is valid")
    });
    if !crate::output::strip_ansi(row).starts_with("❯ ") {
        return false;
    }
    let mut italic = false;
    let mut visible = Vec::new();
    for token in TOKEN.captures_iter(row) {
        if let Some(params) = token.get(1) {
            let mut codes = params.as_str().split(';');
            while let Some(code) = codes.next() {
                match code {
                    "" | "0" | "23" => italic = false,
                    "3" => italic = true,
                    // Extended colours carry their arguments in the following parameters.
                    "38" | "48" | "58" => match codes.next() {
                        Some("5") => drop(codes.next()),
                        Some("2") => {
                            for _ in 0..3 {
                                codes.next();
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        } else if let Some(character) = token.get(2) {
            visible.push((character.as_str(), italic));
        }
    }
    visible
        .iter()
        .skip(2)
        .all(|(character, italic)| *italic || character.trim().is_empty())
}

/// The effective Hermes home for the configuration base `base`: the directory of the file
/// `hermes config path` names, which follows a sticky `active_profile`.
pub(crate) fn effective_home(
    hermes: &dyn HermesConfig,
    until: Instant,
) -> Result<PathBuf, CafleetError> {
    hermes
        .config_path(until)
        .and_then(|config| home_of_config_path(&config))
        .map_err(|cause| {
            CafleetError::App(format!(
                "cannot read hermes configuration through 'hermes config path': {cause}"
            ))
        })
}

/// The Hermes home that owns the file `hermes config path` printed as `config_path`.
fn home_of_config_path(config_path: &str) -> Result<PathBuf, String> {
    Path::new(config_path)
        .parent()
        .filter(|home| home.is_absolute())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("printed no absolute file path: {config_path}"))
}

/// Stops and verifies a Hermes run's session work through its pane.
pub(crate) fn settle_run(
    conn: &rusqlite::Connection,
    mux: &dyn crate::multiplexer::Multiplexer,
    run: &crate::broker::runtime::RunRecord,
    pane_id: &str,
    deadline: crate::time::Deadline,
) -> Result<(), CafleetError> {
    let session = crate::broker::runtime::native_session(conn, run.run_id)?.ok_or_else(|| {
        CafleetError::App(format!(
            "hermes work for member {} could not be verified as stopped: member run {} has no Hermes session record; member was not deleted.",
            run.member_id, run.run_id
        ))
    })?;
    let mut host = PaneSettlementHost {
        conn,
        mux,
        pane_id,
        run_id: run.run_id,
        deadline,
    };
    if settle(run.member_id, &session, &mut host, deadline.instant())?
        == Settlement::ClientAlreadyExited
    {
        eprintln!("{}", already_exited_warning(run.member_id));
    }
    Ok(())
}

const CAPTURE_LINES: i64 = 400;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Settlement through the member's live pane while the deletion holds its pane lock.
struct PaneSettlementHost<'a> {
    conn: &'a rusqlite::Connection,
    mux: &'a dyn crate::multiplexer::Multiplexer,
    pane_id: &'a str,
    run_id: i64,
    deadline: crate::time::Deadline,
}

impl HermesSettlementHost for PaneSettlementHost<'_> {
    fn client_exited(&mut self) -> Result<bool, CafleetError> {
        Ok(crate::broker::runtime::get_run(self.conn, self.run_id)?
            .status
            .terminal())
    }
    fn send_keys(&mut self, keys: &[&str]) -> Result<(), CafleetError> {
        let keys = crate::multiplexer::PressKeys::parse(
            &keys.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>(),
        )?;
        Ok(self.mux.send_keys(self.pane_id, &keys, self.deadline)?)
    }
    fn capture(&mut self) -> Result<String, CafleetError> {
        Ok(self.mux.capture_pane(self.pane_id, CAPTURE_LINES)?)
    }
    fn record_session_id(&mut self, session_id: &str) -> Result<(), CafleetError> {
        let mut session = crate::broker::runtime::native_session(self.conn, self.run_id)?
            .ok_or_else(|| {
                CafleetError::App(format!(
                    "member run {} has no Hermes session record",
                    self.run_id
                ))
            })?;
        session.session_id = Some(session_id.to_owned());
        crate::broker::runtime::set_native_session(self.conn, self.run_id, &session)
    }
    fn processes_json(&mut self, home: &Path) -> Result<Option<String>, CafleetError> {
        let path = home.join("processes.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(cause) => Err(CafleetError::App(format!(
                "cannot read {}: {cause}",
                path.display()
            ))),
        }
    }
    fn registered_process_running(
        &mut self,
        pid: u32,
        host_start_time: u64,
    ) -> Result<bool, CafleetError> {
        let Some(identity) = crate::runtime::ProcessIdentity::read(i64::from(pid))? else {
            return Ok(false);
        };
        if cfg!(target_os = "macos") {
            return Ok(true);
        }
        let started = identity
            .start_identity
            .rsplit_once(':')
            .and_then(|(_, start)| start.parse::<u64>().ok())
            .expect("a Linux process identity ends with its stat start field");
        Ok(started == host_start_time)
    }
    fn pause(&mut self) {
        std::thread::sleep(POLL_INTERVAL);
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
mod tests {
    mod cli {
        use crate::coding_agent::hermes::*;
        use crate::coding_agent::test_support::executable_stub;
        use std::time::Duration;

        fn with_busy_retry<T>(mut run: impl FnMut() -> Result<T, String>) -> Result<T, String> {
            for _ in 0..50 {
                match run() {
                    Err(cause) if cause.contains("Text file busy") => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    result => return result,
                }
            }
            panic!("an executable stub stayed busy")
        }

        fn cli(body: &str) -> (tempfile::TempDir, HermesCli) {
            let dir = tempfile::tempdir().unwrap();
            let executable = executable_stub(dir.path(), "hermes", body);
            let base = dir.path().join("hermes home");
            (dir, HermesCli::new(executable, base))
        }

        fn deadline() -> Instant {
            Instant::now() + Duration::from_secs(5)
        }

        #[test]
        fn config_path_runs_under_the_resolved_base_and_trims_its_stdout() {
            let (dir, hermes) = cli(
                "[ \"$*\" = 'config path' ] || exit 9\nprintf '  %s/config.yaml  \\n' \"$HERMES_HOME\"",
            );
            assert_eq!(
                with_busy_retry(|| hermes.config_path(deadline())).unwrap(),
                format!("{}/config.yaml", dir.path().join("hermes home").display())
            );
        }
    }

    mod settlement {
        use crate::coding_agent::hermes::*;
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use std::time::Duration;

        mod composer_reliability {
            use crate::coding_agent::hermes::tests::settlement::*;

            pub(super) fn frame(region: &str) -> String {
                let border = "─".repeat(80);
                format!("{border}\n{region}\n{border}")
            }

            pub(super) fn frame_draft(draft: &str) -> String {
                if draft.is_empty() {
                    return frame("❯ \x1b[3mtype a message\x1b[23m");
                }
                let mut rows = Vec::new();
                for line in draft.split('\n') {
                    let characters = line.chars().collect::<Vec<_>>();
                    if characters.is_empty() {
                        rows.push(String::new());
                    } else {
                        rows.extend(
                            characters
                                .chunks(78)
                                .map(|chunk| chunk.iter().collect::<String>()),
                        );
                    }
                }
                rows[0].insert_str(0, "❯ ");
                frame(&rows.join("\n"))
            }

            #[derive(Debug)]
            enum Recorded {
                Keys(Vec<String>),
                Pause,
                Capture,
            }

            struct ClearingHost {
                inner: Host,
                trace: Vec<Recorded>,
                elapsed: Duration,
                send_cost: Duration,
                frozen: bool,
                changing_colours: bool,
                captures: usize,
                first_empty_at: Option<Duration>,
            }

            impl ClearingHost {
                fn new(draft: &str) -> Self {
                    let mut inner = Host::working();
                    inner.screen = vec![IDLE.into()];
                    inner.composer = draft.into();
                    Self {
                        inner,
                        trace: Vec::new(),
                        elapsed: Duration::ZERO,
                        send_cost: Duration::from_millis(50),
                        frozen: false,
                        changing_colours: false,
                        captures: 0,
                        first_empty_at: None,
                    }
                }

                fn clear_batches(&self) -> Vec<&[String]> {
                    self.trace
                        .iter()
                        .filter_map(|event| match event {
                            Recorded::Keys(keys) if keys.iter().any(|key| key == "Backspace") => {
                                Some(keys.as_slice())
                            }
                            _ => None,
                        })
                        .collect()
                }

                fn settle(&mut self) -> Result<Settlement, CafleetError> {
                    let until = self.inner.origin + Duration::from_secs(20);
                    settle(MEMBER, &session(None), self, until)
                }
            }

            impl HermesSettlementHost for ClearingHost {
                fn client_exited(&mut self) -> Result<bool, CafleetError> {
                    self.inner.client_exited()
                }
                fn send_keys(&mut self, keys: &[&str]) -> Result<(), CafleetError> {
                    self.trace.push(Recorded::Keys(
                        keys.iter().map(|key| (*key).to_owned()).collect(),
                    ));
                    self.elapsed += self.send_cost;
                    if self.frozen {
                        let keys = keys
                            .iter()
                            .copied()
                            .filter(|key| *key != "Backspace")
                            .collect::<Vec<_>>();
                        self.inner.send_keys(&keys)
                    } else {
                        self.inner.send_keys(keys)
                    }
                }
                fn capture(&mut self) -> Result<String, CafleetError> {
                    self.trace.push(Recorded::Capture);
                    self.captures += 1;
                    if self.inner.composer.is_empty() && self.first_empty_at.is_none() {
                        self.first_empty_at = Some(self.elapsed);
                    }
                    let region = frame_draft(&self.inner.composer);
                    let region = if self.changing_colours {
                        format!("\x1b[3{}m{region}\x1b[0m", self.captures % 8)
                    } else {
                        region
                    };
                    Ok(format!("{}\n{region}", self.inner.screen.join("\n")))
                }
                fn record_session_id(&mut self, id: &str) -> Result<(), CafleetError> {
                    self.inner.record_session_id(id)
                }
                fn processes_json(&mut self, home: &Path) -> Result<Option<String>, CafleetError> {
                    self.inner.processes_json(home)
                }
                fn registered_process_running(
                    &mut self,
                    pid: u32,
                    start: u64,
                ) -> Result<bool, CafleetError> {
                    self.inner.registered_process_running(pid, start)
                }
                fn pause(&mut self) {
                    self.trace.push(Recorded::Pause);
                    self.elapsed += Duration::from_millis(200);
                }
                fn now(&self) -> Instant {
                    self.inner.origin + self.elapsed
                }
            }

            #[test]
            fn composer_region_uses_sgr_to_distinguish_placeholders_from_typed_text() {
                for (row, empty) in [
                    ("❯ ", true),
                    ("\x1b[1m❯ \x1b[0m", true),
                    ("\x1b[1m❯ \x1b[0m\x1b[3;2mhint\x1b[0m", true),
                    ("❯ \x1b[3mhint\x1b[23m", true),
                    ("❯ \x1b[3mhint\x1b[23m   ", true),
                    ("❯ typed", false),
                    ("❯ \x1b[3mhint\x1b[23mx", false),
                    ("❯ \x1b[3mhint\x1b[0mx", false),
                    ("❯ \x1b[38;5;3mtyped\x1b[0m", false),
                    ("❯ \x1b[38;2;3;3;3mtyped\x1b[0m", false),
                ] {
                    assert_eq!(composer_empty(&frame(row)), empty, "{row:?}");
                }
            }

            #[test]
            fn composer_region_with_multiple_rows_including_blank_rows_is_nonempty() {
                for region in [
                    "❯ \n",
                    "❯ \n ",
                    "❯ wrapped\nliteral",
                    "❯ \x1b[3mhint\x1b[23m\n",
                ] {
                    assert!(!composer_empty(&frame(region)), "{region:?}");
                }
                assert!(!composer_empty("❯ \x1b[3mhint\x1b[23m"));
            }

            #[test]
            fn composer_region_selects_the_live_borders_instead_of_historical_prompt_rows() {
                let history = "❯ earlier submitted prompt\n❯ typed history";
                assert!(composer_empty(&format!(
                    "{history}\n{}",
                    frame("❯ \x1b[3mhint\x1b[23m")
                )));
                assert!(!composer_empty(&format!(
                    "{}\n{}",
                    frame("❯ "),
                    frame("❯ retained draft")
                )));
                let titled = format!(
                    "─ Composer {}\n❯ \x1b[3mhint\x1b[23m\n{}",
                    "─".repeat(69),
                    "─".repeat(80)
                );
                assert!(composer_empty(&titled));
            }

            #[test]
            fn measured_long_hermes_composers_remain_nonempty() {
                use crate::coding_agent::detector::test_support::bundled_screen;
                for id in [
                    "hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-tmux-120-15",
                    "hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-herdr-119-15",
                    "hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-tmux-120-56",
                    "hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-herdr-119-53",
                ] {
                    let document: Value = serde_json::from_str(&bundled_screen(id)).unwrap();
                    let capture = document["lines"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(!composer_empty(&capture), "{id}");
                }
            }

            #[test]
            fn hermes_clear_loop_uses_sixteen_key_batches_then_one_pause_per_round() {
                for (draft, rounds) in [
                    ("x".repeat(2057), 3),
                    ("x".repeat(8192), 9),
                    ("あ".repeat(8192), 9),
                    (format!("{}\n{}", "x".repeat(4095), "y".repeat(4096)), 9),
                ] {
                    let mut host = ClearingHost::new(&draft);
                    assert_eq!(host.settle().unwrap(), verified());
                    assert_eq!(host.inner.submitted, ["/stop", "/exit"]);
                    let batches = host.clear_batches();
                    assert_eq!(batches.len(), rounds * 16);
                    for round in batches.chunks(16) {
                        assert_eq!(round[0].len(), 64);
                        assert_eq!(round[0][0], "Escape");
                        assert!(round[0][1..].iter().all(|key| key == "Backspace"));
                        for batch in &round[1..] {
                            assert_eq!(batch.len(), 64);
                            assert!(batch.iter().all(|key| key == "Backspace"));
                        }
                    }
                    for (start, event) in host.trace.iter().enumerate() {
                        if let Recorded::Keys(keys) = event
                            && keys.first().is_some_and(|key| key == "Escape")
                            && keys.iter().any(|key| key == "Backspace")
                        {
                            assert!(
                                host.trace[start..start + 16]
                                    .iter()
                                    .all(|event| matches!(event, Recorded::Keys(_)))
                            );
                            assert!(matches!(host.trace[start + 16], Recorded::Pause));
                            assert!(matches!(host.trace[start + 17], Recorded::Capture));
                        }
                    }
                    assert!(
                        host.first_empty_at
                            .expect("a clear round observes the empty composer")
                            <= Duration::from_secs(10)
                    );
                }
            }

            #[test]
            fn hermes_clear_loop_compares_stripped_region_progress() {
                let mut host = ClearingHost::new("retained\nmultiline");
                host.frozen = true;
                host.changing_colours = true;
                assert_unverified(
                    host.settle(),
                    "the Hermes composer still holds text after clearing",
                );
                assert_eq!(host.clear_batches().len(), 16);
                assert!(host.inner.submitted.is_empty());
                assert_eq!(host.inner.composer, "retained\nmultiline");
            }

            #[test]
            fn hermes_clear_loop_stops_at_the_settlement_budget_before_typing_commands() {
                let mut host = ClearingHost::new(&"x".repeat(8192));
                let until = host.inner.origin + Duration::from_secs(1);
                let result = settle(MEMBER, &session(None), &mut host, until);
                assert!(result.is_err());
                assert!(host.now() >= until);
                assert!(host.inner.submitted.is_empty());
                assert!(!host.inner.composer.is_empty());
            }

            #[test]
            fn hermes_clear_loop_observes_an_empty_composer_without_backspace_batches() {
                let mut host = ClearingHost::new("");
                assert_eq!(host.settle().unwrap(), verified());
                assert!(host.clear_batches().is_empty());
                assert_eq!(host.inner.submitted, ["/stop", "/exit"]);
            }
        }

        const MEMBER: i64 = 7;
        const HOME: &str = "/srv/hermes/profiles/work";
        const SESSION: &str = "local-ab12cd34";
        const BUSY: &str = "☤ ❯ type to redirect this turn · Tab queues it for after";
        const IDLE: &str = "❯ Turn these notes into a to-do list";

        /// A Hermes client pane: keys edit its composer, and Enter submits the composer text unless
        /// it arrives before the completion menu rendered (`dropped_enters`).
        struct Host {
            screen: Vec<String>,
            composer: String,
            interrupt_stops: bool,
            reports_stop: bool,
            prints_summary: bool,
            exits_on_exit: bool,
            exited: bool,
            dropped_enters: usize,
            interrupts: usize,
            enters: usize,
            submitted: Vec<String>,
            recorded: Vec<String>,
            homes: Vec<PathBuf>,
            registry: Option<String>,
            live: Vec<(u32, u64)>,
            checked: Vec<(u32, u64)>,
            pauses: u32,
            origin: Instant,
        }

        impl Host {
            fn working() -> Self {
                Self {
                    screen: vec![
                        "│  Session: ab12cd34".into(),
                        "❯ Run the long task".into(),
                        BUSY.into(),
                    ],
                    composer: String::new(),
                    interrupt_stops: true,
                    reports_stop: true,
                    prints_summary: true,
                    exits_on_exit: true,
                    exited: false,
                    dropped_enters: 0,
                    interrupts: 0,
                    enters: 0,
                    submitted: Vec::new(),
                    recorded: Vec::new(),
                    homes: Vec::new(),
                    registry: None,
                    live: Vec::new(),
                    checked: Vec::new(),
                    pauses: 0,
                    origin: Instant::now(),
                }
            }

            fn until(&self) -> Instant {
                self.origin + Duration::from_secs(10)
            }

            fn submit(&mut self) {
                let command = std::mem::take(&mut self.composer);
                match command.as_str() {
                    "/stop" if self.reports_stop => self.screen.extend([
                        "Stopping 1 background process(es)...".into(),
                        "✅ Stopped 1 process(es).".into(),
                    ]),
                    "/exit" => {
                        if self.prints_summary {
                            self.screen.extend([
                                format!("  hermes --resume {SESSION}"),
                                format!("Session: {SESSION}"),
                            ]);
                        }
                        self.exited = self.exits_on_exit;
                    }
                    _ => {}
                }
                self.submitted.push(command);
            }
        }

        impl HermesSettlementHost for Host {
            fn client_exited(&mut self) -> Result<bool, CafleetError> {
                Ok(self.exited)
            }
            fn send_keys(&mut self, keys: &[&str]) -> Result<(), CafleetError> {
                assert!(!self.exited, "keys reach only a live client");
                for key in keys {
                    match *key {
                        "Ctrl+C" => {
                            self.interrupts += 1;
                            if self.interrupt_stops {
                                self.screen.retain(|line| line != BUSY);
                                self.screen.push(IDLE.into());
                            }
                        }
                        "Escape" => {}
                        "Backspace" => {
                            self.composer.pop();
                        }
                        "Enter" => {
                            self.enters += 1;
                            if self.dropped_enters > 0 {
                                self.dropped_enters -= 1;
                            } else {
                                self.submit();
                            }
                        }
                        typed => {
                            assert_eq!(typed.chars().count(), 1, "unexpected key {typed}");
                            self.composer.push_str(typed);
                        }
                    }
                }
                Ok(())
            }
            fn capture(&mut self) -> Result<String, CafleetError> {
                Ok(format!(
                    "{}\n{}",
                    self.screen.join("\n"),
                    composer_reliability::frame_draft(&self.composer)
                ))
            }
            fn record_session_id(&mut self, session_id: &str) -> Result<(), CafleetError> {
                self.recorded.push(session_id.to_owned());
                Ok(())
            }
            fn processes_json(&mut self, home: &Path) -> Result<Option<String>, CafleetError> {
                self.homes.push(home.to_path_buf());
                Ok(self.registry.clone())
            }
            fn registered_process_running(
                &mut self,
                pid: u32,
                host_start_time: u64,
            ) -> Result<bool, CafleetError> {
                self.checked.push((pid, host_start_time));
                Ok(self.live.contains(&(pid, host_start_time)))
            }
            fn pause(&mut self) {
                self.pauses += 1;
            }
            fn now(&self) -> Instant {
                self.origin + Duration::from_millis(100 * u64::from(self.pauses))
            }
        }

        fn session(session_id: Option<&str>) -> NativeSession {
            NativeSession {
                hermes_home: PathBuf::from(HOME),
                session_id: session_id.map(str::to_owned),
            }
        }

        fn registry(entries: &[(u32, u64, &str)]) -> String {
            serde_json::Value::Array(
                entries
                    .iter()
                    .map(|(pid, start, owner)| {
                        serde_json::json!({
                            "session_id": format!("proc_{pid}"),
                            "pid": pid,
                            "host_start_time": start,
                            "owner_task_id": owner,
                            "session_key": "tui",
                        })
                    })
                    .collect(),
            )
            .to_string()
        }

        fn verified() -> Settlement {
            Settlement::Verified
        }

        fn settled(host: &mut Host, recorded: Option<&str>) -> Result<Settlement, CafleetError> {
            let until = host.until();
            settle(MEMBER, &session(recorded), host, until)
        }

        fn assert_unverified(result: Result<Settlement, CafleetError>, cause: &str) {
            let failure = result.expect_err(cause);
            assert!(
                matches!(failure, CafleetError::App(_)),
                "{}",
                failure.message()
            );
            assert_eq!(failure.exit_code(), 1);
            assert_eq!(
                failure.message(),
                format!(
                    "hermes work for member {MEMBER} could not be verified as stopped: {cause}; member was not deleted."
                )
            );
        }

        #[test]
        fn settlement_interrupts_a_running_turn_stops_and_exits_then_verifies_its_own_processes() {
            let mut host = Host::working();
            host.registry = Some(registry(&[
                (4242, 900, SESSION),
                (5151, 901, "local-ffff0000"),
            ]));
            host.live = vec![(5151, 901)];
            assert_eq!(settled(&mut host, None).unwrap(), verified());
            assert_eq!(host.interrupts, 1);
            assert_eq!(host.submitted, ["/stop", "/exit"]);
            assert_eq!(host.recorded, [SESSION]);
            assert_eq!(host.homes, [PathBuf::from(HOME)]);
            assert_eq!(
                host.checked,
                [(4242, 900)],
                "only the session's own entries are checked"
            );
        }

        #[test]
        fn settlement_of_an_idle_client_sends_no_interrupt_and_an_absent_registry_verifies() {
            let mut host = Host::working();
            host.screen = vec!["❯ Run the long task".into(), IDLE.into()];
            assert_eq!(settled(&mut host, None).unwrap(), verified());
            assert_eq!(host.interrupts, 0);
            assert_eq!(host.submitted, ["/stop", "/exit"]);
            assert!(host.checked.is_empty());
        }

        #[test]
        fn settlement_clears_a_leftover_draft_before_typing_each_command() {
            let mut host = Host::working();
            host.screen = vec![IDLE.into()];
            host.composer = "a half-typed draft the operator left".into();
            assert_eq!(settled(&mut host, None).unwrap(), verified());
            assert_eq!(host.submitted, ["/stop", "/exit"]);
        }

        #[test]
        fn settlement_presses_enter_again_when_the_first_one_arrives_before_the_menu() {
            let mut host = Host::working();
            host.dropped_enters = 1;
            assert_eq!(settled(&mut host, None).unwrap(), verified());
            assert_eq!(host.submitted, ["/stop", "/exit"]);
            assert_eq!(host.enters, 3, "one dropped Enter, then one per command");
            let mut host = Host::working();
            host.dropped_enters = usize::MAX;
            assert_unverified(settled(&mut host, None), "Hermes did not report /stop");
            assert_eq!(
                host.enters, MAX_SUBMITS,
                "Enter is retried at most MAX_SUBMITS times"
            );
            assert!(host.submitted.is_empty());
            assert!(host.now() >= host.until());
        }

        #[test]
        fn settlement_of_a_client_that_already_exited_without_a_session_reports_it_and_checks_nothing()
         {
            let mut host = Host::working();
            host.exited = true;
            host.registry = Some(registry(&[(4242, 900, SESSION)]));
            host.live = vec![(4242, 900)];
            assert_eq!(
                settled(&mut host, None).unwrap(),
                Settlement::ClientAlreadyExited
            );
            assert_eq!((host.interrupts, host.enters), (0, 0));
            assert!(host.composer.is_empty());
            assert!(host.recorded.is_empty());
            assert!(host.homes.is_empty());
            assert!(host.checked.is_empty());
            assert_eq!(
                already_exited_warning(MEMBER),
                format!(
                    "warning: hermes client for member {MEMBER} had already exited; its session work may still be running in the Hermes gateway"
                )
            );
        }

        #[test]
        fn one_member_settlement_gets_twenty_seconds_from_its_start() {
            let before = Instant::now();
            let deadline = settlement_deadline().instant();
            let after = Instant::now();
            let budget = Duration::from_secs(20);
            assert!(
                deadline >= before + budget && deadline <= after + budget,
                "{:?} after the call",
                deadline.saturating_duration_since(after)
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn pane_host_counts_a_registered_pid_as_running_only_with_its_recorded_start_time() {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            let mux = crate::multiplexer::TmuxMultiplexer::for_server(
                std::rc::Rc::new(crate::runtime::system::SystemRunner),
                "/nonexistent/tmux-socket",
            );
            let mut host = PaneSettlementHost {
                conn: &conn,
                mux: &mux,
                pane_id: "%0",
                run_id: 1,
                deadline: settlement_deadline(),
            };
            let current = crate::runtime::ProcessIdentity::current().unwrap();
            let pid = u32::try_from(current.pid).unwrap();
            let start: u64 = current
                .start_identity
                .rsplit_once(':')
                .unwrap()
                .1
                .parse()
                .unwrap();
            assert!(host.registered_process_running(pid, start).unwrap());
            assert!(
                !host.registered_process_running(pid, start + 1).unwrap(),
                "a live pid with another start time is a reused pid"
            );
            let mut exited = std::process::Command::new("/bin/true").spawn().unwrap();
            exited.wait().unwrap();
            assert!(!host.registered_process_running(exited.id(), start).unwrap());
        }

        #[test]
        fn settlement_fails_while_a_registered_process_of_the_session_runs_and_a_retry_verifies_it()
        {
            let mut host = Host::working();
            host.registry = Some(registry(&[(4242, 900, SESSION), (4343, 905, SESSION)]));
            host.live = vec![(4242, 900), (4343, 905)];
            assert_unverified(
                settled(&mut host, None),
                &format!("background processes of session {SESSION} are still running: 4242, 4343"),
            );
            assert_eq!(
                host.recorded,
                [SESSION],
                "the verified exit records the session"
            );
            assert!(host.exited);
            let enters = host.enters;
            host.live.clear();
            assert_eq!(settled(&mut host, Some(SESSION)).unwrap(), verified());
            assert_eq!(
                host.enters, enters,
                "a retry sends no keys to the exited client"
            );
            assert_eq!(host.submitted, ["/stop", "/exit"]);
            assert_eq!(host.recorded, [SESSION]);
        }

        #[test]
        fn settlement_reports_each_unanswered_control_within_the_deadline() {
            for (change, cause) in [
                (
                    (|host: &mut Host| host.interrupt_stops = false) as fn(&mut Host),
                    "the Hermes turn did not stop after Ctrl-C",
                ),
                (
                    |host: &mut Host| host.reports_stop = false,
                    "Hermes did not report /stop",
                ),
                (
                    |host: &mut Host| {
                        host.prints_summary = false;
                        host.exits_on_exit = false;
                    },
                    "Hermes printed no exit summary after /exit",
                ),
                (
                    |host: &mut Host| host.exits_on_exit = false,
                    "the Hermes client did not exit after /exit",
                ),
            ] {
                let mut host = Host::working();
                change(&mut host);
                assert_unverified(settled(&mut host, None), cause);
                assert!(host.now() >= host.until(), "{cause}");
                assert!(
                    host.now() < host.until() + Duration::from_millis(200),
                    "{cause}: {} pauses past the deadline",
                    host.pauses
                );
                assert!(host.recorded.is_empty(), "{cause}");
            }
        }

        #[test]
        fn settlement_waits_for_a_new_stop_report_rather_than_one_already_on_screen() {
            let mut host = Host::working();
            host.screen = vec!["No running background processes.".into(), IDLE.into()];
            host.reports_stop = false;
            assert_unverified(settled(&mut host, None), "Hermes did not report /stop");
        }

        #[test]
        fn settlement_rejects_a_registry_that_is_not_a_hermes_process_list() {
            for text in ["{}", "[{\"pid\":1}]", "not json"] {
                let mut host = Host::working();
                host.registry = Some(text.into());
                let failure = settled(&mut host, None).expect_err(text);
                let prefix = format!(
                    "hermes work for member {MEMBER} could not be verified as stopped: /srv/hermes/profiles/work/processes.json is not a Hermes process registry: "
                );
                assert!(
                    failure.message().starts_with(&prefix),
                    "{}",
                    failure.message()
                );
                assert!(failure.message().ends_with("; member was not deleted."));
            }
        }

        #[test]
        fn native_session_records_the_home_and_adds_the_session_id_once_verified() {
            let created = session(None);
            assert_eq!(
                serde_json::to_value(&created).unwrap(),
                serde_json::json!({"hermes_home": HOME})
            );
            let verified = session(Some(SESSION));
            let stored = serde_json::json!({"hermes_home": HOME, "session_id": SESSION});
            assert_eq!(serde_json::to_value(&verified).unwrap(), stored);
            assert_eq!(
                serde_json::from_value::<NativeSession>(stored).unwrap(),
                verified
            );
            assert!(
                serde_json::from_value::<NativeSession>(
                    serde_json::json!({"hermes_home": HOME, "pid": 1})
                )
                .is_err()
            );
        }

        fn resolved_home(body: &str) -> Result<PathBuf, CafleetError> {
            let dir = tempfile::tempdir().unwrap();
            let path = executable_stub(dir.path(), "hermes", body);
            let hermes = crate::coding_agent::hermes::HermesCli::new(path, "/srv/hermes".into());
            run_stub(|| effective_home(&hermes, Instant::now() + Duration::from_secs(2)))
        }

        #[test]
        fn effective_home_is_the_directory_of_the_config_path_hermes_reports_for_the_base() {
            assert_eq!(
                resolved_home(
                    "[ \"$*\" = 'config path' ] && [ \"$HERMES_HOME\" = /srv/hermes ] || exit 3\nprintf '/srv/hermes/profiles/work/config.yaml\\n'"
                )
                .unwrap(),
                PathBuf::from(HOME)
            );
            assert_eq!(
                home_of_config_path("config.yaml").unwrap_err(),
                "printed no absolute file path: config.yaml"
            );
            let failure = resolved_home("echo 'no profile' >&2\nexit 4").unwrap_err();
            assert_eq!(failure.exit_code(), 1);
            assert!(
                failure
                    .message()
                    .starts_with("cannot read hermes configuration through 'hermes config path': "),
                "{}",
                failure.message()
            );
            assert!(
                failure.message().contains("no profile"),
                "{}",
                failure.message()
            );
        }
    }
}
