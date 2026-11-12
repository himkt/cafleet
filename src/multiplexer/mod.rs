use crate::error::CafleetError;
pub(crate) use crate::multiplexer::herdr::HerdrMultiplexer;
pub(crate) use crate::multiplexer::tmux::TmuxMultiplexer;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

pub(crate) mod caller;
pub(crate) mod herdr;
pub(crate) mod tmux;

/// The concrete session, window and pane returned by context discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiplexerContext {
    pub(crate) session: String,
    pub(crate) window_id: String,
    pub(crate) pane_id: String,
}

/// The outcome of backend-owned pane compensation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaneCleanup {
    Attempted {
        pane_id: String,
        error: Option<String>,
    },
    Unknown,
}

/// A backend failure with optional pane ownership information.
#[derive(Debug)]
pub(crate) struct MultiplexerError {
    message: String,
    pane_cleanup: Option<PaneCleanup>,
    submission_started: bool,
}

impl MultiplexerError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            pane_cleanup: None,
            submission_started: false,
        }
    }

    pub(crate) fn submission_started(&self) -> bool {
        self.submission_started
    }

    pub(crate) fn after_write(mut self) -> Self {
        self.submission_started = true;
        self
    }

    pub(crate) fn with_pane_cleanup(mut self, cleanup: PaneCleanup) -> Self {
        self.pane_cleanup = Some(cleanup);
        self
    }
}

#[cfg(test)]
impl MultiplexerError {
    pub(crate) fn pane_cleanup(&self) -> Option<&PaneCleanup> {
        self.pane_cleanup.as_ref()
    }
}

impl fmt::Display for MultiplexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        match &self.pane_cleanup {
            Some(PaneCleanup::Attempted {
                pane_id,
                error: Some(error),
            }) => write!(f, "\ncleanup failed for pane {pane_id}: {error}"),
            Some(PaneCleanup::Unknown) => {
                f.write_str("\npane ID unknown; pane cleanup unconfirmed")
            }
            _ => Ok(()),
        }
    }
}

/// Owns a known pane until transfer or an explicit cleanup attempt. Drop is
/// only a last resort for unwinding before either operation.
pub(crate) struct PaneOwnership<F: Fn(&str) -> Result<(), MultiplexerError>> {
    pane_id: Option<String>,
    kill: F,
}

impl<F: Fn(&str) -> Result<(), MultiplexerError>> PaneOwnership<F> {
    pub(crate) fn new(pane_id: String, kill: F) -> Self {
        Self {
            pane_id: Some(pane_id),
            kill,
        }
    }

    pub(crate) fn rollback(&mut self) -> PaneCleanup {
        let pane_id = self.pane_id.take().expect("pane is still owned");
        let error = (self.kill)(&pane_id).err().map(|error| error.to_string());
        PaneCleanup::Attempted { pane_id, error }
    }

    pub(crate) fn finish(mut self) -> String {
        self.pane_id.take().expect("pane is still owned")
    }
}

impl<F: Fn(&str) -> Result<(), MultiplexerError>> Drop for PaneOwnership<F> {
    fn drop(&mut self) {
        if let Some(pane_id) = self.pane_id.take() {
            let _ = (self.kill)(&pane_id);
        }
    }
}

impl std::error::Error for MultiplexerError {}

impl From<MultiplexerError> for CafleetError {
    fn from(error: MultiplexerError) -> Self {
        CafleetError::App(error.to_string())
    }
}

/// A subprocess failure below the backend's message mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunError {
    BinaryNotFound(String),
    NotStarted(String),
    Timeout,
    Failed { stderr: String },
}

