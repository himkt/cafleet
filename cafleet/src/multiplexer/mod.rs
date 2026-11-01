pub(crate) mod herdr;
mod keys;
pub(crate) mod tmux;
pub(crate) use keys::PressKeys;

use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(crate) use herdr::HerdrMultiplexer;
pub(crate) use tmux::TmuxMultiplexer;

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

    #[cfg(test)]
    pub(crate) fn pane_cleanup(&self) -> Option<&PaneCleanup> {
        self.pane_cleanup.as_ref()
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
    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError>;
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        self.run(argv, Some(whole_seconds(timeout)))
    }
    fn run_until(&self, argv: &[String], deadline: Instant) -> Result<String, TimedRunError> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(TimedRunError::before_launch)?;
        self.run_for(argv, remaining)
            .map_err(|source| TimedRunError {
                source,
                cleanup_diagnostics: Vec::new(),
            })
    }
    fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
        Ok(None)
    }
}

fn whole_seconds(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

const PLACEMENT_MISMATCH: &str = "press target context does not match the recorded placement";

fn map_run_error(
    name: &str,
    argv: &[String],
    timeout_secs: Option<u64>,
    error: RunError,
) -> MultiplexerError {
    let joined = argv.join(" ");
    match error {
        RunError::NotStarted(detail) => MultiplexerError::new(format!(
            "{name} command was not started: {detail}: {joined}"
        )),
        RunError::BinaryNotFound(detail) => {
            MultiplexerError::new(format!("{name} binary not found: {detail}"))
        }
        RunError::Timeout => match timeout_secs {
            Some(secs) => {
                MultiplexerError::new(format!("{name} command timed out after {secs}s: {joined}"))
            }
            None => MultiplexerError::new(format!(
                "{name} command timed out at the pane input deadline: {joined}"
            )),
        },
        RunError::Failed { stderr } => MultiplexerError::new(format!(
            "{name} command failed: {joined}\nstderr: {}",
            stderr.trim()
        )),
    }
}

fn check_deadline(deadline: crate::input::Deadline) -> Result<(), MultiplexerError> {
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

/// The runner, environment and error mapping shared by every backend; a
/// backend adds only its own argv construction and output parsing.
struct BackendCore {
    runner: Rc<dyn CommandRunner>,
    env: std::collections::HashMap<String, String>,
    name: &'static str,
    session_variable: &'static str,
    pane_gone: fn(&str) -> bool,
}

impl BackendCore {
    fn bounded(&self, deadline: crate::input::Deadline) -> Self {
        Self {
            runner: DeadlineRunner::wrap(self.runner.clone(), deadline),
            env: self.env.clone(),
            name: self.name,
            session_variable: self.session_variable,
            pane_gone: self.pane_gone,
        }
    }

    fn env_var(&self, name: &str) -> Option<&str> {
        self.env
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    fn map_error(
        &self,
        argv: &[String],
        timeout_secs: Option<u64>,
        error: RunError,
    ) -> MultiplexerError {
        map_run_error(self.name, argv, timeout_secs, error)
    }

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, MultiplexerError> {
        self.runner
            .run(argv, timeout_secs)
            .map_err(|error| self.map_error(argv, timeout_secs, error))
    }

    fn run_tolerating_missing(
        &self,
        argv: &[String],
        ignore_missing: bool,
        timeout_secs: Option<u64>,
    ) -> Result<(), MultiplexerError> {
        match self.runner.run(argv, timeout_secs) {
            Ok(_) => Ok(()),
            Err(RunError::Failed { ref stderr }) if ignore_missing && (self.pane_gone)(stderr) => {
                Ok(())
            }
            Err(error) => Err(self.map_error(argv, timeout_secs, error)),
        }
    }

    fn ensure_available(&self) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists(self.name) {
            return Err(MultiplexerError::new(format!(
                "{} binary not found on PATH",
                self.name
            )));
        }
        if self.env_var(self.session_variable).is_none() {
            return Err(MultiplexerError::new(format!(
                "cafleet member commands must be run inside a {} session",
                self.name
            )));
        }
        Ok(())
    }

    fn press_run(
        &self,
        argv: &[String],
        deadline: crate::input::Deadline,
        writing: bool,
    ) -> Result<String, MultiplexerError> {
        check_deadline(deadline)?;
        self.runner
            .run_until(argv, deadline.instant())
            .map_err(|error| {
                let started =
                    writing && matches!(&error.source, RunError::Failed { .. } | RunError::Timeout);
                let mut mapped = self.map_error(argv, None, error.source);
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
        deadline: crate::input::Deadline,
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
    deadline: crate::input::Deadline,
}

impl DeadlineRunner {
    pub(crate) fn wrap(
        runner: Rc<dyn CommandRunner>,
        deadline: crate::input::Deadline,
    ) -> Rc<dyn CommandRunner> {
        Rc::new(Self { runner, deadline })
    }
}

impl CommandRunner for DeadlineRunner {
    fn binary_exists(&self, name: &str) -> bool {
        self.runner.binary_exists(name)
    }

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError> {
        let remaining = self
            .remaining_budget()?
            .expect("deadline runner has a budget");
        self.run_for(
            argv,
            timeout_secs
                .map(Duration::from_secs)
                .map_or(remaining, |timeout| timeout.min(remaining)),
        )
    }

    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        let remaining = self
            .remaining_budget()?
            .expect("deadline runner has a budget");
        self.runner.run_for(argv, timeout.min(remaining))
    }

    fn run_until(&self, argv: &[String], deadline: Instant) -> Result<String, TimedRunError> {
        let deadline = deadline.min(self.deadline.instant());
        if Instant::now() >= deadline {
            return Err(TimedRunError::before_launch());
        }
        self.runner.run_until(argv, deadline)
    }

    fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
        let remaining = self
            .deadline
            .remaining()
            .map_err(|_| RunError::deadline_expired())?;
        Ok(Some(
            self.runner
                .remaining_budget()?
                .map_or(remaining, |inner| inner.min(remaining)),
        ))
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

/// The closed backend dispatch the resolver returns.
pub(crate) enum AnyMultiplexer {
    Tmux(TmuxMultiplexer),
    Herdr(HerdrMultiplexer),
}

pub(crate) enum PostCloseLayout {
    None,
    HerdrTab(String),
}

impl AnyMultiplexer {
    pub(crate) fn close_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<PostCloseLayout, MultiplexerError> {
        match self {
            Self::Tmux(mux) => {
                mux.kill_pane(target_pane_id, ignore_missing)?;
                Ok(PostCloseLayout::None)
            }
            Self::Herdr(mux) => Ok(mux
                .close_pane(target_pane_id, ignore_missing)?
                .map_or(PostCloseLayout::None, PostCloseLayout::HerdrTab)),
        }
    }

    pub(crate) fn restore_after_close(
        &self,
        layout: PostCloseLayout,
    ) -> Result<(), MultiplexerError> {
        match (self, layout) {
            (_, PostCloseLayout::None) => Ok(()),
            (Self::Herdr(mux), PostCloseLayout::HerdrTab(tab_id)) => {
                mux.resize_after_close(&tab_id)
            }
            (Self::Tmux(_), PostCloseLayout::HerdrTab(_)) => {
                unreachable!("post-close layout belongs to its multiplexer backend");
            }
        }
    }

    pub(crate) fn bounded(&self, deadline: crate::input::Deadline) -> Self {
        match self {
            Self::Tmux(mux) => Self::Tmux(mux.bounded(deadline)),
            Self::Herdr(mux) => Self::Herdr(mux.bounded(deadline)),
        }
    }
}

impl AnyMultiplexer {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Tmux(mux) => mux.name(),
            Self::Herdr(mux) => mux.name(),
        }
    }

    pub(crate) fn ensure_available(&self) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.ensure_available(),
            Self::Herdr(mux) => mux.ensure_available(),
        }
    }

    pub(crate) fn validate_key_target(
        &self,
        placement: &crate::broker::records::Placement,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.validate_key_target(placement, deadline),
            Self::Herdr(mux) => mux.validate_key_target(placement, deadline),
        }
    }

    pub(crate) fn send_keys(
        &self,
        pane_id: &str,
        keys: &PressKeys,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.send_keys(pane_id, keys, deadline),
            Self::Herdr(mux) => mux.send_keys(pane_id, keys, deadline),
        }
    }

    pub(crate) fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.context_discovery(),
            Self::Herdr(mux) => mux.context_discovery(),
        }
    }

    pub(crate) fn split_window(
        &self,
        reference: &MultiplexerContext,
        env: &[(String, String)],
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.split_window(reference, env, command),
            Self::Herdr(mux) => mux.split_window(reference, env, command),
        }
    }

    pub(crate) fn capture_pane(
        &self,
        target_pane_id: &str,
        lines: i64,
    ) -> Result<String, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.capture_pane(target_pane_id, lines),
            Self::Herdr(mux) => mux.capture_pane(target_pane_id, lines),
        }
    }

    pub(crate) fn kill_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.kill_pane(target_pane_id, ignore_missing),
            Self::Herdr(mux) => mux.kill_pane(target_pane_id, ignore_missing),
        }
    }
}

