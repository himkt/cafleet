use crate::multiplexer::BackendCore;
use crate::multiplexer::CommandRunner;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MultiplexerContext;
use crate::multiplexer::MultiplexerError;
use crate::multiplexer::PLACEMENT_MISMATCH;
use std::collections::HashMap;
use std::rc::Rc;

const PANE_GONE_MARKERS: [&str; 2] = ["can't find pane", "no such pane"];

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
        Self::with_server(runner, env, None)
    }

    pub(crate) fn for_server(runner: Rc<dyn CommandRunner>, socket: &str) -> Self {
        Self::with_server(runner, HashMap::new(), Some(socket.to_owned()))
    }

    fn with_server(
        runner: Rc<dyn CommandRunner>,
        env: HashMap<String, String>,
        server: Option<String>,
    ) -> Self {
        TmuxMultiplexer {
            core: BackendCore {
                runner,
                env,
                name: "tmux",
                pane_gone: is_pane_gone,
                server_flag: "-S",
                server,
            },
        }
    }

    /// `pane`'s session, window and pane as `display-message` prints them.
    fn read_context(
        &self,
        pane: &str,
    ) -> Result<(String, Option<MultiplexerContext>), MultiplexerError> {
        let out = self.core.run(
            &self.core.argv(&[
                "tmux",
                "display-message",
                "-p",
                "-t",
                pane,
                "#{session_name}|#{window_id}|#{pane_id}",
            ]),
            None,
        )?;
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
}

impl Multiplexer for TmuxMultiplexer {
    fn core(&self) -> &BackendCore {
        &self.core
    }

    fn bounded(&self, deadline: crate::time::Deadline) -> Box<dyn Multiplexer> {
        Box::new(Self {
            core: self.core.bounded(deadline),
        })
    }

    /// tmux prints `||` and exits 0 for a missing pane, so an empty field or another pane id
    /// means the server has no such pane.
    fn pane_context(&self, pane: &str) -> Result<Option<MultiplexerContext>, MultiplexerError> {
        let (out, context) = self.read_context(pane)?;
        let context = context.ok_or_else(|| {
            MultiplexerError::new(format!("unexpected tmux display-message output: {out:?}"))
        })?;
        let found =
            !context.session.is_empty() && !context.window_id.is_empty() && context.pane_id == pane;
        Ok(found.then_some(context))
    }