impl RunError {
    pub(crate) fn deadline_expired() -> Self {
        Self::NotStarted("command deadline expired before launch".into())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TimedRunError {
    pub(crate) source: RunError,
    pub(crate) cleanup_diagnostics: Vec<String>,
}

impl TimedRunError {
    pub(crate) fn before_launch() -> Self {
        Self {
            source: RunError::deadline_expired(),
            cleanup_diagnostics: Vec::new(),
        }
    }
}

pub(crate) trait CommandRunner {
    fn binary_exists(&self, name: &str) -> bool;
    fn run(&self, argv: &[String]) -> Result<String, RunError>;
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError>;
    fn run_until(&self, argv: &[String], deadline: Instant) -> Result<String, TimedRunError>;
}

const PLACEMENT_MISMATCH: &str = "press target context does not match the recorded placement";

fn map_run_error(name: &str, argv: &[String], error: RunError) -> MultiplexerError {
    let joined = argv.join(" ");
    match error {
        RunError::NotStarted(detail) => MultiplexerError::new(format!(
            "{name} command was not started: {detail}: {joined}"
        )),
        RunError::BinaryNotFound(detail) => {
            MultiplexerError::new(format!("{name} binary not found: {detail}"))
        }
        RunError::Timeout => MultiplexerError::new(format!(
            "{name} command timed out at the pane input deadline: {joined}"
        )),
        RunError::Failed { stderr } => MultiplexerError::new(format!(
            "{name} command failed: {joined}\nstderr: {}",
            stderr.trim()
        )),
    }
}

fn check_deadline(deadline: crate::time::Deadline) -> Result<(), MultiplexerError> {
    deadline
        .remaining()
        .map(|_| ())
        .map_err(|error| MultiplexerError::new(error.message()))
}

fn check_capture_lines(lines: i64) -> Result<(), MultiplexerError> {
    if lines <= 0 {
        return Err(MultiplexerError::new(format!(
            "capture_pane: lines must be positive, got {lines}"
        )));
    }
    Ok(())
}

pub(crate) fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

/// The runner, server selector and error mapping shared by every backend; a
/// backend adds only its own argv construction and output parsing.
pub(crate) struct BackendCore {
    runner: Rc<dyn CommandRunner>,
    name: &'static str,
    pane_gone: fn(&str) -> bool,
    server_flag: &'static str,
    server: String,
}

impl BackendCore {
    fn bounded(&self, deadline: crate::time::Deadline) -> Self {
        Self {
            runner: DeadlineRunner::wrap(self.runner.clone(), deadline),
            name: self.name,
            pane_gone: self.pane_gone,
            server_flag: self.server_flag,
            server: self.server.clone(),
        }
    }

    /// `parts` with the server selector inserted after the binary.
    fn argv(&self, parts: &[&str]) -> Vec<String> {
        let (binary, rest) = parts.split_first().expect("an argv names its binary");
        let mut command = vec![
            binary.to_string(),
            self.server_flag.to_owned(),
            self.server.clone(),
        ];
        command.extend(argv(rest));
        command
    }

    fn map_error(&self, argv: &[String], error: RunError) -> MultiplexerError {
        map_run_error(self.name, argv, error)
    }

    fn run(&self, argv: &[String]) -> Result<String, MultiplexerError> {
        self.runner
            .run(argv)
            .map_err(|error| self.map_error(argv, error))
    }

    fn run_tolerating_missing(&self, argv: &[String]) -> Result<(), MultiplexerError> {
        match self.runner.run(argv) {
            Ok(_) => Ok(()),
            Err(RunError::Failed { ref stderr }) if (self.pane_gone)(stderr) => Ok(()),
            Err(error) => Err(self.map_error(argv, error)),
        }
    }

    fn ensure_available(&self) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists(self.name) {
            return Err(MultiplexerError::new(format!(
                "{} binary not found on PATH",
                self.name
            )));
        }
        Ok(())
    }

    fn press_run(
        &self,
        argv: &[String],
        deadline: crate::time::Deadline,
        writing: bool,
    ) -> Result<String, MultiplexerError> {
        check_deadline(deadline)?;
        self.runner
            .run_until(argv, deadline.instant())
            .map_err(|error| {
                let started =
                    writing && matches!(&error.source, RunError::Failed { .. } | RunError::Timeout);
                let mut mapped = self.map_error(argv, error.source);
                for diagnostic in error.cleanup_diagnostics {
                    mapped.message.push('\n');
                    mapped.message.push_str(&diagnostic);
                }
                if started {
                    mapped.after_write()
                } else {
                    mapped
                }
            })
    }

    fn send_keys(
        &self,
        argv: &[String],
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError> {
        let output = self.press_run(argv, deadline, true)?;
        if !output.trim().is_empty() {
            return Err(MultiplexerError::new(format!(
                "{} key acknowledgment was unreadable",
                self.name
            ))
            .after_write());
        }
        Ok(())
    }
}

pub(crate) struct DeadlineRunner {
    runner: Rc<dyn CommandRunner>,
    deadline: crate::time::Deadline,
}

impl DeadlineRunner {
    pub(crate) fn wrap(
        runner: Rc<dyn CommandRunner>,
        deadline: crate::time::Deadline,
    ) -> Rc<dyn CommandRunner> {
        Rc::new(Self { runner, deadline })
    }

    fn remaining(&self) -> Result<Duration, RunError> {
        self.deadline
            .remaining()
            .map_err(|_| RunError::deadline_expired())
    }
}

impl CommandRunner for DeadlineRunner {
    fn binary_exists(&self, name: &str) -> bool {
        self.runner.binary_exists(name)
    }

    fn run(&self, argv: &[String]) -> Result<String, RunError> {
        let remaining = self.remaining()?;
        self.run_for(argv, remaining)
    }

    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        let remaining = self.remaining()?;
        self.runner.run_for(argv, timeout.min(remaining))
    }

    fn run_until(&self, argv: &[String], deadline: Instant) -> Result<String, TimedRunError> {
        let deadline = deadline.min(self.deadline.instant());
        if Instant::now() >= deadline {
            return Err(TimedRunError::before_launch());
        }
        self.runner.run_until(argv, deadline)
    }
}

/// The capture over-fetch margin: both backends fetch `lines + 1000` so a blank
/// tail deeper than the requested window still leaves drawn lines to keep.
pub(crate) const CAPTURE_OVER_FETCH_LINES: i64 = 1000;

/// Split the fetched
/// buffer on `\n` only (so `\r` survives for the CLI-side defrag), drop the
/// trailing run of visually-blank lines (whitespace-only after per-line CSI
/// stripping — the emptiness check only; kept lines keep their bytes), and
/// keep the last `lines` remaining lines — a small window shows the pane's
/// drawn bottom, not the blank area under the cursor.
pub(crate) fn capture_window(raw: &str, lines: i64) -> String {
    let mut parts: Vec<&str> = raw.split('\n').collect();
    while parts
        .last()
        .is_some_and(|line| crate::output::is_visually_blank(line))
    {
        parts.pop();
    }
    let keep = usize::try_from(lines).expect("lines is positive");
    let start = parts.len().saturating_sub(keep);
    parts[start..].join("\n")
}

/// A multiplexer backend. The provided pane close and layout restoration are tmux's; herdr
/// overrides them to rebalance the closed pane's tab after deregistration.
pub(crate) trait Multiplexer {
    fn core(&self) -> &BackendCore;

