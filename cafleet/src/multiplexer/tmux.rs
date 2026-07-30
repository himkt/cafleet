//! tmux backend (SPEC §6.5 *TmuxMultiplexer*) — exact argv forms, the
//! Esc-first matrix, the two keystroke delays, capture semantics, pane-gone
//! tolerance, and the best-effort-vs-fail-fast split. The colocated tests pin
//! the contract; see [`super::test_support`] for the API.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::multiplexer::test_support::{
        FakeRunner, argv, env, run_event, sleep_event,
    };
    use crate::multiplexer::{MultiplexerContext, TmuxMultiplexer, build_wake_payload, RunError};

    fn tmux_env() -> std::collections::HashMap<String, String> {
        env(&[("TMUX", "/tmp/tmux-1000/default,123,0"), ("TMUX_PANE", "%1")])
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
        assert!(TmuxMultiplexer::new(runner, tmux_env()).ensure_available().is_ok());
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
        assert!(mux.send_poll_trigger("%5", 3, 14));
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
                        "cafleet message poll --fleet-id 3 --member-id 14",
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
        assert!(!mux.send_poll_trigger("%5", 3, 14), "tmux missing → false");
        assert!(runner.events().is_empty(), "no keystroke is attempted");

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert!(!mux.send_poll_trigger("%5", 3, 14), "any error → false");
    }

    #[test]
    fn send_inline_preview_types_the_two_line_payload_esc_first() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        assert!(mux.send_inline_preview("%5", 5, 2, ts, "a\r\nb\nc\rd"));
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
    fn send_wake_trigger_has_no_esc_and_types_the_shared_payload() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let due = [json!({
            "member_id": 4,
            "name": "worker",
            "is_director": false,
            "coding_agent": "codex",
            "wake_reasons": ["interval"],
        })];
        let director = json!({"member_id": 1, "coding_agent": "claude"});
        assert!(mux.send_wake_trigger("%9", &due, &director).unwrap());

        let payload = build_wake_payload(&due, &director).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%9", "-l", &payload], Some(5)),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%9", "Enter"], Some(5)),
            ],
            "no leading Escape and no settle sleep on the wake path"
        );
    }

    #[test]
    fn send_wake_trigger_aborts_on_an_invalid_agent_without_keystrokes() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let due = [json!({
            "member_id": 4,
            "name": "worker",
            "is_director": false,
            "coding_agent": "python",
            "wake_reasons": ["interval"],
        })];
        let director = json!({"member_id": 1, "coding_agent": "claude"});
        assert!(mux.send_wake_trigger("%9", &due, &director).is_err());
        assert!(runner.events().is_empty());
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
    fn send_prompt_shell_form_prefixes_bang_and_skips_the_esc() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        mux.send_prompt("%5", " ls -la ", true).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "-l", "! ls -la"], None),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], None),
            ]
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
            mux.send_prompt("%5", "a\nb", false).unwrap_err().to_string(),
            "send_prompt: text may not contain newlines"
        );
        assert_eq!(
            mux.send_prompt("%5", "a\rb", true).unwrap_err().to_string(),
            "send_prompt: text may not contain newlines"
        );
        assert!(runner.events().is_empty(), "validation precedes any keystroke");
    }

    #[test]
    fn send_exit_types_exit_without_esc_and_tolerates_a_missing_pane() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "can't find pane %5".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        mux.send_exit("%5", true).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["tmux", "send-keys", "-t", "%5", "-l", "/exit"], None),
                sleep_event(1.0),
                run_event(&["tmux", "send-keys", "-t", "%5", "Enter"], None),
            ],
            "the tolerated pane-gone failure does not abort the sequence"
        );
    }

    #[test]
    fn send_exit_without_tolerance_propagates_the_failure() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "can't find pane %5".to_string(),
        }));
        let mux = TmuxMultiplexer::new(runner, tmux_env());
        assert!(mux.send_exit("%5", false).is_err());
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
    fn capture_pane_keeps_the_last_lines_and_the_final_newline() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("l1\nl2\nl3\nx\rY\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let captured = mux.capture_pane("%5", 2).unwrap();
        assert_eq!(
            captured, "l3\nx\rY\n",
            "split on \\n only — the \\r survives for CLI-side defrag"
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["tmux", "capture-pane", "-p", "-t", "%5", "-S", "-2"],
                None,
            )]
        );
    }

    #[test]
    fn list_pane_ids_returns_the_whitespace_split_set() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("%1\n%2\n".to_string()));
        let mux = TmuxMultiplexer::new(runner.clone(), tmux_env());
        let panes = mux.list_pane_ids().unwrap();
        assert_eq!(
            panes,
            ["%1", "%2"].iter().map(|s| s.to_string()).collect()
        );
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
}
