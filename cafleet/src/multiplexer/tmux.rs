use std::collections::HashMap;
use std::rc::Rc;

use super::{
    BackendCore, CommandRunner, Lead, MultiplexerContext, MultiplexerError, PLACEMENT_MISMATCH,
    PaneInput,
};

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

pub(crate) struct TmuxMultiplexer {
    core: BackendCore,
}

impl TmuxMultiplexer {
    pub(crate) fn new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>) -> Self {
        TmuxMultiplexer {
            core: BackendCore {
                runner,
                env,
                name: "tmux",
                session_variable: "TMUX",
                pane_gone: is_pane_gone,
            },
        }
    }

    pub(crate) fn bounded(&self, deadline: crate::input::Deadline) -> Self {
        Self {
            core: self.core.bounded(deadline),
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        self.core.name
    }

    pub(crate) fn ensure_available(&self) -> Result<(), MultiplexerError> {
        self.core.ensure_available()
    }

    /// The caller pane's session, window and pane, read through `run`.
    fn caller_context(
        &self,
        run: impl Fn(&[String]) -> Result<String, MultiplexerError>,
    ) -> Result<(String, Option<MultiplexerContext>), MultiplexerError> {
        let caller = self.core.env_var("TMUX_PANE").ok_or_else(|| {
            MultiplexerError::new("TMUX_PANE is not set; not running inside a tmux pane")
        })?;
        let out = run(&tmux_argv(&[
            "tmux",
            "display-message",
            "-p",
            "-t",
            caller,
            "#{session_name}|#{window_id}|#{pane_id}",
        ]))?;
        let parts: Vec<&str> = out.trim().splitn(3, '|').collect();
        let context = match parts.as_slice() {
            [session, window_id, pane_id] => Some(MultiplexerContext {
                session: session.to_string(),
                window_id: window_id.to_string(),
                pane_id: pane_id.to_string(),
            }),
            _ => None,
        };
        Ok((out, context))
    }

    pub(crate) fn validate_key_target(
        &self,
        placement: &crate::broker::records::Placement,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        super::check_deadline(deadline)?;
        self.ensure_available()?;
        super::check_deadline(deadline)?;
        let (_, current) =
            self.caller_context(|argv| self.core.press_run(argv, deadline, false))?;
        if !current.is_some_and(|context| {
            !context.session.is_empty()
                && !context.window_id.is_empty()
                && !context.pane_id.is_empty()
                && context.session == placement.mux_session
        }) {
            return Err(MultiplexerError::new(PLACEMENT_MISMATCH));
        }
        let pane = placement
            .mux_pane_id
            .as_deref()
            .ok_or_else(|| MultiplexerError::new("press target has no pane"))?;
        let target = self.core.press_run(
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
            return Err(MultiplexerError::new(PLACEMENT_MISMATCH));
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
        self.core.send_keys(&argv, deadline)
    }

    pub(crate) fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        let (out, context) = self.caller_context(|argv| self.core.run(argv, None))?;
        context.ok_or_else(|| {
            MultiplexerError::new(format!("unexpected tmux display-message output: {out:?}"))
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
            .core
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
        let _ = self.core.run(
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
        self.core.send_prompt(
            text,
            &PaneInput {
                escape: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Escape"]),
                literal: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "-l", "--", text]),
                enter: tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Enter"]),
            },
            lead,
        )
    }

    /// Capture the last `lines` drawn lines of the pane buffer via the shared
    /// capture windowing ([`super::capture_window`]).
    pub(crate) fn capture_pane(
        &self,
        target_pane_id: &str,
        lines: i64,
    ) -> Result<String, MultiplexerError> {
        super::check_capture_lines(lines)?;
        let fetch_depth = lines + super::CAPTURE_OVER_FETCH_LINES;
        let raw = self.core.run(
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
        self.core.run_tolerating_missing(
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
