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
pub(crate) const ESCAPE_SETTLE_DELAY: f64 = 0.5;
pub(crate) const ESCAPE_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const MIN_LITERAL_BUDGET: Duration = Duration::from_millis(500);
pub(crate) const REVALIDATION_MARGIN: Duration = Duration::from_millis(500);
pub(crate) const INPUT_RESERVATION: Duration = REVALIDATION_MARGIN
    .saturating_add(Duration::from_millis((ESCAPE_SETTLE_DELAY * 1000.0) as u64))
    .saturating_add(MIN_LITERAL_BUDGET)
    .saturating_add(Duration::from_millis((SUBMIT_DELAY * 1000.0) as u64))
    .saturating_add(Duration::from_secs(1));
pub(crate) const INSUFFICIENT_INPUT_BUDGET: &str =
    "insufficient pane input budget for literal input, settle and Enter";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lead {
    Escape,
    Direct,
}

impl Lead {
    /// The settle and Enter time a submission keeps back from its literal input.
    pub(crate) fn reserved(self) -> Duration {
        let submit = Duration::from_secs_f64(SUBMIT_DELAY) + Duration::from_secs(1);
        match self {
            Lead::Escape => submit + Duration::from_secs_f64(ESCAPE_SETTLE_DELAY),
            Lead::Direct => submit,
        }
    }
}

#[cfg(test)]
mod submission_regressions {
    use super::test_support::{Event, FakeRunner, env, herdr_error_stderr, sleep_event};
    use super::*;

    const INSUFFICIENT_BUDGET: &str =
        "insufficient pane input budget for literal input, settle and Enter";

