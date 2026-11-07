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

pub(crate) fn read_plugin_state(
    hermes: &dyn HermesConfig,
    deadline: Instant,
) -> Result<PluginState, String> {
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
    fn capture(&mut self) -> Result<String, CafleetError>;
    /// Whether the composer holds no typed text; its placeholder suggestion does not count.
    fn composer_empty(&mut self) -> Result<bool, CafleetError>;
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
// Escape closes an open completion menu; the composer has no line-kill key, and Ctrl+C on an
// empty composer ends the client without stopping its session, so leftover text is erased by
// backspacing and the composer is verified empty before each command.
const CLEAR_KEYS: [&str; 61] = {
    let mut keys = ["Backspace"; 61];
    keys[0] = "Escape";
    keys
};
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
    Verified { session_id: String },
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
    Ok(Settlement::Verified { session_id })
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
    host.send_keys(&CLEAR_KEYS)?;
    host.pause();
    if !host.composer_empty()? {
        return Err(CafleetError::App(
            "the Hermes composer still holds text after clearing".into(),
        ));
    }
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
    REPORT.find_iter(screen).count()
}

fn exit_summary_session(screen: &str) -> Option<String> {
    static SUMMARY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^Session:\s+(local-[0-9a-f]+)\s*$")
            .expect("the exit summary pattern is valid")
    });
    SUMMARY
        .captures_iter(screen)
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

