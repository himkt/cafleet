pub mod herdr;
#[cfg(test)]
pub mod test_support;
pub mod tmux;

use std::fmt;
use std::rc::Rc;
use std::time::Duration;

pub use herdr::HerdrMultiplexer;
pub use tmux::TmuxMultiplexer;

pub(crate) const SUBMIT_DELAY: f64 = 1.0;

/// Resolved pane identity, returned by `context_discovery` (SPEC §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiplexerContext {
    pub session: String,
    pub window_id: String,
    pub pane_id: String,
}

/// The outcome of backend-owned pane compensation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneCleanup {
    Attempted {
        pane_id: String,
        error: Option<String>,
    },
    Unknown,
}

/// A backend failure with optional pane ownership information.
#[derive(Debug)]
pub struct MultiplexerError {
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

    pub fn pane_cleanup(&self) -> Option<&PaneCleanup> {
        self.pane_cleanup.as_ref()
    }

    pub fn submission_started(&self) -> bool {
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
pub enum RunError {
    BinaryNotFound(String),
    Timeout,
    Failed { stderr: String },
}

pub trait CommandRunner {
    fn binary_exists(&self, name: &str) -> bool;
    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, RunError>;
    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        self.run(
            argv,
            Some(timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0)),
        )
    }
    fn sleep(&self, seconds: f64);
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
        let remaining = self.deadline.remaining().map_err(|_| RunError::Timeout)?;
        self.run_for(
            argv,
            timeout_secs
                .map(Duration::from_secs)
                .map_or(remaining, |timeout| timeout.min(remaining)),
        )
    }

    fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
        let remaining = self.deadline.remaining().map_err(|_| RunError::Timeout)?;
        self.runner.run_for(argv, timeout.min(remaining))
    }

    fn sleep(&self, seconds: f64) {
        if let Ok(remaining) = self.deadline.remaining() {
            self.runner.sleep(
                Duration::from_secs_f64(seconds)
                    .min(remaining)
                    .as_secs_f64(),
            );
        }
    }
}

/// The A8 over-fetch margin: both backends fetch `lines + 1000` so a blank
/// tail deeper than the requested window still leaves drawn lines to keep.
pub(crate) const CAPTURE_OVER_FETCH_LINES: i64 = 1000;

/// The shared capture windowing (SPEC §6.5, amendment A8): split the fetched
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

pub(crate) fn inline_preview_payload(
    message_id: i64,
    sender_id: i64,
    ts: &str,
    text: &str,
) -> String {
    fn visible(text: &str) -> String {
        let mut result = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    result.push('⏎');
                }
                '\n' => result.push('⏎'),
                '\t' => result.push('␉'),
                '\u{00}'..='\u{1f}' | '\u{7f}' => result.push_str(&format!("\\x{:02x}", ch as u32)),
                _ => result.push(ch),
            }
        }
        result
    }
    format!(
        "[cafleet msg {message_id} from {sender_id} {ts}] {}",
        visible(text)
    )
}

pub trait Multiplexer {
    fn name(&self) -> &'static str;
    fn ensure_available(&self) -> Result<(), MultiplexerError>;
    fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError>;
    fn split_window(
        &self,
        reference: &MultiplexerContext,
        env: &[(String, String)],
        command: &[String],
    ) -> Result<String, MultiplexerError>;
    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), MultiplexerError>;
    fn send_prompt(&self, target_pane_id: &str, text: &str) -> Result<(), MultiplexerError>;
    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError>;
    fn list_pane_ids(&self) -> Result<std::collections::BTreeSet<String>, MultiplexerError>;
    fn kill_pane(&self, target_pane_id: &str, ignore_missing: bool)
    -> Result<(), MultiplexerError>;
}

/// The closed backend dispatch the resolver returns.
pub enum AnyMultiplexer {
    Tmux(TmuxMultiplexer),
    Herdr(HerdrMultiplexer),
}

impl AnyMultiplexer {
    pub fn bounded(&self, deadline: crate::input::Deadline) -> Self {
        match self {
            Self::Tmux(mux) => Self::Tmux(mux.bounded(deadline)),
            Self::Herdr(mux) => Self::Herdr(mux.bounded(deadline)),
        }
    }
}

macro_rules! dispatch {
    ($self:ident, $mux:ident => $call:expr) => {
        match $self {
            AnyMultiplexer::Tmux($mux) => $call,
            AnyMultiplexer::Herdr($mux) => $call,
        }
    };
}

