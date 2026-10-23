//! tmux backend (SPEC §6.5 *TmuxMultiplexer*) — exact argv forms, the
//! Esc-first matrix, the two keystroke delays, capture semantics, pane-gone
//! tolerance, and the best-effort-vs-fail-fast split. The colocated tests pin
//! the contract; see [`super::test_support`] for the API.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use super::{
    CommandRunner, ESC_SETTLE_DELAY, MultiplexerContext, MultiplexerError, RunError, SUBMIT_DELAY,
    WakeEntry, build_wake_payload_from_entries,
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

fn map_run_error(argv: &[String], timeout_secs: Option<u64>, error: RunError) -> MultiplexerError {
    let joined = argv.join(" ");
    match error {
        RunError::BinaryNotFound(detail) => {
            MultiplexerError::new(format!("tmux binary not found: {detail}"))
        }
        RunError::Timeout => {
            let secs = timeout_secs.expect("a timeout error implies a timeout was set");
            MultiplexerError::new(format!("tmux command timed out after {secs}s: {joined}"))
        }
        RunError::Failed { stderr } => MultiplexerError::new(format!(
            "tmux command failed: {joined}\nstderr: {}",
            stderr.trim()
        )),
    }
}

pub struct TmuxMultiplexer {
    runner: Rc<dyn CommandRunner>,
    env: HashMap<String, String>,
}

