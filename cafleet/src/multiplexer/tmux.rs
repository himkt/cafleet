//! tmux backend (SPEC §6.5 *TmuxMultiplexer*) — exact argv forms, the
//! Esc-first matrix, the two keystroke delays, capture semantics, pane-gone
//! tolerance, and the best-effort-vs-fail-fast split. The colocated tests pin
//! the contract; see [`super::test_support`] for the API.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

#[cfg(test)]
use serde_json::Value;

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
        let pane = super::PaneOwnership::new(pane_id, |id: &str| self.kill_pane(id, true));
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
        Ok(pane.finish())
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

    // Temporary JSON entry point for unchanged backend tests.
    #[cfg(test)]
    pub fn send_wake_trigger(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[Value],
        director: &Value,
    ) -> Result<bool, MultiplexerError> {
        let members = members
            .iter()
            .map(WakeEntry::from_legacy)
            .collect::<Result<Vec<_>, _>>()?;
        self.send_wake_entries(
            target_pane_id,
            fleet_id,
            &members,
            &WakeEntry::from_legacy(director)?,
        )
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
    ) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists("tmux") {
            return Err(MultiplexerError::new("tmux binary not found on PATH"));
        }
        let sanitized = text.replace("\r\n", "⏎").replace(['\n', '\r'], "⏎");
        let payload = format!("[cafleet msg {message_id} from {sender_id} {ts}]\n{sanitized}");
        self.send_literal_then_enter(target_pane_id, &payload, Some(5), false, true)
    }

    pub fn send_prompt(
        &self,
        target_pane_id: &str,
        text: &str,
        shell: bool,
    ) -> Result<(), MultiplexerError> {
        let stripped = text.trim();
        if stripped.is_empty() {
            return Err(MultiplexerError::new("send_prompt: text may not be empty"));
        }
        if text.contains('\n') || text.contains('\r') {
            return Err(MultiplexerError::new(
                "send_prompt: text may not contain newlines",
            ));
        }
        let payload = if shell {
            format!("! {stripped}")
        } else {
            stripped.to_string()
        };
        self.send_literal_then_enter(target_pane_id, &payload, None, false, true)
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
    use serde_json::json;

    use crate::multiplexer::test_support::{FakeRunner, argv, env, run_event, sleep_event};
    use crate::multiplexer::{MultiplexerContext, RunError, TmuxMultiplexer, build_wake_payload};

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
    fn send_poll_trigger_keystrokes_esc_payload_enter_with_the_two_delays() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(mux.send_poll_trigger("%5", 14));
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], Some(5)),
                sleep_event(0.1),
                run_event(
                    &[
                        "tmux",
                        "send-keys",
                        "-t",
                        "%5",
                        "-l",
                        "cafleet message poll 14 — then resume \
                         your work if something was still running.",
                    ],
                    Some(5),
                ),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], Some(5)),
            ]
        );
    }

    #[test]
    fn send_poll_trigger_is_best_effort() {
        let runner = FakeRunner::without_binaries();
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(!mux.send_poll_trigger("%5", 14), "tmux missing → false");
        assert!(runner.events().is_empty(), "no keystroke is attempted");

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert!(!mux.send_poll_trigger("%5", 14), "any error → false");
    }

    #[test]
    fn send_inline_preview_types_the_two_line_payload_esc_first() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        assert!(
            mux.send_inline_preview("%5", 5, 2, ts, "a\r\nb\nc\rd")
                .is_ok()
        );
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], Some(5)),
                sleep_event(0.1),
                run_event(
                    &[
                        "tmux",
                        "send-keys",
                        "-t",
                        "%5",
                        "-l",
                        &format!("[cafleet msg 5 from 2 {ts}]\na⏎b⏎c⏎d"),
                    ],
                    Some(5),
                ),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], Some(5)),
            ],
            "one -l keystroke + one Enter: the embedded newline is a soft break"
        );
    }

    #[test]
    fn send_inline_preview_without_the_binary_is_the_exact_path_error() {
        let runner = FakeRunner::without_binaries();
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let err = mux
            .send_inline_preview("%5", 5, 2, "2026-07-30T09:00:00.000000+00:00", "hi")
            .unwrap_err();
        assert_eq!(err.to_string(), "tmux binary not found on PATH");
        assert!(
            runner.events().is_empty(),
            "the precheck precedes any keystroke"
        );
    }

    #[test]
    fn send_inline_preview_propagates_an_escape_failure_with_the_raw_detail() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "  pane died  ".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let err = mux
            .send_inline_preview("%5", 5, 2, "2026-07-30T09:00:00.000000+00:00", "hi")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "tmux command failed: tmux send-keys -t %5 Escape\nstderr: pane died"
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["tmux", "send-keys", "-t", "%5", "Escape"],
                Some(5),
            )],
            "a failed Escape aborts before the payload"
        );
    }

    #[test]
    fn send_inline_preview_propagates_a_payload_failure_with_argv_and_stderr() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "no such pane: %5".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        let err = mux.send_inline_preview("%5", 5, 2, ts, "a\nb").unwrap_err();
        let payload = format!("[cafleet msg 5 from 2 {ts}]\na⏎b");
        assert_eq!(
            err.to_string(),
            format!(
                "tmux command failed: tmux send-keys -t %5 -l {payload}\nstderr: no such pane: %5"
            )
        );
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], Some(5)),
                sleep_event(0.1),
                run_event(&["tmux", "send-keys", "-t", "%5", "-l", &payload], Some(5)),
            ],
            "one attempt only — the payload failure aborts before Enter"
        );
    }

    #[test]
    fn send_inline_preview_propagates_a_trailing_enter_failure() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok(String::new()));
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "submit lost".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        let err = mux.send_inline_preview("%5", 5, 2, ts, "hi").unwrap_err();
        assert_eq!(
            err.to_string(),
            "tmux command failed: tmux send-keys -t %5 Enter\nstderr: submit lost"
        );
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], Some(5)),
                sleep_event(0.1),
                run_event(
                    &[
                        "tmux",
                        "send-keys",
                        "-t",
                        "%5",
                        "-l",
                        &format!("[cafleet msg 5 from 2 {ts}]\nhi"),
                    ],
                    Some(5),
                ),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], Some(5)),
            ],
            "the full choreography ran once; no retry follows the lost Enter"
        );
    }

    #[test]
    fn send_wake_trigger_is_esc_first_and_types_the_shared_payload() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let members = [json!({
            "member_id": 4,
            "name": "worker",
            "coding_agent": "codex",
            "pending_count": 0,
        })];
        let director = json!({
            "member_id": 2,
            "name": "Director",
            "coding_agent": "claude",
            "pending_count": 1,
        });
        assert!(mux.send_wake_trigger("%9", 3, &members, &director).unwrap());

        let payload = build_wake_payload(3, &members, &director).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%9", "Escape"], Some(5)),
                sleep_event(0.1),
                run_event(&["tmux", "send-keys", "-t", "%9", "-l", &payload], Some(5)),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%9", "Enter"], Some(5)),
            ],
            "the wake matches the established Esc-first keystroke norm"
        );
    }

    #[test]
    fn send_wake_trigger_aborts_on_an_invalid_agent_without_keystrokes() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let members = [json!({
            "member_id": 4,
            "name": "worker",
            "coding_agent": "python",
            "pending_count": 0,
        })];
        let director = json!({
            "member_id": 2,
            "name": "Director",
            "coding_agent": "claude",
            "pending_count": 0,
        });
        assert!(mux.send_wake_trigger("%9", 3, &members, &director).is_err());
        assert!(runner.events().is_empty());
    }

    #[test]
    fn send_wake_trigger_remains_best_effort_on_delivery_failure() {
        let members = [json!({
            "member_id": 4,
            "name": "worker",
            "coding_agent": "codex",
            "pending_count": 0,
        })];
        let director = json!({
            "member_id": 2,
            "name": "Director",
            "coding_agent": "claude",
            "pending_count": 0,
        });

        let runner = FakeRunner::without_binaries();
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(
            !mux.send_wake_trigger("%9", 3, &members, &director).unwrap(),
            "tmux missing → Ok(false)"
        );
        assert!(runner.events().is_empty());

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert!(
            !mux.send_wake_trigger("%9", 3, &members, &director).unwrap(),
            "a keystroke error stays Ok(false), never an Err"
        );
    }

    #[test]
    fn send_prompt_plain_form_is_esc_safeguarded_and_stripped() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        mux.send_prompt("%5", "  hi tmux  ", false).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], None),
                sleep_event(0.1),
                run_event(&["tmux", "send-keys", "-t", "%5", "-l", "hi tmux"], None),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], None),
            ]
        );
    }

    #[test]
    fn send_prompt_shell_form_is_esc_safeguarded_and_prefixes_bang() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        mux.send_prompt("%5", " ls -la ", true).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], None),
                sleep_event(0.1),
                run_event(&["tmux", "send-keys", "-t", "%5", "-l", "! ls -la"], None),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], None),
            ]
        );

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(
            mux.send_prompt("%5", "ls -la", true).is_err(),
            "the shell form propagates an Esc failure like the plain form"
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["tmux", "send-keys", "-t", "%5", "Escape"],
                None,
            )],
            "a failed Esc aborts before the payload"
        );
    }

    #[test]
    fn send_prompt_rejects_empty_and_multiline_text() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert_eq!(
            mux.send_prompt("%5", "   ", false).unwrap_err().to_string(),
            "send_prompt: text may not be empty"
        );
        assert_eq!(
            mux.send_prompt("%5", "a\nb", false)
                .unwrap_err()
                .to_string(),
            "send_prompt: text may not contain newlines"
        );
        assert_eq!(
            mux.send_prompt("%5", "a\rb", true).unwrap_err().to_string(),
            "send_prompt: text may not contain newlines"
        );
        assert!(
            runner.events().is_empty(),
            "validation precedes any keystroke"
        );
    }

    #[test]
    fn send_exit_is_esc_safeguarded_and_tolerates_a_missing_pane() {
        for missing_keystroke in 0..3 {
            let runner = FakeRunner::with_binary("tmux");
            for _ in 0..missing_keystroke {
                runner.respond(Ok(String::new()));
            }
            runner.respond(Err(RunError::Failed {
                stderr: "can't find pane %5".to_string(),
            }));
            let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
            mux.send_exit("%5", true).unwrap();
            assert_eq!(
                runner.events(),
                vec![
                    run_event(&["tmux", "send-keys", "-t", "%5", "Escape"], None),
                    sleep_event(0.1),
                    run_event(&["tmux", "send-keys", "-t", "%5", "-l", "/exit"], None),
                    sleep_event(1.0),
                    run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], None),
                ],
                "a missing pane at keystroke {missing_keystroke} is tolerated"
            );
        }
    }

    #[test]
    fn send_exit_without_tolerance_propagates_the_failure() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "can't find pane %5".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert!(mux.send_exit("%5", false).is_err());
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["tmux", "send-keys", "-t", "%5", "Escape"],
                None,
            )],
            "without tolerance, an Esc failure aborts before the payload"
        );
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
                &["tmux", "capture-pane", "-p", "-t", "%5", "-S", "-1002"],
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
    fn agent_status_is_always_absent_on_tmux() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        assert_eq!(mux.agent_status("%5").unwrap(), None);
        assert!(runner.events().is_empty(), "no native state to query");
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