/// Resolve the selected backend and construct it over the
/// given runner and environment snapshot.
pub(crate) fn resolve_multiplexer(
    override_value: Option<&str>,
    env: std::collections::HashMap<String, String>,
    runner: std::rc::Rc<dyn CommandRunner>,
) -> Result<AnyMultiplexer, MultiplexerError> {
    let name = resolve_multiplexer_name(override_value, |key| env.get(key).cloned())?;
    Ok(match name {
        "herdr" => AnyMultiplexer::Herdr(HerdrMultiplexer::new(runner, env)),
        "tmux" => AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner, env)),
        other => unreachable!("the resolver returned an unsupported multiplexer: {other}"),
    })
}

/// An explicit backend override must name
/// a registry key; otherwise auto-detect from `HERDR_ENV` / `TMUX` (an empty
/// value counts as unset); ambiguity and absence are hard errors.
fn resolve_multiplexer_name(
    override_value: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> Result<&'static str, MultiplexerError> {
    if let Some(value) = override_value {
        return match value {
            "tmux" => Ok("tmux"),
            "herdr" => Ok("herdr"),
            other => Err(MultiplexerError::new(format!(
                "CAFLEET_MULTIPLEXER='{other}' is not a supported multiplexer \
                 (expected one of: herdr, tmux)"
            ))),
        };
    }
    let present = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    match (present("HERDR_ENV"), present("TMUX")) {
        (true, true) => Err(MultiplexerError::new(
            "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
             set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate",
        )),
        (true, false) => Ok("herdr"),
        (false, true) => Ok("tmux"),
        (false, false) => Err(MultiplexerError::new(
            "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
             run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER",
        )),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;

    use super::{CommandRunner, RunError};

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum Event {
        Run {
            argv: Vec<String>,
            timeout_secs: Option<u64>,
        },
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

        fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError> {
            self.events.borrow_mut().push(Event::Run {
                argv: argv.to_vec(),
                timeout_secs,
            });
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| panic!("unexpected process call: {argv:?}"))
        }
    }

    pub(crate) fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    pub(crate) fn run_event(parts: &[&str], timeout_secs: Option<u64>) -> Event {
        Event::Run {
            argv: argv(parts),
            timeout_secs,
        }
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
}

#[cfg(test)]
mod tests {
    use super::keys::test_support::parse;
    use super::test_support::{FakeRunner, argv, env, herdr_envelope};
    use super::*;
    use crate::broker::records::Placement;
    use crate::input::Deadline;
    use std::cell::RefCell;

    fn adapter(backend: &str, runner: Rc<FakeRunner>) -> AnyMultiplexer {
        match backend {
            "tmux" => AnyMultiplexer::Tmux(TmuxMultiplexer::new(
                runner,
                env(&[("TMUX", "fixture"), ("TMUX_PANE", "%0")]),
            )),
            "herdr" => AnyMultiplexer::Herdr(HerdrMultiplexer::new(
                runner,
                env(&[("HERDR_ENV", "fixture")]),
            )),
            _ => panic!("fixture backend"),
        }
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
                for value in &mut actual[0][4..9] {
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
        }
    }

    #[test]
    fn press_target_validation_rejects_a_wrong_tmux_tuple_and_ambiguous_herdr_target() {
        let tmux = FakeRunner::with_binary("tmux");
        tmux.respond(Ok("main|@1|%0\n".into()));
        tmux.respond(Ok("%2|@9|main\n".into()));
        assert!(
            adapter("tmux", tmux.clone())
                .validate_key_target(&placement("tmux"), Deadline::new())
                .is_err()
        );
        assert!(
            tmux.run_argvs()
                .iter()
                .all(|args| args[1] == "display-message")
        );

        let pane = serde_json::json!({"workspace_id":"main","tab_id":"@1","pane_id":"w1:p2"});
        let valid = FakeRunner::with_binary("herdr");
        valid.respond(Ok(herdr_envelope(
            serde_json::json!({"pane":{"workspace_id":"main","tab_id":"@1","pane_id":"w1:p0"}}),
        )));
        valid.respond(Ok(herdr_envelope(
            serde_json::json!({"panes":[pane.clone()]}),
        )));
        adapter("herdr", valid.clone())
            .validate_key_target(&placement("herdr"), Deadline::new())
            .unwrap();
        assert_eq!(
            valid.run_argvs(),
            vec![
                argv(&["herdr", "pane", "current"]),
                argv(&["herdr", "pane", "list"])
            ]
        );
        let duplicate = FakeRunner::with_binary("herdr");
        duplicate.respond(Ok(herdr_envelope(
            serde_json::json!({"pane":{"workspace_id":"main","tab_id":"@1","pane_id":"w1:p0"}}),
        )));
        duplicate.respond(Ok(herdr_envelope(
            serde_json::json!({"panes":[pane.clone(),pane]}),
        )));
        assert!(
            adapter("herdr", duplicate.clone())
                .validate_key_target(&placement("herdr"), Deadline::new())
                .is_err()
        );
        assert_eq!(duplicate.run_argvs().len(), 2);
    }

    struct AbsoluteRecorder(RefCell<Vec<Instant>>);

    impl CommandRunner for AbsoluteRecorder {
        fn binary_exists(&self, _: &str) -> bool {
            true
        }
        fn run(&self, _: &[String], _: Option<u64>) -> Result<String, RunError> {
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
    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_valid_override_wins() {
        assert_eq!(
            resolve_multiplexer_name(Some("tmux"), no_env).unwrap(),
            "tmux"
        );
        assert_eq!(
            resolve_multiplexer_name(Some("herdr"), no_env).unwrap(),
            "herdr"
        );
    }

    #[test]
    fn an_unknown_override_is_rejected_with_the_sorted_key_list() {
        let err = resolve_multiplexer_name(Some("bogus"), no_env)
            .expect_err("an unknown override must be rejected");
        assert_eq!(
            err.to_string(),
            "CAFLEET_MULTIPLEXER='bogus' is not a supported multiplexer \
                 (expected one of: herdr, tmux)"
        );
    }

    #[test]
    fn auto_detect_picks_the_single_present_backend() {
        let herdr_only = |name: &str| match name {
            "HERDR_ENV" => Some("1".to_string()),
            _ => None,
        };
        assert_eq!(resolve_multiplexer_name(None, herdr_only).unwrap(), "herdr");

        let tmux_only = |name: &str| match name {
            "TMUX" => Some("/tmp/tmux-1000/default,123,0".to_string()),
            _ => None,
        };
        assert_eq!(resolve_multiplexer_name(None, tmux_only).unwrap(), "tmux");
    }

    #[test]
    fn empty_presence_values_count_as_unset() {
        let empty_both = |name: &str| match name {
            "HERDR_ENV" | "TMUX" => Some(String::new()),
            _ => None,
        };
        let err = resolve_multiplexer_name(None, empty_both)
            .expect_err("empty presence vars mean no backend");
        assert!(
            err.to_string()
                .starts_with("no supported multiplexer detected:")
        );
    }

    #[test]
    fn both_present_is_a_hard_error() {
        let both = |name: &str| match name {
            "HERDR_ENV" => Some("1".to_string()),
            "TMUX" => Some("/tmp/tmux".to_string()),
            _ => None,
        };
        let err = resolve_multiplexer_name(None, both).expect_err("ambiguous env");
        assert_eq!(
            err.to_string(),
            "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
                 set CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate"
        );
    }

    #[test]
    fn neither_present_is_a_hard_error() {
        let err = resolve_multiplexer_name(None, no_env).expect_err("no backend");
        assert_eq!(
            err.to_string(),
            "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; \
                 run cafleet inside a tmux or herdr session, or set CAFLEET_MULTIPLEXER"
        );
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
        let mux = AnyMultiplexer::Herdr(HerdrMultiplexer::new(
            runner.clone(),
            test_support::env(&[]),
        ))
        .bounded(crate::input::Deadline::until(std::time::Instant::now()));
        assert!(
            mux.restore_after_close(PostCloseLayout::HerdrTab("tab".into()))
                .is_err()
        );
        assert!(runner.events().is_empty());
    }
}