impl TmuxMultiplexer {
    pub fn new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>) -> Self {
        TmuxMultiplexer { runner, env }
    }

    pub fn name(&self) -> &'static str {
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

    /// The literal-then-Enter keystroke shape shared by every submit path:
    /// optional `Escape` safeguard + settle, the `-l` literal payload, the
    /// submit delay, and the trailing `Enter`.
    fn send_literal_then_enter(
        &self,
        target_pane_id: &str,
        payload: &str,
        timeout_secs: Option<u64>,
        ignore_missing: bool,
        esc_first: bool,
    ) -> Result<(), MultiplexerError> {
        if esc_first {
            self.run_tolerating_pane_gone(
                &tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Escape"]),
                ignore_missing,
                timeout_secs,
            )?;
            self.runner.sleep(ESC_SETTLE_DELAY);
        }
        self.run_tolerating_pane_gone(
            &tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "-l", payload]),
            ignore_missing,
            timeout_secs,
        )?;
        self.runner.sleep(SUBMIT_DELAY);
        self.run_tolerating_pane_gone(
            &tmux_argv(&["tmux", "send-keys", "-t", target_pane_id, "Enter"]),
            ignore_missing,
            timeout_secs,
        )
    }

    fn best_effort_send(&self, target_pane_id: &str, payload: &str, esc_first: bool) -> bool {
        if !self.runner.binary_exists("tmux") {
            return false;
        }
        self.send_literal_then_enter(target_pane_id, payload, Some(5), false, esc_first)
            .is_ok()
    }

    pub fn ensure_available(&self) -> Result<(), MultiplexerError> {
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

    pub fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
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
    pub fn split_window(
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

    pub fn send_exit(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        self.send_literal_then_enter(target_pane_id, "/exit", None, ignore_missing, true)
    }

    pub fn send_poll_trigger(&self, target_pane_id: &str, member_id: i64) -> bool {
        let payload = format!(
            "cafleet message poll {member_id} \
             — then resume your work if something was still running."
        );
        self.best_effort_send(target_pane_id, &payload, true)
    }

    pub fn send_wake_entries(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[WakeEntry<'_>],
        director: &WakeEntry<'_>,
    ) -> Result<bool, MultiplexerError> {
        let payload = build_wake_payload_from_entries(fleet_id, members, director)?;
        Ok(self.best_effort_send(target_pane_id, &payload, true))
    }

    /// Result-returning (SPEC §6.5): the missing-binary precheck fails with
    /// the exact PATH string, and any Escape/payload/Enter failure propagates
    /// as the raw error — no boolean wrapper, no retry.
    pub fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
        note: Option<&str>,
    ) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists("tmux") {
            return Err(MultiplexerError::new("tmux binary not found on PATH"));
        }
        let payload = super::inline_preview_payload(message_id, sender_id, ts, text, note);
        self.send_literal_then_enter(target_pane_id, &payload, Some(5), false, true)
    }

    pub fn send_prompt(&self, target_pane_id: &str, text: &str) -> Result<(), MultiplexerError> {
        let stripped = text.trim();
        if stripped.is_empty() {
            return Err(MultiplexerError::new("send_prompt: text may not be empty"));
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(MultiplexerError::new(
                "send_prompt: text may not contain newlines",
            ));
        }
        self.send_literal_then_enter(target_pane_id, stripped, None, false, true)
    }

    /// Capture the last `lines` drawn lines of the pane buffer via the shared
    /// A8 windowing ([`super::capture_window`]).
    pub fn capture_pane(
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

    pub fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        let out = self.run(
            &tmux_argv(&["tmux", "list-panes", "-a", "-F", "#{pane_id}"]),
            Some(5),
        )?;
        Ok(out.split_whitespace().map(str::to_string).collect())
    }

    pub fn kill_pane(
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

    /// tmux tracks no native agent state.
    pub fn agent_status(&self, _target_pane_id: &str) -> Result<Option<String>, MultiplexerError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {

    use crate::multiplexer::test_support::{FakeRunner, argv, env, run_event, sleep_event};
    use crate::multiplexer::{MultiplexerContext, RunError, TmuxMultiplexer};

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
    fn name_is_tmux() {
        let runner = FakeRunner::with_binary("tmux");
        assert_eq!(TmuxMultiplexer::new(runner, tmux_env()).name(), "tmux");
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
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let text = "  /help with literal $HOME; !  ";
        mux.send_prompt("%5", text).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "-l", text]),
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
    fn send_poll_trigger_uses_only_literal_json_poll_settle_and_enter() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(mux.send_poll_trigger("%5", 14));
        let text = "cafleet message poll 14 --json";
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "-l", text]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        assert_eq!(runner.events()[1], sleep_event(1.0));
    }

    #[test]
    fn send_inline_preview_submits_one_visible_literal_line() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let ts = "2026-10-23T00:00:00.000000+00:00";
        mux.send_inline_preview("%5", 5, 2, ts, "a\r\nb\nc\rd\tx\x1b\x7f", None)
            .unwrap();
        let expected = format!("[cafleet msg 5 from 2 {ts}] a⏎b⏎c⏎d␉x\\x1b\\x7f");
        let text = expected.as_str();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "-l", text]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        assert_eq!(runner.events()[1], sleep_event(1.0));
    }

    #[test]
    fn first_preview_write_failure_propagates_cause_without_enter_or_retry() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "injected first write failure".into(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux
            .send_inline_preview("%5", 5, 2, "timestamp", "hello", None)
            .unwrap_err();
        assert!(
            error.to_string().contains("injected first write failure"),
            "{error}"
        );
        let text = "[cafleet msg 5 from 2 timestamp] hello";
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&["tmux", "send-keys", "-t", "%5", "-l", text])]
        );
        assert_eq!(runner.events().len(), 1);
    }

    #[test]
    fn lost_preview_enter_propagates_cause_after_one_literal_write() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "injected enter failure".into(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux
            .send_inline_preview("%5", 5, 2, "timestamp", "hello", None)
            .unwrap_err();
        assert!(
            error.to_string().contains("injected enter failure"),
            "{error}"
        );
        let text = "[cafleet msg 5 from 2 timestamp] hello";
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["tmux", "send-keys", "-t", "%5", "-l", text]),
                argv(&["tmux", "send-keys", "-t", "%5", "Enter"])
            ]
        );
        assert_eq!(runner.events().len(), 3);
    }

    #[test]
    fn inline_preview_requires_the_backend_binary_before_any_io() {
        let runner = FakeRunner::without_binaries();
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let error = mux
            .send_inline_preview("%5", 5, 2, "timestamp", "hello", None)
            .unwrap_err();
        assert_eq!(error.to_string(), "tmux binary not found on PATH");
        assert!(runner.events().is_empty());
    }

    #[test]
    fn prompt_line_validation_precedes_transport() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        for text in ["", "  ", "a\nb", "a\rb", "a\tb", "a\0b", "a\x1bb", "a\x7fb"] {
            assert!(mux.send_prompt("%5", text).is_err(), "{text:?}");
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
             the trailing blank line is dropped before the slice (A8)"
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
            "A8 over-fetches by the fixed 1000-line margin"
        );
    }

    // A8: the fetch depth is requested lines + 1000, so a blank tail deeper
    // than N still leaves the drawn bottom inside the fetched buffer.
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

    // A8: a small --lines window on a tall pane must show the drawn bottom,
    // not the blank area under the cursor.
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

    // A8: TUI-painted empty rows carry ANSI sequences — a trailing line is
    // blank when it is whitespace-only AFTER per-line CSI stripping; the kept
    // lines keep their original bytes.
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
    fn list_pane_ids_returns_the_whitespace_split_set() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("%1\n%2\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let panes = mux.list_pane_ids().unwrap();
        assert_eq!(panes, ["%1", "%2"].iter().map(|s| s.to_string()).collect());
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["tmux", "list-panes", "-a", "-F", "#{pane_id}"],
                Some(5),
            )]
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
    fn runner_failures_map_to_the_pinned_error_texts() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "  boom  ".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.kill_pane("%5", false).unwrap_err().to_string(),
            "tmux command failed: tmux kill-pane -t %5\nstderr: boom"
        );

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Timeout));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.list_pane_ids().unwrap_err().to_string(),
            "tmux command timed out after 5s: tmux list-panes -a -F #{pane_id}"
        );

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::BinaryNotFound(
            "No such file or directory".to_string(),
        )));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert_eq!(
            mux.capture_pane("%5", 3).unwrap_err().to_string(),
            "tmux binary not found: No such file or directory"
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
