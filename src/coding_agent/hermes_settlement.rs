use crate::error::CafleetError;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

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
pub(crate) trait HermesSettlementHost {
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

pub(crate) const INTERRUPT_KEYS: [&str; 1] = ["Ctrl+C"];
pub(crate) const STOP_COMMAND: [&str; 5] = ["/", "s", "t", "o", "p"];
pub(crate) const EXIT_COMMAND: [&str; 5] = ["/", "e", "x", "i", "t"];
// Escape closes an open completion menu; the composer has no line-kill key, and Ctrl+C on an
// empty composer ends the client without stopping its session, so leftover text is erased by
// backspacing and the composer is verified empty before each command.
pub(crate) const CLEAR_KEYS: [&str; 61] = {
    let mut keys = ["Backspace"; 61];
    keys[0] = "Escape";
    keys
};
pub(crate) const MENU_CLOSE_KEYS: [&str; 1] = ["Escape"];
pub(crate) const SUBMIT_KEYS: [&str; 1] = ["Enter"];
// A typed slash command opens a completion menu once it has rendered, so the menu is closed
// before the first Enter and each Enter waits for its effect before the next one.
pub(crate) const MAX_SUBMITS: usize = 3;
pub(crate) const SUBMIT_WINDOW: std::time::Duration = std::time::Duration::from_millis(1000);

const BUSY_MARKERS: [&str; 2] = ["type to redirect this turn", "↑/↓ to select"];
const BUSY_TAIL_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settlement {
    /// The session's work stopped and none of its registered processes runs.
    Verified { session_id: String },
    /// The client had exited before settlement could stop its session.
    ClientAlreadyExited,
}

/// Stops the member's Hermes session work and verifies that no process it registered still runs.
pub(crate) fn settle(
    member_id: i64,
    session: &NativeSession,
    host: &mut dyn HermesSettlementHost,
    until: Instant,
) -> Result<Settlement, CafleetError> {
    settle_session(session, host, until).map_err(|cause| {
        CafleetError::App(format!(
            "hermes work for member {member_id} could not be verified as stopped: {cause}; member was not deleted."
        ))
    })
}

fn settle_session(
    session: &NativeSession,
    host: &mut dyn HermesSettlementHost,
    until: Instant,
) -> Result<Settlement, String> {
    let exited = host.client_exited().map_err(message)?;
    let session_id = match (&session.session_id, exited) {
        (Some(recorded), true) => recorded.clone(),
        (None, true) => return Ok(Settlement::ClientAlreadyExited),
        (_, false) => {
            let session_id = stop_client(host, until)?;
            host.record_session_id(&session_id).map_err(message)?;
            session_id
        }
    };
    verify_processes(host, &session.hermes_home, &session_id)?;
    Ok(Settlement::Verified { session_id })
}

pub(crate) fn already_exited_warning(member_id: i64) -> String {
    format!(
        "warning: hermes client for member {member_id} had already exited; its session work may still be running in the Hermes gateway"
    )
}

/// The budget of one Hermes member's settlement, which takes several native round trips.
pub(crate) fn settlement_deadline() -> crate::time::Deadline {
    crate::time::Deadline::until(Instant::now() + SETTLEMENT_BUDGET)
}

pub(crate) const SETTLEMENT_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

fn stop_client(host: &mut dyn HermesSettlementHost, until: Instant) -> Result<String, String> {
    if busy(&host.capture().map_err(message)?) {
        host.send_keys(&INTERRUPT_KEYS).map_err(message)?;
        wait_for(
            host,
            until,
            "the Hermes turn did not stop after Ctrl-C",
            |screen| (!busy(screen)).then_some(()),
        )?;
    }
    let reports = stop_reports(&host.capture().map_err(message)?);
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
    loop {
        if host.client_exited().map_err(message)? {
            return Ok(session_id);
        }
        if host.now() >= until {
            return Err("the Hermes client did not exit after /exit".into());
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
) -> Result<T, String> {
    host.send_keys(&CLEAR_KEYS).map_err(message)?;
    host.pause();
    if !host.composer_empty().map_err(message)? {
        return Err("the Hermes composer still holds text after clearing".into());
    }
    host.send_keys(command).map_err(message)?;
    host.pause();
    host.send_keys(&MENU_CLOSE_KEYS).map_err(message)?;
    for _ in 0..MAX_SUBMITS {
        host.pause();
        if host.client_exited().map_err(message)? {
            break;
        }
        host.send_keys(&SUBMIT_KEYS).map_err(message)?;
        let window = (host.now() + SUBMIT_WINDOW).min(until);
        loop {
            if let Some(value) = done(&host.capture().map_err(message)?) {
                return Ok(value);
            }
            if host.now() >= window {
                break;
            }
            host.pause();
        }
    }
    wait_for(host, until, timeout, done)
}

fn wait_for<T>(
    host: &mut dyn HermesSettlementHost,
    until: Instant,
    timeout: &str,
    mut done: impl FnMut(&str) -> Option<T>,
) -> Result<T, String> {
    loop {
        if let Some(value) = done(&host.capture().map_err(message)?) {
            return Ok(value);
        }
        if host.now() >= until {
            return Err(timeout.into());
        }
        host.pause();
    }
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
) -> Result<(), String> {
    let Some(text) = host.processes_json(home).map_err(message)? else {
        return Ok(());
    };
    let registry = home.join("processes.json");
    let processes = serde_json::from_str::<Vec<RegisteredProcess>>(&text).map_err(|cause| {
        format!(
            "{} is not a Hermes process registry: {cause}",
            registry.display()
        )
    })?;
    let mut running = Vec::new();
    for process in processes
        .iter()
        .filter(|process| process.owner_task_id == session_id)
    {
        if host
            .registered_process_running(process.pid, process.host_start_time)
            .map_err(message)?
        {
            running.push(process.pid.to_string());
        }
    }
    if running.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "background processes of session {session_id} are still running: {}",
            running.join(", ")
        ))
    }
}

