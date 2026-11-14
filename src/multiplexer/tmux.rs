use crate::multiplexer::BackendCore;
use crate::multiplexer::ColumnTarget;
use crate::multiplexer::CommandRunner;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MultiplexerContext;
use crate::multiplexer::MultiplexerError;
use crate::multiplexer::PLACEMENT_MISMATCH;
use crate::multiplexer::PaneCleanup;
use crate::multiplexer::PaneTarget;
use crate::multiplexer::RunError;
use crate::multiplexer::SpawnedPane;
use crate::multiplexer::argv;
use crate::multiplexer::layout::ColumnSide;
use std::cell::RefCell;
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
    closed_window: RefCell<Option<String>>,
}

impl TmuxMultiplexer {
    pub(crate) fn for_server(runner: Rc<dyn CommandRunner>, socket: &str) -> Self {
        TmuxMultiplexer {
            core: BackendCore {
                runner,
                name: "tmux",
                pane_gone: is_pane_gone,
                server_flag: "-S",
                server: socket.to_owned(),
            },
            closed_window: RefCell::new(None),
        }
    }

    /// `format` for `pane` as `display-message` prints it.
    fn display(&self, pane: &str, format: &str) -> Result<String, MultiplexerError> {
        self.core.run(
            &self
                .core
                .argv(&["tmux", "display-message", "-p", "-t", pane, format]),
        )
    }
}

fn unexpected_output(out: &str) -> MultiplexerError {
    MultiplexerError::new(format!("unexpected tmux display-message output: {out:?}"))
}

impl Multiplexer for TmuxMultiplexer {
    fn core(&self) -> &BackendCore {
        &self.core
    }

    fn bounded(&self, deadline: crate::time::Deadline) -> Box<dyn Multiplexer> {
        Box::new(Self {
            core: self.core.bounded(deadline),
            closed_window: RefCell::new(None),
        })
    }

    /// tmux prints `||` and exits 0 for a missing pane, so an empty field or another pane id
    /// means the server has no such pane.
    fn pane_context(&self, pane: &str) -> Result<Option<MultiplexerContext>, MultiplexerError> {
        let out = self.display(pane, "#{session_name}|#{window_id}|#{pane_id}")?;
        let [session, window_id, pane_id] = out
            .trim()
            .splitn(3, '|')
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| unexpected_output(&out))?;
        let found = !session.is_empty() && !window_id.is_empty() && pane_id == pane;
        Ok(found.then(|| MultiplexerContext {
            session: session.to_owned(),
            window_id: window_id.to_owned(),
            pane_id: pane_id.to_owned(),
        }))
    }

    /// tmux prints an empty line for a missing window, so the echoed window ID must match.
    fn window_name(&self, window_id: &str) -> Result<String, MultiplexerError> {
        let out = self.display(window_id, "#{window_id}|#{window_name}")?;
        let (echoed, name) = out
            .trim_end_matches(['\n', '\r'])
            .split_once('|')
            .ok_or_else(|| unexpected_output(&out))?;
        if echoed != window_id {
            return Err(unexpected_output(&out));
        }
        Ok(name.to_owned())
    }

    /// A reply naming another or no pane means the server has no such pane.
    fn pane_process(&self, pane: &str) -> Result<Option<i64>, MultiplexerError> {
        let out = self.display(pane, "#{pane_id}|#{pane_pid}")?;
        let (pane_id, pid) = out
            .trim()
            .split_once('|')
            .ok_or_else(|| unexpected_output(&out))?;
        if pane_id != pane {
            return Ok(None);
        }
        let pid = pid
            .parse::<i64>()
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| unexpected_output(&out))?;
        Ok(Some(pid))
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
                placement.concrete_window_id(),
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

    fn window_height(&self, window_id: &str, _anchors: &[String]) -> Result<i64, MultiplexerError> {
        let out = self.display(window_id, "#{window_height}")?;
        out.trim()
            .parse::<i64>()
            .ok()
            .filter(|height| *height > 0)
            .ok_or_else(|| unexpected_output(&out))
    }

    fn spawn_member_pane(
        &self,
        target: &PaneTarget,
        pane_name: &str,
        cwd: &str,
        command: &[String],
    ) -> Result<SpawnedPane, MultiplexerError> {
        let spawned = match target {
            PaneTarget::Column(column) => SpawnedPane {
                pane_id: self.split_into(column, command)?,
                window_id: column.window_id.clone(),
            },
            PaneTarget::NewWindow {
                session,
                window_name,
            } => self.new_window(session, window_name, cwd, command)?,
        };
        let _ = self.core.run(&self.core.argv(&[
            "tmux",
            "select-pane",
            "-t",
            &spawned.pane_id,
            "-T",
            pane_name,
        ]));
        let _ = self.equalize_window(&spawned.window_id);
        Ok(spawned)
    }

    /// Capture the last `lines` drawn lines of the pane buffer via the shared
    /// capture windowing ([`crate::multiplexer::capture_window`]).
    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError> {
        crate::multiplexer::check_capture_lines(lines)?;
        let fetch_depth = lines + crate::multiplexer::CAPTURE_OVER_FETCH_LINES;
        let raw = self.core.run(&self.core.argv(&[
            "tmux",
            "capture-pane",
            "-p",
            "-e",
            "-t",
            target_pane_id,
            "-S",
            &format!("-{fetch_depth}"),
        ]))?;
        Ok(crate::multiplexer::capture_window(&raw, lines))
    }

    fn close_pane(&self, target_pane_id: &str, window_id: &str) -> Result<(), MultiplexerError> {
        self.core.run_tolerating_missing(&self.core.argv(&[
            "tmux",
            "kill-pane",
            "-t",
            target_pane_id,
        ]))?;
        *self.closed_window.borrow_mut() = Some(window_id.to_owned());
        Ok(())
    }

    fn restore_after_close(&self) -> Result<(), MultiplexerError> {
        match self.closed_window.take() {
            Some(window_id) => self.equalize_window(&window_id),
            None => Ok(()),
        }
    }
}

