pub mod herdr;
mod keys;
#[cfg(test)]
mod press_tests;
pub mod tmux;
pub(crate) use keys::PressKeys;

use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

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

/// The concrete session, window and pane returned by context discovery.
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

#[derive(Debug, Clone)]
pub struct TimedRunError {
    pub source: RunError,
    pub cleanup_diagnostics: Vec<String>,
}

impl TimedRunError {
    pub(crate) fn before_launch() -> Self {
        Self {
            source: RunError::NotStarted("command deadline expired before launch".into()),
            cleanup_diagnostics: Vec::new(),
        }
    }
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

    fn run_until(&self, argv: &[String], deadline: Instant) -> Result<String, TimedRunError> {
        let deadline = deadline.min(self.deadline.instant());
        if Instant::now() >= deadline {
            return Err(TimedRunError::before_launch());
        }
        self.runner.run_until(argv, deadline)
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

impl AnyMultiplexer {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tmux(mux) => mux.name(),
            Self::Herdr(mux) => mux.name(),
        }
    }

    pub fn ensure_available(&self) -> Result<(), MultiplexerError> {
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

    pub fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.context_discovery(),
            Self::Herdr(mux) => mux.context_discovery(),
        }
    }

    pub fn split_window(
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

    pub fn send_prompt(
        &self,
        target_pane_id: &str,
        text: &str,
        lead: Lead,
    ) -> Result<(), MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.send_prompt(target_pane_id, text, lead),
            Self::Herdr(mux) => mux.send_prompt(target_pane_id, text, lead),
        }
    }

    pub fn capture_pane(
        &self,
        target_pane_id: &str,
        lines: i64,
    ) -> Result<String, MultiplexerError> {
        match self {
            Self::Tmux(mux) => mux.capture_pane(target_pane_id, lines),
            Self::Herdr(mux) => mux.capture_pane(target_pane_id, lines),
        }
    }

    pub fn kill_pane(
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
pub fn resolve_multiplexer(
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
        use crate::multiplexer::recording::{FakeRunner, argv, env};
        use crate::multiplexer::{AnyMultiplexer, HerdrMultiplexer, Lead, TmuxMultiplexer};

        #[test]
        fn dispatch_forwards_the_lead_to_each_backend() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(3);
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
            runner.succeed(2);
            let mux = AnyMultiplexer::Herdr(HerdrMultiplexer::new(
                runner.clone(),
                env(&[("HERDR_ENV", "1")]),
            ));
            mux.send_prompt("w1:p2", "hi", Lead::Direct).unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    argv(&["herdr", "pane", "send-text", "w1:p2", "hi"]),
                    argv(&["herdr", "pane", "send-keys", "w1:p2", "enter"]),
                ]
            );
        }
    }
}