    fn muxes(runner: Rc<dyn CommandRunner>) -> [AnyMultiplexer; 2] {
        [
            AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), env(&[]))),
            AnyMultiplexer::Herdr(HerdrMultiplexer::new(runner, env(&[]))),
        ]
    }

    fn escape_argv(backend: usize) -> Vec<String> {
        let parts: &[&str] = match backend {
            0 => &["tmux", "send-keys", "-t", "pane", "Escape"],
            _ => &["herdr", "pane", "send-keys", "pane", "escape"],
        };
        super::test_support::argv(parts)
    }

    fn mux_for(backend: usize, runner: Rc<FakeRunner>) -> AnyMultiplexer {
        muxes(runner).into_iter().nth(backend).unwrap()
    }

    #[test]
    fn transport_constants_match_the_reserved_budget() {
        assert_eq!(ESCAPE_SETTLE_DELAY, 0.5);
        assert_eq!(ESCAPE_TIMEOUT, Duration::from_secs(1));
        assert_eq!(MIN_LITERAL_BUDGET, Duration::from_millis(500));
        assert_eq!(REVALIDATION_MARGIN, Duration::from_millis(500));
        assert_eq!(INPUT_RESERVATION, Duration::from_millis(3500));
        assert_eq!(
            REVALIDATION_MARGIN
                + Duration::from_secs_f64(ESCAPE_SETTLE_DELAY)
                + MIN_LITERAL_BUDGET
                + Duration::from_secs_f64(SUBMIT_DELAY)
                + Duration::from_secs(1),
            INPUT_RESERVATION
        );
    }

    #[test]
    fn escape_lead_presses_escape_settles_then_types_the_literal_and_enter() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries();
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap();
            let events = runner.events();
            assert_eq!(events.len(), 5, "{events:?}");
            assert!(
                matches!(&events[0], Event::Run { argv, .. } if *argv == escape_argv(backend)),
                "{events:?}"
            );
            assert_eq!(events[1], sleep_event(ESCAPE_SETTLE_DELAY));
            assert!(
                matches!(&events[2], Event::Run { argv, .. } if argv.last().unwrap() == "assignment"),
                "{events:?}"
            );
            assert_eq!(events[3], sleep_event(SUBMIT_DELAY));
            assert!(
                matches!(&events[4], Event::Run { argv, .. } if argv.last().unwrap().eq_ignore_ascii_case("enter")),
                "{events:?}"
            );
        }
    }

    #[test]
    fn direct_lead_types_the_literal_and_enter_without_escape() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries();
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Direct)
                .unwrap();
            let events = runner.events();
            assert_eq!(events.len(), 3, "{events:?}");
            assert!(
                matches!(&events[0], Event::Run { argv, .. } if argv.last().unwrap() == "assignment"),
                "{events:?}"
            );
            assert_eq!(events[1], sleep_event(SUBMIT_DELAY));
            assert!(
                !runner
                    .run_argvs()
                    .iter()
                    .any(|argv| argv.last().unwrap().eq_ignore_ascii_case("escape")),
                "{events:?}"
            );
        }
    }

    #[test]
    fn the_escape_timeout_is_capped_by_the_escape_timeout_constant() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries().with_budget(Duration::from_secs(4));
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap();
            let timeouts = runner.run_timeouts();
            assert_eq!(timeouts[0].0, escape_argv(backend));
            assert_eq!(timeouts[0].1, ESCAPE_TIMEOUT);
        }
    }

    #[test]
    fn the_escape_timeout_is_capped_by_the_budget_left_after_the_reservation() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries().with_budget(Duration::from_secs(3));
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap();
            let timeouts = runner.run_timeouts();
            assert_eq!(timeouts[0].0, escape_argv(backend));
            assert_eq!(
                timeouts[0].1,
                Duration::from_millis(500),
                "3 s remaining minus the 2.5 s Escape reservation"
            );
        }
    }

    #[test]
    fn the_literal_budget_is_recomputed_after_the_escape_settle() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries().with_budget(Duration::from_secs(4));
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap();
            let timeouts = runner.run_timeouts();
            assert_eq!(timeouts[1].0.last().unwrap(), "assignment");
            assert_eq!(
                timeouts[1].1,
                Duration::from_millis(1500),
                "4 s − 0.5 s settle − (SUBMIT_DELAY + 1 s)"
            );
        }
    }

    #[test]
    fn escape_failures_type_no_literal_and_are_not_started() {
        for backend in 0..2 {
            for failure in [
                RunError::NotStarted("deadline expired before launch".into()),
                RunError::BinaryNotFound("missing executable".into()),
                RunError::Timeout,
                RunError::Failed {
                    stderr: "connection lost".into(),
                },
                RunError::Failed {
                    stderr: "can't find pane: pane".into(),
                },
                RunError::Failed {
                    stderr: herdr_error_stderr("pane_not_found"),
                },
            ] {
                let runner = FakeRunner::without_binaries();
                runner.respond(Err(failure.clone()));
                let error = mux_for(backend, runner.clone())
                    .send_prompt("pane", "assignment", Lead::Escape)
                    .unwrap_err();
                assert!(!error.submission_started(), "{failure:?}: {error}");
                assert_eq!(
                    runner.run_argvs(),
                    vec![escape_argv(backend)],
                    "{failure:?}: no literal input after a failed Escape"
                );
                assert_eq!(runner.events().len(), 1, "{failure:?}: no settle either");
            }
        }
    }

    #[test]
    fn insufficient_budget_for_the_escape_reservation_starts_no_process() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries().with_budget(Duration::from_millis(2400));
            let error = mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap_err();
            assert_eq!(error.to_string(), INSUFFICIENT_BUDGET);
            assert!(!error.submission_started());
            assert!(runner.events().is_empty(), "{:?}", runner.events());

            let runner = FakeRunner::without_binaries().with_budget(Duration::from_millis(2400));
            mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Direct)
                .unwrap();
            assert_eq!(
                runner.run_argvs().len(),
                2,
                "the same budget covers a Direct submission without the Escape settle"
            );
        }
    }

    #[test]
    fn a_slow_escape_that_consumes_the_slack_starts_no_literal() {
        for backend in 0..2 {
            let runner = FakeRunner::without_binaries().with_budget(Duration::from_secs(3));
            runner.cost_next_run(Duration::from_millis(600));
            let error = mux_for(backend, runner.clone())
                .send_prompt("pane", "assignment", Lead::Escape)
                .unwrap_err();
            assert_eq!(error.to_string(), INSUFFICIENT_BUDGET);
            assert!(!error.submission_started());
            assert_eq!(runner.run_argvs(), vec![escape_argv(backend)]);
        }
    }

    #[test]
    fn insufficient_settle_and_enter_budget_starts_no_literal_input() {
        for lead in [Lead::Escape, Lead::Direct] {
            let runner = FakeRunner::without_binaries();
            for mux in muxes(runner.clone()) {
                let deadline = crate::input::Deadline::until(
                    std::time::Instant::now() + Duration::from_millis(500),
                );
                let error = mux
                    .bounded(deadline)
                    .send_prompt("pane", "assignment", lead)
                    .unwrap_err();
                assert!(!error.submission_started());
                assert!(runner.events().is_empty());
            }
        }
    }

    #[test]
    fn confirmed_missing_panes_report_no_submission() {
        for (index, stderr) in [
            "can't find pane: pane".to_owned(),
            herdr_error_stderr("pane_not_found"),
        ]
        .into_iter()
        .enumerate()
        {
            let runner = FakeRunner::without_binaries();
            runner.respond(Err(RunError::Failed { stderr }));
            let mux = muxes(runner.clone()).into_iter().nth(index).unwrap();
            let error = mux
                .send_prompt("pane", "assignment", Lead::Direct)
                .unwrap_err();
            assert!(!error.submission_started(), "{error}");
            assert_eq!(runner.run_argvs().len(), 1);
        }
    }

    #[test]
    fn option_like_prompt_text_follows_the_literal_option_terminator() {
        for mux_index in 0..2 {
            for (lead, literal_index) in [(Lead::Direct, 0), (Lead::Escape, 1)] {
                let runner = FakeRunner::without_binaries();
                let mux = muxes(runner.clone()).into_iter().nth(mux_index).unwrap();
                mux.send_prompt("pane", "-R --help --target=other", lead)
                    .unwrap();
                let calls = runner.run_argvs();
                let literal = &calls[literal_index];
                assert_eq!(
                    &literal[literal.len() - 2..],
                    ["--", "-R --help --target=other"]
                );
                assert_eq!(calls.len(), literal_index + 2);
            }
        }
    }

    #[test]
    fn literal_failures_after_a_successful_escape_keep_the_existing_classification() {
        for backend in 0..2 {
            for (failure, started) in [
                (
                    RunError::NotStarted("deadline expired before launch".into()),
                    false,
                ),
                (RunError::Timeout, true),
                (
                    RunError::Failed {
                        stderr: "connection lost".into(),
                    },
                    true,
                ),
            ] {
                let runner = FakeRunner::without_binaries();
                runner.respond(Ok(String::new()));
                runner.respond(Err(failure.clone()));
                let error = mux_for(backend, runner.clone())
                    .send_prompt("pane", "assignment", Lead::Escape)
                    .unwrap_err();
                assert_eq!(error.submission_started(), started, "{failure:?}: {error}");
                assert_eq!(runner.run_argvs().len(), 2);
            }
        }
    }

    #[test]
    fn launch_failures_are_not_sent_and_started_write_or_enter_failures_are_uncertain() {
        for backend in 0..2 {
            for (failure, started) in [
                (
                    RunError::NotStarted("deadline expired before launch".into()),
                    false,
                ),
                (RunError::BinaryNotFound("missing executable".into()), false),
                (RunError::Timeout, true),
                (
                    RunError::Failed {
                        stderr: "connection lost".into(),
                    },
                    true,
                ),
            ] {
                let runner = FakeRunner::without_binaries();
                runner.respond(Err(failure));
                let error = muxes(runner.clone())
                    .into_iter()
                    .nth(backend)
                    .unwrap()
                    .send_prompt("pane", "assignment", Lead::Direct)
                    .unwrap_err();
                assert_eq!(error.submission_started(), started, "{error}");
                assert_eq!(runner.run_argvs().len(), 1);
            }
            let runner = FakeRunner::without_binaries();
            runner.respond(Ok(String::new()));
            runner.respond(Err(RunError::NotStarted(
                "Enter deadline expired before launch".into(),
            )));
            let error = muxes(runner.clone())
                .into_iter()
                .nth(backend)
                .unwrap()
                .send_prompt("pane", "assignment", Lead::Direct)
                .unwrap_err();
            assert!(
                error.submission_started(),
                "the literal input already arrived: {error}"
            );
        }
    }

    #[test]
    fn expired_optional_layout_returns_a_diagnostic_without_starting_another_process() {
        let runner = FakeRunner::without_binaries();
        let mux = muxes(runner.clone())
            .into_iter()
            .nth(1)
            .unwrap()
            .bounded(crate::input::Deadline::until(std::time::Instant::now()));
        let error = mux
            .restore_after_close(PostCloseLayout::HerdrTab("tab".into()))
            .unwrap_err();
        assert!(error.to_string().contains("deadline"));
        assert!(runner.events().is_empty());
    }
}

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
    NotStarted(String),
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
    fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
        Ok(None)
    }
}