/// Whether the bottom-most composer row (`❯ `) of an SGR-carrying capture holds no typed text.
/// Hermes draws its placeholder suggestion in italics and typed text without, so the row is
/// empty when every visible character after the prefix is italic or blank.
fn composer_row_empty(screen: &str) -> bool {
    static TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\x1b\[([0-9;]*)m|\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|([^\x1b])",
        )
        .expect("the capture token pattern is valid")
    });
    let Some(row) = screen
        .lines()
        .rev()
        .find(|line| crate::output::strip_ansi(line).starts_with("❯ "))
    else {
        return false;
    };
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
        self.mux
            .send_keys(self.pane_id, &keys, self.deadline)
            .map_err(|cause| CafleetError::App(cause.to_string()))
    }
    fn capture(&mut self) -> Result<String, CafleetError> {
        self.mux
            .capture_pane(self.pane_id, CAPTURE_LINES)
            .map(|screen| crate::output::strip_ansi(&screen))
            .map_err(|cause| CafleetError::App(cause.to_string()))
    }
    fn composer_empty(&mut self) -> Result<bool, CafleetError> {
        self.mux
            .capture_pane(self.pane_id, CAPTURE_LINES)
            .map(|screen| composer_row_empty(&screen))
            .map_err(|cause| CafleetError::App(cause.to_string()))
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
    use crate::coding_agent::hermes::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Serves `config path`, `get` and `set` from an in-memory map. A set listed in `ignored`
    /// exits 0 without the value sticking, as under a managed scope.
    struct Fake {
        config_path: String,
        config: Mutex<BTreeMap<String, serde_json::Value>>,
        ignored: Vec<&'static str>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(plugins: Option<serde_json::Value>) -> Self {
            Self {
                config_path: "/home/tester/.hermes/config.yaml".into(),
                config: Mutex::new(
                    plugins
                        .map(|plugins| BTreeMap::from([(PLUGINS_KEY.to_owned(), plugins)]))
                        .unwrap_or_default(),
                ),
                ignored: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn plugins(&self) -> serde_json::Value {
            self.config.lock().unwrap()[PLUGINS_KEY].clone()
        }
    }

    impl HermesConfig for Fake {
        fn config_path(&self, _: Instant) -> Result<String, String> {
            self.calls.lock().unwrap().push("config path".into());
            Ok(self.config_path.clone())
        }

        fn get(&self, key: &str, _: Instant) -> Result<Option<serde_json::Value>, String> {
            self.calls.lock().unwrap().push(format!("get {key}"));
            assert_eq!(
                key, PLUGINS_KEY,
                "setup and doctor read only the plugins subtree"
            );
            Ok(self.config.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str, _: Instant) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("set {key} {value}"));
            if self.ignored.contains(&key) {
                return Ok(());
            }
            let value: serde_json::Value = serde_json::from_str(value).unwrap();
            let mut config = self.config.lock().unwrap();
            let plugins = config
                .entry(PLUGINS_KEY.to_owned())
                .or_insert_with(|| json!({}));
            let pointer = format!(
                "/{}",
                key.strip_prefix("plugins.").unwrap().replace('.', "/")
            );
            let mut target = plugins;
            for part in pointer.trim_start_matches('/').split('/') {
                target = target
                    .as_object_mut()
                    .unwrap()
                    .entry(part)
                    .or_insert_with(|| json!({}));
            }
            *target = value;
            Ok(())
        }
    }

    fn state(
        disabled_lists_cafleet: bool,
        enabled: &[&str],
        injection_allowed: bool,
    ) -> PluginState {
        PluginState {
            disabled_lists_cafleet,
            enabled: enabled.iter().map(|name| (*name).to_owned()).collect(),
            injection_allowed,
        }
    }

    fn budget() -> Duration {
        Duration::from_secs(10)
    }

    #[test]
    fn a_not_set_plugins_key_and_each_absent_member_are_not_set() {
        assert_eq!(plugin_state(None).unwrap(), state(false, &[], false));
        assert_eq!(
            plugin_state(Some(json!({}))).unwrap(),
            state(false, &[], false)
        );
        assert_eq!(
            plugin_state(Some(json!({"enabled": ["alpha"]}))).unwrap(),
            state(false, &["alpha"], false)
        );
    }

    #[test]
    fn the_plugins_subtree_yields_disabled_enabled_and_injection_states() {
        assert_eq!(
            plugin_state(Some(json!({
                "disabled": ["other", "cafleet"],
                "enabled": ["alpha", "cafleet"],
                "entries": {"cafleet": {"allow_gateway_injection": true}},
                "unrelated": 1
            })))
            .unwrap(),
            state(true, &["alpha", "cafleet"], true)
        );
        for injection in [json!(false), json!("true"), json!(1), json!(null)] {
            assert!(
                !plugin_state(Some(
                    json!({"entries": {"cafleet": {"allow_gateway_injection": injection}}})
                ))
                .unwrap()
                .injection_allowed,
                "{injection}"
            );
        }
    }

    #[test]
    fn plugin_lists_must_be_arrays_of_strings_and_plugins_an_object() {
        for (plugins, prefix) in [
            (
                json!({"disabled": "cafleet"}),
                "plugins.disabled is not a JSON array of strings: ",
            ),
            (
                json!({"enabled": ["cafleet", null]}),
                "plugins.enabled is not a JSON array of strings: ",
            ),
        ] {
            let failure = plugin_state(Some(plugins.clone())).unwrap_err();
            assert!(failure.starts_with(prefix), "{plugins}: {failure}");
        }
        assert_eq!(
            plugin_state(Some(json!(["cafleet"]))).unwrap_err(),
            "plugins is not a JSON object"
        );
    }

    #[test]
    fn missing_names_the_enablement_keys_in_order() {
        assert_eq!(
            state(false, &["alpha"], false).missing(),
            [PLUGINS_ENABLED_KEY, GATEWAY_INJECTION_KEY]
        );
        assert_eq!(
            state(false, &["cafleet"], false).missing(),
            [GATEWAY_INJECTION_KEY]
        );
        assert_eq!(state(false, &[], true).missing(), [PLUGINS_ENABLED_KEY]);
        assert!(state(true, &["cafleet"], true).missing().is_empty());
    }

    #[test]
    fn read_batch_runs_config_path_and_the_plugins_read_concurrently_under_one_deadline() {
        let (sender, receiver) = mpsc::channel();
        let (started, first) = mpsc::channel::<()>();
        /// The first read waits for the second to start, so a serial batch fails.
        struct Batch {
            first_started: Mutex<Option<mpsc::Sender<()>>>,
            first: Mutex<mpsc::Receiver<()>>,
            second_started: Mutex<mpsc::Sender<()>>,
            second: Mutex<mpsc::Receiver<()>>,
            deadlines: Mutex<Vec<Instant>>,
        }
        impl Batch {
            fn meet(&self, deadline: Instant) {
                self.deadlines.lock().unwrap().push(deadline);
                let first = self.first_started.lock().unwrap().take();
                match first {
                    Some(signal) => {
                        signal.send(()).unwrap();
                        self.second
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .expect("the other read starts while this one runs");
                    }
                    None => {
                        self.first
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .expect("the first read started");
                        self.second_started.lock().unwrap().send(()).unwrap();
                    }
                }
            }
        }
        impl HermesConfig for Batch {
            fn config_path(&self, deadline: Instant) -> Result<String, String> {
                self.meet(deadline);
                Ok("/home/tester/.hermes/config.yaml".into())
            }
            fn get(
                &self,
                key: &str,
                deadline: Instant,
            ) -> Result<Option<serde_json::Value>, String> {
                assert_eq!(key, PLUGINS_KEY);
                self.meet(deadline);
                Ok(Some(json!({"enabled": ["cafleet"]})))
            }
            fn set(&self, _: &str, _: &str, _: Instant) -> Result<(), String> {
                panic!("the read batch writes nothing")
            }
        }
        let batch = Batch {
            first_started: Mutex::new(Some(started)),
            first: Mutex::new(first),
            second_started: Mutex::new(sender),
            second: Mutex::new(receiver),
            deadlines: Mutex::new(Vec::new()),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let (path, plugins) = read_batch(&batch, deadline);
        assert_eq!(path.unwrap(), "/home/tester/.hermes/config.yaml");
        assert_eq!(plugins.unwrap(), Some(json!({"enabled": ["cafleet"]})));
        assert_eq!(*batch.deadlines.lock().unwrap(), [deadline, deadline]);
    }

    #[test]
    fn an_enabled_plugin_is_left_unwritten_and_unread() {
        let fake = Fake::new(None);
        assert!(
            !enable_plugin(&fake, &state(false, &["alpha", "cafleet"], true), budget()).unwrap()
        );
        assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    }

    #[test]
    fn a_first_install_appends_cafleet_sets_injection_serially_then_re_reads_plugins_once() {
        let fake = Fake::new(Some(json!({"enabled": ["alpha", "beta"]})));
        let before = plugin_state(Some(fake.plugins())).unwrap();
        assert!(enable_plugin(&fake, &before, budget()).unwrap());
        assert_eq!(
            fake.calls(),
            [
                format!("set {PLUGINS_ENABLED_KEY} [\"alpha\",\"beta\",\"cafleet\"]"),
                format!("set {GATEWAY_INJECTION_KEY} true"),
                format!("get {PLUGINS_KEY}"),
            ]
        );
        assert_eq!(
            plugin_state(Some(fake.plugins())).unwrap(),
            state(false, &["alpha", "beta", "cafleet"], true)
        );
    }

    #[test]
    fn only_the_missing_key_is_written() {
        let fake = Fake::new(Some(json!({"enabled": ["cafleet"]})));
        assert!(enable_plugin(&fake, &state(false, &["cafleet"], false), budget()).unwrap());
        assert_eq!(
            fake.calls(),
            [
                format!("set {GATEWAY_INJECTION_KEY} true"),
                format!("get {PLUGINS_KEY}")
            ]
        );
    }

    #[test]
    fn a_disabled_entry_for_cafleet_is_refused_before_any_call() {
        let fake = Fake::new(None);
        assert_eq!(
            enable_plugin(&fake, &state(true, &[], false), budget()).unwrap_err(),
            "plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'"
        );
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn a_write_that_does_not_stick_fails_the_re_read() {
        for (ignored, expected) in [
            (
                PLUGINS_ENABLED_KEY,
                "plugins.enabled still lacks cafleet after the write",
            ),
            (
                GATEWAY_INJECTION_KEY,
                "plugins.entries.cafleet.allow_gateway_injection is still not true after the write",
            ),
        ] {
            let mut fake = Fake::new(None);
            fake.ignored.push(ignored);
            assert_eq!(
                enable_plugin(&fake, &state(false, &[], false), budget()).unwrap_err(),
                expected
            );
            assert_eq!(fake.calls().last().unwrap(), &format!("get {PLUGINS_KEY}"));
        }
    }

    #[test]
    fn plugin_dir_names_the_cafleet_plugin_under_the_home_owning_the_config_path() {
        assert_eq!(
            plugin_dir("/srv/hermes/profiles/work/config.yaml").unwrap(),
            std::path::PathBuf::from("/srv/hermes/profiles/work/plugins/cafleet")
        );
        assert!(plugin_dir("config.yaml").is_err());
    }

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

        #[test]
        fn get_reads_json_and_treats_only_the_exact_not_set_line_under_exit_one_as_absent() {
            let (_dir, hermes) = cli(
                "case \"$*\" in\n  'config get --json plugins') printf '%s\\n' '{\"enabled\":[\"cafleet\"]}' ;;\n  'config get --json absent') printf 'noise\\n  Config key not set: absent  \\n' >&2; exit 1 ;;\n  'config get --json other') printf 'Config key not set: absent\\n' >&2; exit 1 ;;\n  'config get --json exit2') printf 'Config key not set: exit2\\n' >&2; exit 2 ;;\n  'config get --json near') printf 'Config key not set: near.extra\\n' >&2; exit 1 ;;\n  'config get --json prefixed') printf 'Error: Config key not set: prefixed\\n' >&2; exit 1 ;;\n  'config get --json bare') printf 'noise\\n' >&2; exit 1 ;;\n  'config get --json text') printf 'not json\\n' ;;\n  *) exit 9 ;;\nesac",
            );
            for (key, cause) in [
                ("near", "Config key not set: near.extra"),
                ("prefixed", "Error: Config key not set: prefixed"),
                ("bare", "noise"),
            ] {
                assert_eq!(
                    with_busy_retry(|| hermes.get(key, deadline())).unwrap_err(),
                    cause,
                    "{key}"
                );
            }
            assert_eq!(
                with_busy_retry(|| hermes.get("plugins", deadline())).unwrap(),
                Some(serde_json::json!({"enabled": ["cafleet"]}))
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("absent", deadline())).unwrap(),
                None
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("other", deadline())).unwrap_err(),
                "Config key not set: absent"
            );
            assert_eq!(
                with_busy_retry(|| hermes.get("exit2", deadline())).unwrap_err(),
                "Config key not set: exit2"
            );
            assert!(
                with_busy_retry(|| hermes.get("text", deadline()))
                    .unwrap_err()
                    .starts_with("text printed no JSON value: ")
            );
        }

        #[test]
        fn set_passes_the_key_and_value_and_reports_a_failure_with_its_stderr() {
            let (dir, hermes) = cli(
                "[ \"$1 $2\" = 'config set' ] || exit 9\ncase \"$3\" in\n  locked) printf 'config.yaml is locked\\n' >&2; exit 2 ;;\n  silent) exit 4 ;;\nesac\nprintf '%s=%s' \"$3\" \"$4\" > \"$(dirname \"$0\")/set.log\"",
            );
            with_busy_retry(|| hermes.set("plugins.enabled", "[\"cafleet\"]", deadline())).unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.path().join("set.log")).unwrap(),
                "plugins.enabled=[\"cafleet\"]"
            );
            assert_eq!(
                with_busy_retry(|| hermes.set("locked", "true", deadline())).unwrap_err(),
                "config.yaml is locked"
            );
            assert_eq!(
                with_busy_retry(|| hermes.set("silent", "true", deadline())).unwrap_err(),
                "exited with code 4"
            );
        }
    }

    mod settlement {
        use crate::coding_agent::hermes::*;
        use crate::coding_agent::test_support::executable_stub;
        use crate::coding_agent::test_support::run_stub;
        use std::time::Duration;

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
                Ok(self.screen.join("\n"))
            }
            fn composer_empty(&mut self) -> Result<bool, CafleetError> {
                Ok(self.composer.is_empty())
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
            Settlement::Verified {
                session_id: SESSION.into(),
            }
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
        fn settlement_fails_without_typing_a_command_when_clearing_leaves_text_in_the_composer() {
            let mut host = Host::working();
            host.screen = vec![IDLE.into()];
            let draft = "x".repeat(CLEAR_KEYS.len() + 9);
            host.composer = draft.clone();
            assert_unverified(
                settled(&mut host, None),
                "the Hermes composer still holds text after clearing",
            );
            assert_eq!(host.composer, draft[..draft.len() - (CLEAR_KEYS.len() - 1)]);
            assert!(host.submitted.is_empty());
            assert_eq!(host.enters, 0);
            assert!(host.recorded.is_empty());
        }

        #[test]
        fn composer_row_is_empty_only_when_its_bottom_row_shows_the_italic_placeholder_or_nothing()
        {
            for (screen, empty) in [
                ("❯ ", true),
                ("\x1b[1m❯ \x1b[0m", true),
                (
                    "\x1b[1m❯ \x1b[0m\x1b[3;2mTurn these notes into a to-do list\x1b[0m",
                    true,
                ),
                ("❯ \x1b[3mhint\x1b[23m   ", true),
                ("❯ typed draft", false),
                ("❯ \x1b[3mhint\x1b[23mx", false),
                ("❯ \x1b[3mhint\x1b[0mx", false),
                ("❯ \x1b[38;5;3mcolored draft\x1b[0m", false),
                ("❯ \x1b[38;2;3;3;3mcolored draft\x1b[0m", false),
                ("❯ earlier prompt\n❯ \x1b[3mhint\x1b[0m", true),
                ("❯ \x1b[3mhint\x1b[0m\n❯ typed", false),
                ("╭──╮\n│ no composer │", false),
                ("", false),
            ] {
                assert_eq!(composer_row_empty(screen), empty, "{screen:?}");
            }
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