#[cfg(test)]
mod submission_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct BudgetRunner {
        budgets: RefCell<VecDeque<Duration>>,
        calls: RefCell<Vec<(String, Duration)>>,
        sleeps: RefCell<Vec<f64>>,
        failure: Option<(String, RunError)>,
    }

    impl BudgetRunner {
        fn new(budgets: &[u64], failure: Option<(&str, RunError)>) -> Self {
            Self {
                budgets: RefCell::new(budgets.iter().copied().map(Duration::from_millis).collect()),
                calls: RefCell::new(Vec::new()),
                sleeps: RefCell::new(Vec::new()),
                failure: failure.map(|(stage, error)| (stage.into(), error)),
            }
        }
    }

    impl CommandRunner for BudgetRunner {
        fn binary_exists(&self, _: &str) -> bool {
            panic!("submission has already resolved its backend")
        }

        fn run(&self, argv: &[String], timeout: Option<u64>) -> Result<String, RunError> {
            self.run_for(
                argv,
                Duration::from_secs(timeout.expect("Enter is bounded")),
            )
        }

        fn run_for(&self, argv: &[String], timeout: Duration) -> Result<String, RunError> {
            let [stage] = argv else {
                panic!("one stage token is required")
            };
            assert!(matches!(stage.as_str(), "escape" | "literal" | "enter"));
            self.calls.borrow_mut().push((stage.clone(), timeout));
            match &self.failure {
                Some((failed, error)) if stage == failed => Err(error.clone()),
                _ => Ok(String::new()),
            }
        }

        fn sleep(&self, seconds: f64) {
            self.sleeps.borrow_mut().push(seconds);
        }

        fn remaining_budget(&self) -> Result<Option<Duration>, RunError> {
            Ok(Some(
                self.budgets
                    .borrow_mut()
                    .pop_front()
                    .expect("each remaining-budget query is specified"),
            ))
        }
    }

    fn submit(runner: &BudgetRunner, lead: Lead) -> Result<(), MultiplexerError> {
        submit_prompt(
            runner,
            &PaneInput {
                escape: vec!["escape".into()],
                literal: vec!["literal".into()],
                enter: vec!["enter".into()],
            },
            lead,
            |_, _, error| MultiplexerError::new(format!("{error:?}")),
            |cause| cause == "gone",
        )
    }

    #[test]
    fn reserved_budgets_cap_escape_and_recompute_literal_after_settling() {
        let runner = BudgetRunner::new(&[3000, 2500], None);
        submit(&runner, Lead::Escape).unwrap();
        assert_eq!(
            *runner.calls.borrow(),
            [
                ("escape".into(), Duration::from_millis(500)),
                ("literal".into(), Duration::from_millis(500)),
                ("enter".into(), Duration::from_secs(5))
            ]
        );
        assert_eq!(*runner.sleeps.borrow(), [0.5, 1.0]);
        let runner = BudgetRunner::new(&[5000, 5000], None);
        submit(&runner, Lead::Escape).unwrap();
        assert_eq!(runner.calls.borrow()[0].1, Duration::from_secs(1));
        let runner = BudgetRunner::new(&[2500], None);
        assert!(submit(&runner, Lead::Escape).is_err());
        assert!(runner.calls.borrow().is_empty());
        let runner = BudgetRunner::new(&[3000, 2000], None);
        assert!(submit(&runner, Lead::Escape).is_err());
        assert_eq!(runner.calls.borrow().len(), 1);
    }

    #[test]
    fn only_started_literal_or_enter_failures_make_submission_uncertain() {
        for (stage, failure, uncertain, calls) in [
            ("escape", RunError::Timeout, false, 1),
            ("literal", RunError::NotStarted("expired".into()), false, 2),
            (
                "literal",
                RunError::BinaryNotFound("missing".into()),
                false,
                2,
            ),
            (
                "literal",
                RunError::Failed {
                    stderr: "gone".into(),
                },
                false,
                2,
            ),
            ("literal", RunError::Timeout, true, 2),
            (
                "literal",
                RunError::Failed {
                    stderr: "unknown".into(),
                },
                true,
                2,
            ),
            ("enter", RunError::NotStarted("expired".into()), true, 3),
        ] {
            let runner = BudgetRunner::new(&[5000, 4500], Some((stage, failure)));
            assert_eq!(
                submit(&runner, Lead::Escape)
                    .unwrap_err()
                    .submission_started(),
                uncertain
            );
            assert_eq!(runner.calls.borrow().len(), calls);
        }
    }

    #[test]
    fn expired_deadlines_start_no_native_command_or_optional_layout_restore() {
        let runner = recording::FakeRunner::without_binaries();
        let mux = AnyMultiplexer::Herdr(HerdrMultiplexer::new(runner.clone(), recording::env(&[])))
            .bounded(crate::input::Deadline::until(std::time::Instant::now()));
        assert!(
            mux.restore_after_close(PostCloseLayout::HerdrTab("tab".into()))
                .is_err()
        );
        assert!(runner.events().is_empty());
    }

    #[test]
    fn each_backend_classifies_confirmed_missing_panes_before_literal_submission() {
        for backend in ["tmux", "herdr"] {
            let runner = recording::FakeRunner::without_binaries();
            runner.respond(Err(RunError::Failed {
                stderr: if backend == "tmux" {
                    "no such pane: missing".into()
                } else {
                    recording::herdr_error_stderr("pane_not_found")
                },
            }));
            let mux = match backend {
                "tmux" => {
                    AnyMultiplexer::Tmux(TmuxMultiplexer::new(runner.clone(), recording::env(&[])))
                }
                "herdr" => AnyMultiplexer::Herdr(HerdrMultiplexer::new(
                    runner.clone(),
                    recording::env(&[]),
                )),
                _ => unreachable!(),
            };
            assert!(
                !mux.send_prompt("missing", "literal", Lead::Direct)
                    .unwrap_err()
                    .submission_started()
            );
            assert_eq!(runner.run_argvs().len(), 1);
        }
    }
}

#[cfg(test)]
pub(crate) mod recording {
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::rc::Rc;

    use super::{CommandRunner, RunError};

    #[derive(Debug, Clone, PartialEq)]
    pub enum Event {
        Run {
            argv: Vec<String>,
            timeout_secs: Option<u64>,
        },
        Sleep {
            seconds: f64,
        },
    }

    pub struct FakeRunner {
        pub binaries: Vec<String>,
        pub events: RefCell<Vec<Event>>,
        pub responses: RefCell<VecDeque<Result<String, RunError>>>,
    }

    impl FakeRunner {
        fn build(binaries: Vec<String>) -> Rc<Self> {
            Rc::new(FakeRunner {
                binaries,
                events: RefCell::new(Vec::new()),
                responses: RefCell::new(VecDeque::new()),
            })
        }

        pub fn with_binary(name: &str) -> Rc<Self> {
            Self::build(vec![name.to_string()])
        }

        pub fn without_binaries() -> Rc<Self> {
            Self::build(Vec::new())
        }

        pub fn succeed(&self, calls: usize) {
            for _ in 0..calls {
                self.respond(Ok(String::new()));
            }
        }

        pub fn respond(&self, response: Result<String, RunError>) {
            self.responses.borrow_mut().push_back(response);
        }

        pub fn events(&self) -> Vec<Event> {
            self.events.borrow().clone()
        }

        /// Just the argv of each `Run` event, in order.
        pub fn run_argvs(&self) -> Vec<Vec<String>> {
            self.events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Run { argv, .. } => Some(argv.clone()),
                    Event::Sleep { .. } => None,
                })
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

        fn sleep(&self, seconds: f64) {
            self.events.borrow_mut().push(Event::Sleep { seconds });
        }
    }

    pub fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    pub fn run_event(parts: &[&str], timeout_secs: Option<u64>) -> Event {
        Event::Run {
            argv: argv(parts),
            timeout_secs,
        }
    }

    pub fn sleep_event(seconds: f64) -> Event {
        Event::Sleep { seconds }
    }

    pub fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A herdr success envelope wrapping `result`.
    pub fn herdr_envelope(result: serde_json::Value) -> String {
        serde_json::json!({"id": 1, "result": result, "type": "response"}).to_string()
    }

    /// A herdr error `stderr` body carrying `code`, for `RunError::Failed`.
    pub fn herdr_error_stderr(code: &str) -> String {
        serde_json::json!({"error": {"code": code, "message": "boom"}, "id": 1}).to_string()
    }
}