pub(crate) struct PaneInput {
    pub escape: Vec<String>,
    pub literal: Vec<String>,
    pub enter: Vec<String>,
}

fn whole_seconds(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

fn submit_prompt(
    runner: &dyn CommandRunner,
    input: &PaneInput,
    lead: Lead,
    map_error: fn(&[String], Option<u64>, RunError) -> MultiplexerError,
    pane_gone: fn(&str) -> bool,
) -> Result<(), MultiplexerError> {
    let PaneInput {
        escape,
        literal,
        enter,
    } = input;
    let unreserved = |reserved: Duration| {
        runner
            .remaining_budget()
            .map_err(|error| map_error(literal, None, error))?
            .expect("submission uses a deadline-bounded runner")
            .checked_sub(reserved)
            .filter(|budget| !budget.is_zero())
            .ok_or_else(|| MultiplexerError::new(INSUFFICIENT_INPUT_BUDGET))
    };
    let slack = unreserved(lead.reserved())?;
    if lead == Lead::Escape {
        let timeout = slack.min(ESCAPE_TIMEOUT);
        runner
            .run_for(escape, timeout)
            .map_err(|error| map_error(escape, Some(whole_seconds(timeout)), error))?;
        runner.sleep(ESCAPE_SETTLE_DELAY);
    }
    let literal_budget = unreserved(Lead::Direct.reserved())?;
    runner.run_for(literal, literal_budget).map_err(|error| {
        let not_sent = match &error {
            RunError::BinaryNotFound(_) | RunError::NotStarted(_) => true,
            RunError::Failed { stderr } => pane_gone(stderr),
            RunError::Timeout => false,
        };
        let error = map_error(literal, Some(whole_seconds(literal_budget)), error);
        if not_sent { error } else { error.after_write() }
    })?;
    runner.sleep(SUBMIT_DELAY);
    runner
        .run(enter, Some(5))
        .map_err(|error| map_error(enter, Some(5), error).after_write())?;
    Ok(())
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

    fn sleep(&self, seconds: f64) {
        if let Ok(remaining) = self.deadline.remaining() {
            self.runner.sleep(
                Duration::from_secs_f64(seconds)
                    .min(remaining)
                    .as_secs_f64(),
            );
        }
    }

    fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
        let remaining = self
            .deadline
            .remaining()
            .map_err(|_| RunError::NotStarted("command deadline expired before launch".into()))?;
        Ok(Some(
            self.runner
                .remaining_budget()?
                .map_or(remaining, |inner| inner.min(remaining)),
        ))
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
    fn send_prompt(
        &self,
        target_pane_id: &str,
        text: &str,
        lead: Lead,
    ) -> Result<(), MultiplexerError>;
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

    fn send_prompt(
        &self,
        target_pane_id: &str,
        text: &str,
        lead: Lead,
    ) -> Result<(), MultiplexerError> {
        dispatch!(self, mux => mux.send_prompt(target_pane_id, text, lead))
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

            fn send_prompt(
                &self,
                target_pane_id: &str,
                text: &str,
                lead: Lead,
            ) -> Result<(), MultiplexerError> {
                $backend::send_prompt(self, target_pane_id, text, lead)
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

    mod any_multiplexer_send_prompt_tests {
        use crate::multiplexer::test_support::{FakeRunner, argv, env};
        use crate::multiplexer::{
            AnyMultiplexer, HerdrMultiplexer, Lead, Multiplexer, TmuxMultiplexer,
        };

        #[test]
        fn dispatch_forwards_the_lead_to_each_backend() {
            let runner = FakeRunner::with_binary("tmux");
            let mux = AnyMultiplexer::Tmux(TmuxMultiplexer::new(
                runner.clone(),
                env(&[("TMUX", "/tmp/tmux"), ("TMUX_PANE", "%1")]),
            ));
            mux.send_prompt("%5", "hi", Lead::Escape).unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    argv(&["tmux", "send-keys", "-t", "%5", "Escape"]),
                    argv(&["tmux", "send-keys", "-t", "%5", "-l", "--", "hi"]),
                    argv(&["tmux", "send-keys", "-t", "%5", "Enter"]),
                ]
            );

            let runner = FakeRunner::with_binary("herdr");
            let mux = AnyMultiplexer::Herdr(HerdrMultiplexer::new(
                runner.clone(),
                env(&[("HERDR_ENV", "1")]),
            ));
            mux.send_prompt("w1:p2", "hi", Lead::Direct).unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    argv(&["herdr", "pane", "send-text", "w1:p2", "--", "hi"]),
                    argv(&["herdr", "pane", "send-keys", "w1:p2", "enter"]),
                ]
            );
        }
    }
}