impl TmuxMultiplexer {
    /// The new pane's ID from a split; any failure leaves no confirmed pane to clean up.
    fn split_into(
        &self,
        column: &ColumnTarget,
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        let mut args = match column.column_panes.last() {
            Some(bottom) => self
                .core
                .argv(&["tmux", "split-window", "-v", "-d", "-t", bottom]),
            None => {
                let beside = column
                    .other_panes
                    .first()
                    .expect("an empty column's window holds the opposite column");
                let mut parts = vec!["tmux", "split-window", "-h", "-f"];
                if column.side == ColumnSide::Left {
                    parts.push("-b");
                }
                parts.extend(["-l", "50%", "-d", "-t", beside]);
                self.core.argv(&parts)
            }
        };
        args.extend(argv(&["-P", "-F", "#{pane_id}"]));
        args.extend(command.iter().cloned());
        let pane_id = self
            .core
            .run(&args)
            .map_err(|error| error.with_pane_cleanup(PaneCleanup::Unknown))?
            .trim()
            .to_string();
        if pane_id.is_empty() {
            return Err(
                MultiplexerError::new("tmux split-window returned an empty pane ID")
                    .with_pane_cleanup(PaneCleanup::Unknown),
            );
        }
        Ok(pane_id)
    }

    fn new_window(
        &self,
        session: &str,
        window_name: &str,
        cwd: &str,
        command: &[String],
    ) -> Result<SpawnedPane, MultiplexerError> {
        let mut args = self.core.argv(&[
            "tmux",
            "new-window",
            "-d",
            "-t",
            &format!("{session}:"),
            "-n",
            window_name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}|#{pane_id}",
        ]);
        args.extend(command.iter().cloned());
        let out = self
            .core
            .run(&args)
            .map_err(|error| error.with_pane_cleanup(PaneCleanup::Unknown))?;
        match out.trim().split_once('|') {
            Some((window_id, pane_id)) if !window_id.is_empty() && !pane_id.is_empty() => {
                Ok(SpawnedPane {
                    pane_id: pane_id.to_owned(),
                    window_id: window_id.to_owned(),
                })
            }
            _ => Err(
                MultiplexerError::new(format!("unexpected tmux new-window output: {out:?}"))
                    .with_pane_cleanup(PaneCleanup::Unknown),
            ),
        }
    }

    /// Spreads each column (panes sharing `pane_left`) of `window_id` evenly; a window that no
    /// longer exists has nothing to restore.
    fn equalize_window(&self, window_id: &str) -> Result<(), MultiplexerError> {
        let argv = self.core.argv(&[
            "tmux",
            "list-panes",
            "-t",
            window_id,
            "-F",
            "#{pane_id}|#{pane_left}",
        ]);
        let listing = match self.core.runner.run(&argv) {
            Ok(listing) => listing,
            Err(RunError::Failed { ref stderr }) if stderr.contains("can't find window") => {
                return Ok(());
            }
            Err(error) => return Err(self.core.map_error(&argv, error)),
        };
        let mut columns: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in listing.lines().filter(|line| !line.trim().is_empty()) {
            let (pane_id, left) = line.split_once('|').ok_or_else(|| {
                MultiplexerError::new(format!("unexpected tmux list-panes output: {listing:?}"))
            })?;
            match columns
                .iter_mut()
                .find(|(column_left, _)| *column_left == left)
            {
                Some((_, panes)) => panes.push(pane_id),
                None => columns.push((left, vec![pane_id])),
            }
        }
        for (_, panes) in columns.iter().filter(|(_, panes)| panes.len() >= 2) {
            self.core.run(
                &self
                    .core
                    .argv(&["tmux", "select-layout", "-E", "-t", panes[0]]),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::multiplexer::ColumnTarget;
    use crate::multiplexer::Multiplexer;
    use crate::multiplexer::PaneCleanup;
    use crate::multiplexer::PaneTarget;
    use crate::multiplexer::RunError;
    use crate::multiplexer::SpawnedPane;
    use crate::multiplexer::TmuxMultiplexer;
    use crate::multiplexer::layout::ColumnSide;
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
            vec![run_event(&[
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
            ],)],
            "Capture over-fetches by the fixed 1000-line margin"
        );
    }

    const CWD: &str = "/work/project";
    const LIST_FORMAT: &str = "#{pane_id}|#{pane_left}";

    fn command() -> Vec<String> {
        argv(&[
            "/abs/cafleet",
            "__member-run",
            "--descriptor",
            "/run/d.json",
        ])
    }

    fn column(side: ColumnSide, column_panes: &[&str], other_panes: &[&str]) -> PaneTarget {
        PaneTarget::Column(ColumnTarget {
            window_id: "@1".into(),
            side,
            column_panes: argv(column_panes),
            other_panes: argv(other_panes),
        })
    }

    fn new_window() -> PaneTarget {
        PaneTarget::NewWindow {
            session: "main".into(),
            window_name: "cafleet-3-1".into(),
        }
    }

    fn with_command(parts: &[&str]) -> Vec<String> {
        let mut call = tmux(parts);
        call.extend(command());
        call
    }

    fn failed(stderr: &str) -> Result<String, RunError> {
        Err(RunError::Failed {
            stderr: stderr.into(),
        })
    }

    mod spawn {
        use crate::multiplexer::tmux::tests::*;

        #[test]
        fn a_column_with_members_splits_below_its_last_pane_then_names_and_equalizes() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("%9\n".into()));
            runner.succeed(1);
            runner.respond(Ok("%0|0\n%2|61\n%5|61\n%9|61\n".into()));
            runner.succeed(1);
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &["%2", "%5"], &["%0"]),
                    "drafter",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(
                spawned,
                SpawnedPane {
                    pane_id: "%9".into(),
                    window_id: "@1".into(),
                }
            );
            assert_eq!(
                runner.run_argvs(),
                vec![
                    with_command(&[
                        "split-window",
                        "-v",
                        "-d",
                        "-t",
                        "%5",
                        "-P",
                        "-F",
                        "#{pane_id}"
                    ]),
                    tmux(&["select-pane", "-t", "%9", "-T", "drafter"]),
                    tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                    tmux(&["select-layout", "-E", "-t", "%2"]),
                ]
            );
        }

        #[test]
        fn an_empty_right_column_takes_a_full_height_right_half_beside_the_other_column() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("%9\n".into()));
            runner.succeed(1);
            runner.respond(Ok("%0|0\n%9|61\n".into()));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &[], &["%0"]),
                    "drafter",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(spawned.pane_id, "%9");
            assert_eq!(
                runner.run_argvs(),
                vec![
                    with_command(&[
                        "split-window",
                        "-h",
                        "-f",
                        "-l",
                        "50%",
                        "-d",
                        "-t",
                        "%0",
                        "-P",
                        "-F",
                        "#{pane_id}",
                    ]),
                    tmux(&["select-pane", "-t", "%9", "-T", "drafter"]),
                    tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                ],
                "single-pane columns need no select-layout"
            );
        }

        #[test]
        fn an_empty_left_column_takes_a_full_height_left_half_before_the_other_column() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("%9\n".into()));
            runner.succeed(1);
            runner.respond(Ok("%9|0\n%3|61\n%4|61\n".into()));
            runner.succeed(1);
            on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Left, &[], &["%3", "%4"]),
                    "reviewer",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    with_command(&[
                        "split-window",
                        "-h",
                        "-f",
                        "-b",
                        "-l",
                        "50%",
                        "-d",
                        "-t",
                        "%3",
                        "-P",
                        "-F",
                        "#{pane_id}",
                    ]),
                    tmux(&["select-pane", "-t", "%9", "-T", "reviewer"]),
                    tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                    tmux(&["select-layout", "-E", "-t", "%3"]),
                ]
            );
        }

        #[test]
        fn a_new_window_opens_detached_named_in_the_session_with_the_cwd() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("@6|%12\n".into()));
            runner.succeed(1);
            runner.respond(Ok("%12|0\n".into()));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(&new_window(), "tester", CWD, &command())
                .unwrap();
            assert_eq!(
                spawned,
                SpawnedPane {
                    pane_id: "%12".into(),
                    window_id: "@6".into(),
                }
            );
            assert_eq!(
                runner.run_argvs(),
                vec![
                    with_command(&[
                        "new-window",
                        "-d",
                        "-t",
                        "main:",
                        "-n",
                        "cafleet-3-1",
                        "-c",
                        CWD,
                        "-P",
                        "-F",
                        "#{window_id}|#{pane_id}",
                    ]),
                    tmux(&["select-pane", "-t", "%12", "-T", "tester"]),
                    tmux(&["list-panes", "-t", "@6", "-F", LIST_FORMAT]),
                ]
            );
        }

        #[test]
        fn equalization_runs_once_per_column_of_two_or_more_panes() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("%9\n".into()));
            runner.succeed(1);
            runner.respond(Ok("%7|0\n%8|0\n%9|61\n%10|61\n%11|61\n".into()));
            runner.succeed(2);
            on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &["%10", "%11"], &["%7", "%8"]),
                    "worker",
                    CWD,
                    &command(),
                )
                .unwrap();
            let calls = runner.run_argvs();
            assert_eq!(
                calls[3..],
                [
                    tmux(&["select-layout", "-E", "-t", "%7"]),
                    tmux(&["select-layout", "-E", "-t", "%9"]),
                ]
            );
        }

        #[test]
        fn naming_and_equalization_failures_never_fail_the_spawn() {
            for (rename, list, layout) in [
                (
                    failed("rename boom"),
                    Ok("%0|0\n%2|61\n%9|61\n".to_string()),
                    failed("layout boom"),
                ),
                (Ok(String::new()), failed("list boom"), Ok(String::new())),
                (
                    Ok(String::new()),
                    Ok("garbage without separator\n".to_string()),
                    Ok(String::new()),
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok("%9\n".into()));
                runner.respond(rename);
                runner.respond(list);
                runner.respond(layout);
                let spawned = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Right, &["%2"], &["%0"]),
                        "drafter",
                        CWD,
                        &command(),
                    )
                    .unwrap();
                assert_eq!(spawned.pane_id, "%9");
                assert!(
                    runner.run_argvs().iter().all(|call| call[3] != "kill-pane"),
                    "a best-effort failure never closes the spawned pane"
                );
            }
        }

        #[test]
        fn a_failed_or_empty_split_reply_leaves_pane_cleanup_unknown() {
            for reply in [Ok("   \n".to_string()), failed("split failed")] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply);
                let error = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Right, &["%2"], &["%0"]),
                        "drafter",
                        CWD,
                        &command(),
                    )
                    .unwrap_err();
                assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown));
                assert!(
                    error
                        .to_string()
                        .contains("pane ID unknown; pane cleanup unconfirmed")
                );
                assert_eq!(runner.run_argvs().len(), 1);
            }
        }

        #[test]
        fn a_malformed_new_window_reply_leaves_pane_cleanup_unknown() {
            for reply in ["\n", "@6\n", "@6|\n", "|%12\n"] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok(reply.into()));
                let error = on_server(runner.clone())
                    .spawn_member_pane(&new_window(), "tester", CWD, &command())
                    .unwrap_err();
                assert_eq!(
                    error.pane_cleanup(),
                    Some(&PaneCleanup::Unknown),
                    "{reply:?}"
                );
                let message = error.to_string();
                assert!(
                    message.starts_with(&format!("unexpected tmux new-window output: {reply:?}"))
                        || message.starts_with(&format!(
                            "unexpected tmux new-window output: {:?}",
                            reply.trim()
                        )),
                    "{reply:?}: {message}"
                );
                assert_eq!(runner.run_argvs().len(), 1, "{reply:?}");
            }
        }

        #[test]
        fn a_failed_new_window_command_leaves_pane_cleanup_unknown() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("no server running"));
            let error = on_server(runner.clone())
                .spawn_member_pane(&new_window(), "tester", CWD, &command())
                .unwrap_err();
            assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown));
            assert_eq!(runner.run_argvs().len(), 1);
        }
    }

    mod window_reads {
        use crate::multiplexer::tmux::tests::*;

        const NAME_FORMAT: &str = "#{window_id}|#{window_name}";

        #[test]
        fn window_name_reads_the_named_window_and_keeps_every_separator_after_the_first() {
            for (reply, expected) in [
                ("@1|main\n", "main"),
                ("@1|\n", ""),
                ("@1|agents|left\n", "agents|left"),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok(reply.into()));
                assert_eq!(
                    on_server(runner.clone()).window_name("@1").unwrap(),
                    expected,
                    "{reply:?}"
                );
                assert_eq!(
                    runner.run_argvs(),
                    vec![tmux(&["display-message", "-p", "-t", "@1", NAME_FORMAT])]
                );
            }
        }

        #[test]
        fn window_name_rejects_an_empty_or_other_window_reply() {
            for reply in ["\n", "", "main\n", "@2|main\n", "|main\n"] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok(reply.into()));
                let error = on_server(runner).window_name("@1").unwrap_err().to_string();
                assert!(
                    error.starts_with("unexpected tmux display-message output: "),
                    "{reply:?}: {error}"
                );
            }
        }

        #[test]
        fn window_name_propagates_a_command_failure() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("no server running"));
            assert!(on_server(runner).window_name("@1").is_err());
        }

        #[test]
        fn window_height_reads_the_window_by_id_and_ignores_the_anchors() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Ok("120\n".into()));
            assert_eq!(
                on_server(runner.clone())
                    .window_height("@4", &argv(&["%5", "%6"]))
                    .unwrap(),
                120
            );
            assert_eq!(
                runner.run_argvs(),
                vec![tmux(&[
                    "display-message",
                    "-p",
                    "-t",
                    "@4",
                    "#{window_height}"
                ])]
            );
        }

        #[test]
        fn window_height_rejects_an_empty_or_non_positive_integer_reply() {
            for reply in ["\n", "", "abc\n", "0\n", "-3\n", "12x\n", "4.5\n"] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok(reply.into()));
                let error = on_server(runner)
                    .window_height("@4", &[])
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.starts_with("unexpected tmux display-message output: "),
                    "{reply:?}: {error}"
                );
            }
        }

        #[test]
        fn window_height_propagates_a_command_failure() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("no server running"));
            assert!(on_server(runner).window_height("@4", &[]).is_err());
        }
    }

    mod close {
        use crate::multiplexer::tmux::tests::*;

        #[test]
        fn compensation_kill_closes_then_equalizes_every_column_of_the_given_window() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(1);
            runner.respond(Ok("%0|0\n%2|61\n%3|61\n".into()));
            runner.succeed(1);
            on_server(runner.clone()).kill_pane("%7", "@1").unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    tmux(&["kill-pane", "-t", "%7"]),
                    tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                    tmux(&["select-layout", "-E", "-t", "%2"]),
                ]
            );
        }

        #[test]
        fn kill_pane_tolerates_only_pane_gone_close_failures() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("No Such Pane: %5"));
            runner.respond(Ok("%0|0\n".into()));
            on_server(runner.clone()).kill_pane("%5", "@1").unwrap();
            assert_eq!(runner.run_argvs()[0], tmux(&["kill-pane", "-t", "%5"]));

            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("server exited unexpectedly"));
            assert!(
                on_server(runner.clone()).kill_pane("%5", "@1").is_err(),
                "close never swallows a non-pane-gone failure"
            );
            assert_eq!(runner.run_argvs().len(), 1);
        }

        #[test]
        fn kill_pane_restoration_is_best_effort() {
            for (list, layout) in [
                (failed("server exited unexpectedly"), None),
                (
                    Ok("%0|0\n%2|61\n%3|61\n".to_string()),
                    Some(failed("layout boom")),
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.succeed(1);
                runner.respond(list);
                if let Some(layout) = layout {
                    runner.respond(layout);
                }
                on_server(runner).kill_pane("%7", "@1").unwrap();
            }
        }

        #[test]
        fn closing_the_last_pane_of_a_window_leaves_nothing_to_restore() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(1);
            runner.respond(failed("can't find window: @6"));
            on_server(runner.clone()).kill_pane("%12", "@6").unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    tmux(&["kill-pane", "-t", "%12"]),
                    tmux(&["list-panes", "-t", "@6", "-F", LIST_FORMAT]),
                ]
            );
        }

        #[test]
        fn close_pane_closes_only_and_restoration_equalizes_the_recorded_window_later() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(1);
            let mux = on_server(runner.clone());
            mux.close_pane("%7", "@4").unwrap();
            assert_eq!(runner.run_argvs(), vec![tmux(&["kill-pane", "-t", "%7"])]);
            runner.respond(Ok("%20|0\n%21|0\n%22|61\n".into()));
            runner.succeed(1);
            mux.restore_after_close().unwrap();
            assert_eq!(
                runner.run_argvs()[1..],
                [
                    tmux(&["list-panes", "-t", "@4", "-F", LIST_FORMAT]),
                    tmux(&["select-layout", "-E", "-t", "%20"]),
                ]
            );
        }

        #[test]
        fn restoring_a_closed_window_does_nothing_more() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(1);
            let mux = on_server(runner.clone());
            mux.close_pane("%12", "@6").unwrap();
            runner.respond(failed("can't find window: @6"));
            mux.restore_after_close().unwrap();
            assert_eq!(runner.run_argvs().len(), 2);
        }

        #[test]
        fn close_pane_tolerates_a_pane_gone_failure_and_refuses_others() {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("can't find pane: %7"));
            on_server(runner).close_pane("%7", "@4").unwrap();
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(failed("server exited unexpectedly"));
            assert!(on_server(runner).close_pane("%7", "@4").is_err());
        }
    }
}
