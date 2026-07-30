//! herdr backend (SPEC §6.5 *HerdrMultiplexer*) — JSON envelope parsing, the
//! layout-ratio equalization arithmetic, `agent_status`, and the kill/rebalance
//! phases. The colocated tests pin the contract; see [`super::test_support`]
//! for the API.

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::multiplexer::test_support::{
        FakeRunner, argv, env, herdr_envelope, herdr_error_stderr, run_event, sleep_event,
    };
    use crate::multiplexer::{HerdrMultiplexer, MultiplexerContext, RunError, build_wake_payload};

    fn herdr_env() -> std::collections::HashMap<String, String> {
        env(&[("HERDR_ENV", "1")])
    }

    fn reference() -> MultiplexerContext {
        MultiplexerContext {
            session: "w1".to_string(),
            window_id: "w1:t1".to_string(),
            pane_id: "w1:p1".to_string(),
        }
    }

    fn cwd() -> String {
        std::env::current_dir().unwrap().to_str().unwrap().to_string()
    }

    #[test]
    fn name_is_herdr() {
        let runner = FakeRunner::with_binary("herdr");
        assert_eq!(HerdrMultiplexer::new(runner, herdr_env()).name(), "herdr");
    }

    #[test]
    fn ensure_available_requires_the_binary_then_the_herdr_env() {
        let runner = FakeRunner::without_binaries();
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert_eq!(
            mux.ensure_available().unwrap_err().to_string(),
            "herdr binary not found on PATH"
        );

        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner, env(&[]));
        assert_eq!(
            mux.ensure_available().unwrap_err().to_string(),
            "cafleet member commands must be run inside a herdr session"
        );

        let runner = FakeRunner::with_binary("herdr");
        assert!(
            HerdrMultiplexer::new(runner, herdr_env())
                .ensure_available()
                .is_ok()
        );
    }

    #[test]
    fn context_discovery_reads_the_single_pane_current_envelope() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "pane": {"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1"}
        }))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let context = mux.context_discovery().unwrap();
        assert_eq!(context.session, "w1");
        assert_eq!(context.window_id, "w1:t1");
        assert_eq!(context.pane_id, "w1:p1");
        assert_eq!(
            runner.events(),
            vec![run_event(&["herdr", "pane", "current"], None)]
        );
    }

    #[test]
    fn structured_reads_reject_non_json_and_missing_result() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("not json".to_string()));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        let err = mux.context_discovery().unwrap_err();
        assert!(
            err.to_string().starts_with("herdr returned non-JSON output:"),
            "got: {err}"
        );

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(r#"{"id":1}"#.to_string()));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        let err = mux.context_discovery().unwrap_err();
        assert!(
            err.to_string()
                .starts_with("herdr output missing 'result' object:"),
            "got: {err}"
        );
    }

    #[test]
    fn split_window_first_member_splits_the_director_rightward() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1"}]
        }))));
        runner.respond(Ok(herdr_envelope(json!({"pane": {"pane_id": "w1:p2"}}))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let pane = mux
            .split_window(
                &reference(),
                &[(
                    "CAFLEET_DATABASE_URL".to_string(),
                    "sqlite:///x.db".to_string(),
                )],
                &argv(&["claude", "--name", "w 1"]),
            )
            .unwrap();
        assert_eq!(pane, "w1:p2");
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["herdr", "pane", "list"]),
                argv(&[
                    "herdr",
                    "pane",
                    "split",
                    "w1:p1",
                    "--direction",
                    "right",
                    "--no-focus",
                    "--cwd",
                    &cwd(),
                    "--env",
                    "CAFLEET_DATABASE_URL=sqlite:///x.db",
                ]),
                argv(&["herdr", "pane", "run", "w1:p2", "claude --name 'w 1'"]),
            ],
            "pane run submits ONE shell line — the argv is shlex-quoted"
        );
    }

    #[test]
    fn split_window_appends_downward_and_equalizes_the_column() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [
                {"pane_id": "w1:p1", "tab_id": "w1:t1"},
                {"pane_id": "w1:p2", "tab_id": "w1:t1"},
                {"pane_id": "w2:p9", "tab_id": "w2:t1"},
            ]
        }))));
        runner.respond(Ok(herdr_envelope(json!({"pane": {"pane_id": "w1:p3"}}))));
        runner.respond(Ok(herdr_envelope(json!({
            "layout": {
                "panes": [
                    {"pane_id": "w1:p1", "rect": {"x": 0, "y": 0, "width": 60, "height": 30}},
                    {"pane_id": "w1:p2", "rect": {"x": 60, "y": 0, "width": 60, "height": 15}},
                    {"pane_id": "w1:p3", "rect": {"x": 60, "y": 15, "width": 60, "height": 15}},
                ],
                "splits": [
                    {"direction": "right", "rect": {"x": 0, "y": 0}, "ratio": 0.5},
                    {"direction": "down", "rect": {"x": 60, "y": 15}, "ratio": 0.7},
                ]
            }
        }))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let pane = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap();
        assert_eq!(pane, "w1:p3");
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["herdr", "pane", "list"]),
                argv(&[
                    "herdr", "pane", "split", "w1:p2", "--direction", "down", "--no-focus",
                    "--cwd", &cwd(),
                ]),
                argv(&["herdr", "pane", "layout", "--pane", "w1:p1"]),
                argv(&[
                    "herdr", "pane", "resize", "--pane", "w1:p3", "--direction", "up",
                    "--amount", "0.2",
                ]),
                argv(&["herdr", "pane", "run", "w1:p3", "claude"]),
            ],
            "split targets the column's max pane; one signed resize drives the \
             down split to ratio 1/2; the read anchors on the Director's pane"
        );
    }

    #[test]
    fn split_window_equalization_is_best_effort() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [
                {"pane_id": "w1:p1", "tab_id": "w1:t1"},
                {"pane_id": "w1:p2", "tab_id": "w1:t1"},
            ]
        }))));
        runner.respond(Ok(herdr_envelope(json!({"pane": {"pane_id": "w1:p3"}}))));
        runner.respond(Err(RunError::Failed {
            stderr: "layout read boom".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        let pane = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap();
        assert_eq!(pane, "w1:p3", "a rebalance failure never fails the spawn");
    }

    #[test]
    fn send_exit_runs_the_exit_line_and_tolerates_only_pane_not_found() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        mux.send_exit("w1:p2", true).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&["herdr", "pane", "run", "w1:p2", "/exit"])]
        );

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("internal"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(
            mux.send_exit("w1:p2", true).is_err(),
            "only the pane_not_found code is tolerated"
        );
    }

    #[test]
    fn send_poll_trigger_is_esc_then_run() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert!(mux.send_poll_trigger("w1:p2", 3, 14));
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                sleep_event(0.1),
                run_event(
                    &[
                        "herdr",
                        "pane",
                        "run",
                        "w1:p2",
                        "cafleet message poll --fleet-id 3 --member-id 14",
                    ],
                    Some(5),
                ),
            ],
            "pane run submits atomically — no separate Enter, no submit delay"
        );
    }

    #[test]
    fn send_poll_trigger_is_best_effort() {
        let runner = FakeRunner::without_binaries();
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert!(!mux.send_poll_trigger("w1:p2", 3, 14));
        assert!(runner.events().is_empty());

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(!mux.send_poll_trigger("w1:p2", 3, 14));
    }

    #[test]
    fn send_wake_trigger_is_a_single_run_without_esc() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let due = [json!({
            "member_id": 4,
            "name": "worker",
            "is_director": false,
            "coding_agent": "codex",
            "wake_reasons": ["interval"],
        })];
        let director = json!({"member_id": 1, "coding_agent": "claude"});
        assert!(mux.send_wake_trigger("w1:p9", &due, &director).unwrap());

        let payload = build_wake_payload(&due, &director).unwrap();
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["herdr", "pane", "run", "w1:p9", &payload],
                Some(5),
            )],
            "the payload is byte-identical to the tmux backend's"
        );
    }

    #[test]
    fn send_inline_preview_is_esc_send_text_delay_enter() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        assert!(mux.send_inline_preview("w1:p2", 5, 2, ts, "a\nb"));
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                sleep_event(0.1),
                run_event(
                    &[
                        "herdr",
                        "pane",
                        "send-text",
                        "w1:p2",
                        &format!("[cafleet msg 5 from 2 {ts}]\na⏎b"),
                    ],
                    Some(5),
                ),
                sleep_event(1.0),
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "enter"], Some(5)),
            ],
            "send-text is raw; the one trailing enter submits the 2-line payload"
        );
    }

    #[test]
    fn send_prompt_shell_and_plain_forms() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        mux.send_prompt("w1:p2", " ls ", true).unwrap();
        assert_eq!(
            runner.events(),
            vec![run_event(&["herdr", "pane", "run", "w1:p2", "! ls"], None)]
        );

        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        mux.send_prompt("w1:p2", "hi there", false).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                sleep_event(0.1),
                run_event(&["herdr", "pane", "run", "w1:p2", "hi there"], None),
            ]
        );

        let mux = HerdrMultiplexer::new(FakeRunner::with_binary("herdr"), herdr_env());
        assert_eq!(
            mux.send_prompt("w1:p2", " ", false).unwrap_err().to_string(),
            "send_prompt: text may not be empty"
        );
        assert_eq!(
            mux.send_prompt("w1:p2", "a\nb", false)
                .unwrap_err()
                .to_string(),
            "send_prompt: text may not contain newlines"
        );
    }

    #[test]
    fn capture_pane_returns_the_raw_buffer() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("raw\nbuffer\n".to_string()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(mux.capture_pane("w1:p2", 20).unwrap(), "raw\nbuffer\n");
        assert_eq!(
            runner.events(),
            vec![run_event(
                &[
                    "herdr",
                    "pane",
                    "read",
                    "w1:p2",
                    "--source",
                    "recent-unwrapped",
                    "--lines",
                    "20",
                ],
                None,
            )]
        );

        assert_eq!(
            mux.capture_pane("w1:p2", 0).unwrap_err().to_string(),
            "capture_pane: lines must be positive, got 0"
        );
    }

    #[test]
    fn list_pane_ids_reads_the_pane_list_envelope() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [{"pane_id": "w1:p1"}, {"pane_id": "w1:p2"}]
        }))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let panes = mux.list_pane_ids().unwrap();
        assert_eq!(
            panes,
            ["w1:p1", "w1:p2"].iter().map(|s| s.to_string()).collect()
        );
        assert_eq!(
            runner.events(),
            vec![run_event(&["herdr", "pane", "list"], Some(5))]
        );
    }

    #[test]
    fn kill_pane_restores_the_director_full_width_after_the_last_member() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(
            json!({"pane": {"pane_id": "w1:p2", "tab_id": "w1:t1"}}),
        )));
        runner.respond(Ok(String::new()));
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1"}]
        }))));
        runner.respond(Ok(herdr_envelope(json!({
            "layout": {
                "panes": [{"pane_id": "w1:p1", "rect": {"x": 0, "y": 0}}],
                "splits": [{"direction": "right", "rect": {"x": 0, "y": 0}, "ratio": 0.6}]
            }
        }))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        mux.kill_pane("w1:p2", false).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["herdr", "pane", "get", "w1:p2"]),
                argv(&["herdr", "pane", "close", "w1:p2"]),
                argv(&["herdr", "pane", "list"]),
                argv(&["herdr", "pane", "layout", "--pane", "w1:p1"]),
                argv(&[
                    "herdr", "pane", "resize", "--pane", "w1:p1", "--direction", "right",
                    "--amount", "0.4",
                ]),
            ],
            "pre-close tab read → close → anchor on a survivor → restore width"
        );
    }

    #[test]
    fn kill_pane_proceeds_when_the_pre_close_read_fails() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        runner.respond(Ok(String::new()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        mux.kill_pane("w1:p2", false).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["herdr", "pane", "get", "w1:p2"]),
                argv(&["herdr", "pane", "close", "w1:p2"]),
            ],
            "a failed tab read never blocks the close; the rebalance is skipped"
        );
    }

    #[test]
    fn kill_pane_close_error_honors_ignore_missing() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        mux.kill_pane("w1:p2", true).unwrap();

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(
            mux.kill_pane("w1:p2", false).is_err(),
            "without ignore_missing the close failure propagates"
        );
    }

    #[test]
    fn agent_status_reads_the_native_state() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!(
            {"pane": {"pane_id": "w1:p2", "tab_id": "w1:t1", "agent_status": "working"}}
        ))));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.agent_status("w1:p2").unwrap(),
            Some("working".to_string())
        );
        assert_eq!(
            runner.events(),
            vec![run_event(&["herdr", "pane", "get", "w1:p2"], None)]
        );
    }

    #[test]
    fn agent_status_treats_absent_state_and_a_gone_pane_as_none() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(
            json!({"pane": {"pane_id": "w1:p2", "tab_id": "w1:t1"}}),
        )));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert_eq!(mux.agent_status("w1:p2").unwrap(), None);

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!(
            {"pane": {"pane_id": "w1:p2", "tab_id": "w1:t1", "agent_status": ""}}
        ))));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert_eq!(mux.agent_status("w1:p2").unwrap(), None, "empty → None");

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert_eq!(
            mux.agent_status("w1:p2").unwrap(),
            None,
            "a teardown race is not an error"
        );

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("internal"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(mux.agent_status("w1:p2").is_err());
    }
}