impl Multiplexer for AnyMultiplexer {
    fn name(&self) -> &'static str {
        dispatch!(self, mux => mux.name())
    }

    fn ensure_available(&self) -> Result<(), MultiplexerError> {
        dispatch!(self, mux => mux.ensure_available())
    }

    fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        dispatch!(self, mux => mux.context_discovery())
    }

    fn split_window(
        &self,
        reference: &MultiplexerContext,
        env: &[(String, String)],
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        dispatch!(self, mux => mux.split_window(reference, env, command))
    }

    fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), MultiplexerError> {
        dispatch!(self, mux => mux.send_inline_preview(target_pane_id, message_id, sender_id, ts, text))
    }

    fn send_prompt(&self, target_pane_id: &str, text: &str) -> Result<(), MultiplexerError> {
        dispatch!(self, mux => mux.send_prompt(target_pane_id, text))
    }

    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError> {
        dispatch!(self, mux => mux.capture_pane(target_pane_id, lines))
    }

    fn list_pane_ids(&self) -> Result<std::collections::BTreeSet<String>, MultiplexerError> {
        dispatch!(self, mux => mux.list_pane_ids())
    }

    fn kill_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        dispatch!(self, mux => mux.kill_pane(target_pane_id, ignore_missing))
    }
}

// Concrete backends expose the same operations as the selected backend.
macro_rules! implement_backend {
    ($backend:ident) => {
        impl Multiplexer for $backend {
            fn name(&self) -> &'static str {
                $backend::name(self)
            }

            fn ensure_available(&self) -> Result<(), MultiplexerError> {
                $backend::ensure_available(self)
            }

            fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
                $backend::context_discovery(self)
            }

            fn split_window(
                &self,
                reference: &MultiplexerContext,
                env: &[(String, String)],
                command: &[String],
            ) -> Result<String, MultiplexerError> {
                $backend::split_window(self, reference, env, command)
            }

            fn send_inline_preview(
                &self,
                target_pane_id: &str,
                message_id: i64,
                sender_id: i64,
                ts: &str,
                text: &str,
            ) -> Result<(), MultiplexerError> {
                $backend::send_inline_preview(self, target_pane_id, message_id, sender_id, ts, text)
            }

            fn send_prompt(
                &self,
                target_pane_id: &str,
                text: &str,
            ) -> Result<(), MultiplexerError> {
                $backend::send_prompt(self, target_pane_id, text)
            }

            fn capture_pane(
                &self,
                target_pane_id: &str,
                lines: i64,
            ) -> Result<String, MultiplexerError> {
                $backend::capture_pane(self, target_pane_id, lines)
            }

            fn list_pane_ids(
                &self,
            ) -> Result<std::collections::BTreeSet<String>, MultiplexerError> {
                $backend::list_pane_ids(self)
            }

            fn kill_pane(
                &self,
                target_pane_id: &str,
                ignore_missing: bool,
            ) -> Result<(), MultiplexerError> {
                $backend::kill_pane(self, target_pane_id, ignore_missing)
            }
        }
    };
}
implement_backend!(TmuxMultiplexer);
implement_backend!(HerdrMultiplexer);

/// Resolve the backend per the SPEC precedence and construct it over the
/// given runner and environment snapshot.
pub fn resolve_multiplexer(
    override_value: Option<&str>,
    env: std::collections::HashMap<String, String>,
    runner: std::rc::Rc<dyn CommandRunner>,
) -> Result<AnyMultiplexer, MultiplexerError> {
    let name = resolve_multiplexer_name(override_value, |key| env.get(key).cloned())?;
    Ok(match name {
        "herdr" => AnyMultiplexer::Herdr(HerdrMultiplexer::new(runner, env)),
        _ => AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner, env)),
    })
}

/// Backend resolution precedence (SPEC §6.5): an explicit override must name
/// a registry key; otherwise auto-detect from `HERDR_ENV` / `TMUX` (an empty
/// value counts as unset); ambiguity and absence are hard errors.
pub fn resolve_multiplexer_name(
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
mod tests {
    use crate::multiplexer::resolve_multiplexer_name;

    mod resolver_tests {
        use super::*;

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
    }

    mod any_multiplexer_inline_preview_tests {
        use crate::multiplexer::test_support::{FakeRunner, env};
        use crate::multiplexer::{AnyMultiplexer, HerdrMultiplexer, Multiplexer, TmuxMultiplexer};

        const TS: &str = "2026-07-30T09:00:00.000000+00:00";

        #[test]
        fn dispatch_returns_each_backend_inline_preview_result() {
            let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
                FakeRunner::without_binaries(),
                env(&[("TMUX", "/tmp/tmux"), ("TMUX_PANE", "%1")]),
            ));
            assert_eq!(
                mux.send_inline_preview("%5", 5, 2, TS, "hi")
                    .unwrap_err()
                    .to_string(),
                "tmux binary not found on PATH"
            );

            let mux = AnyMultiplexer::Herdr(HerdrMultiplexer::new(
                FakeRunner::without_binaries(),
                env(&[("HERDR_ENV", "1")]),
            ));
            assert_eq!(
                mux.send_inline_preview("w1:p2", 5, 2, TS, "hi")
                    .unwrap_err()
                    .to_string(),
                "herdr binary not found on PATH"
            );

            let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
                FakeRunner::with_binary("tmux"),
                env(&[("TMUX", "/tmp/tmux"), ("TMUX_PANE", "%1")]),
            ));
            assert!(mux.send_inline_preview("%5", 5, 2, TS, "hi").is_ok());
        }
    }
}
