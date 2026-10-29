use std::collections::HashMap;
use std::rc::Rc;

use super::{CommandRunner, Lead, MultiplexerContext, MultiplexerError, PaneInput, RunError};

const PANE_GONE_MARKERS: [&str; 2] = ["can't find pane", "no such pane"];

fn tmux_argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn is_pane_gone(stderr: &str) -> bool {
    let lowered = stderr.to_lowercase();
    PANE_GONE_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn map_run_error(argv: &[String], timeout_secs: Option<u64>, error: RunError) -> MultiplexerError {
    let joined = argv.join(" ");
    match error {
        RunError::NotStarted(detail) => {
            MultiplexerError::new(format!("tmux command was not started: {detail}: {joined}"))
        }
        RunError::BinaryNotFound(detail) => {
            MultiplexerError::new(format!("tmux binary not found: {detail}"))
        }
        RunError::Timeout => match timeout_secs {
            Some(secs) => {
                MultiplexerError::new(format!("tmux command timed out after {secs}s: {joined}"))
            }
            None => MultiplexerError::new(format!(
                "tmux command timed out at the pane input deadline: {joined}"
            )),
        },
        RunError::Failed { stderr } => MultiplexerError::new(format!(
            "tmux command failed: {joined}\nstderr: {}",
            stderr.trim()
        )),
    }
}

pub(crate) struct TmuxMultiplexer {
    runner: Rc<dyn CommandRunner>,
    env: HashMap<String, String>,
}

impl TmuxMultiplexer {
    pub(crate) fn new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>) -> Self {
        TmuxMultiplexer { runner, env }
    }

    pub(crate) fn bounded(&self, deadline: crate::input::Deadline) -> Self {
        Self {
            runner: super::DeadlineRunner::wrap(self.runner.clone(), deadline),
            env: self.env.clone(),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        "tmux"
    }

    fn env_var(&self, name: &str) -> Option<&str> {
        self.env
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, MultiplexerError> {
        self.runner
            .run(argv, timeout_secs)
            .map_err(|error| map_run_error(argv, timeout_secs, error))
    }

    fn run_tolerating_pane_gone(
        &self,
        argv: &[String],
        ignore_missing: bool,
        timeout_secs: Option<u64>,
    ) -> Result<(), MultiplexerError> {
        match self.runner.run(argv, timeout_secs) {
            Ok(_) => Ok(()),
            Err(RunError::Failed { ref stderr }) if ignore_missing && is_pane_gone(stderr) => {
                Ok(())
            }
            Err(error) => Err(map_run_error(argv, timeout_secs, error)),
        }
    }

    pub(crate) fn ensure_available(&self) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists("tmux") {
            return Err(MultiplexerError::new("tmux binary not found on PATH"));
        }
        if self.env_var("TMUX").is_none() {
            return Err(MultiplexerError::new(
                "cafleet member commands must be run inside a tmux session",
            ));
        }
        Ok(())
    }

    fn press_run(
        &self,
        argv: &[String],
        deadline: crate::input::Deadline,
        writing: bool,
    ) -> Result<String, MultiplexerError> {
        deadline
            .remaining()
            .map_err(|error| MultiplexerError::new(error.message()))?;
        self.runner
            .run_until(argv, deadline.instant())
            .map_err(|error| {
                let started =
                    writing && matches!(&error.source, RunError::Failed { .. } | RunError::Timeout);
                let mut mapped = map_run_error(argv, None, error.source);
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

    pub(crate) fn validate_key_target(
        &self,
        placement: &crate::broker::records::Placement,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        deadline
            .remaining()
            .map_err(|error| MultiplexerError::new(error.message()))?;
        self.ensure_available()?;
        deadline
            .remaining()
            .map_err(|error| MultiplexerError::new(error.message()))?;
        let caller = self.env_var("TMUX_PANE").ok_or_else(|| {
            MultiplexerError::new("TMUX_PANE is not set; not running inside a tmux pane")
        })?;
        let current = self.press_run(
            &tmux_argv(&[
                "tmux",
                "display-message",
                "-p",
                "-t",
                caller,
                "#{session_name}|#{window_id}|#{pane_id}",
            ]),
            deadline,
            false,
        )?;
        let parts: Vec<_> = current.trim().splitn(3, '|').collect();
        if !matches!(parts.as_slice(), [session, window, pane] if !session.is_empty() && !window.is_empty() && !pane.is_empty() && *session == placement.mux_session)
        {
            return Err(MultiplexerError::new(
                "press target context does not match the recorded placement",
            ));
        }
        let pane = placement
            .mux_pane_id
            .as_deref()
            .ok_or_else(|| MultiplexerError::new("press target has no pane"))?;
        let target = self.press_run(
            &tmux_argv(&[
                "tmux",
                "display-message",
                "-p",
                "-t",
                pane,
                "#{pane_id}|#{window_id}|#{session_name}",
            ]),
            deadline,
            false,
        )?;
        let parts: Vec<_> = target.trim().splitn(3, '|').collect();
        if parts
            != [
                pane,
                placement.mux_window_id.as_str(),
                placement.mux_session.as_str(),
            ]
        {
            return Err(MultiplexerError::new(
                "press target context does not match the recorded placement",
            ));
        }
        Ok(())
    }

    pub(crate) fn send_keys(
        &self,
        pane_id: &str,
        keys: &super::PressKeys,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        let mut argv = tmux_argv(&["tmux", "send-keys", "-t", pane_id]);
        argv.extend(keys.tmux_arguments());
        let output = self.press_run(&argv, deadline, true)?;
        if !output.trim().is_empty() {
            return Err(
                MultiplexerError::new("tmux key acknowledgment was unreadable").after_write(),
            );
        }
        Ok(())
    }

    pub(crate) fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        let Some(tmux_pane) = self.env_var("TMUX_PANE") else {
            return Err(MultiplexerError::new(
                "TMUX_PANE is not set; not running inside a tmux pane",
            ));
        };
        let out = self.run(
            &tmux_argv(&[
                "tmux",
                "display-message",
                "-p",
                "-t",
                tmux_pane,
                "#{session_name}|#{window_id}|#{pane_id}",
            ]),
            None,
        )?;
        let parts: Vec<&str> = out.trim().splitn(3, '|').collect();
        let [session, window_id, pane_id] = parts.as_slice() else {
            return Err(MultiplexerError::new(format!(
                "unexpected tmux display-message output: {out:?}"
            )));
        };
        Ok(MultiplexerContext {
            session: session.to_string(),
            window_id: window_id.to_string(),
            pane_id: pane_id.to_string(),
        })
    }

    /// Split `reference.window_id` detached (`-d`) and return the new pane
    /// id; the post-split `main-vertical` reflow is best-effort.
    pub(crate) fn split_window(
        &self,
        reference: &MultiplexerContext,
        env: &[(String, String)],
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        let mut args = tmux_argv(&[
            "tmux",
            "split-window",
            "-t",
            &reference.window_id,
            "-P",
            "-F",
            "#{pane_id}",
            "-d",
        ]);
        for (key, value) in env {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }
        args.extend(command.iter().cloned());
        let pane_id = self
            .run(&args, None)
            .map_err(|error| error.with_pane_cleanup(super::PaneCleanup::Unknown))?
            .trim()
            .to_string();
        if pane_id.is_empty() {
            return Err(
                MultiplexerError::new("tmux split-window returned an empty pane ID")
                    .with_pane_cleanup(super::PaneCleanup::Unknown),
            );
        }
        let _ = self.run(
            &tmux_argv(&[
                "tmux",
                "select-layout",
                "-t",
                &reference.window_id,
                "main-vertical",
            ]),
            None,
        );
        Ok(pane_id)
    }

    pub(crate) fn send_prompt(
        &self,
        target_pane_id: &str,
        text: &str,
        lead: Lead,
    ) -> Result<(), MultiplexerError> {
        if !crate::input::valid_input(text) {
            return Err(MultiplexerError::new(
                "send_prompt: text must be one non-empty line without control characters",
            ));
        }
        let bounded = self.bounded(crate::input::Deadline::new());
        super::submit_prompt(
            bounded.runner.as_ref(),
            &PaneInput {
                escape: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Escape"]),
                literal: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "-l", "--", text]),
                enter: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Enter"]),
            },
            lead,
            map_run_error,
            is_pane_gone,
        )
    }

    /// Capture the last `lines` drawn lines of the pane buffer via the shared
    /// capture windowing ([`super::capture_window`]).
    pub(crate) fn capture_pane(
        &self,
        target_pane_id: &str,
        lines: i64,
    ) -> Result<String, MultiplexerError> {
        if lines <= 0 {
            return Err(MultiplexerError::new(format!(
                "capture_pane: lines must be positive, got {lines}"
            )));
        }
        let fetch_depth = lines + super::CAPTURE_OVER_FETCH_LINES;
        let raw = self.run(
            &tmux_argv(&[
                "tmux",
                "capture-pane",
                "-p",
                "-e",
                "-t",
                target_pane_id,
                "-S",
                &format!("-{fetch_depth}"),
            ]),
            None,
        )?;
        Ok(super::capture_window(&raw, lines))
    }

    pub(crate) fn kill_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        self.run_tolerating_pane_gone(
            &tmux_argv(&["tmux", "kill-pane", "-t", target_pane_id]),
            ignore_missing,
            None,
        )
    }
}