fn message(cause: CafleetError) -> String {
    cause.message().into_owned()
}

/// Whether the bottom-most composer row (`❯ `) of an SGR-carrying capture holds no typed text.
/// Hermes draws its placeholder suggestion in italics and typed text without, so the row is
/// empty when every visible character after the prefix is italic or blank.
pub(crate) fn composer_row_empty(screen: &str) -> bool {
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
    executable: &Path,
    base: &Path,
    until: Instant,
) -> Result<PathBuf, CafleetError> {
    let config = crate::coding_agent::run_bounded(
        executable,
        &["config", "path"],
        &[("HERMES_HOME", base)],
        until,
    )
    .map_err(|cause| {
        CafleetError::App(format!(
            "cannot read hermes configuration through 'hermes config path': {cause}"
        ))
    })?;
    Path::new(config.trim())
        .parent()
        .filter(|home| home.is_absolute())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            CafleetError::App(format!(
                "cannot read hermes configuration through 'hermes config path': printed no absolute file path: {}",
                config.trim()
            ))
        })
}

/// Stops and verifies a Hermes run's session work through its pane.
pub(crate) fn settle_run(
    conn: &rusqlite::Connection,
    mux: &crate::multiplexer::AnyMultiplexer,
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
pub(crate) struct PaneSettlementHost<'a> {
    pub(crate) conn: &'a rusqlite::Connection,
    pub(crate) mux: &'a crate::multiplexer::AnyMultiplexer,
    pub(crate) pane_id: &'a str,
    pub(crate) run_id: i64,
    pub(crate) deadline: crate::time::Deadline,
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
    use crate::coding_agent::hermes_settlement::*;
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
        assert_eq!(
            settled(&mut host, None).unwrap_err().message(),
            format!(
                "hermes work for member {MEMBER} could not be verified as stopped: the Hermes composer still holds text after clearing; member was not deleted."
            )
        );
        assert_eq!(host.composer, draft[..draft.len() - (CLEAR_KEYS.len() - 1)]);
        assert!(host.submitted.is_empty());
        assert_eq!(host.enters, 0);
        assert!(host.recorded.is_empty());
    }

    #[test]
    fn composer_row_is_empty_only_when_its_bottom_row_shows_the_italic_placeholder_or_nothing() {
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

    #[test]
    fn settlement_counts_a_live_pid_with_another_start_time_as_stopped() {
        let mut host = Host::working();
        host.registry = Some(registry(&[(4242, 900, SESSION)]));
        host.live = vec![(4242, 901)];
        assert_eq!(settled(&mut host, None).unwrap(), verified());
        assert_eq!(host.checked, [(4242, 900)]);
    }

    #[test]
    fn settlement_fails_while_a_registered_process_of_the_session_runs_and_a_retry_verifies_it() {
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
        run_stub(|| {
            effective_home(
                &path,
                Path::new("/srv/hermes"),
                Instant::now() + Duration::from_secs(2),
            )
        })
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
            resolved_home("printf 'config.yaml\\n'")
                .unwrap_err()
                .message(),
            "cannot read hermes configuration through 'hermes config path': printed no absolute file path: config.yaml"
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