    fn bounded(&self, deadline: crate::time::Deadline) -> Box<dyn Multiplexer>;

    /// Confirms that the recorded `pane` still sits at `placement`'s session and window.
    fn check_key_target(
        &self,
        pane: &str,
        placement: &crate::broker::Placement,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError>;

    fn send_keys(
        &self,
        pane_id: &str,
        keys: &PressKeys,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError>;

    /// The session, window and pane of `pane` on the targeted server, or `None` when the server
    /// has no such pane.
    fn pane_context(&self, pane: &str) -> Result<Option<MultiplexerContext>, MultiplexerError>;

    /// The pid of `pane`'s process on the targeted server, or `None` when the server has no such
    /// pane.
    fn pane_process(&self, pane: &str) -> Result<Option<i64>, MultiplexerError>;

    fn split_window(
        &self,
        reference: &MultiplexerContext,
        command: &[String],
    ) -> Result<String, MultiplexerError>;

    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError>;

    fn kill_pane(&self, target_pane_id: &str) -> Result<(), MultiplexerError>;

    fn name(&self) -> &'static str {
        self.core().name
    }

    fn ensure_available(&self) -> Result<(), MultiplexerError> {
        self.core().ensure_available()
    }

    fn validate_key_target(
        &self,
        placement: &crate::broker::Placement,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError> {
        check_deadline(deadline)?;
        let pane = placement
            .mux_pane_id
            .as_deref()
            .ok_or_else(|| MultiplexerError::new("press target has no pane"))?;
        self.check_key_target(pane, placement, deadline)
    }

    fn close_pane(&self, target_pane_id: &str) -> Result<(), MultiplexerError> {
        self.kill_pane(target_pane_id)
    }

    /// Restores the layout after the most recent `close_pane`.
    fn restore_after_close(&self) -> Result<(), MultiplexerError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MuxBackend {
    Tmux,
    Herdr,
}

impl MuxBackend {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
        }
    }

    pub(crate) fn parse(name: &str) -> Result<Self, MultiplexerError> {
        match name {
            "tmux" => Ok(Self::Tmux),
            "herdr" => Ok(Self::Herdr),
            other => Err(MultiplexerError::new(format!(
                "unsupported recorded multiplexer backend '{other}'"
            ))),
        }
    }

    /// The multiplexer that targets `server` on this backend explicitly.
    pub(crate) fn for_server(
        self,
        server: &str,
        runner: Rc<dyn CommandRunner>,
    ) -> Box<dyn Multiplexer> {
        match self {
            Self::Tmux => Box::new(TmuxMultiplexer::for_server(runner, server)),
            Self::Herdr => Box::new(HerdrMultiplexer::for_server(runner, server)),
        }
    }

    /// Auto-detects the backend from `HERDR_ENV` / `TMUX` (an empty value counts as unset);
    /// ambiguity and absence are hard errors.
    pub(crate) fn detect(env: impl Fn(&str) -> Option<String>) -> Result<Self, MultiplexerError> {
        let present = |name: &str| env(name).is_some_and(|value| !value.is_empty());
        match (present("HERDR_ENV"), present("TMUX")) {
            (true, true) => Err(MultiplexerError::new(
                "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
                 unset the one that does not belong to this session",
            )),
            (true, false) => Ok(Self::Herdr),
            (false, true) => Ok(Self::Tmux),
            (false, false) => Err(MultiplexerError::new(
                "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
                 run cafleet inside a tmux or herdr session",
            )),
        }
    }
}

/// The set caller-environment variables multiplexer detection and `fleet create` read; a
/// non-UTF-8 one fails with its name.
pub(crate) fn caller_env()
-> Result<std::collections::HashMap<String, String>, crate::error::CafleetError> {
    let names = "TMUX TMUX_PANE HERDR_ENV HERDR_SESSION HERDR_SOCKET_PATH HERDR_PANE_ID";
    let set = |name: &str| Ok(crate::config::process_env(name)?.map(|value| (name.into(), value)));
    names
        .split(' ')
        .filter_map(|name| set(name).transpose())
        .collect()
}

/// The multiplexer that targets `server` on the recorded `backend` explicitly.
pub(crate) fn multiplexer_for_server(
    backend: &str,
    server: &str,
    runner: Rc<dyn CommandRunner>,
) -> Result<Box<dyn Multiplexer>, MultiplexerError> {
    Ok(MuxBackend::parse(backend)?.for_server(server, runner))
}

/// The caller's server on `backend` when `fleet create` receives no `--mux-server`.
pub(crate) fn caller_server(
    backend: MuxBackend,
    env: impl Fn(&str) -> Option<String>,
    runner: &dyn CommandRunner,
) -> Result<String, MultiplexerError> {
    let present = |name: &str| env(name).filter(|value| !value.is_empty());
    match backend {
        MuxBackend::Tmux => match present("TMUX") {
            Some(tmux) => Ok(tmux
                .rsplitn(3, ',')
                .last()
                .expect("rsplitn yields a first segment")
                .to_owned()),
            None => tmux_socket_reply(runner, &argv(DEFAULT_TMUX_SOCKET)),
        },
        MuxBackend::Herdr => {
            if let Some(session) = present("HERDR_SESSION") {
                return Ok(session);
            }
            match present("HERDR_SOCKET_PATH") {
                Some(socket) => herdr_session_for_socket(runner, &socket),
                None => Ok(DEFAULT_HERDR_SESSION.to_owned()),
            }
        }
    }
}

pub(crate) const DEFAULT_HERDR_SESSION: &str = "default";

/// `CommandRunner` passes the process environment to every child, so a stale `TMUX` would
/// select its server instead of the default one.
pub(crate) const DEFAULT_TMUX_SOCKET: &[&str] = &[
    "env",
    "-u",
    "TMUX",
    "tmux",
    "display-message",
    "-p",
    "#{socket_path}",
];

/// The default tmux server's socket path as `argv` prints it.
pub(crate) fn tmux_socket_reply(
    runner: &dyn CommandRunner,
    argv: &[String],
) -> Result<String, MultiplexerError> {
    let no_server =
        || MultiplexerError::new("no tmux server found; pass --mux-server <socket path>");
    let socket = match runner.run(argv) {
        Ok(out) => out.trim().to_owned(),
        Err(RunError::Failed { .. }) => return Err(no_server()),
        Err(error) => return Err(map_run_error("tmux", argv, error)),
    };
    if socket.is_empty() {
        return Err(no_server());
    }
    Ok(socket)
}

fn herdr_session_for_socket(
    runner: &dyn CommandRunner,
    socket: &str,
) -> Result<String, MultiplexerError> {
    let argv = argv(&["herdr", "session", "list", "--json"]);
    let out = runner
        .run(&argv)
        .map_err(|error| map_run_error("herdr", &argv, error))?;
    let listing: serde_json::Value = serde_json::from_str(&out).map_err(|_| {
        MultiplexerError::new(format!(
            "herdr session list returned non-JSON output: {out:?}"
        ))
    })?;
    let sessions = listing["sessions"]
        .as_array()
        .ok_or_else(|| MultiplexerError::new("herdr session list missing 'sessions' field"))?;
    Ok(sessions
        .iter()
        .find(|session| session["socket_path"].as_str() == Some(socket))
        .and_then(|session| session["name"].as_str())
        .unwrap_or(DEFAULT_HERDR_SESSION)
        .to_owned())
}

const NAMED_KEYS: &[(&str, &str, &str)] = &[
    ("Space", "Space", "space"),
    ("Escape", "Escape", "esc"),
    ("Enter", "Enter", "enter"),
    ("Tab", "Tab", "tab"),
    ("Backspace", "BSpace", "backspace"),
    ("Up", "Up", "up"),
    ("Down", "Down", "down"),
    ("Left", "Left", "left"),
    ("Right", "Right", "right"),
];

#[derive(Debug)]
enum PressKey {
    Literal(u8),
    Named(&'static (&'static str, &'static str, &'static str)),
    Function(u8),
    Control(u8),
    ShiftTab,
}

#[derive(Debug)]
pub(crate) struct PressKeys(Vec<PressKey>);

impl PressKeys {
    pub(crate) fn parse(keys: &[String]) -> Result<Self, CafleetError> {
        if !(1..=64).contains(&keys.len()) {
            return Err(CafleetError::Usage(
                "press requires between 1 and 64 keys".into(),
            ));
        }
        let mut parsed = Vec::with_capacity(keys.len());
        for (index, token) in keys.iter().enumerate() {
            if token.len() > 32 {
                return Err(CafleetError::Usage(format!(
                    "press key at position {} exceeds 32 UTF-8 bytes",
                    index + 1
                )));
            }
            let key = if token.len() == 1 && (b'!'..=b'~').contains(&token.as_bytes()[0]) {
                Some(PressKey::Literal(token.as_bytes()[0]))
            } else if let Some(named) = NAMED_KEYS
                .iter()
                .find(|named| token.eq_ignore_ascii_case(named.0))
            {
                Some(PressKey::Named(named))
            } else if token.eq_ignore_ascii_case("Shift+Tab") {
                Some(PressKey::ShiftTab)
            } else if token.len() == 6
                && token
                    .get(..5)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Ctrl+"))
                && token.as_bytes()[5].is_ascii_alphabetic()
            {
                Some(PressKey::Control(token.as_bytes()[5].to_ascii_lowercase()))
            } else {
                (1..=12)
                    .find(|n| token.eq_ignore_ascii_case(&format!("F{n}")))
                    .map(PressKey::Function)
            };
            parsed.push(key.ok_or_else(|| {
                CafleetError::Usage(format!(
                    "invalid press key at position {}: {}",
                    index + 1,
                    serde_json::to_string(token).expect("strings serialize")
                ))
            })?);
        }
        Ok(Self(parsed))
    }

