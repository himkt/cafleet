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
use crate::multiplexer::ScreenCapture;
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

const SCREEN_METADATA: &str =
    "#{pane_height} #{pane_width} #{cursor_y} #{cursor_x} #{alternate_on}";
const SCREEN_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);
const LITERAL_CHUNK_CHARS: usize = 256;

/// Parses a [`SCREEN_METADATA`] line into rows, columns, cursor and screen mode.
fn screen_metadata(line: &str) -> Result<(u16, u16, (u16, u16), bool), MultiplexerError> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    let [rows, cols, cursor_y, cursor_x, alternate] = fields.as_slice() else {
        return Err(unexpected_output(line));
    };
    let number = |field: &str| field.parse::<u16>().map_err(|_| unexpected_output(line));
    let alternate = match *alternate {
        "0" => false,
        "1" => true,
        _ => return Err(unexpected_output(line)),
    };
    Ok((
        number(rows)?,
        number(cols)?,
        (number(cursor_y)?, number(cursor_x)?),
        alternate,
    ))
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

    fn rename_window(&self, window_id: &str, name: &str) -> Result<(), MultiplexerError> {
        self.core.run(
            &self
                .core
                .argv(&["tmux", "rename-window", "-t", window_id, name]),
        )?;
        Ok(())
    }

    /// tmux prints an empty line for a missing window, so the echoed window ID must match.
    fn window_name(&self, window_id: &str) -> Result<String, MultiplexerError> {
        let out = self.display(window_id, "#{window_id}|#{window_name}")?;
        let (echoed, name) = out
            .strip_suffix('\n')
            .ok_or_else(|| unexpected_output(&out))?
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

    /// Reads the metadata, the body, then the metadata again, and retries the whole capture
    /// while the two metadata lines differ.
    fn capture_screen(&self, pane: &str) -> Result<ScreenCapture, MultiplexerError> {
        let body_argv = self
            .core
            .argv(&["tmux", "capture-pane", "-p", "-e", "-N", "-t", pane]);
        loop {
            let before = self.display(pane, SCREEN_METADATA)?;
            let body = self.core.run(&body_argv)?;
            let after = self.display(pane, SCREEN_METADATA)?;
            if before == after {
                let (rows, cols, cursor, alternate) = screen_metadata(&before)?;
                return Ok(ScreenCapture {
                    rows,
                    cols,
                    cursor: Some(cursor),
                    alternate_screen: Some(alternate),
                    body,
                });
            }
            std::thread::sleep(SCREEN_RETRY_INTERVAL);
        }
    }

    /// A failure after the first chunk's process started may leave a partial literal.
    fn type_literal_until(
        &self,
        pane: &str,
        literal: &str,
        stopped: &dyn Fn() -> bool,
    ) -> Result<bool, MultiplexerError> {
        let characters = literal.chars().collect::<Vec<_>>();
        for (index, chunk) in characters.chunks(LITERAL_CHUNK_CHARS).enumerate() {
            if index > 0 && stopped() {
                return Ok(false);
            }
            let mut chunk = chunk.iter().collect::<String>();
            if chunk.ends_with(';') {
                // tmux parses a trailing semicolon as a command separator, even after `--`.
                chunk.insert(chunk.len() - 1, '\\');
            }
            let argv = self
                .core
                .argv(&["tmux", "send-keys", "-t", pane, "-l", "--", &chunk]);
            self.core
                .acknowledged_run(&argv, "literal")
                .map_err(|error| {
                    if index > 0 {
                        error.after_write()
                    } else {
                        error
                    }
                })?;
        }
        Ok(true)
    }

    fn press_submit(&self, pane: &str, submit: &[u8]) -> Result<(), MultiplexerError> {
        let mut argv = self.core.argv(&["tmux", "send-keys", "-t", pane, "-H"]);
        argv.extend(submit.iter().map(|byte| format!("{byte:02x}")));
        self.core.acknowledged_run(&argv, "submit key")
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
    fn pane_process_reads_the_pid_of_the_matching_pane_only() {
        for (name, reply, expected) in [
            ("matching pane", Ok("%3|400625\n"), Ok(Some(400625))),
            ("empty pane id and pid", Ok("|\n"), Ok(None)),
            ("empty pane id", Ok("|400625\n"), Ok(None)),
            ("other pane", Ok("%4|400625\n"), Ok(None)),
            ("pane id with the same prefix", Ok("%30|400625\n"), Ok(None)),
            ("non-integer pid", Ok("%3|abc\n"), Err("%3|abc")),
            ("empty pid", Ok("%3|\n"), Err("%3|")),
            ("zero pid", Ok("%3|0\n"), Err("%3|0")),
            ("negative pid", Ok("%3|-5\n"), Err("%3|-5")),
            ("pid with a suffix", Ok("%3|12x\n"), Err("%3|12x")),
            (
                "command failure",
                Err("no server running on /tmp/tmux-1000/default"),
                Err(""),
            ),
        ] {
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(reply.map(String::from).map_err(failed_error));
            let result = on_server(runner.clone()).pane_process("%3");
            match expected {
                Ok(pid) => assert_eq!(result.unwrap(), pid, "{name}"),
                Err(detail) => {
                    let error = result.unwrap_err().to_string();
                    assert!(error.contains(detail), "{name}: {error}");
                    if reply.is_ok() {
                        assert!(
                            error.starts_with("unexpected tmux display-message output: "),
                            "{name}: {error}"
                        );
                    }
                }
            }
            assert_eq!(
                runner.run_argvs(),
                vec![tmux(&[
                    "display-message",
                    "-p",
                    "-t",
                    "%3",
                    "#{pane_id}|#{pane_pid}",
                ])],
                "{name}"
            );
        }
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

    fn failed_error(stderr: &str) -> RunError {
        RunError::Failed {
            stderr: stderr.into(),
        }
    }

    fn failed(stderr: &str) -> Result<String, RunError> {
        Err(failed_error(stderr))
    }

    mod spawn {
        use crate::multiplexer::tmux::tests::*;

        struct SpawnCase {
            name: &'static str,
            target: PaneTarget,
            title: &'static str,
            replies: Vec<Result<String, RunError>>,
            spawned: (&'static str, &'static str),
            calls: Vec<Vec<String>>,
        }

        fn ok(reply: &str) -> Result<String, RunError> {
            Ok(reply.into())
        }

        #[test]
        fn spawn_member_pane_splits_names_and_equalizes_each_target() {
            let split_below = |target: &str| {
                with_command(&[
                    "split-window",
                    "-v",
                    "-d",
                    "-t",
                    target,
                    "-P",
                    "-F",
                    "#{pane_id}",
                ])
            };
            let cases = [
                SpawnCase {
                    name: "a column with members splits below its last pane",
                    target: column(ColumnSide::Right, &["%2", "%5"], &["%0"]),
                    title: "drafter",
                    replies: vec![
                        ok("%9\n"),
                        ok(""),
                        ok("%0|0\n%2|61\n%5|61\n%9|61\n"),
                        ok(""),
                    ],
                    spawned: ("%9", "@1"),
                    calls: vec![
                        split_below("%5"),
                        tmux(&["select-pane", "-t", "%9", "-T", "drafter"]),
                        tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                        tmux(&["select-layout", "-E", "-t", "%2"]),
                    ],
                },
                SpawnCase {
                    name: "an empty right column takes a full-height right half and needs no layout",
                    target: column(ColumnSide::Right, &[], &["%0"]),
                    title: "drafter",
                    replies: vec![ok("%9\n"), ok(""), ok("%0|0\n%9|61\n")],
                    spawned: ("%9", "@1"),
                    calls: vec![
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
                },
                SpawnCase {
                    name: "an empty left column takes a full-height left half before the other column",
                    target: column(ColumnSide::Left, &[], &["%3", "%4"]),
                    title: "reviewer",
                    replies: vec![ok("%9\n"), ok(""), ok("%9|0\n%3|61\n%4|61\n"), ok("")],
                    spawned: ("%9", "@1"),
                    calls: vec![
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
                    ],
                },
                SpawnCase {
                    name: "a new window opens detached and named in the session with the cwd",
                    target: new_window(),
                    title: "tester",
                    replies: vec![ok("@6|%12\n"), ok(""), ok("%12|0\n")],
                    spawned: ("%12", "@6"),
                    calls: vec![
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
                    ],
                },
                SpawnCase {
                    name: "equalization runs once per column of two or more panes",
                    target: column(ColumnSide::Right, &["%10", "%11"], &["%7", "%8"]),
                    title: "worker",
                    replies: vec![
                        ok("%9\n"),
                        ok(""),
                        ok("%7|0\n%8|0\n%9|61\n%10|61\n%11|61\n"),
                        ok(""),
                        ok(""),
                    ],
                    spawned: ("%9", "@1"),
                    calls: vec![
                        split_below("%11"),
                        tmux(&["select-pane", "-t", "%9", "-T", "worker"]),
                        tmux(&["list-panes", "-t", "@1", "-F", LIST_FORMAT]),
                        tmux(&["select-layout", "-E", "-t", "%7"]),
                        tmux(&["select-layout", "-E", "-t", "%9"]),
                    ],
                },
            ];
            for case in cases {
                let runner = FakeRunner::with_binary("tmux");
                for reply in case.replies {
                    runner.respond(reply);
                }
                let spawned = on_server(runner.clone())
                    .spawn_member_pane(&case.target, case.title, CWD, &command())
                    .unwrap();
                assert_eq!(
                    spawned,
                    SpawnedPane {
                        pane_id: case.spawned.0.into(),
                        window_id: case.spawned.1.into(),
                    },
                    "{}",
                    case.name
                );
                assert_eq!(runner.run_argvs(), case.calls, "{}", case.name);
            }
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
        fn a_failed_or_malformed_creation_reply_leaves_pane_cleanup_unknown() {
            let split = || column(ColumnSide::Right, &["%2"], &["%0"]);
            for (name, target, reply, message) in [
                (
                    "blank split reply",
                    split(),
                    ok("   \n"),
                    "pane ID unknown; pane cleanup unconfirmed",
                ),
                (
                    "failed split",
                    split(),
                    failed("split failed"),
                    "pane ID unknown; pane cleanup unconfirmed",
                ),
                (
                    "empty new-window reply",
                    new_window(),
                    ok("\n"),
                    "unexpected tmux new-window output: ",
                ),
                (
                    "new-window reply without a pane",
                    new_window(),
                    ok("@6\n"),
                    "unexpected tmux new-window output: \"@6",
                ),
                (
                    "new-window reply with an empty pane",
                    new_window(),
                    ok("@6|\n"),
                    "unexpected tmux new-window output: \"@6|",
                ),
                (
                    "new-window reply without a window",
                    new_window(),
                    ok("|%12\n"),
                    "unexpected tmux new-window output: \"|%12",
                ),
                (
                    "failed new-window",
                    new_window(),
                    failed("no server running"),
                    "",
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply);
                let error = on_server(runner.clone())
                    .spawn_member_pane(&target, "drafter", CWD, &command())
                    .unwrap_err();
                assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown), "{name}");
                let text = error.to_string();
                assert!(text.contains(message), "{name}: {text}");
                assert_eq!(runner.run_argvs().len(), 1, "{name}");
            }
        }
    }

    mod window_reads {
        use crate::multiplexer::tmux::tests::*;

        const NAME_FORMAT: &str = "#{window_id}|#{window_name}";

        #[test]
        fn rename_window_targets_the_recorded_socket_and_concrete_window() {
            for (name, reply, denied) in [
                ("rename succeeds", Ok(String::new()), false),
                ("rename fails", failed("rename denied"), true),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply);
                let result = on_server(runner.clone()).rename_window("@17", "cafleet-42-0");
                match result {
                    Ok(()) => assert!(!denied, "{name}"),
                    Err(error) => {
                        assert!(denied, "{name}");
                        assert!(
                            error.to_string().contains("rename denied"),
                            "{name}: {error}"
                        );
                    }
                }
                assert_eq!(
                    runner.run_argvs(),
                    vec![tmux(&["rename-window", "-t", "@17", "cafleet-42-0"])],
                    "{name}"
                );
            }
        }

        #[test]
        fn window_name_reads_the_named_window_and_keeps_every_separator_after_the_first() {
            for (name, reply, expected) in [
                ("plain name", Ok("@1|main\n"), Ok("main")),
                ("empty name", Ok("@1|\n"), Ok("")),
                (
                    "name with a separator",
                    Ok("@1|agents|left\n"),
                    Ok("agents|left"),
                ),
                (
                    "name with spaces",
                    Ok("@1| agents | left \n"),
                    Ok(" agents | left "),
                ),
                ("name ending in CR", Ok("@1|agents\r\n"), Ok("agents\r")),
                (
                    "name with a trailing blank line",
                    Ok("@1|agents\n\n"),
                    Ok("agents\n"),
                ),
                ("name of CR LF", Ok("@1|\r\n\n"), Ok("\r\n")),
                ("blank reply", Ok("\n"), Err(true)),
                ("empty reply", Ok(""), Err(true)),
                ("reply without a window", Ok("main\n"), Err(true)),
                ("other window", Ok("@2|main\n"), Err(true)),
                ("empty window id", Ok("|main\n"), Err(true)),
                ("command failure", Err("no server running"), Err(false)),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply.map(String::from).map_err(failed_error));
                let result = on_server(runner.clone()).window_name("@1");
                match expected {
                    Ok(expected) => assert_eq!(result.unwrap(), expected, "{name}"),
                    Err(output) => {
                        let error = result.unwrap_err().to_string();
                        assert_eq!(
                            error.starts_with("unexpected tmux display-message output: "),
                            output,
                            "{name}: {error}"
                        );
                    }
                }
                assert_eq!(
                    runner.run_argvs(),
                    vec![tmux(&["display-message", "-p", "-t", "@1", NAME_FORMAT])],
                    "{name}"
                );
            }
        }

        #[test]
        fn window_height_reads_the_window_by_id_and_ignores_the_anchors() {
            for (name, reply, expected) in [
                ("positive height", Ok("120\n"), Ok(120)),
                ("blank reply", Ok("\n"), Err(true)),
                ("empty reply", Ok(""), Err(true)),
                ("non-integer", Ok("abc\n"), Err(true)),
                ("zero", Ok("0\n"), Err(true)),
                ("negative", Ok("-3\n"), Err(true)),
                ("integer with a suffix", Ok("12x\n"), Err(true)),
                ("fraction", Ok("4.5\n"), Err(true)),
                ("command failure", Err("no server running"), Err(false)),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply.map(String::from).map_err(failed_error));
                let result = on_server(runner.clone()).window_height("@4", &argv(&["%5", "%6"]));
                match expected {
                    Ok(expected) => assert_eq!(result.unwrap(), expected, "{name}"),
                    Err(output) => {
                        let error = result.unwrap_err().to_string();
                        assert_eq!(
                            error.starts_with("unexpected tmux display-message output: "),
                            output,
                            "{name}: {error}"
                        );
                    }
                }
                assert_eq!(
                    runner.run_argvs(),
                    vec![tmux(&[
                        "display-message",
                        "-p",
                        "-t",
                        "@4",
                        "#{window_height}"
                    ])],
                    "{name}"
                );
            }
        }
    }

    mod close {
        use crate::multiplexer::tmux::tests::*;

        #[test]
        fn kill_pane_closes_then_equalizes_the_window_and_tolerates_only_a_gone_pane() {
            let kill = |pane: &str| tmux(&["kill-pane", "-t", pane]);
            let list = |window: &str| tmux(&["list-panes", "-t", window, "-F", LIST_FORMAT]);
            let layout = |pane: &str| tmux(&["select-layout", "-E", "-t", pane]);
            for (name, pane, window, replies, closes, calls) in [
                (
                    "close then equalize every column",
                    "%7",
                    "@1",
                    vec![
                        Ok(String::new()),
                        Ok("%0|0\n%2|61\n%3|61\n".into()),
                        Ok(String::new()),
                    ],
                    true,
                    vec![kill("%7"), list("@1"), layout("%2")],
                ),
                (
                    "a gone pane still restores the window",
                    "%5",
                    "@1",
                    vec![failed("No Such Pane: %5"), Ok("%0|0\n".into())],
                    true,
                    vec![kill("%5"), list("@1")],
                ),
                (
                    "another close failure is refused before restoration",
                    "%5",
                    "@1",
                    vec![failed("server exited unexpectedly")],
                    false,
                    vec![kill("%5")],
                ),
                (
                    "a failed list leaves the close successful",
                    "%7",
                    "@1",
                    vec![Ok(String::new()), failed("server exited unexpectedly")],
                    true,
                    vec![kill("%7"), list("@1")],
                ),
                (
                    "a failed layout leaves the close successful",
                    "%7",
                    "@1",
                    vec![
                        Ok(String::new()),
                        Ok("%0|0\n%2|61\n%3|61\n".into()),
                        failed("layout boom"),
                    ],
                    true,
                    vec![kill("%7"), list("@1"), layout("%2")],
                ),
                (
                    "the last pane of a window leaves nothing to restore",
                    "%12",
                    "@6",
                    vec![Ok(String::new()), failed("can't find window: @6")],
                    true,
                    vec![kill("%12"), list("@6")],
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                for reply in replies {
                    runner.respond(reply);
                }
                let result = on_server(runner.clone()).kill_pane(pane, window);
                assert_eq!(result.is_ok(), closes, "{name}: {result:?}");
                assert_eq!(runner.run_argvs(), calls, "{name}");
            }
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

    mod director_pane {
        use crate::multiplexer::tmux::tests::*;
        use crate::time::Deadline;
        use std::time::Duration;
        use std::time::Instant;

        const PANE: &str = "%5";
        const METADATA: &str =
            "#{pane_height} #{pane_width} #{cursor_y} #{cursor_x} #{alternate_on}";

        fn metadata_call() -> Vec<String> {
            tmux(&["display-message", "-p", "-t", PANE, METADATA])
        }

        fn body_call() -> Vec<String> {
            tmux(&["capture-pane", "-p", "-e", "-N", "-t", PANE])
        }

        fn literal_call(chunk: &str) -> Vec<String> {
            tmux(&["send-keys", "-t", PANE, "-l", "--", chunk])
        }

        fn ok(reply: &str) -> Result<String, RunError> {
            Ok(reply.into())
        }

        #[test]
        fn capture_screen_reads_metadata_then_the_body_then_the_same_metadata() {
            for (name, metadata, alternate) in [
                ("primary screen", "24 80 23 2 0\n", false),
                ("alternate screen", "24 80 23 2 1\n", true),
            ] {
                let body = "\x1b[1mtitle\x1b[0m\n› \n";
                let runner = FakeRunner::with_binary("tmux");
                for reply in [metadata, body, metadata] {
                    runner.respond(ok(reply));
                }
                let capture = on_server(runner.clone()).capture_screen(PANE).unwrap();
                assert_eq!((capture.rows, capture.cols), (24, 80), "{name}");
                assert_eq!(capture.cursor, Some((23, 2)), "{name}");
                assert_eq!(capture.alternate_screen, Some(alternate), "{name}");
                assert_eq!(capture.body, body, "{name}");
                assert_eq!(
                    runner.run_argvs(),
                    [metadata_call(), body_call(), metadata_call()],
                    "{name}"
                );
            }
        }

        #[test]
        fn capture_screen_retries_the_whole_capture_when_the_metadata_line_changes() {
            let runner = FakeRunner::with_binary("tmux");
            for reply in [
                "24 80 23 2 0\n",
                "stale body\n",
                "30 100 29 4 0\n",
                "30 100 29 4 0\n",
                "settled body\n",
                "30 100 29 4 0\n",
            ] {
                runner.respond(ok(reply));
            }
            let capture = on_server(runner.clone()).capture_screen(PANE).unwrap();
            assert_eq!((capture.rows, capture.cols), (30, 100));
            assert_eq!(capture.cursor, Some((29, 4)));
            assert_eq!(capture.body, "settled body\n");
            assert_eq!(
                runner.run_argvs(),
                [
                    metadata_call(),
                    body_call(),
                    metadata_call(),
                    metadata_call(),
                    body_call(),
                    metadata_call(),
                ]
            );
        }

        #[test]
        fn capture_screen_fails_when_the_geometry_never_settles_within_the_deadline() {
            let runner = FakeRunner::with_binary("tmux");
            for round in 0..200 {
                runner.respond(ok("24 80 23 2 0\n"));
                runner.respond(ok("body\n"));
                runner.respond(ok(&format!("24 80 23 {} 0\n", round % 7 + 3)));
            }
            let started = Instant::now();
            let deadline = Deadline::until(started + Duration::from_millis(300));
            let result = on_server(runner).bounded(deadline).capture_screen(PANE);
            assert!(result.is_err(), "unstable geometry is a capture failure");
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "the retries stop at the deadline: {:?}",
                started.elapsed()
            );
        }

        #[test]
        fn capture_screen_fails_on_a_malformed_metadata_line_or_a_failed_command() {
            for (name, metadata) in [
                ("non-numeric field", ok("24 80 x 2 0\n")),
                ("missing field", ok("24 80 23 2\n")),
                ("extra field", ok("24 80 23 2 0 9\n")),
                ("empty line", ok("\n")),
                ("missing pane", failed("can't find pane: %5")),
                ("server error", failed("server exited unexpectedly")),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(metadata.clone());
                runner.respond(ok("body\n"));
                runner.respond(metadata);
                assert!(
                    on_server(runner).capture_screen(PANE).is_err(),
                    "{name} is a capture failure"
                );
            }
        }

        #[test]
        fn type_literal_sends_literal_chunks_of_at_most_256_characters_in_order() {
            let literal = format!("{}{}", "あ".repeat(300), "-; tail");
            let characters = literal.chars().collect::<Vec<_>>();
            let chunks = characters
                .chunks(256)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>();
            assert_eq!(chunks.len(), 2);
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(chunks.len());
            on_server(runner.clone())
                .type_literal(PANE, &literal)
                .unwrap();
            assert_eq!(
                runner.run_argvs(),
                chunks
                    .iter()
                    .map(|chunk| literal_call(chunk))
                    .collect::<Vec<_>>()
            );
        }

        #[test]
        fn type_literal_sends_a_short_literal_in_one_process() {
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(1);
            on_server(runner.clone())
                .type_literal(PANE, "[cafleet msg 7 from 3 2026-11-23T10:00:00+00:00] hi")
                .unwrap();
            assert_eq!(
                runner.run_argvs(),
                [literal_call(
                    "[cafleet msg 7 from 3 2026-11-23T10:00:00+00:00] hi"
                )]
            );
        }

        #[test]
        fn type_literal_preserves_semicolons_at_the_end_of_tmux_arguments() {
            let first = "a".repeat(255);
            let literal = format!(r"{first};tail\;;");
            let runner = FakeRunner::with_binary("tmux");
            runner.succeed(2);
            on_server(runner.clone())
                .type_literal(PANE, &literal)
                .unwrap();
            assert_eq!(
                runner.run_argvs(),
                [
                    literal_call(&format!(r"{first}\;")),
                    literal_call(r"tail\;\;")
                ]
            );
        }

        #[test]
        fn press_submit_sends_the_submit_bytes_as_hex_keys() {
            for (submit, hex) in [
                (&b"\r"[..], &["0d"][..]),
                (&b"\x1b\r"[..], &["1b", "0d"][..]),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.succeed(1);
                on_server(runner.clone())
                    .press_submit(PANE, submit)
                    .unwrap();
                let mut expected = tmux(&["send-keys", "-t", PANE, "-H"]);
                expected.extend(argv(hex));
                assert_eq!(runner.run_argvs(), [expected], "{hex:?}");
            }
        }

        #[test]
        fn type_literal_failures_report_whether_a_process_may_have_written() {
            let long = "a".repeat(300);
            for (name, literal, replies, written, calls) in [
                (
                    "spawn failure before the first process",
                    "short",
                    vec![Err(RunError::NotStarted("spawn failed".into()))],
                    false,
                    1,
                ),
                (
                    "missing binary",
                    "short",
                    vec![Err(RunError::BinaryNotFound("tmux".into()))],
                    false,
                    1,
                ),
                (
                    "first process failed",
                    "short",
                    vec![failed("server exited unexpectedly")],
                    true,
                    1,
                ),
                (
                    "first process timed out",
                    "short",
                    vec![Err(RunError::Timeout)],
                    true,
                    1,
                ),
                (
                    "unreadable acknowledgment",
                    "short",
                    vec![ok("unexpected output")],
                    true,
                    1,
                ),
                (
                    "a later chunk not started",
                    long.as_str(),
                    vec![ok(""), Err(RunError::NotStarted("spawn failed".into()))],
                    true,
                    2,
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                for reply in replies {
                    runner.respond(reply);
                }
                let error = on_server(runner.clone())
                    .type_literal(PANE, literal)
                    .unwrap_err();
                assert_eq!(error.submission_started(), written, "{name}: {error}");
                assert_eq!(runner.run_argvs().len(), calls, "{name}");
            }
        }

        #[test]
        fn an_expired_deadline_writes_no_literal_and_no_submit_key() {
            let runner = FakeRunner::with_binary("tmux");
            let expired = Deadline::until(Instant::now() - Duration::from_millis(1));
            let mux = on_server(runner.clone()).bounded(expired);
            let literal = mux.type_literal(PANE, "short").unwrap_err();
            assert!(!literal.submission_started(), "{literal}");
            let submit = mux.press_submit(PANE, b"\r").unwrap_err();
            assert!(!submit.submission_started(), "{submit}");
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn press_submit_failures_report_whether_the_key_may_have_been_written() {
            for (name, reply, written) in [
                ("process failed", failed("server exited unexpectedly"), true),
                ("process timed out", Err(RunError::Timeout), true),
                ("unreadable acknowledgment", ok("unexpected output"), true),
                (
                    "process not started",
                    Err(RunError::NotStarted("spawn failed".into())),
                    false,
                ),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply);
                let error = on_server(runner).press_submit(PANE, b"\r").unwrap_err();
                assert_eq!(error.submission_started(), written, "{name}: {error}");
            }
        }
    }
}