    fn check_key_target(
        &self,
        pane: &str,
        placement: &crate::broker::Placement,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError> {
        let target = self.core.press_run(
            &self.core.argv(&[
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

    fn send_keys(
        &self,
        pane_id: &str,
        keys: &crate::multiplexer::PressKeys,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError> {
        let mut argv = self.core.argv(&["tmux", "send-keys", "-t", pane_id]);
        argv.extend(keys.tmux_arguments());
        self.core.send_keys(&argv, deadline)
    }

    /// The caller pane's context, read through `TMUX_PANE`.
    fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        let caller = self.core.env_var("TMUX_PANE").ok_or_else(|| {
            MultiplexerError::new("TMUX_PANE is not set; not running inside a tmux pane")
        })?;
        let (out, context) = self.read_context(caller)?;
        context.ok_or_else(|| {
            MultiplexerError::new(format!("unexpected tmux display-message output: {out:?}"))
        })
    }

    /// Split `reference.window_id` detached (`-d`) and return the new pane
    /// id; the post-split `main-vertical` reflow is best-effort.
    fn split_window(
        &self,
        reference: &MultiplexerContext,
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        let mut args = self.core.argv(&[
            "tmux",
            "split-window",
            "-t",
            &reference.window_id,
            "-P",
            "-F",
            "#{pane_id}",
            "-d",
        ]);
        args.extend(command.iter().cloned());
        let pane_id = self
            .core
            .run(&args, None)
            .map_err(|error| error.with_pane_cleanup(crate::multiplexer::PaneCleanup::Unknown))?
            .trim()
            .to_string();
        if pane_id.is_empty() {
            return Err(
                MultiplexerError::new("tmux split-window returned an empty pane ID")
                    .with_pane_cleanup(crate::multiplexer::PaneCleanup::Unknown),
            );
        }
        let _ = self.core.run(
            &self.core.argv(&[
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

    /// Capture the last `lines` drawn lines of the pane buffer via the shared
    /// capture windowing ([`crate::multiplexer::capture_window`]).
    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError> {
        crate::multiplexer::check_capture_lines(lines)?;
        let fetch_depth = lines + crate::multiplexer::CAPTURE_OVER_FETCH_LINES;
        let raw = self.core.run(
            &self.core.argv(&[
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
        Ok(crate::multiplexer::capture_window(&raw, lines))
    }

    fn kill_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        self.core.run_tolerating_missing(
            &self.core.argv(&["tmux", "kill-pane", "-t", target_pane_id]),
            ignore_missing,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::multiplexer::Multiplexer;
    use crate::multiplexer::MultiplexerContext;
    use crate::multiplexer::RunError;
    use crate::multiplexer::TmuxMultiplexer;
    use crate::multiplexer::test_support::FakeRunner;
    use crate::multiplexer::test_support::argv;
    use crate::multiplexer::test_support::run_event;
    use std::rc::Rc;

    const SOCKET: &str = "/tmp/tmux-1000/default";

    fn on_server(runner: Rc<FakeRunner>) -> TmuxMultiplexer {
        TmuxMultiplexer::for_server(runner, SOCKET)
    }

    /// `tmux -S <SOCKET> <parts…>`.
    fn tmux(parts: &[&str]) -> Vec<String> {
        let mut command = argv(&["tmux", "-S", SOCKET]);
        command.extend(argv(parts));
        command
    }

    fn reference() -> MultiplexerContext {
        MultiplexerContext {
            session: "main".to_string(),
            window_id: "@1".to_string(),
            pane_id: "%1".to_string(),
        }
    }

    #[test]
    fn pane_process_reads_the_pane_pid_on_the_recorded_server() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("%3|400625\n".to_string()));
        assert_eq!(
            on_server(runner.clone()).pane_process("%3").unwrap(),
            Some(400625)
        );
        assert_eq!(
            runner.run_argvs(),
            vec![tmux(&[
                "display-message",
                "-p",
                "-t",
                "%3",
                "#{pane_id}|#{pane_pid}",
            ])]
        );
    }

    #[test]
    fn pane_process_of_an_empty_or_other_pane_id_is_none() {
        for reply in ["|\n", "|400625\n", "%4|400625\n", "%30|400625\n"] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok(reply.to_string()));
            assert_eq!(
                on_server(runner.clone()).pane_process("%3").unwrap(),
                None,
                "{reply:?}"
            );
            assert_eq!(runner.run_argvs().len(), 1);
        }
    }

    #[test]
    fn pane_process_with_a_matching_pane_and_a_non_positive_integer_pid_fails() {
        for reply in ["%3|abc\n", "%3|\n", "%3|0\n", "%3|-5\n", "%3|12x\n"] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok(reply.to_string()));
            let error = on_server(runner)
                .pane_process("%3")
                .unwrap_err()
                .to_string();
            assert!(
                error.starts_with("unexpected tmux display-message output: ")
                    && error.contains(reply.trim_end()),
                "{reply:?}: {error}"
            );
        }
    }

    #[test]
    fn pane_process_propagates_a_command_failure() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "no server running on /tmp/tmux-1000/default".to_string(),
        }));
        assert!(on_server(runner).pane_process("%3").is_err());
    }

    #[test]
    fn split_window_builds_the_detached_argv_and_swallows_layout_errors() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("%7\n".to_string()));
        runner.respond(Err(RunError::Failed {
            stderr: "layout boom".to_string(),
        }));
        let mux = on_server(runner.clone());
        let pane = mux
            .split_window(&reference(), &argv(&["claude", "--name", "worker"]))
            .unwrap();
        assert_eq!(pane, "%7", "the layout failure never breaks the spawn");
        assert_eq!(
            runner.run_argvs(),
            vec![
                tmux(&[
                    "split-window",
                    "-t",
                    "@1",
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "-d",
                    "claude",
                    "--name",
                    "worker",
                ]),
                tmux(&["select-layout", "-t", "@1", "main-vertical"]),
            ]
        );
    }

    #[test]
    fn capture_pane_rejects_non_positive_lines() {
        let runner = FakeRunner::with_binary("tmux");
        let mux = on_server(runner);
        assert_eq!(
            mux.capture_pane("%5", 0).unwrap_err().to_string(),
            "capture_pane: lines must be positive, got 0"
        );
    }

    #[test]
    fn capture_pane_keeps_the_last_lines_after_dropping_the_blank_tail() {
        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Ok("l1\nl2\nl3\nx\rY\n".to_string()));
        let mux = on_server(runner.clone());
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
                    "-S",
                    SOCKET,
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
        let mux = on_server(runner.clone());
        mux.kill_pane("%5", true).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![tmux(&["kill-pane", "-t", "%5"])],
            "the marker match is case-insensitive"
        );

        let runner = FakeRunner::with_binary("tmux");
        runner.respond(Err(RunError::Failed {
            stderr: "server exited unexpectedly".to_string(),
        }));
        let mux = on_server(runner);
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
            let mux = on_server(runner.clone());
            let error = mux
                .split_window(&reference(), &argv(&["claude"]))
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
            assert_eq!(calls[0][..4], tmux(&["split-window"]));
        }
    }
}