    pub(crate) fn canonical(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(byte) => char::from(*byte).to_string(),
                PressKey::Named(named) => named.0.into(),
                PressKey::Function(n) => format!("F{n}"),
                PressKey::Control(byte) => format!("Ctrl+{}", char::from(*byte)),
                PressKey::ShiftTab => "Shift+Tab".into(),
            })
            .collect()
    }

    pub(super) fn tmux_arguments(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(byte) => format!("0x{byte:02x}"),
                PressKey::Named(named) => named.1.into(),
                PressKey::Function(n) => format!("F{n}"),
                PressKey::Control(byte) => format!("C-{}", char::from(*byte)),
                PressKey::ShiftTab => "BTab".into(),
            })
            .collect()
    }

    pub(super) fn herdr_arguments(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|key| match key {
                PressKey::Literal(b'+') => "plus".into(),
                PressKey::Literal(b'-') => "minus".into(),
                PressKey::Literal(b';') => "semicolon".into(),
                PressKey::Literal(byte) => char::from(*byte).to_string(),
                PressKey::Named(named) => named.2.into(),
                PressKey::Function(n) => format!("f{n}"),
                PressKey::Control(byte) => format!("ctrl+{}", char::from(*byte)),
                PressKey::ShiftTab => "shift+tab".into(),
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use crate::multiplexer::CommandRunner;
    use crate::multiplexer::RunError;

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum Event {
        Run { argv: Vec<String> },
    }

    pub(crate) struct FakeRunner {
        pub(crate) binaries: Vec<String>,
        pub(crate) events: RefCell<Vec<Event>>,
        pub(crate) responses: RefCell<VecDeque<Result<String, RunError>>>,
    }

    impl FakeRunner {
        fn build(binaries: Vec<String>) -> Rc<Self> {
            Rc::new(FakeRunner {
                binaries,
                events: RefCell::new(Vec::new()),
                responses: RefCell::new(VecDeque::new()),
            })
        }

        pub(crate) fn with_binary(name: &str) -> Rc<Self> {
            Self::build(vec![name.to_string()])
        }

        pub(crate) fn without_binaries() -> Rc<Self> {
            Self::build(Vec::new())
        }

        pub(crate) fn succeed(&self, calls: usize) {
            for _ in 0..calls {
                self.respond(Ok(String::new()));
            }
        }

        pub(crate) fn respond(&self, response: Result<String, RunError>) {
            self.responses.borrow_mut().push_back(response);
        }

        pub(crate) fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }

        pub(crate) fn run_argvs(&self) -> Vec<Vec<String>> {
            self.events
                .borrow()
                .iter()
                .map(|Event::Run { argv, .. }| argv.clone())
                .collect()
        }
    }

    impl CommandRunner for FakeRunner {
        fn binary_exists(&self, name: &str) -> bool {
            self.binaries.iter().any(|b| b == name)
        }

        fn run(&self, argv: &[String]) -> Result<String, RunError> {
            self.events.borrow_mut().push(Event::Run {
                argv: argv.to_vec(),
            });
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected process call: {argv:?}"))
        }

        fn run_for(&self, argv: &[String], _: std::time::Duration) -> Result<String, RunError> {
            self.run(argv)
        }

        fn run_until(
            &self,
            argv: &[String],
            deadline: std::time::Instant,
        ) -> Result<String, crate::multiplexer::TimedRunError> {
            run_until_through_run_for(self, argv, deadline)
        }
    }

    /// The relative-budget form of `run_until` shared by the fakes that time out through `run_for`.
    pub(crate) fn run_until_through_run_for(
        runner: &dyn CommandRunner,
        argv: &[String],
        deadline: std::time::Instant,
    ) -> Result<String, crate::multiplexer::TimedRunError> {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(crate::multiplexer::TimedRunError::before_launch)?;
        runner
            .run_for(argv, remaining)
            .map_err(|source| crate::multiplexer::TimedRunError {
                source,
                cleanup_diagnostics: Vec::new(),
            })
    }

    pub(crate) use crate::multiplexer::argv;

    pub(crate) fn run_event(parts: &[&str]) -> Event {
        Event::Run { argv: argv(parts) }
    }

    pub(crate) fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A herdr success envelope wrapping `result`.
    pub(crate) fn herdr_envelope(result: serde_json::Value) -> String {
        serde_json::json!({"id": 1, "result": result, "type": "response"}).to_string()
    }

    /// A herdr error `stderr` body carrying `code`, for `RunError::Failed`.
    pub(crate) fn herdr_error_stderr(code: &str) -> String {
        serde_json::json!({"error": {"code": code, "message": "boom"}, "id": 1}).to_string()
    }
    use crate::multiplexer::*;
    pub(crate) fn parse(tokens: &[&str]) -> PressKeys {
        PressKeys::parse(
            &tokens
                .iter()
                .map(|token| (*token).into())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use crate::broker::Placement;
    use crate::multiplexer::test_support::FakeRunner;
    use crate::multiplexer::test_support::argv;
    use crate::multiplexer::test_support::env;
    use crate::multiplexer::test_support::herdr_envelope;
    use crate::multiplexer::test_support::parse;
    use crate::multiplexer::*;
    use crate::time::Deadline;
    use std::cell::RefCell;

    const TMUX_SOCKET: &str = "/tmp/tmux-1000/default";
    const HERDR_SESSION: &str = "default";

    fn server(backend: &str) -> &'static str {
        match backend {
            "tmux" => TMUX_SOCKET,
            "herdr" => HERDR_SESSION,
            _ => panic!("fixture backend"),
        }
    }

    fn adapter(backend: &str, runner: Rc<FakeRunner>) -> Box<dyn Multiplexer> {
        multiplexer_for_server(backend, server(backend), runner).unwrap()
    }

    #[test]
    fn press_adapters_request_only_the_ordered_keys_and_qualify_stdout_acknowledgment() {
        let keys = parse(&[
            "A",
            "a",
            "+",
            "-",
            ";",
            "Space",
            "Escape",
            "Enter",
            "Tab",
            "Backspace",
            "Up",
            "F12",
            "Ctrl+A",
            "Shift+Tab",
        ]);
        for (backend, pane, expected) in [
            (
                "tmux",
                "%2",
                argv(&[
                    "tmux",
                    "-S",
                    TMUX_SOCKET,
                    "send-keys",
                    "-t",
                    "%2",
                    "0x41",
                    "0x61",
                    "0x2b",
                    "0x2d",
                    "0x3b",
                    "Space",
                    "Escape",
                    "Enter",
                    "Tab",
                    "BSpace",
                    "Up",
                    "F12",
                    "C-a",
                    "BTab",
                ]),
            ),
            (
                "herdr",
                "w1:p2",
                argv(&[
                    "herdr",
                    "--session",
                    HERDR_SESSION,
                    "pane",
                    "send-keys",
                    "w1:p2",
                    "A",
                    "a",
                    "plus",
                    "minus",
                    "semicolon",
                    "space",
                    "esc",
                    "enter",
                    "tab",
                    "backspace",
                    "up",
                    "f12",
                    "ctrl+a",
                    "shift+tab",
                ]),
            ),
        ] {
            let runner = FakeRunner::with_binary(backend);
            runner.respond(Ok(" \n\t".into()));
            adapter(backend, runner.clone())
                .send_keys(pane, &keys, Deadline::new())
                .unwrap();
            let mut actual = runner.run_argvs();
            assert_eq!(actual.len(), 1);
            if backend == "tmux" {
                for value in &mut actual[0][6..11] {
                    assert!(value.starts_with("0x") && value.len() == 4);
                    value.make_ascii_lowercase();
                }
            }
            assert_eq!(actual, vec![expected]);
            assert!(runner.responses.borrow().is_empty());

            let unreadable = FakeRunner::with_binary(backend);
            unreadable.respond(Ok("unexpected acknowledgment".into()));
            let failure = adapter(backend, unreadable.clone())
                .send_keys(pane, &keys, Deadline::new())
                .expect_err("non-whitespace stdout leaves actual effects uncertain");
            assert!(failure.submission_started());
            assert_eq!(unreadable.run_argvs().len(), 1);
        }
    }

    fn placement(backend: &str) -> Placement {
        Placement {
            backend: backend.into(),
            mux_session: "main".into(),
            mux_window_id: "@1".into(),
            mux_pane_id: Some(if backend == "tmux" { "%2" } else { "w1:p2" }.into()),
            coding_agent: "codex".into(),
            created_at: "2026-10-27T10:33:00+00:00".into(),
            mux_server: server(backend).into(),
        }
    }

    #[test]
    fn press_target_validation_reads_only_the_recorded_target_on_the_recorded_server() {
        let tmux_target = argv(&[
            "tmux",
            "-S",
            TMUX_SOCKET,
            "display-message",
            "-p",
            "-t",
            "%2",
            "#{pane_id}|#{window_id}|#{session_name}",
        ]);
        let valid = FakeRunner::with_binary("tmux");
        valid.respond(Ok("%2|@1|main\n".into()));
        adapter("tmux", valid.clone())
            .validate_key_target(&placement("tmux"), Deadline::new())
            .unwrap();
        assert_eq!(valid.run_argvs(), vec![tmux_target.clone()]);

        let moved = FakeRunner::with_binary("tmux");
        moved.respond(Ok("%2|@9|main\n".into()));
        let failure = adapter("tmux", moved.clone())
            .validate_key_target(&placement("tmux"), Deadline::new())
            .unwrap_err();
        assert_eq!(failure.to_string(), PLACEMENT_MISMATCH);
        assert_eq!(moved.run_argvs(), vec![tmux_target]);

        let herdr_list = argv(&["herdr", "--session", HERDR_SESSION, "pane", "list"]);
        let pane = serde_json::json!({"workspace_id":"main","tab_id":"@1","pane_id":"w1:p2"});
        let valid = FakeRunner::with_binary("herdr");
        valid.respond(Ok(herdr_envelope(
            serde_json::json!({"panes":[pane.clone()]}),
        )));
        adapter("herdr", valid.clone())
            .validate_key_target(&placement("herdr"), Deadline::new())
            .unwrap();
        assert_eq!(valid.run_argvs(), vec![herdr_list.clone()]);

        let duplicate = FakeRunner::with_binary("herdr");
        duplicate.respond(Ok(herdr_envelope(
            serde_json::json!({"panes":[pane.clone(),pane]}),
        )));
        let failure = adapter("herdr", duplicate.clone())
            .validate_key_target(&placement("herdr"), Deadline::new())
            .unwrap_err();
        assert_eq!(failure.to_string(), PLACEMENT_MISMATCH);
        assert_eq!(duplicate.run_argvs(), vec![herdr_list]);
    }

    #[test]
    fn a_recorded_server_needs_only_the_binary_and_rejects_an_unsupported_backend() {
        for backend in ["tmux", "herdr"] {
            adapter(backend, FakeRunner::with_binary(backend))
                .ensure_available()
                .unwrap();
            assert_eq!(
                adapter(backend, FakeRunner::without_binaries())
                    .ensure_available()
                    .unwrap_err()
                    .to_string(),
                format!("{backend} binary not found on PATH")
            );
        }
        let failure = match multiplexer_for_server("screen", "s", FakeRunner::without_binaries()) {
            Ok(_) => panic!("an unsupported recorded backend builds no multiplexer"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure.to_string(),
            "unsupported recorded multiplexer backend 'screen'"
        );
    }

    #[test]
    fn a_recorded_tmux_or_herdr_server_reads_the_requested_pane_and_reports_a_missing_one() {
        for (reply, found) in [
            ("main|@1|%2\n", true),
            ("||\n", false),
            ("main|@1|%3\n", false),
        ] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok(reply.into()));
            let context = adapter("tmux", runner.clone()).pane_context("%2").unwrap();
            assert_eq!(context.is_some(), found, "{reply:?}");
            assert_eq!(
                runner.run_argvs(),
                vec![argv(&[
                    "tmux",
                    "-S",
                    TMUX_SOCKET,
                    "display-message",
                    "-p",
                    "-t",
                    "%2",
                    "#{session_name}|#{window_id}|#{pane_id}",
                ])]
            );
        }

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(
            serde_json::json!({"pane":{"workspace_id":"w1","tab_id":"w1:t1","pane_id":"w1:p2"}}),
        )));
        let context = adapter("herdr", runner.clone())
            .pane_context("w1:p2")
            .unwrap()
            .unwrap();
        assert_eq!(
            (context.session, context.window_id, context.pane_id),
            ("w1".into(), "w1:t1".into(), "w1:p2".into())
        );
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&[
                "herdr",
                "--session",
                HERDR_SESSION,
                "pane",
                "current",
                "--pane",
                "w1:p2",
            ])]
        );
        let missing = FakeRunner::with_binary("herdr");
        missing.respond(Err(RunError::Failed {
            stderr: test_support::herdr_error_stderr("pane_not_found"),
        }));
        assert!(
            adapter("herdr", missing)
                .pane_context("w1:p9")
                .unwrap()
                .is_none()
        );
    }

    struct AbsoluteRecorder(RefCell<Vec<Instant>>);

    impl CommandRunner for AbsoluteRecorder {
        fn binary_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&self, _: &[String]) -> Result<String, RunError> {
            panic!("absolute execution retains its Instant")
        }
        fn run_for(&self, _: &[String], _: Duration) -> Result<String, RunError> {
            panic!("absolute execution retains its Instant")
        }
        fn run_until(&self, _: &[String], deadline: Instant) -> Result<String, TimedRunError> {
            self.0.borrow_mut().push(deadline);
            Ok(String::new())
        }
    }

    #[test]
    fn press_deadline_runner_clips_the_actual_absolute_call_and_refuses_expiry() {
        let recorder = Rc::new(AbsoluteRecorder(RefCell::default()));
        let original = Instant::now() + Duration::from_secs(2);
        let bounded = DeadlineRunner::wrap(recorder.clone(), Deadline::until(original));
        assert!(
            bounded
                .run_until(&argv(&["fixture"]), original + Duration::from_secs(5))
                .is_ok()
        );
        let earlier = original - Duration::from_secs(1);
        assert!(bounded.run_until(&argv(&["fixture"]), earlier).is_ok());
        assert_eq!(*recorder.0.borrow(), vec![original, earlier]);
        let expired = DeadlineRunner::wrap(recorder.clone(), Deadline::until(Instant::now()));
        let failure = match expired.run_until(&argv(&["fixture"]), original) {
            Ok(_) => panic!("expired work starts no underlying command"),
            Err(failure) => failure,
        };
        assert!(matches!(failure.source, RunError::NotStarted(_)));
        assert_eq!(recorder.0.borrow().len(), 2);
    }
    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars = env(pairs);
        move |name: &str| vars.get(name).cloned()
    }

    #[test]
    fn auto_detection_resolves_the_single_present_backend_or_fails() {
        let ambiguous = "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
                         unset the one that does not belong to this session";
        let absent = "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
                      run cafleet inside a tmux or herdr session";
        for (pairs, expected) in [
            (&[("HERDR_ENV", "1")][..], Ok(MuxBackend::Herdr)),
            (
                &[("TMUX", "/tmp/tmux-1000/default,123,0")],
                Ok(MuxBackend::Tmux),
            ),
            (&[("HERDR_ENV", ""), ("TMUX", "")], Err(absent)),
            (&[("HERDR_ENV", "1"), ("TMUX", "/tmp/tmux")], Err(ambiguous)),
            (&[], Err(absent)),
        ] {
            assert_eq!(
                MuxBackend::detect(lookup(pairs)).map_err(|error| error.to_string()),
                expected.map_err(str::to_owned),
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn tmux_caller_server_is_the_tmux_socket_path_before_its_last_two_commas() {
        let runner = FakeRunner::with_binary("tmux");
        assert_eq!(
            caller_server(
                MuxBackend::Tmux,
                lookup(&[("TMUX", "/tmp/a,b/default,123,0")]),
                runner.as_ref()
            )
            .unwrap(),
            "/tmp/a,b/default"
        );
        assert!(runner.events().is_empty());
    }

    #[test]
    fn tmux_caller_server_without_tmux_asks_the_default_server_for_its_socket() {
        let default_socket = argv(DEFAULT_TMUX_SOCKET);
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("/tmp/tmux-0/default\n".into()));
        assert_eq!(
            caller_server(MuxBackend::Tmux, lookup(&[("TMUX", "")]), runner.as_ref()).unwrap(),
            "/tmp/tmux-0/default"
        );
        assert_eq!(runner.run_argvs(), vec![default_socket.clone()]);

        for response in [
            Err(RunError::Failed {
                stderr: "no server running on /tmp/tmux-0/default".into(),
            }),
            Ok("\n".into()),
        ] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(response);
            let failure =
                caller_server(MuxBackend::Tmux, lookup(&[]), runner.as_ref()).unwrap_err();
            assert_eq!(
                failure.to_string(),
                "no tmux server found; pass --mux-server <socket path>"
            );
            assert_eq!(runner.run_argvs(), vec![default_socket.clone()]);
        }
    }

    #[test]
    fn herdr_caller_server_prefers_the_session_then_the_matching_socket_then_default() {
        let runner = FakeRunner::with_binary("herdr");
        assert_eq!(
            caller_server(
                MuxBackend::Herdr,
                lookup(&[
                    ("HERDR_SESSION", "work"),
                    ("HERDR_SOCKET_PATH", "/run/other.sock")
                ]),
                runner.as_ref()
            )
            .unwrap(),
            "work"
        );
        assert_eq!(
            caller_server(
                MuxBackend::Herdr,
                lookup(&[("HERDR_SESSION", "")]),
                runner.as_ref()
            )
            .unwrap(),
            "default"
        );
        assert!(runner.events().is_empty());

        let listing = serde_json::json!({"sessions": [
            {"name": "default", "status": "running", "socket_path": "/run/herdr.sock"},
            {"name": "work", "status": "running", "socket_path": "/run/work.sock"},
        ]})
        .to_string();
        for (socket, expected) in [("/run/work.sock", "work"), ("/run/gone.sock", "default")] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(listing.clone()));
            assert_eq!(
                caller_server(
                    MuxBackend::Herdr,
                    lookup(&[("HERDR_SOCKET_PATH", socket)]),
                    runner.as_ref()
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                runner.run_argvs(),
                vec![argv(&["herdr", "session", "list", "--json"])]
            );
        }
    }

    #[test]
    fn capture_window_drops_only_the_visually_blank_tail_then_keeps_the_last_lines() {
        for (raw, lines, expected) in [
            ("l1\nl2\nl3\nx\rY\n", 2, "l3\nx\rY"),
            ("drawn1\ndrawn2\n\n\n\n\n\n\n", 2, "drawn1\ndrawn2"),
            (
                "scrollback\nprompt>\ncomposer\n \n\t\n\n\n",
                2,
                "prompt>\ncomposer",
            ),
            ("a\n\nb\n\n\n", 3, "a\n\nb"),
            (
                "top\nkept \x1b[31mred\x1b[0m\n\x1b[39m\x1b[49m\n\x1b[2K \n\n",
                2,
                "top\nkept \x1b[31mred\x1b[0m",
            ),
            (
                "one\ntwo\x1b[0m\n\x1b[39m\x1b[49m\n \x1b[2K\n",
                2,
                "one\ntwo\x1b[0m",
            ),
            ("one\ntwo\nthree\n\n\n", 2, "two\nthree"),
            ("raw\nbuffer\n\n \n", 20, "raw\nbuffer"),
            ("\n \n\n", 2, ""),
        ] {
            assert_eq!(capture_window(raw, lines), expected, "{raw:?} {lines}");
        }
    }

    #[test]
    fn expired_deadlines_start_no_native_command_or_optional_layout_restore() {
        let runner = test_support::FakeRunner::without_binaries();
        let mux = HerdrMultiplexer::for_server(runner.clone(), HERDR_SESSION)
            .bounded(crate::time::Deadline::until(std::time::Instant::now()));
        assert!(mux.close_pane("w1:p2").is_err());
        mux.restore_after_close().unwrap();
        assert!(runner.events().is_empty());
    }

    fn usage(tokens: &[String], expected: &str) {
        let failure = match PressKeys::parse(tokens) {
            Ok(_) => panic!("the key vector is outside the common grammar"),
            Err(failure) => failure,
        };
        assert!(matches!(&failure, CafleetError::Usage(message) if message == expected));
        assert_eq!(failure.exit_code(), 2);
    }
    #[test]
    fn press_keys_validate_the_complete_common_grammar_and_byte_count_bounds() {
        let keys = parse(&[
            "A",
            "a",
            "+",
            "-",
            ";",
            "!",
            "~",
            "sPaCe",
            "ESCAPE",
            "enter",
            "Tab",
            "Backspace",
            "up",
            "down",
            "left",
            "right",
            "f1",
            "F12",
            "cTrL+A",
            "CTRL+Z",
            "sHiFt+tAb",
        ]);
        assert_eq!(
            keys.canonical(),
            argv(&[
                "A",
                "a",
                "+",
                "-",
                ";",
                "!",
                "~",
                "Space",
                "Escape",
                "Enter",
                "Tab",
                "Backspace",
                "Up",
                "Down",
                "Left",
                "Right",
                "F1",
                "F12",
                "Ctrl+a",
                "Ctrl+z",
                "Shift+Tab",
            ])
        );
        assert_eq!(
            PressKeys::parse(&vec!["a".into(); 64])
                .unwrap()
                .canonical()
                .len(),
            64
        );
        usage(&[], "press requires between 1 and 64 keys");
        usage(
            &vec!["a".into(); 65],
            "press requires between 1 and 64 keys",
        );
        let long = "é".repeat(17);
        usage(
            &["Enter".into(), long],
            "press key at position 2 exceeds 32 UTF-8 bytes",
        );
        for invalid in ["F0", "F13", "Ctrl+1", " ", "é"] {
            usage(
                &["a".into(), invalid.into()],
                &format!(
                    "invalid press key at position 2: {}",
                    serde_json::to_string(invalid).unwrap()
                ),
            );
        }
    }
}