#[cfg(test)]
mod tests {

    use crate::multiplexer::test_support::{FakeRunner, argv, env, run_event, sleep_event};
    use crate::multiplexer::{Lead, MultiplexerContext, RunError, TmuxMultiplexer};

    fn tmux_env() -> std::collections::HashMap<String, String> {
        env(&[
            ("TMUX", "/tmp/tmux-1000/default,123,0"),
            ("TMUX_PANE", "%1"),
        ])
    }

    fn reference() -> MultiplexerContext {
        MultiplexerContext {
            session: "main".to_string(),
            window_id: "@1".to_string(),
            pane_id: "%1".to_string(),
        }
    }

    #[test]
    fn ensure_available_requires_the_binary_then_the_tmux_env() {
        let runner = FakeRunner::without_binaries();
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.ensure_available().unwrap_err().to_string(),
            "tmux binary not found on PATH"
        );

        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner, env(&[]));
        assert_eq!(
            mux.ensure_available().unwrap_err().to_string(),
            "cafleet member commands must be run inside a tmux session"
        );

        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner, env(&[("TMUX", "")]));
        assert_eq!(
            mux.ensure_available().unwrap_err().to_string(),
            "cafleet member commands must be run inside a tmux session"
        );

        let runner = FakeRunner::with_binary("tmux");
        assert!(
            TmuxMultiplexer::new(runner, tmux_env())
                .ensure_available()
                .is_ok()
        );
    }

    #[test]
    fn context_discovery_requires_tmux_pane() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner, env(&[("TMUX", "/tmp/tmux")]));
        assert_eq!(
            mux.context_discovery().unwrap_err().to_string(),
            "TMUX_PANE is not set; not running inside a tmux pane"
        );
    }

    #[test]
    fn context_discovery_resolves_the_calling_pane() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("main|@1|%1\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let context = mux.context_discovery().unwrap();
        assert_eq!(context.session, "main");
        assert_eq!(context.window_id, "@1");
        assert_eq!(context.pane_id, "%1");
        assert_eq!(
            runner.events(),
            vec![run_event(
                &[
                    "tmux",
                    "display-message",
                    "-p",
                    "-t",
                    "%1",
                    "#{session_name}|#{window_id}|#{pane_id}",
                ],
                None,
            )]
        );
    }

    #[test]
    fn context_discovery_splits_on_the_first_two_pipes_only() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("se|ss|@1|%1\n".to_string()));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        let context = mux.context_discovery().unwrap();
        assert_eq!(context.session, "se");
        assert_eq!(context.window_id, "ss");
        assert_eq!(context.pane_id, "@1|%1");
    }

    #[test]
    fn context_discovery_rejects_a_malformed_reply() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("only|two\n".to_string()));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        let err = mux.context_discovery().unwrap_err();
        assert!(
            err.to_string()
                .starts_with("unexpected tmux display-message output:"),
            "got: {err}"
        );
    }

    #[test]
    fn split_window_builds_the_detached_argv_and_swallows_layout_errors() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("%7\n".to_string()));
        runner.respond(Err(RunError::Failed {
            stderr: "layout boom".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let pane = mux
            .split_window(
                &reference(),
                &[(
                    "CAFLEET_DATABASE_URL".to_string(),
                    "sqlite:///x.db".to_string(),
                )],
                &argv(&["claude", "--name", "worker"]),
            )
            .unwrap();
        assert_eq!(pane, "%7", "the layout failure never breaks the spawn");
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&[
                    "tmux",
                    "split-window",
                    "-t",
                    "@1",
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "-d",
                    "-e",
                    "CAFLEET_DATABASE_URL=sqlite:///x.db",
                    "claude",
                    "--name",
                    "worker",
                ]),
                argv(&["tmux", "select-layout", "-t", "@1", "main-vertical"]),
            ]
        );
    }

    #[test]
    fn send_prompt_preserves_literal_text_and_settles_before_enter() {
        let runner = FakeRunner::with_binary("tmux");
        runner.succeed(2);
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let text = "  /help with literal $HOME; !  ";
        mux.send_prompt("%5", text, Lead::Direct).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "-l", "--", text]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        let events = runner.events();
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[1], sleep_event(1.0));
        for event in events {
            if let crate::multiplexer::test_support::Event::Run { timeout_secs, .. } = event {
                assert!(timeout_secs.is_some_and(|seconds| seconds > 0 && seconds <= 5));
            }
        }
    }

    #[test]
    fn escape_lead_sends_the_escape_key_and_settles_before_the_literal() {
        let runner = FakeRunner::with_binary("tmux");
        runner.succeed(3);
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let text = "[cafleet msg 5 from 2 timestamp] hello";
        mux.send_prompt("%5", text, Lead::Escape).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "Escape"]),
                argv(&["tmux", "send-keys", "-t", "%5", "-l", "--", text]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        let events = runner.events();
        assert_eq!(events.len(), 5, "{events:?}");
        assert_eq!(
            events[0],
            run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], Some(1))
        );
        assert_eq!(events[1], sleep_event(0.5));
        assert_eq!(events[3], sleep_event(1.0));
    }

    #[test]
    fn escape_failure_propagates_cause_without_literal_input() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "injected escape failure".into(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux.send_prompt("%5", "hello", Lead::Escape).unwrap_err();
        assert!(
            error.to_string().contains("injected escape failure"),
            "{error}"
        );
        assert!(!error.submission_started(), "{error}");
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&["tmux", "send-keys", "-t", "%5", "Escape"])]
        );
        assert_eq!(runner.events().len(), 1);
    }

    #[test]
    fn first_write_failure_propagates_cause_without_enter_or_retry() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "injected first write failure".into(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux.send_prompt("%5", "hello", Lead::Direct).unwrap_err();
        assert!(
            error.to_string().contains("injected first write failure"),
            "{error}"
        );
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&[
                "tmux",
                "send-keys",
                "-t",
                "%5",
                "-l",
                "--",
                "hello"
            ])]
        );
        assert_eq!(runner.events().len(), 1);
    }

    #[test]
    fn lost_enter_propagates_cause_after_one_literal_write() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok(String::new()));
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "injected enter failure".into(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux.send_prompt("%5", "hello", Lead::Escape).unwrap_err();
        assert!(
            error.to_string().contains("injected enter failure"),
            "{error}"
        );
        assert!(error.submission_started(), "{error}");
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "Escape"]),
                argv(&["tmux", "send-keys", "-t", "%5", "-l", "--", "hello"]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        assert_eq!(runner.events().len(), 5);
    }

    #[test]
    fn prompt_line_validation_precedes_transport() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        for lead in [Lead::Escape, Lead::Direct] {
            for text in ["", "  ", "a\nb", "a\rb", "a\tb", "a\0b", "a\x1bb", "a\x7fb"] {
                assert!(mux.send_prompt("%5", text, lead).is_err(), "{text:?}");
            }
        }
        assert!(runner.events().is_empty());
    }

    #[test]
    fn capture_pane_rejects_non_positive_lines() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 0).unwrap_err().to_string(),
            "capture_pane: lines must be positive, got 0"
        );
    }

    #[test]
    fn capture_pane_keeps_the_last_lines_after_dropping_the_blank_tail() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("l1\nl2\nl3\nx\rY\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let captured = mux.capture_pane("%5", 2).unwrap();
        assert_eq!(
            captured, "l3\nx\rY",
            "split on \\n only — the \\r survives for CLI-side defrag; \
             the trailing blank line is dropped before the slice"
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &[
                    "tmux",
                    "capture-pane",
                    "-p",
                    "-e",
                    "-t",
                    "%5",
                    "-S",
                    "-1002"
                ],
                None,
            )],
            "Capture over-fetches by the fixed 1000-line margin"
        );
    }

    #[test]
    fn capture_pane_over_fetch_survives_a_blank_tail_deeper_than_n() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("drawn1\ndrawn2\n\n\n\n\n\n\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 2).unwrap(),
            "drawn1\ndrawn2",
            "a depth-N fetch window would have been all-blank here"
        );
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&[
                "tmux",
                "capture-pane",
                "-p",
                "-e",
                "-t",
                "%5",
                "-S",
                "-1002"
            ])]
        );
    }

    #[test]
    fn capture_pane_drops_trailing_whitespace_only_lines_before_the_slice() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("scrollback\nprompt>\ncomposer\n \n\t\n\n\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 2).unwrap(),
            "prompt>\ncomposer",
            "spaces, tabs, and empty lines all count as a blank tail"
        );
    }

    #[test]
    fn capture_pane_keeps_interior_blank_lines() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("a\n\nb\n\n\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 3).unwrap(),
            "a\n\nb",
            "only the trailing blank run is dropped"
        );
    }

    #[test]
    fn capture_pane_blank_detection_is_ansi_aware() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok(
            "top\nkept \x1b[31mred\x1b[0m\n\x1b[39m\x1b[49m\n\x1b[2K \n\n".to_string(),
        ));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 2).unwrap(),
            "top\nkept \x1b[31mred\x1b[0m",
            "CSI-only trailing rows are visually blank and dropped; the kept \
             lines' ANSI survives untouched"
        );
    }

    #[test]
    fn capture_pane_of_an_all_blank_buffer_is_empty() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("\n \n\n".to_string()));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(mux.capture_pane("%5", 2).unwrap(), "");
    }

    #[test]
    fn kill_pane_tolerates_only_pane_gone_failures() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "No Such Pane: %5".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        mux.kill_pane("%5", true).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&["tmux", "kill-pane", "-t", "%5"])],
            "the marker match is case-insensitive"
        );

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "server exited unexpectedly".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert!(
            mux.kill_pane("%5", true).is_err(),
            "ignore_missing never swallows a non-pane-gone failure"
        );
    }

    #[test]
    fn creation_unknown_id_failure_never_guesses_or_closes_a_pane() {
        for response in [
            Ok("   \n".into()),
            Err(RunError::Failed {
                stderr: "split failed".into(),
            }),
        ] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(response);
            let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
            let error = mux
                .split_window(&reference(), &[], &argv(&["claude"]))
                .unwrap_err();
            assert!(matches!(
                error.pane_cleanup(),
                Some(crate::multiplexer::PaneCleanup::Unknown)
            ));
            assert!(
                error
                    .to_string()
                    .contains("pane ID unknown; pane cleanup unconfirmed")
            );
            drop(mux);
            let calls = runner.run_argvs();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0][1], "split-window");
        }
    }
}
